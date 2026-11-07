//! The cloud setup route's terminal over a scripted durable app-server: the `command/exec`
//! messages are the original's (`terminal-tab`, `createEnvironmentBackend`), not a recording.

mod common;

use anyhow::{Context as _, Result, ensure};
use app::{Route, terminal::CloudTarget};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use common::Fx;
use serde_json::{Value, json};
use std::io::{BufRead as _, BufReader, LineWriter, Write as _};
use std::sync::{Arc, Mutex};
use std::time::Duration;

const SHOTS: &str = "CGX_CLOUD_TERMINAL_SHOTS";

/// The durable app-server's side of the pipe: answers the environment requests, records the
/// shell's `command/exec` requests and input, and fails the next `fail` executions.
#[derive(Clone, Default)]
struct Durable {
    writer: Arc<Mutex<Option<LineWriter<std::io::PipeWriter>>>>,
    execs: Arc<Mutex<Vec<Value>>>,
    input: Arc<Mutex<String>>,
    fail: Arc<Mutex<usize>>,
}

impl Durable {
    fn send(&self, line: Value) {
        let mut writer = self.writer.lock().unwrap();
        writeln!(writer.as_mut().unwrap(), "{line}").unwrap();
    }

    fn prompt(&self, process: &Value) {
        self.send(json!({"method": "command/exec/outputDelta", "params": {
            "processId": process, "stream": "stdout",
            "deltaBase64": STANDARD.encode("root@cloud:/workspace/repo# "), "capReached": false,
        }}));
    }

    /// Answers the latest `command/exec` with `answer` (`result` or `error`).
    fn end(&self, answer: Value) {
        let id = self.execs.lock().unwrap().last().unwrap()["id"].clone();
        let mut line = answer;
        line["id"] = id;
        self.send(line);
    }

    fn start(&self) -> Arc<bridge::Bridge> {
        let (client_input, server_output) = std::io::pipe().unwrap();
        let (server_input, client_output) = std::io::pipe().unwrap();
        *self.writer.lock().unwrap() = Some(LineWriter::new(server_output));
        let (bridge, _) = bridge::Bridge::from_io(client_input, client_output, None);
        let server = self.clone();
        std::thread::spawn(move || {
            for line in BufReader::new(server_input).lines() {
                let request: Value = serde_json::from_str(&line.unwrap()).unwrap();
                let id = request["id"].clone();
                let params = &request["params"];
                match request["method"].as_str().unwrap() {
                    "environment/info" => {
                        server.send(json!({"id": id, "result": {"cwd": "/workspace/repo"}}))
                    }
                    "fs/getMetadata" => {
                        server.send(json!({"id": id, "result": {"isDirectory": true}}))
                    }
                    "command/exec" => {
                        server.execs.lock().unwrap().push(request.clone());
                        let mut fail = server.fail.lock().unwrap();
                        if *fail > 0 {
                            *fail -= 1;
                            server.send(json!({"id": id, "error": {"code": -32000, "message": "executor unavailable"}}));
                        } else {
                            server.prompt(&params["processId"]);
                        }
                    }
                    "command/exec/write" => {
                        let bytes = STANDARD
                            .decode(params["deltaBase64"].as_str().unwrap())
                            .unwrap();
                        server
                            .input
                            .lock()
                            .unwrap()
                            .push_str(&String::from_utf8(bytes).unwrap());
                        server.send(json!({"id": id, "result": {}}));
                    }
                    _ => server.send(json!({"id": id, "result": {}})),
                }
            }
        });
        Arc::new(bridge)
    }
}

fn status(fx: &Fx) -> Vec<&'static str> {
    fx.read(|root, cx| {
        root.terminal_panel()
            .map(|panel| panel.read(cx).cloud_status())
            .unwrap_or_default()
    })
}

fn wait_status(fx: &mut Fx, want: &[&str]) -> Result<()> {
    fx.wait(&format!("cloud terminal status {want:?}"), |fx| {
        status(fx) == want
    })
}

/// The `execs`-th shell is running: the server has its `command/exec` and the tab shows no status.
fn wait_running(fx: &mut Fx, durable: &Durable, execs: usize) -> Result<()> {
    fx.wait(&format!("cloud shell {execs} running"), |fx| {
        durable.execs.lock().unwrap().len() == execs && status(fx).is_empty()
    })
}

fn set_target(fx: &mut Fx, target: CloudTarget) -> Result<()> {
    let page = fx
        .read(|root, _| root.remote_page().cloned())
        .context("cloud setup page")?;
    fx.root.update(&mut fx.cx, |root, cx| {
        root.set_cloud_terminal(&page, target, cx)
    });
    Ok(())
}

fn click_status_action(fx: &mut Fx) -> Result<()> {
    let bounds = fx
        .cx
        .debug_bounds(fx.window, "terminal-status-action")?
        .context("no status action")?;
    fx.click((
        f32::from(bounds.origin.x + bounds.size.width / 2.),
        f32::from(bounds.origin.y + bounds.size.height / 2.),
    ))
}

#[test]
fn cloud_setup_terminal_runs_in_the_thread_environment() -> Result<()> {
    let mut fx = Fx::on_route(Route::CloudSetup {
        config_id: "asenvcfg_fixture".into(),
        repositories: Vec::new(),
    })?;
    fx.menu("View", "Open Terminal")?;
    ensure!(
        fx.read(|root, _| root.terminal_visible()),
        "no terminal on the cloud setup page"
    );
    wait_status(&mut fx, &["Waiting for environment…"])?;
    fx.capture_evidence(SHOTS, "1-waiting-for-environment")?;

    let durable = Durable::default();
    let bridge = durable.start();
    let mut target = CloudTarget {
        bridge: Some(bridge),
        ..Default::default()
    };
    target.apply("thread/environment/connected", "env-1");
    set_target(&mut fx, target.clone())?;
    wait_running(&mut fx, &durable, 1)?;
    let exec = durable.execs.lock().unwrap()[0]["params"].clone();
    ensure!(
        exec["environmentId"] == "env-1"
            && exec["cwd"] == "/workspace/repo"
            && exec["tty"] == true
            && exec["command"][0] == "sh",
        "unexpected command/exec: {exec}"
    );
    fx.type_text("pwd")?;
    fx.wait("input reaches the cloud shell", |_| {
        *durable.input.lock().unwrap() == "pwd"
    })?;
    fx.capture_evidence(SHOTS, "2-running")?;

    // The connection drops: three attempts, then `Couldn't reconnect` with Retry.
    *durable.fail.lock().unwrap() = 3;
    durable.end(json!({"error": {"code": -32000, "message": "connection lost"}}));
    wait_status(&mut fx, &["Reconnecting…"])?;
    fx.capture_evidence(SHOTS, "3-reconnecting")?;
    // The 1, 2 and 4 s reconnect delays run on the test clock.
    fx.wait("reconnect attempts to run out", |fx| {
        fx.advance(Duration::from_millis(250)).is_ok() && status(fx) == ["Couldn’t reconnect"]
    })?;
    ensure!(
        durable.execs.lock().unwrap().len() == 4,
        "not three reconnect attempts"
    );
    fx.capture_evidence(SHOTS, "4-could-not-reconnect")?;

    click_status_action(&mut fx)?;
    wait_running(&mut fx, &durable, 5)?;
    durable.end(json!({"result": {"exitCode": 0, "stdout": "", "stderr": ""}}));
    wait_status(&mut fx, &["Shell exited"])?;
    fx.capture_evidence(SHOTS, "5-shell-exited")?;

    click_status_action(&mut fx)?;
    wait_running(&mut fx, &durable, 6)?;
    target.apply("thread/environment/connected", "env-2");
    set_target(&mut fx, target)?;
    wait_status(&mut fx, &["Environment changed"])?;
    fx.capture_evidence(SHOTS, "6-environment-changed")?;
    // `New terminal` opens a tab in the thread's current environment.
    click_status_action(&mut fx)?;
    wait_running(&mut fx, &durable, 7)?;
    let execs = durable.execs.lock().unwrap();
    ensure!(execs.last().unwrap()["params"]["environmentId"] == "env-2");
    Ok(())
}
