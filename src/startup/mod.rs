//! Things that start automatically: login items, launch agents and launch daemons.

pub mod launchctl;

use std::fs;
use std::path::{Path, PathBuf};

use objc2_app_kit::NSWorkspace;
use objc2_foundation::NSString;
use plist::Value;

use crate::util::{
    app_bundle_of, applescript_escape, current_uid, home_dir, run, run_applescript, run_privileged,
    shell_quote,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum StartupKind {
    LoginItem,
    UserAgent,
    GlobalAgent,
    SystemDaemon,
}

impl StartupKind {
    pub const ALL: [StartupKind; 4] = [
        StartupKind::LoginItem,
        StartupKind::UserAgent,
        StartupKind::GlobalAgent,
        StartupKind::SystemDaemon,
    ];

    pub fn title(self) -> &'static str {
        match self {
            StartupKind::LoginItem => "Login Items",
            StartupKind::UserAgent => "Your Launch Agents",
            StartupKind::GlobalAgent => "Launch Agents (all users)",
            StartupKind::SystemDaemon => "Launch Daemons (system)",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            StartupKind::LoginItem => "Apps that open when you log in.",
            StartupKind::UserAgent => {
                "Background helpers in ~/Library/LaunchAgents that run while you're logged in."
            }
            StartupKind::GlobalAgent => {
                "Helpers in /Library/LaunchAgents that run for every user. Turning one off only affects your account."
            }
            StartupKind::SystemDaemon => {
                "System-wide services in /Library/LaunchDaemons. Changing them asks for an administrator password."
            }
        }
    }
}

#[derive(Clone, Debug)]
pub struct StartupItem {
    pub kind: StartupKind,
    /// launchd label, or the login item's name.
    pub label: String,
    pub name: String,
    pub plist: Option<PathBuf>,
    /// Executable for launchd jobs, app path for login items.
    pub program: Option<String>,
    pub run_at_load: bool,
    pub keep_alive: bool,
    pub enabled: bool,
    pub loaded: bool,
    pub pid: Option<i32>,
}

impl StartupItem {
    pub fn key(&self) -> String {
        format!("{:?}/{}", self.kind, self.label)
    }
}

#[derive(Default)]
pub struct StartupScan {
    pub items: Vec<StartupItem>,
    pub login_items_error: Option<String>,
}

struct Job {
    label: String,
    program: Option<String>,
    run_at_load: bool,
    keep_alive: bool,
    disabled_in_plist: bool,
    associated_bundle: Option<String>,
}

/// Collects every startup item. Runs `launchctl` and `osascript`, so call it off the UI thread.
pub fn scan() -> StartupScan {
    let uid = current_uid();
    let user_jobs = run("launchctl", &["list"])
        .map(|out| launchctl::parse_list(&out))
        .unwrap_or_default();
    let user_overrides = run("launchctl", &["print-disabled", &format!("gui/{uid}")])
        .map(|out| launchctl::parse_print_disabled(&out))
        .unwrap_or_default();
    let system_overrides = run("launchctl", &["print-disabled", "system"])
        .map(|out| launchctl::parse_print_disabled(&out))
        .unwrap_or_default();

    let sources = [
        (
            StartupKind::UserAgent,
            home_dir().join("Library/LaunchAgents"),
        ),
        (
            StartupKind::GlobalAgent,
            PathBuf::from("/Library/LaunchAgents"),
        ),
        (
            StartupKind::SystemDaemon,
            PathBuf::from("/Library/LaunchDaemons"),
        ),
    ];
    let mut items = Vec::new();
    for (kind, dir) in sources {
        for plist in plist_files(&dir) {
            let Some(job) = read_job(&plist) else {
                continue;
            };
            let (loaded, pid) = if kind == StartupKind::SystemDaemon {
                match run("launchctl", &["print", &format!("system/{}", job.label)]) {
                    Ok(out) => (true, launchctl::parse_print_pid(&out)),
                    Err(_) => (false, None),
                }
            } else {
                match user_jobs.get(&job.label) {
                    Some(pid) => (true, *pid),
                    None => (false, None),
                }
            };
            let overrides = if kind == StartupKind::SystemDaemon {
                &system_overrides
            } else {
                &user_overrides
            };
            let enabled = !overrides
                .get(&job.label)
                .copied()
                .unwrap_or(job.disabled_in_plist);
            items.push(StartupItem {
                kind,
                name: display_name(&job),
                label: job.label,
                plist: Some(plist),
                program: job.program,
                run_at_load: job.run_at_load,
                keep_alive: job.keep_alive,
                enabled,
                loaded,
                pid,
            });
        }
    }

    let login_items_error = match run_applescript(LIST_LOGIN_ITEMS) {
        Ok(out) => {
            items.extend(parse_login_items(&out));
            None
        }
        Err(e) => Some(explain_automation_error(e)),
    };
    items.sort_by(|a, b| {
        a.kind
            .cmp(&b.kind)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    StartupScan {
        items,
        login_items_error,
    }
}

fn plist_files(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|ext| ext == "plist"))
        .collect()
}

fn read_job(path: &Path) -> Option<Job> {
    let value = Value::from_file(path).ok()?;
    let dict = value.as_dictionary()?;
    let label = dict
        .get("Label")
        .and_then(Value::as_string)
        .map(str::to_owned)
        .or_else(|| path.file_stem().map(|s| s.to_string_lossy().into_owned()))?;
    let program = dict
        .get("Program")
        .and_then(Value::as_string)
        .or_else(|| {
            dict.get("ProgramArguments")?
                .as_array()?
                .first()?
                .as_string()
        })
        .map(str::to_owned);
    let keep_alive = match dict.get("KeepAlive") {
        Some(Value::Boolean(on)) => *on,
        Some(Value::Dictionary(_)) => true,
        _ => false,
    };
    let associated_bundle = match dict.get("AssociatedBundleIdentifiers") {
        Some(Value::String(id)) => Some(id.clone()),
        Some(Value::Array(ids)) => ids.first().and_then(Value::as_string).map(str::to_owned),
        _ => None,
    };
    Some(Job {
        label,
        program,
        run_at_load: dict
            .get("RunAtLoad")
            .and_then(Value::as_boolean)
            .unwrap_or(false),
        keep_alive,
        disabled_in_plist: dict
            .get("Disabled")
            .and_then(Value::as_boolean)
            .unwrap_or(false),
        associated_bundle,
    })
}

fn display_name(job: &Job) -> String {
    let program = job.program.as_deref().map(Path::new);
    if let Some((name, _)) = program.and_then(app_bundle_of) {
        return name;
    }
    if let Some(name) = job
        .associated_bundle
        .as_deref()
        .and_then(app_name_for_bundle_id)
    {
        return name;
    }
    // Jobs that run `/bin/sh` or `/usr/bin/open` are better described by their label.
    let is_system_tool = program.is_some_and(|p| {
        ["/bin/", "/usr/bin/", "/sbin/", "/usr/sbin/"]
            .iter()
            .any(|dir| p.starts_with(dir))
    });
    match program.and_then(Path::file_name) {
        Some(name) if !is_system_tool => name.to_string_lossy().into_owned(),
        _ => job.label.clone(),
    }
}

fn app_name_for_bundle_id(bundle_id: &str) -> Option<String> {
    objc2::rc::autoreleasepool(|_| {
        let url = NSWorkspace::sharedWorkspace()
            .URLForApplicationWithBundleIdentifier(&NSString::from_str(bundle_id))?;
        let path = url.path()?.to_string();
        app_bundle_of(Path::new(&path)).map(|(name, _)| name)
    })
}

const LIST_LOGIN_ITEMS: &str = r#"set output to ""
tell application "System Events"
    repeat with itemRef in login items
        set output to output & (name of itemRef) & tab & (path of itemRef) & linefeed
    end repeat
end tell
return output"#;

fn parse_login_items(output: &str) -> Vec<StartupItem> {
    output
        .lines()
        .filter_map(|line| {
            let (name, path) = line.split_once('\t')?;
            let name = name.trim();
            let path = path.trim();
            (!name.is_empty()).then(|| StartupItem {
                kind: StartupKind::LoginItem,
                label: name.to_owned(),
                name: name.to_owned(),
                plist: None,
                program: (!path.is_empty()).then(|| path.to_owned()),
                run_at_load: true,
                keep_alive: false,
                enabled: true,
                loaded: true,
                pid: None,
            })
        })
        .collect()
}

fn explain_automation_error(error: String) -> String {
    if error.contains("-1743") || error.contains("Not authorized") {
        "MacCleaner isn't allowed to read login items. Allow it under System Settings › Privacy & Security › Automation › System Events.".to_owned()
    } else {
        error
    }
}

/// Turns a startup item on or off, now and for future logins.
pub fn set_enabled(item: &StartupItem, enable: bool) -> Result<(), String> {
    match item.kind {
        StartupKind::LoginItem => set_login_item(item, enable),
        StartupKind::UserAgent | StartupKind::GlobalAgent => set_agent(item, enable),
        StartupKind::SystemDaemon => set_daemon(item, enable),
    }
}

fn set_agent(item: &StartupItem, enable: bool) -> Result<(), String> {
    let domain = format!("gui/{}", current_uid());
    let target = format!("{domain}/{}", item.label);
    let verb = if enable { "enable" } else { "disable" };
    if run("launchctl", &[verb, &target]).is_err() {
        run_privileged(&format!("launchctl {verb} {}", shell_quote(&target)))?;
    }
    // Start or stop it right away too; failures here (already loaded, not running) are harmless.
    if enable && !item.loaded {
        if let Some(plist) = &item.plist {
            let _ = run(
                "launchctl",
                &["bootstrap", &domain, &plist.to_string_lossy()],
            );
        }
    } else if !enable && item.loaded {
        let _ = run("launchctl", &["bootout", &target]);
    }
    Ok(())
}

fn set_daemon(item: &StartupItem, enable: bool) -> Result<(), String> {
    let target = shell_quote(&format!("system/{}", item.label));
    let mut command;
    if enable {
        command = format!("launchctl enable {target}");
        if let (Some(plist), false) = (&item.plist, item.loaded) {
            let plist = shell_quote(&plist.to_string_lossy());
            command += &format!(" && (launchctl bootstrap system {plist} 2>/dev/null || true)");
        }
    } else {
        command = format!("launchctl disable {target}");
        if item.loaded {
            command += &format!(" && (launchctl bootout {target} 2>/dev/null || true)");
        }
    }
    run_privileged(&command).map(|_| ())
}

fn set_login_item(item: &StartupItem, enable: bool) -> Result<(), String> {
    let script = if enable {
        let path = item
            .program
            .as_deref()
            .ok_or("The app's location is unknown, add it again in System Settings.")?;
        format!(
            r#"tell application "System Events" to make login item at end with properties {{path:"{}", hidden:false}}"#,
            applescript_escape(path)
        )
    } else {
        format!(
            r#"tell application "System Events" to delete login item "{}""#,
            applescript_escape(&item.label)
        )
    };
    run_applescript(&script)
        .map(|_| ())
        .map_err(explain_automation_error)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_login_items() {
        let items = parse_login_items(
            "Spotify\t/Applications/Spotify.app\nRectangle\t/Applications/Rectangle.app\n\n",
        );
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].name, "Spotify");
        assert_eq!(
            items[1].program.as_deref(),
            Some("/Applications/Rectangle.app")
        );
    }

    #[test]
    fn reads_launch_agent_plist() {
        let dir = std::env::temp_dir().join(format!("maccleaner-plist-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("com.example.helper.plist");
        fs::write(
            &path,
            r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
  <key>Label</key><string>com.example.helper</string>
  <key>ProgramArguments</key><array><string>/Applications/Example.app/Contents/MacOS/helper</string><string>--daemon</string></array>
  <key>RunAtLoad</key><true/>
  <key>KeepAlive</key><dict><key>SuccessfulExit</key><false/></dict>
</dict></plist>"#,
        )
        .unwrap();
        let job = read_job(&path).unwrap();
        assert_eq!(job.label, "com.example.helper");
        assert!(job.run_at_load && job.keep_alive && !job.disabled_in_plist);
        assert_eq!(display_name(&job), "Example");
        fs::remove_dir_all(&dir).unwrap();
    }
}
