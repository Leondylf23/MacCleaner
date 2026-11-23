use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

/// System Settings pane where the user grants Full Disk Access.
pub const FULL_DISK_ACCESS_URL: &str =
    "x-apple.systempreferences:com.apple.preference.security?Privacy_AllFiles";
/// System Settings pane listing login items and app-managed background items.
pub const LOGIN_ITEMS_URL: &str =
    "x-apple.systempreferences:com.apple.LoginItems-Settings.extension";

pub fn home_dir() -> &'static Path {
    static HOME: OnceLock<PathBuf> = OnceLock::new();
    HOME.get_or_init(|| dirs::home_dir().unwrap_or_else(|| PathBuf::from("/")))
}

pub fn current_uid() -> u32 {
    // SAFETY: getuid has no preconditions and cannot fail.
    unsafe { libc::getuid() }
}

/// Formats a disk size with decimal units, the way Finder does.
pub fn format_size(bytes: u64) -> String {
    format_with_base(bytes, 1000.0)
}

/// Formats a memory amount with binary units, the way Activity Monitor does.
pub fn format_memory(bytes: u64) -> String {
    format_with_base(bytes, 1024.0)
}

fn format_with_base(bytes: u64, base: f64) -> String {
    const UNITS: [&str; 6] = ["B", "KB", "MB", "GB", "TB", "PB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= base && unit < UNITS.len() - 1 {
        value /= base;
        unit += 1;
    }
    match unit {
        0 => format!("{bytes} B"),
        _ if value >= 100.0 => format!("{value:.0} {}", UNITS[unit]),
        _ if value >= 10.0 => format!("{value:.1} {}", UNITS[unit]),
        _ => format!("{value:.2} {}", UNITS[unit]),
    }
}

/// Shortens paths inside the home folder to `~/...` for display.
pub fn display_path(path: &Path) -> String {
    match path.strip_prefix(home_dir()) {
        Ok(rest) if rest.as_os_str().is_empty() => "~".to_owned(),
        Ok(rest) => format!("~/{}", rest.display()),
        Err(_) => path.display().to_string(),
    }
}

/// Name of the outermost `.app` bundle containing `path`, plus the bundle's path.
pub fn app_bundle_of(path: &Path) -> Option<(String, PathBuf)> {
    let mut bundle = PathBuf::new();
    for component in path.components() {
        bundle.push(component);
        let name = component.as_os_str().to_string_lossy();
        if let Some(stem) = name.strip_suffix(".app") {
            return Some((stem.to_owned(), bundle));
        }
    }
    None
}

pub fn reveal_in_finder(path: &Path) {
    let _ = Command::new("open").arg("-R").arg(path).spawn();
}

pub fn open_url(url: &str) {
    let _ = Command::new("open").arg(url).spawn();
}

/// Runs a command and returns its stdout, or its stderr as the error.
pub fn run(program: &str, args: &[&str]) -> Result<String, String> {
    let output = Command::new(program)
        .args(args)
        .output()
        .map_err(|e| format!("failed to run {program}: {e}"))?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        Err(if stderr.is_empty() {
            format!("{program} exited with {}", output.status)
        } else {
            stderr
        })
    }
}

pub fn run_applescript(script: &str) -> Result<String, String> {
    run("osascript", &["-e", script])
}

/// Runs a shell command as root, showing the standard macOS administrator password prompt.
pub fn run_privileged(shell_command: &str) -> Result<String, String> {
    let script = format!(
        "do shell script \"{}\" with administrator privileges",
        applescript_escape(shell_command)
    );
    run_applescript(&script).map_err(|e| {
        if e.contains("-128") {
            "Cancelled by user".to_owned()
        } else {
            e
        }
    })
}

/// Escapes text for use inside an AppleScript string literal.
pub fn applescript_escape(text: &str) -> String {
    text.replace('\\', "\\\\").replace('"', "\\\"")
}

/// Quotes text for safe use as a single POSIX shell word.
pub fn shell_quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', "'\\''"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_sizes() {
        assert_eq!(format_size(0), "0 B");
        assert_eq!(format_size(999), "999 B");
        assert_eq!(format_size(1_500), "1.50 KB");
        assert_eq!(format_size(12_340_000), "12.3 MB");
        assert_eq!(format_size(456_000_000_000), "456 GB");
        assert_eq!(format_memory(16 * 1024 * 1024 * 1024), "16.0 GB");
        assert_eq!(format_memory(512 * 1024), "512 KB");
    }

    #[test]
    fn finds_outermost_app_bundle() {
        let helper = Path::new(
            "/Applications/Google Chrome.app/Contents/Frameworks/Helpers/Google Chrome Helper.app/Contents/MacOS/Google Chrome Helper",
        );
        let (name, bundle) = app_bundle_of(helper).unwrap();
        assert_eq!(name, "Google Chrome");
        assert_eq!(bundle, Path::new("/Applications/Google Chrome.app"));
        assert!(app_bundle_of(Path::new("/usr/bin/ssh")).is_none());
    }

    #[test]
    fn escapes_for_shell_and_applescript() {
        assert_eq!(shell_quote("it's"), "'it'\\''s'");
        assert_eq!(applescript_escape(r#"say "hi" \o/"#), r#"say \"hi\" \\o/"#);
    }
}
