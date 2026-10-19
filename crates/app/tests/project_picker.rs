#[cfg(target_os = "macos")]
mod common;

use anyhow::Result;
use app::host::{Host, LocalHost};
use serde_json::{Value, json};
use std::fs;

#[test]
fn adding_a_folder_preserves_projects_and_unrelated_host_state() -> Result<()> {
    let profile = tempfile::tempdir()?;
    let alpha = profile.path().join("alpha");
    let beta = profile.path().join("beta");
    fs::create_dir(&alpha)?;
    fs::create_dir(&beta)?;
    let file = profile.path().join(".codex-global-state.json");
    fs::write(
        &file,
        serde_json::to_vec(&json!({
            "unrelated": {"keep": true},
            "local-projects": {"already": {
                "id": "already", "name": "Saved alpha", "rootPaths": [alpha.join(".")],
                "createdAt": 1, "updatedAt": 2,
            }},
            "project-order": ["already"],
        }))?,
    )?;
    let host = LocalHost::new(profile.path());
    assert_eq!(
        host.add_project(&alpha)?,
        alpha.canonicalize()?.to_str().unwrap()
    );
    let first = host.projects()?;
    host.add_project(&beta)?;
    host.add_project(&alpha.join("."))?;
    let rows = host.projects()?;
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0], first[0]);
    assert_eq!(rows[1].label.as_ref(), "beta");
    let state: Value = serde_json::from_slice(&fs::read(&file)?)?;
    assert_eq!(state["unrelated"], json!({"keep": true}));
    assert_eq!(
        state["local-projects"]["already"]["rootPaths"],
        json!([alpha.join(".")])
    );
    let saved = fs::read(&file)?;
    assert!(host.add_project(&profile.path().join("missing")).is_err());
    assert!(host.add_project(&file).is_err());
    assert_eq!(fs::read(file)?, saved);
    Ok(())
}

#[cfg(target_os = "macos")]
#[test]
fn proxy() {
    common::proxy_child();
}

/// A `Root` on a real proxy child that replays `lines` and logs every client request, with
/// the Home sidebar at 1280x832. Returns the window, the replay file and the request log.
#[cfg(target_os = "macos")]
fn isolated_root(
    dir: &std::path::Path,
    lines: &[Value],
    state_path: &std::path::Path,
) -> Result<(common::Fx, std::path::PathBuf, std::path::PathBuf)> {
    use app::{Root, State, ThemeSetting};
    use bridge::Bridge;
    use common::{CWD, Fx};
    use std::process::Command;

    let recording = dir.join("replay.jsonl");
    fs::write(
        &recording,
        lines
            .iter()
            .map(|line| format!("{line}\n"))
            .collect::<String>(),
    )?;
    let log = dir.join("requests.jsonl");
    fs::write(&log, "")?;
    let mut command = Command::new(std::env::current_exe()?);
    command
        .args(["--exact", "proxy", "--nocapture", "--quiet"])
        .env("CGX_PROXY_SCRIPT", &recording)
        .env("CGX_PROXY_LOG", &log);
    let (bridge, events) = Bridge::spawn(command)?;
    let mut fx = Fx::new()?;
    fx.root = fx.cx.update_window(fx.window, |_, window, cx| {
        window.replace_root(cx, |window, cx| {
            Root::new(
                State {
                    theme: ThemeSetting::Dark,
                    path: Some(state_path.to_owned()),
                    ..State::default()
                },
                bridge,
                events,
                CWD.into(),
                window,
                cx,
            )
        })
    })?;
    fx.wait("isolated root", |fx| fx.read(|root, _| root.ready()))?;
    Ok((fx, recording, log))
}

#[cfg(target_os = "macos")]
#[test]
fn sidebar_project_rows_start_chats_in_their_own_roots_and_leave_open_chats_alone() -> Result<()> {
    use app::{Route, State};
    use common::{GREETING, exchange, recorded_thread, script};
    use std::sync::Arc;

    let temp = tempfile::tempdir()?;
    let [alpha, beta, gone] = ["alpha", "beta", "gone"].map(|name| temp.path().join(name));
    let legacy = temp.path().join("legacy");
    for root in [&alpha, &beta, &gone] {
        fs::create_dir(root)?;
    }
    let state_path = temp.path().join("state.json");
    let host = Arc::new(LocalHost::new(temp.path().join("codex")));
    let [alpha, beta, gone] = [alpha, beta, gone].map(|root| host.add_project(&root).unwrap());
    fs::remove_dir(&gone)?;

    let mut lines = script(None, |_| {})?;
    let mut existing = recorded_thread(GREETING)?;
    existing["cwd"] = json!(legacy);
    for line in &mut lines {
        if line.pointer("/msg/result/thread/id") == Some(&json!(GREETING)) {
            line["msg"]["result"]["thread"]["cwd"] = json!(legacy);
        }
    }
    for (id, root) in [("row-beta", &beta), ("row-alpha", &alpha)] {
        let mut new = existing.clone();
        new["id"] = id.into();
        new["cwd"] = json!(root);
        new["turns"] = json!([]);
        let start = json!({"thread": new, "model": "gpt-6.1-sol", "cwd": root});
        lines.extend(exchange("thread/start", start));
        lines.extend(exchange(
            "turn/start",
            json!({"turn": {"id": id, "items": [], "status": "completed", "error": null}}),
        ));
    }
    let (mut fx, _, log) = isolated_root(temp.path(), &lines, &state_path)?;
    fx.root
        .update(&mut fx.cx, |root, cx| root.set_host(host.clone(), cx));
    fx.wait("three project rows", |fx| {
        fx.read(|root, cx| root.sidebar().read(cx).projects().len() == 3)
    })?;
    let saved_root = || -> Result<Option<String>> {
        Ok(serde_json::from_slice::<State>(&fs::read(&state_path)?)?.project_root)
    };
    // Project rows are 30 px apart + 1 px gap from y 203 (the recorded f1d layout); the
    // new-chat icon is the 20 px box 8 px inside the row's right edge.
    let row = |index: f32| 218. + 31. * index;
    let (label, icon) = (150., 314.);

    fx.click((label, row(0.)))?;
    fx.wait("alpha row selection", |_| {
        saved_root().ok().flatten().as_deref() == Some(alpha.as_str())
    })?;
    assert_eq!(fx.route(), Route::Home);

    // A deleted folder offers no chat: neither its row nor its icon select it.
    fx.hover((label, row(2.)))?;
    fx.click((label, row(2.)))?;
    fx.click((icon, row(2.)))?;
    fx.cx.run_until_parked();
    assert_eq!(saved_root()?.as_deref(), Some(alpha.as_str()));

    fx.hover((label, row(1.)))?;
    fx.click((icon, row(1.)))?;
    fx.wait("beta new chat", |_| {
        saved_root().ok().flatten().as_deref() == Some(beta.as_str())
    })?;
    assert!(fx.composer_focused()?);

    // An open chat keeps its own folder while another project is selected.
    fx.keys("cmd-1")?;
    fx.wait_thread(GREETING)?;
    assert_eq!(
        fx.read(|root, cx| root.thread().unwrap().read(cx).cwd().to_owned()),
        legacy.to_str().unwrap()
    );
    assert_eq!(saved_root()?.as_deref(), Some(beta.as_str()));
    fx.keys("cmd-n")?;
    fx.type_text("Start in beta.")?;
    fx.keys("enter")?;
    fx.wait_thread("row-beta")?;
    assert_eq!(
        fx.read(|root, cx| root.thread().unwrap().read(cx).cwd().to_owned()),
        beta
    );

    fx.hover((label, row(0.)))?;
    fx.click((icon, row(0.)))?;
    fx.wait("alpha new chat", |_| {
        saved_root().ok().flatten().as_deref() == Some(alpha.as_str())
    })?;
    assert_eq!(fx.route(), Route::Home);
    fx.type_text("Start in alpha.")?;
    fx.keys("enter")?;
    fx.wait_thread("row-alpha")?;
    assert_eq!(
        fx.read(|root, cx| root.thread().unwrap().read(cx).cwd().to_owned()),
        alpha
    );
    fx.shot()?.save(format!(
        "{}/project-select-alpha-chat.png",
        env!("CARGO_TARGET_TMPDIR")
    ))?;
    let starts: Vec<String> = fs::read_to_string(&log)?
        .lines()
        .map(serde_json::from_str::<Value>)
        .collect::<serde_json::Result<Vec<_>>>()?
        .into_iter()
        .filter(|request| request["method"] == "thread/start")
        .map(|start| start["params"]["cwd"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(starts, [beta, alpha]);
    Ok(())
}

#[cfg(target_os = "macos")]
#[test]
fn project_selection_changes_new_chats_but_not_existing_chats() -> Result<()> {
    use app::{Root, Route, State};
    use bridge::Bridge;
    use common::{CWD, Fx, GREETING, exchange, recorded_thread, script};
    use std::{process::Command, sync::Arc};

    let temp = tempfile::tempdir()?;
    let alpha = temp.path().join("alpha");
    let beta = temp.path().join("beta");
    fs::create_dir(&alpha)?;
    fs::create_dir(&beta)?;
    for root in [&alpha, &beta] {
        assert!(
            Command::new("git")
                .args(["init", "--quiet"])
                .arg(root)
                .status()?
                .success()
        );
    }
    let alpha = alpha.canonicalize()?;
    let beta = beta.canonicalize()?;
    let state_path = temp.path().join("state.json");
    let host = Arc::new(LocalHost::new(temp.path().join("codex")));
    let mut lines = script(None, |_| {})?;
    let mut existing = recorded_thread(GREETING)?;
    existing["cwd"] = json!(alpha);
    for line in &mut lines {
        if line.pointer("/msg/result/thread/id") == Some(&json!(GREETING)) {
            line["msg"]["result"]["thread"]["cwd"] = json!(alpha);
        }
    }
    let mut new = existing.clone();
    new["id"] = "picker-new".into();
    new["cwd"] = json!(beta);
    new["turns"] = json!([]);
    lines.extend(exchange(
        "thread/start",
        json!({
            "thread": new, "model": "gpt-6.1-sol", "cwd": beta,
        }),
    ));
    lines.extend(exchange("thread/resume", json!({"thread": existing})));
    for id in ["resumed-turn", "new-turn"] {
        lines.extend(exchange(
            "turn/start",
            json!({"turn": {
                "id": id, "items": [], "status": "completed", "error": null,
            }}),
        ));
    }
    let (mut fx, recording, log) = isolated_root(temp.path(), &lines, &state_path)?;
    fx.root
        .update(&mut fx.cx, |root, cx| root.set_host(host.clone(), cx));
    fx.cx.run_until_parked();

    fx.click((515., 695.))?;
    assert!(
        fx.cx.did_prompt_for_paths(),
        "Choose project must open the native picker"
    );
    fx.cx.simulate_path_prompt_response(|options| {
        assert!(options.directories && !options.files && !options.multiple);
        Some(vec![alpha.clone()])
    });
    fx.wait("alpha selection", |_| {
        state_path.exists()
            && fs::read(&state_path)
                .ok()
                .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
                .is_some_and(|state| state["project_root"] == json!(alpha))
    })?;
    assert_eq!(host.projects()?[0].label.as_ref(), "alpha");
    fx.shot()?.save(format!(
        "{}/project-picker-alpha.png",
        env!("CARGO_TARGET_TMPDIR")
    ))?;
    fx.keys("cmd-1")?;
    fx.wait_thread(GREETING)?;
    assert_eq!(
        fx.read(|root, cx| root.thread().unwrap().read(cx).cwd().to_owned()),
        alpha.to_str().unwrap()
    );
    let before = fs::read(&state_path)?;
    fx.keys("cmd-o")?;
    assert!(fx.cx.did_prompt_for_paths());
    fx.cx.simulate_path_prompt_response(|_| None);
    fx.cx.run_until_parked();
    assert_eq!(fx.route(), Route::Thread(GREETING.into()));
    assert_eq!(fs::read(&state_path)?, before);
    assert_eq!(host.projects()?.len(), 1);

    fx.keys("cmd-o")?;
    fx.cx
        .simulate_path_prompt_response(|_| Some(vec![beta.clone()]));
    fx.wait("beta selection", |fx| fx.route() == Route::Home)?;
    assert_eq!(host.projects()?.len(), 2);
    fx.shot()?.save(format!(
        "{}/project-picker-beta.png",
        env!("CARGO_TARGET_TMPDIR")
    ))?;
    fx.keys("cmd-1")?;
    fx.wait_thread(GREETING)?;
    assert_eq!(
        fx.read(|root, cx| root.thread().unwrap().read(cx).cwd().to_owned()),
        alpha.to_str().unwrap()
    );
    fx.type_text("Keep this chat in alpha.")?;
    fx.keys("enter")?;
    fx.wait("existing chat resume", |_| {
        fs::read_to_string(&log)
            .unwrap_or_default()
            .contains("\"thread/resume\"")
    })?;
    fx.keys("cmd-n")?;
    fx.type_text("Start this chat in beta.")?;
    fx.keys("enter")?;
    fx.wait_thread("picker-new")?;
    assert_eq!(
        fx.read(|root, cx| root.thread().unwrap().read(cx).cwd().to_owned()),
        beta.to_str().unwrap()
    );
    let mut requests = Vec::new();
    fx.wait("both submitted turns", |_| {
        let Ok(log) = fs::read_to_string(&log) else {
            return false;
        };
        if !log.ends_with('\n') {
            return false;
        }
        let Ok(snapshot) = log
            .lines()
            .map(serde_json::from_str::<Value>)
            .collect::<serde_json::Result<Vec<_>>>()
        else {
            return false;
        };
        if snapshot
            .iter()
            .filter(|request| request["method"] == "turn/start")
            .count()
            != 2
        {
            return false;
        }
        requests = snapshot;
        true
    })?;
    let start = requests
        .iter()
        .find(|request| request["method"] == "thread/start")
        .unwrap();
    assert_eq!(start["params"]["cwd"], json!(beta));
    let resume = requests
        .iter()
        .find(|request| request["method"] == "thread/resume")
        .unwrap();
    assert_eq!(resume["params"], json!({"threadId": GREETING}));
    let turns: Vec<_> = requests
        .iter()
        .filter(|request| request["method"] == "turn/start")
        .map(|request| request["params"]["threadId"].as_str().unwrap())
        .collect();
    assert_eq!(turns, [GREETING, "picker-new"]);
    fx.shot()?.save(format!(
        "{}/project-picker-new-chat.png",
        env!("CARGO_TARGET_TMPDIR")
    ))?;
    fx.keys("cmd-n cmd-k")?;
    fx.type_text("Open folder")?;
    fx.keys("enter")?;
    assert!(
        fx.cx.did_prompt_for_paths(),
        "Open folder command must use the picker"
    );
    fx.cx.simulate_path_prompt_response(|_| None);
    fx.cx.run_until_parked();
    let saved: State = serde_json::from_slice(&fs::read(&state_path)?)?;
    assert_eq!(saved.project_root.as_deref(), beta.to_str());
    let (bridge, events) = Bridge::replay(&recording)?;
    drop(fx);
    let mut fx = Fx::new()?;
    fx.root = fx.cx.update_window(fx.window, |_, window, cx| {
        window.replace_root(cx, |window, cx| {
            Root::new(saved, bridge, events, CWD.into(), window, cx)
        })
    })?;
    fx.wait("reopened app", |fx| fx.read(|root, _| root.ready()))?;
    fx.root
        .update(&mut fx.cx, |root, cx| root.set_host(host.clone(), cx));
    fx.wait("restored project rows", |fx| {
        fx.read(|root, cx| {
            root.sidebar()
                .read(cx)
                .projects()
                .iter()
                .map(|project| project.label.as_ref())
                .eq(["alpha", "beta"])
        })
    })?;
    fx.shot()?.save(format!(
        "{}/project-picker-reopened.png",
        env!("CARGO_TARGET_TMPDIR")
    ))?;
    println!(
        "Native input passed: two roots, cancel on an existing chat, original resume directory, new thread cwd, command menu, persisted selection."
    );
    Ok(())
}

#[cfg(target_os = "macos")]
#[test]
fn home_file_search_receives_its_session_results() -> Result<()> {
    use bridge::Event;
    use common::{Fx, exchange, script};
    let temp = tempfile::tempdir()?;
    let file = temp.path().join("x.rs");
    fs::write(&file, "fn main() {}\n")?;
    let mut lines = script(None, |_| {})?;
    for method in [
        "fuzzyFileSearch/sessionStart",
        "fuzzyFileSearch/sessionUpdate",
        "fuzzyFileSearch/sessionStop",
        "fuzzyFileSearch/sessionStart",
        "fuzzyFileSearch/sessionUpdate",
        "fuzzyFileSearch/sessionStop",
    ] {
        lines.extend(exchange(method, json!({})));
    }
    let mut fx = Fx::captured("dark", (1280., 832.), lines)?;
    fx.composer().update(&mut fx.cx, |composer, cx| {
        composer.set_search_roots(vec![temp.path().to_string_lossy().into_owned()], cx);
    });
    fx.type_text("@x")?;
    let sent = fx.sent()?;
    let session = sent
        .iter()
        .find(|request| request["method"] == "fuzzyFileSearch/sessionStart")
        .expect("Home must start a file search")["params"]["sessionId"]
        .clone();
    for (method, params) in [
        (
            "fuzzyFileSearch/sessionUpdated",
            json!({
                "sessionId": session, "files": [{"path": file, "file_name": "x.rs"}],
            }),
        ),
        (
            "fuzzyFileSearch/sessionCompleted",
            json!({"sessionId": session}),
        ),
    ] {
        fx.root.update(&mut fx.cx, |root, cx| {
            root.on_event(
                Event::Notification {
                    method: method.into(),
                    params,
                },
                cx,
            );
        });
    }
    let rows = fx.read(|root, cx| root.composer(cx).read(cx).mention_state().unwrap());
    assert!(
        rows.iter()
            .flat_map(|(_, rows)| rows)
            .any(|(label, _)| label.starts_with("x.rs"))
    );
    assert!(
        !rows
            .iter()
            .flat_map(|(_, rows)| rows)
            .any(|(label, _)| label == "Searching…")
    );
    let beta = temp.path().join("beta");
    fs::create_dir(&beta)?;
    let beta_file = beta.join("x-beta.rs");
    fs::write(&beta_file, "fn beta() {}\n")?;
    fx.composer().update(&mut fx.cx, |composer, cx| {
        composer.set_search_roots(vec![beta.to_string_lossy().into_owned()], cx);
    });
    fx.cx.run_until_parked();
    assert_eq!(fx.composer_text()?, "@x");
    let sent = fx.sent()?;
    let next = &sent
        .iter()
        .rev()
        .find(|request| request["method"] == "fuzzyFileSearch/sessionStart")
        .expect("changing roots must restart the current query")["params"];
    assert_ne!(next["sessionId"], session);
    assert_eq!(next["roots"], json!([beta]));
    assert!(sent.iter().any(|request| {
        request["method"] == "fuzzyFileSearch/sessionStop"
            && request["params"]["sessionId"] == session
    }));
    fx.root.update(&mut fx.cx, |root, cx| {
        root.on_event(Event::Notification {
            method: "fuzzyFileSearch/sessionUpdated".into(),
            params: json!({"sessionId": session, "files": [{"path": file, "file_name": "x.rs"}]}),
        }, cx);
    });
    let rows = fx.read(|root, cx| root.composer(cx).read(cx).mention_state().unwrap());
    assert!(
        !rows
            .iter()
            .flat_map(|(_, rows)| rows)
            .any(|(label, _)| label.starts_with("x.rs"))
    );
    for (method, params) in [
        (
            "fuzzyFileSearch/sessionUpdated",
            json!({
                "sessionId": next["sessionId"], "files": [{"path": beta_file, "file_name": "x-beta.rs"}],
            }),
        ),
        (
            "fuzzyFileSearch/sessionCompleted",
            json!({"sessionId": next["sessionId"]}),
        ),
    ] {
        fx.root.update(&mut fx.cx, |root, cx| {
            root.on_event(
                Event::Notification {
                    method: method.into(),
                    params,
                },
                cx,
            );
        });
    }
    fx.wait("the new root's file result", |fx| {
        fx.read(|root, cx| root.composer(cx).read(cx).mention_state())
            .is_some_and(|sections| {
                sections
                    .iter()
                    .flat_map(|(_, rows)| rows)
                    .any(|(label, _)| label.starts_with("x-beta.rs"))
            })
    })?;
    assert!(fx.composer_focused()?);
    let sections = fx.read(|root, cx| root.composer(cx).read(cx).mention_state().unwrap());
    let rows: Vec<_> = sections.iter().flat_map(|(_, rows)| rows).collect();
    let selected = rows.iter().position(|(_, selected)| *selected).unwrap();
    let target = rows
        .iter()
        .position(|(label, _)| label.starts_with("x-beta.rs"))
        .unwrap();
    for _ in 0..(target + rows.len() - selected) % rows.len() {
        fx.keys("down")?;
    }
    fx.keys("enter")?;
    let prompt = fx.read(|root, cx| root.composer(cx).read(cx).prompt());
    assert!(prompt.contains(beta_file.to_str().unwrap()), "{prompt}");
    Ok(())
}
