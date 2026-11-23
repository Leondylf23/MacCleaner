//! Running processes, grouped by the app they belong to.

use std::cmp::Reverse;
use std::collections::HashMap;
use std::ffi::c_void;
use std::path::{Path, PathBuf};

use objc2_app_kit::{NSApplicationActivationPolicy, NSRunningApplication, NSWorkspace};
use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};

use crate::startup::launchctl;
use crate::util::{app_bundle_of, current_uid, run};

/// How a process presents itself, from most to least visible.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ProcKind {
    /// A regular app with a Dock icon.
    App,
    /// An app that lives in the menu bar or has no Dock icon.
    MenuBar,
    /// A daemon, agent, helper or command-line tool.
    Background,
}

impl ProcKind {
    pub fn label(self) -> &'static str {
        match self {
            ProcKind::App => "App",
            ProcKind::MenuBar => "Menu bar",
            ProcKind::Background => "Background",
        }
    }
}

#[derive(Clone, Debug)]
pub struct ProcInfo {
    pub pid: u32,
    pub name: String,
    pub exe: Option<PathBuf>,
    /// Physical footprint (what Activity Monitor shows as "Memory"), or resident size when the
    /// footprint isn't readable.
    pub memory: u64,
    pub cpu: f32,
    pub kind: ProcKind,
    /// Registered as an app with the window server, so it can be asked to quit politely.
    pub is_app: bool,
    /// Belongs to the system or another user; MacCleaner won't quit it.
    pub protected: bool,
    pub launchd_label: Option<String>,
}

#[derive(Clone, Debug)]
pub struct AppGroup {
    pub key: String,
    pub name: String,
    pub kind: ProcKind,
    pub protected: bool,
    pub memory: u64,
    pub cpu: f32,
    pub procs: Vec<ProcInfo>,
}

impl AppGroup {
    pub fn launchd_label(&self) -> Option<&str> {
        self.procs.iter().find_map(|p| p.launchd_label.as_deref())
    }
}

/// Processes that keep the session running. Quitting them logs you out or breaks the desktop.
const CRITICAL: &[&str] = &[
    "kernel_task",
    "launchd",
    "WindowServer",
    "loginwindow",
    "Dock",
    "Finder",
    "SystemUIServer",
    "ControlCenter",
    "NotificationCenter",
    "Spotlight",
    "coreaudiod",
    "cfprefsd",
    "distnoted",
    "UserEventAgent",
];

const SYSTEM_DIRS: &[&str] = &[
    "/System",
    "/usr",
    "/sbin",
    "/bin",
    "/private",
    "/Library/Apple",
    "/Library/Developer/CommandLineTools",
];

fn is_protected(owned_by_user: bool, name: &str, exe: Option<&Path>, kind: ProcKind) -> bool {
    if !owned_by_user || CRITICAL.contains(&name) {
        return true;
    }
    if kind == ProcKind::App {
        return false;
    }
    let Some(exe) = exe else { return true };
    if exe.starts_with("/usr/local") {
        return false;
    }
    SYSTEM_DIRS.iter().any(|dir| exe.starts_with(dir))
}

/// `rusage_info_v0` from <sys/resource.h>.
#[repr(C)]
#[derive(Default)]
#[allow(dead_code)] // mirrors the C layout; only the footprint is read
struct RusageInfoV0 {
    uuid: [u8; 16],
    user_time: u64,
    system_time: u64,
    pkg_idle_wkups: u64,
    interrupt_wkups: u64,
    pageins: u64,
    wired_size: u64,
    resident_size: u64,
    phys_footprint: u64,
    proc_start_abstime: u64,
    proc_exit_abstime: u64,
}

unsafe extern "C" {
    fn proc_pid_rusage(pid: i32, flavor: i32, buffer: *mut c_void) -> i32;
}

fn phys_footprint(pid: u32) -> Option<u64> {
    let mut info = RusageInfoV0::default();
    // SAFETY: flavor 0 (RUSAGE_INFO_V0) writes exactly one rusage_info_v0 into `info`.
    let result = unsafe { proc_pid_rusage(pid as i32, 0, &mut info as *mut _ as *mut c_void) };
    (result == 0 && info.phys_footprint > 0).then_some(info.phys_footprint)
}

struct RunningApp {
    kind: ProcKind,
    name: Option<String>,
}

fn running_apps() -> HashMap<u32, RunningApp> {
    objc2::rc::autoreleasepool(|_| {
        NSWorkspace::sharedWorkspace()
            .runningApplications()
            .iter()
            .filter_map(|app| {
                let pid = u32::try_from(app.processIdentifier()).ok()?;
                let kind = match app.activationPolicy() {
                    NSApplicationActivationPolicy::Regular => ProcKind::App,
                    NSApplicationActivationPolicy::Accessory => ProcKind::MenuBar,
                    _ => ProcKind::Background,
                };
                let name = app.localizedName().map(|n| n.to_string());
                Some((pid, RunningApp { kind, name }))
            })
            .collect()
    })
}

pub struct ProcessCollector {
    system: System,
    uid: u32,
    own_pid: u32,
}

impl ProcessCollector {
    pub fn new() -> Option<Self> {
        Some(Self {
            system: System::new().ok()?,
            uid: current_uid(),
            own_pid: std::process::id(),
        })
    }

    /// Refreshes every process and returns them grouped by app, largest memory user first.
    pub fn collect(&mut self) -> Vec<AppGroup> {
        self.system.refresh_processes_specifics(
            ProcessesToUpdate::All,
            true,
            ProcessRefreshKind::nothing()
                .with_memory()
                .with_cpu()
                .with_exe(UpdateKind::OnlyIfNotSet)
                .with_user(UpdateKind::OnlyIfNotSet),
        );
        let apps = running_apps();
        let launchd_labels: HashMap<i32, String> = run("launchctl", &["list"])
            .map(|out| launchctl::parse_list(&out))
            .unwrap_or_default()
            .into_iter()
            .filter_map(|(label, pid)| Some((pid?, label)))
            .collect();

        let mut groups: HashMap<String, AppGroup> = HashMap::new();
        for (pid, process) in self.system.processes() {
            let pid = pid.as_u32();
            let exe = process.exe().map(Path::to_path_buf);
            let app = apps.get(&pid);
            let kind = app.map_or(ProcKind::Background, |a| a.kind);
            let name = process.name().to_string_lossy().into_owned();
            let owned_by_user = process.user_id().is_some_and(|uid| **uid == self.uid);
            let protected =
                pid == self.own_pid || is_protected(owned_by_user, &name, exe.as_deref(), kind);

            // Helpers live inside their app's bundle, so the outermost .app ties them together.
            let (key, group_name) = match exe.as_deref().and_then(app_bundle_of) {
                Some((bundle_name, bundle)) => (bundle.display().to_string(), bundle_name),
                None => (
                    exe.as_ref()
                        .map_or_else(|| format!("pid:{pid}"), |p| p.display().to_string()),
                    name.clone(),
                ),
            };
            let info = ProcInfo {
                pid,
                name: app.and_then(|a| a.name.clone()).unwrap_or(name),
                memory: phys_footprint(pid).unwrap_or_else(|| process.memory()),
                cpu: process.cpu_usage(),
                exe,
                kind,
                is_app: app.is_some(),
                protected,
                launchd_label: launchd_labels.get(&(pid as i32)).cloned(),
            };
            let group = groups.entry(key.clone()).or_insert_with(|| AppGroup {
                key,
                name: group_name,
                kind,
                protected: true,
                memory: 0,
                cpu: 0.0,
                procs: Vec::new(),
            });
            group.kind = group.kind.min(info.kind);
            group.protected &= info.protected;
            group.memory += info.memory;
            group.cpu += info.cpu;
            group.procs.push(info);
        }

        let mut groups: Vec<AppGroup> = groups.into_values().collect();
        for group in &mut groups {
            group.procs.sort_by_key(|p| Reverse(p.memory));
        }
        groups.sort_by_key(|g| Reverse(g.memory));
        groups
    }
}

/// Quits the group's processes. A normal quit asks the app to quit, as the Dock's Quit does, and
/// lets its helpers exit with it; `force` kills every process immediately.
/// Returns how many processes were signalled.
pub fn quit(group: &AppGroup, force: bool) -> Result<usize, String> {
    let targets: Vec<&ProcInfo> = group.procs.iter().filter(|p| !p.protected).collect();
    if targets.is_empty() {
        return Err(format!("{} is protected and can't be quit", group.name));
    }
    let apps: Vec<&ProcInfo> = targets.iter().copied().filter(|p| p.is_app).collect();
    let targets = if !force && !apps.is_empty() {
        apps
    } else {
        targets
    };

    let mut signalled = 0;
    let mut errors = Vec::new();
    for process in targets {
        match signal(process, force) {
            Ok(()) => signalled += 1,
            Err(e) => errors.push(format!("{} (PID {}): {e}", process.name, process.pid)),
        }
    }
    if signalled == 0 {
        Err(errors.join("; "))
    } else {
        Ok(signalled)
    }
}

fn signal(process: &ProcInfo, force: bool) -> Result<(), String> {
    if process.is_app
        && let Some(app) =
            NSRunningApplication::runningApplicationWithProcessIdentifier(process.pid as i32)
    {
        let sent = if force {
            app.forceTerminate()
        } else {
            app.terminate()
        };
        if sent {
            return Ok(());
        }
    }
    let signal = if force { libc::SIGKILL } else { libc::SIGTERM };
    // SAFETY: kill has no memory-safety preconditions.
    if unsafe { libc::kill(process.pid as i32, signal) } == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error().to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn protects_system_and_foreign_processes() {
        let user_tool = Some(Path::new("/opt/homebrew/bin/node"));
        assert!(!is_protected(true, "node", user_tool, ProcKind::Background));
        assert!(is_protected(false, "node", user_tool, ProcKind::Background));
        assert!(is_protected(true, "Dock", None, ProcKind::App));
        assert!(is_protected(
            true,
            "cloudd",
            Some(Path::new(
                "/System/Library/PrivateFrameworks/CloudKitDaemon.framework/Support/cloudd"
            )),
            ProcKind::Background
        ));
        assert!(!is_protected(
            true,
            "Notes",
            Some(Path::new(
                "/System/Applications/Notes.app/Contents/MacOS/Notes"
            )),
            ProcKind::App
        ));
        assert!(!is_protected(
            true,
            "tool",
            Some(Path::new("/usr/local/bin/tool")),
            ProcKind::Background
        ));
    }

    #[test]
    fn groups_helpers_under_their_app() {
        let mut collector = ProcessCollector::new().unwrap();
        let groups = collector.collect();
        assert!(!groups.is_empty());
        let own = groups
            .iter()
            .flat_map(|g| &g.procs)
            .find(|p| p.pid == std::process::id())
            .expect("own process missing");
        assert!(own.protected);
        assert!(own.memory > 0);
    }
}
