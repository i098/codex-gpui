//! `Hook stats` under a finished turn: the turn's `hook/started` and `hook/completed` runs
//! (`assistantMessage.hookStats.*` in the original's `hook-stats-dialog` chunk).

use bridge::types::{HookOutputEntry, HookRun};
use gpui::Action;

/// The finished hook runs of one turn; `None` from [`HookStats::new`] hides the button.
#[derive(Clone, Debug, PartialEq)]
pub struct HookStats {
    pub blocked: usize,
    pub failed: usize,
    /// Runs without `running` ones, keeping only `warning`, `feedback`, `error` and `stop` output.
    pub runs: Vec<HookRun>,
}

/// Opens the `Hook stats` dialog for one turn.
#[derive(Clone, PartialEq, Action)]
#[action(namespace = thread, no_json)]
pub struct OpenHookStats(pub HookStats);

impl HookStats {
    pub fn new(runs: &[HookRun]) -> Option<Self> {
        let runs: Vec<HookRun> = runs
            .iter()
            .filter(|run| run.status != "running")
            .map(|run| HookRun {
                status_message: run
                    .status_message
                    .as_deref()
                    .map(str::trim)
                    .filter(|m| !m.is_empty())
                    .map(str::to_owned),
                entries: run
                    .entries
                    .iter()
                    .filter(|e| {
                        matches!(e.kind.as_str(), "warning" | "feedback" | "error" | "stop")
                    })
                    .cloned()
                    .collect(),
                ..run.clone()
            })
            .collect();
        (!runs.is_empty()).then(|| Self {
            blocked: runs.iter().filter(|r| r.status == "blocked").count(),
            failed: runs.iter().filter(|r| r.status == "failed").count(),
            runs,
        })
    }

    /// The trigger icon turns to the warning color.
    pub fn warns(&self) -> bool {
        self.blocked > 0 || self.failed > 0
    }
}

/// Folds one `hook/started` or `hook/completed` run into a turn's runs: a run replaces the
/// running or same-completion run with its id, else it is a new run.
pub fn upsert(runs: &mut Vec<HookRun>, run: HookRun) {
    match runs.iter().rposition(|r| {
        r.id == run.id && (r.status == "running" || r.completed_at == run.completed_at)
    }) {
        Some(i) => runs[i] = run,
        None => runs.push(run),
    }
}

pub fn status_label(status: &str) -> &'static str {
    match status {
        "completed" => "Completed",
        "blocked" => "Blocked",
        "failed" => "Failed",
        "stopped" => "Stopped",
        _ => "Unknown",
    }
}

/// The original's `hooks-settings-model` source grouping, then its label.
pub fn source_label(source: Option<&str>) -> &'static str {
    match source {
        Some("plugin") => "Plugin",
        Some("user") => "User",
        Some(
            "admin"
            | "system"
            | "mdm"
            | "cloudRequirements"
            | "cloudManagedConfig"
            | "legacyManagedConfigFile",
        ) => "Admin",
        Some("project") => "Project",
        Some("sessionFlags") => "Session",
        _ => "Unknown",
    }
}

pub fn kind_label(entry: &HookOutputEntry) -> &'static str {
    match entry.kind.as_str() {
        "warning" => "Message",
        "feedback" => "Feedback",
        "error" => "Error",
        "stop" => "Stop reason",
        _ => "Output",
    }
}

/// Shown for a run that is not `completed` and gave no output.
pub fn no_output(run: &HookRun) -> Option<&'static str> {
    (run.entries.is_empty() && run.status != "completed").then(|| match run.status.as_str() {
        "blocked" => "The hook did not provide a reason",
        "failed" => "The hook did not provide error details",
        _ => "The hook did not provide a stop reason",
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn run(id: &str, status: &str, completed_at: Option<i64>) -> HookRun {
        serde_json::from_value(json!({
            "id": id, "eventName": "preToolUse", "source": "project", "status": status,
            "statusMessage": "  ", "completedAt": completed_at,
            "entries": [{"kind": "context", "text": "c"}, {"kind": "feedback", "text": "no"}],
        }))
        .unwrap()
    }

    #[test]
    fn stats_skip_running_runs_and_count_outcomes() {
        assert_eq!(HookStats::new(&[run("a", "running", None)]), None);
        let stats = HookStats::new(&[
            run("a", "blocked", Some(1)),
            run("b", "failed", Some(2)),
            run("c", "running", None),
            run("d", "completed", Some(3)),
        ])
        .unwrap();
        assert_eq!((stats.runs.len(), stats.blocked, stats.failed), (3, 1, 1));
        assert!(stats.warns());
        // `context` output and a blank status message are not shown.
        assert_eq!(stats.runs[0].entries.len(), 1);
        assert_eq!(stats.runs[0].status_message, None);
    }

    #[test]
    fn upsert_completes_the_running_run_and_keeps_reruns() {
        let mut runs = Vec::new();
        upsert(&mut runs, run("a", "running", None));
        upsert(&mut runs, run("a", "completed", Some(1)));
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].status, "completed");
        upsert(&mut runs, run("a", "blocked", Some(2)));
        assert_eq!(runs.len(), 2);
    }
}
