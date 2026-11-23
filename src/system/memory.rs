//! System memory statistics, broken down the way Activity Monitor does.

use std::ffi::{CString, c_void};
use std::mem::size_of;
use std::sync::OnceLock;

use crate::util::run_privileged;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Pressure {
    #[default]
    Unknown,
    Normal,
    Warning,
    Critical,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct MemoryStats {
    pub total: u64,
    /// Anonymous memory owned by apps, minus what they marked purgeable.
    pub app: u64,
    /// Memory the kernel can't page out.
    pub wired: u64,
    /// Memory held by the compressor.
    pub compressed: u64,
    /// File cache plus purgeable memory, which macOS drops whenever something needs room.
    pub cached: u64,
    pub swap_total: u64,
    pub swap_used: u64,
    pub pressure: Pressure,
}

impl MemoryStats {
    /// "Memory Used" in Activity Monitor.
    pub fn used(&self) -> u64 {
        self.app + self.wired + self.compressed
    }

    pub fn free(&self) -> u64 {
        self.total.saturating_sub(self.used() + self.cached)
    }
}

/// `vm_statistics64` from <mach/vm_statistics.h>.
#[repr(C)]
#[allow(dead_code)] // mirrors the C layout; only some fields are read
#[derive(Default)]
struct VmStatistics64 {
    free_count: u32,
    active_count: u32,
    inactive_count: u32,
    wire_count: u32,
    zero_fill_count: u64,
    reactivations: u64,
    pageins: u64,
    pageouts: u64,
    faults: u64,
    cow_faults: u64,
    lookups: u64,
    hits: u64,
    purges: u64,
    purgeable_count: u32,
    speculative_count: u32,
    decompressions: u64,
    compressions: u64,
    swapins: u64,
    swapouts: u64,
    compressor_page_count: u32,
    throttled_count: u32,
    external_page_count: u32,
    internal_page_count: u32,
    total_uncompressed_pages_in_compressor: u64,
}

/// `xsw_usage` from <sys/sysctl.h>.
#[repr(C)]
#[allow(dead_code)] // mirrors the C layout; only some fields are read
#[derive(Clone, Copy, Default)]
struct SwapUsage {
    total: u64,
    available: u64,
    used: u64,
    page_size: u32,
    encrypted: i32,
}

const HOST_VM_INFO64: i32 = 4;

unsafe extern "C" {
    fn mach_host_self() -> u32;
    fn host_statistics64(host: u32, flavor: i32, info: *mut i32, count: *mut u32) -> i32;
    fn host_page_size(host: u32, page_size: *mut usize) -> i32;
}

fn host() -> u32 {
    static HOST: OnceLock<u32> = OnceLock::new();
    // SAFETY: mach_host_self has no preconditions; the port is cached for the process lifetime.
    *HOST.get_or_init(|| unsafe { mach_host_self() })
}

fn page_size() -> u64 {
    static PAGE: OnceLock<u64> = OnceLock::new();
    *PAGE.get_or_init(|| {
        let mut size: usize = 0;
        // SAFETY: `size` is a valid out pointer.
        if unsafe { host_page_size(host(), &mut size) } == 0 && size > 0 {
            size as u64
        } else {
            16384
        }
    })
}

fn vm_statistics() -> Option<VmStatistics64> {
    let mut stats = VmStatistics64::default();
    let mut count = (size_of::<VmStatistics64>() / size_of::<i32>()) as u32;
    // SAFETY: `stats` is a correctly sized, writable vm_statistics64 and `count` holds its size
    // in integer_t units, as host_statistics64 requires.
    let result = unsafe {
        host_statistics64(
            host(),
            HOST_VM_INFO64,
            &mut stats as *mut _ as *mut i32,
            &mut count,
        )
    };
    (result == 0).then_some(stats)
}

fn sysctl<T: Copy + Default>(name: &str) -> Option<T> {
    let name = CString::new(name).ok()?;
    let mut value = T::default();
    let mut len = size_of::<T>();
    // SAFETY: `value` is writable for `len` bytes and sysctlbyname writes at most `len` bytes.
    let result = unsafe {
        libc::sysctlbyname(
            name.as_ptr(),
            &mut value as *mut T as *mut c_void,
            &mut len,
            std::ptr::null_mut(),
            0,
        )
    };
    (result == 0 && len == size_of::<T>()).then_some(value)
}

pub fn read_memory_stats() -> Option<MemoryStats> {
    let vm = vm_statistics()?;
    let page = page_size();
    let pages = |count: u32| count as u64 * page;
    let swap = sysctl::<SwapUsage>("vm.swapusage").unwrap_or_default();
    let pressure = match sysctl::<i32>("kern.memorystatus_vm_pressure_level") {
        Some(1) => Pressure::Normal,
        Some(2) => Pressure::Warning,
        Some(4) => Pressure::Critical,
        _ => Pressure::Unknown,
    };
    Some(MemoryStats {
        total: sysctl::<u64>("hw.memsize").unwrap_or(0),
        app: pages(vm.internal_page_count.saturating_sub(vm.purgeable_count)),
        wired: pages(vm.wire_count),
        compressed: pages(vm.compressor_page_count),
        cached: pages(vm.external_page_count + vm.purgeable_count),
        swap_total: swap.total,
        swap_used: swap.used,
        pressure,
    })
}

/// Flushes the file cache and purgeable memory with `/usr/sbin/purge`, which needs root.
/// Returns how much memory became free.
pub fn purge() -> Result<u64, String> {
    let before = read_memory_stats().map_or(0, |m| m.free());
    run_privileged("/usr/sbin/purge")?;
    std::thread::sleep(std::time::Duration::from_millis(800));
    let after = read_memory_stats().map_or(0, |m| m.free());
    Ok(after.saturating_sub(before))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn struct_matches_kernel_layout() {
        // HOST_VM_INFO64_COUNT is 38 integer_t values.
        assert_eq!(size_of::<VmStatistics64>(), 38 * 4);
    }

    #[test]
    fn reads_plausible_stats() {
        let stats = read_memory_stats().expect("host_statistics64 failed");
        assert!(stats.total > 1 << 30);
        assert!(stats.used() > 0 && stats.used() <= stats.total);
        assert!(stats.wired > 0);
        assert_ne!(stats.pressure, Pressure::Unknown);
    }
}
