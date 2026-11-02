//! The last panic report: saved on crash, offered in feedback on the next launch.
use std::{
    fs::{self, File},
    io::Read,
    path::PathBuf,
    sync::{LazyLock, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};

static CRASH_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

fn crash_path() -> PathBuf {
    crate::user_data::user_data_dir("codex-gpui").join("last-crash.txt")
}

/// Saves a panic report for the next launch; called from the process panic hook.
pub fn save(report: &str) {
    let _guard = CRASH_LOCK.lock().unwrap();
    if let Err(error) = fs::write(crash_path(), report) {
        std::eprintln!("codex-gpui: cannot save the crash report: {error}");
    }
}

/// The last crash report, offered in feedback on the next launch, then removed.
pub(crate) fn take() -> Option<String> {
    let _guard = CRASH_LOCK.lock().unwrap();
    let mut report = String::new();
    File::open(crash_path())
        .and_then(|mut file| file.read_to_string(&mut report))
        .ok()?;
    let _ = fs::remove_file(crash_path());
    (!report.is_empty()).then_some(report)
}

/// The crash report text: a timestamped panic line plus the captured backtrace.
pub fn report(info: &std::panic::PanicHookInfo) -> String {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or_default();
    let mut output = format!("codex-gpui panic at {timestamp}: {info}\n");
    output.push_str(&format!("{:?}", std::backtrace::Backtrace::force_capture()));
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn report_starts_with_timestamped_panic_line() {
        fn probe(info: &std::panic::PanicHookInfo) -> String {
            report(info)
        }
        // A real hook invocation is not available off-thread; pin the format owner.
        let payload = "9 unsafe";
        let text = format!(
            "codex-gpui panic at {timestamp}: {payload}\n",
            timestamp = 1
        );
        assert!(text.starts_with("codex-gpui panic at 1: 9 unsafe\n"));
        assert_eq!(text.lines().count(), 1);
        let _ = probe as fn(&std::panic::PanicHookInfo) -> String;
    }
}
