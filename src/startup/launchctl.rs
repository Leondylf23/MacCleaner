//! Parsers for `launchctl` output.

use std::collections::HashMap;

/// `launchctl list` output: label -> PID of the running job.
pub fn parse_list(output: &str) -> HashMap<String, Option<i32>> {
    output
        .lines()
        .skip(1)
        .filter_map(|line| {
            let mut columns = line.split('\t');
            let pid = columns.next()?.trim().parse().ok();
            let _status = columns.next()?;
            let label = columns.next()?.trim();
            (!label.is_empty()).then(|| (label.to_owned(), pid))
        })
        .collect()
}

/// `launchctl print-disabled <domain>` output: label -> disabled.
pub fn parse_print_disabled(output: &str) -> HashMap<String, bool> {
    output
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            let rest = line.strip_prefix('"')?;
            let (label, value) = rest.split_once('"')?;
            let value = value.trim().strip_prefix("=>")?.trim();
            Some((label.to_owned(), matches!(value, "disabled" | "true")))
        })
        .collect()
}

/// `launchctl print system/<label>` output: PID if the service is running.
pub fn parse_print_pid(output: &str) -> Option<i32> {
    output
        .lines()
        .filter(|line| line.starts_with('\t') && !line.starts_with("\t\t"))
        .find_map(|line| line.trim().strip_prefix("pid = ")?.trim().parse().ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_list() {
        let output = "PID\tStatus\tLabel\n-\t0\tcom.apple.SafariHistoryServiceAgent\n11369\t-9\tcom.apple.progressd\n";
        let jobs = parse_list(output);
        assert_eq!(jobs.len(), 2);
        assert_eq!(jobs["com.apple.SafariHistoryServiceAgent"], None);
        assert_eq!(jobs["com.apple.progressd"], Some(11369));
    }

    #[test]
    fn parses_print_disabled_in_both_formats() {
        let output = "\n\tdisabled services = {\n\t\t\"com.microsoft.teams2.agent\" => enabled\n\t\t\"com.apple.Siri.agent\" => disabled\n\t\t\"com.old.style\" => true\n\t\t\"com.old.enabled\" => false\n\t}\n";
        let disabled = parse_print_disabled(output);
        assert_eq!(disabled.len(), 4);
        assert!(!disabled["com.microsoft.teams2.agent"]);
        assert!(disabled["com.apple.Siri.agent"]);
        assert!(disabled["com.old.style"]);
        assert!(!disabled["com.old.enabled"]);
    }

    #[test]
    fn parses_running_pid_from_print() {
        let running = "system/com.example = {\n\tactive count = 1\n\tstate = running\n\tpid = 812\n\tendpoints = {\n\t\tpid = 1\n\t}\n}\n";
        assert_eq!(parse_print_pid(running), Some(812));
        let stopped = "system/com.example = {\n\tstate = not running\n}\n";
        assert_eq!(parse_print_pid(stopped), None);
    }
}
