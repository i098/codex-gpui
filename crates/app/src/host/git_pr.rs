use super::device_id;
use anyhow::{Context as _, Result, ensure};
use bridge::{Bridge, Event};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{path::Path, sync::Arc, time::Duration};

/// Buffered process output. A set capture-limit flag means the corresponding stream is incomplete.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Output {
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
    pub stdout_cap_reached: bool,
    pub stderr_cap_reached: bool,
}

/// Executes trusted UI commands through the app-server, without a Codex sandbox.
/// Never pass model-provided commands. Spawn and connection errors remain errors.
/// Callers must check the exit code and capture-limit flags before using the output.
pub fn execute(
    bridge: &Arc<Bridge>,
    cwd: &Path,
    command: &[String],
    timeout_ms: u64,
) -> Result<Output> {
    ensure!(cwd.is_absolute(), "process cwd must be absolute");
    ensure!(!command.is_empty(), "process command is empty");
    let events = bridge.subscribe();
    let handle = format!("git-pr:{}", device_id());
    pollster::block_on(bridge.request::<_, Value>(
        "process/spawn",
        json!({
            "processHandle": handle,
            "command": command,
            "cwd": cwd,
            "env": {"GIT_TERMINAL_PROMPT": "0", "GH_PROMPT_DISABLED": "1"},
            "streamStdin": false,
            "streamStdoutStderr": false,
            "timeoutMs": timeout_ms,
        }),
    ))?;
    let deadline =
        std::time::Instant::now() + Duration::from_millis(timeout_ms) + Duration::from_secs(5);
    loop {
        let event =
            events.recv_timeout(deadline.saturating_duration_since(std::time::Instant::now()));
        match event {
            Ok(Event::Notification { method, params })
                if method == "process/exited" && params["processHandle"] == handle =>
            {
                return serde_json::from_value(params).context("invalid process exit response");
            }
            Ok(_) => {}
            Err(error) => {
                let _ = pollster::block_on(
                    bridge.request::<_, Value>("process/kill", json!({"processHandle": handle})),
                );
                return Err(error).context("App-server closed or process command timed out");
            }
        }
    }
}
