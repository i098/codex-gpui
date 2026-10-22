use super::device_id;
use anyhow::{Context as _, Result, ensure};
use bridge::{Bridge, Event};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    future::Future,
    path::Path,
    pin::pin,
    sync::Arc,
    task::{Context as TaskContext, Poll, Wake, Waker},
    thread::{self, Thread},
    time::{Duration, Instant},
};

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
    let deadline = Instant::now() + Duration::from_millis(timeout_ms);
    let events = bridge.subscribe();
    let handle = format!("git-pr:{}", device_id());
    let spawned = wait_until(
        bridge.request::<_, Value>(
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
        ),
        deadline,
    );
    match spawned {
        Some(result) => {
            result?;
        }
        None => {
            cancel(bridge, &handle);
            anyhow::bail!("Timed out waiting for app-server process start");
        }
    }
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            cancel(bridge, &handle);
            anyhow::bail!("Timed out waiting for app-server process exit");
        }
        let event = events.recv_timeout(remaining);
        match event {
            Ok(Event::Notification { method, params })
                if method == "process/exited" && params["processHandle"] == handle =>
            {
                return serde_json::from_value(params).context("invalid process exit response");
            }
            Ok(_) => {}
            Err(error) => {
                cancel(bridge, &handle);
                return Err(error).context("App-server closed or process command timed out");
            }
        }
    }
}

struct NotifyThread(Thread);

impl Wake for NotifyThread {
    fn wake(self: Arc<Self>) {
        self.0.unpark();
    }

    fn wake_by_ref(self: &Arc<Self>) {
        self.0.unpark();
    }
}

fn wait_until<F: Future>(future: F, deadline: Instant) -> Option<F::Output> {
    let waker = Waker::from(Arc::new(NotifyThread(thread::current())));
    let mut context = TaskContext::from_waker(&waker);
    let mut future = pin!(future);
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return None;
        }
        if let Poll::Ready(value) = future.as_mut().poll(&mut context) {
            return Some(value);
        }
        thread::park_timeout(deadline.saturating_duration_since(Instant::now()));
    }
}

fn cancel(bridge: &Bridge, handle: &str) {
    let request = bridge.request::<_, Value>("process/kill", json!({"processHandle": handle}));
    let mut request = pin!(request);
    let _ = request
        .as_mut()
        .poll(&mut TaskContext::from_waker(Waker::noop()));
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::process::Command;

    #[test]
    fn deadline_bounds_spawn_and_cleanup_replies() -> anyhow::Result<()> {
        const SERVER: &str = r#"
import json, sys, time
mode = sys.argv[1]
for line in sys.stdin:
    message = json.loads(line)
    method = message.get("method")
    if method == "initialize":
        result = {"userAgent":"deadline-fixture","codexHome":"/tmp","platformOs":"linux"}
    elif method == "process/spawn":
        if mode == "spawn":
            time.sleep(6)
        result = {}
    elif method == "process/kill":
        time.sleep(6)
        result = {}
    else:
        continue
    print(json.dumps({"id":message["id"],"result":result}), flush=True)
"#;
        for mode in ["spawn", "cleanup"] {
            let mut command = Command::new("python3");
            command.args(["-u", "-c", SERVER, mode]);
            let (bridge, _) = Bridge::spawn(command)?;
            let bridge = Arc::new(bridge);
            pollster::block_on(bridge.initialize("codex_gpui", "0.1.0"))?;
            let start = Instant::now();
            let result = execute(&bridge, Path::new("/tmp"), &["unused".into()], 100);
            let error = result.expect_err("a server without an exit event must time out");
            assert!(
                error.to_string().to_lowercase().contains("timed out"),
                "{error:#}"
            );
            assert!(
                start.elapsed() < Duration::from_secs(3),
                "{mode} exceeded the deadline"
            );
        }
        Ok(())
    }
}
