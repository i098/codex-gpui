mod common;

use anyhow::{Context as _, Result, ensure};
use common::Fx;
use serde_json::Value;

#[test]
fn proxy() {
    common::proxy_child();
}

fn point(value: &Value) -> Result<(f32, f32)> {
    Ok((
        value["x"].as_f64().context("x")? as f32,
        value["y"].as_f64().context("y")? as f32,
    ))
}

fn center(value: &Value) -> Result<(f32, f32)> {
    let (x, y) = point(value)?;
    Ok((
        x + value["w"].as_f64().context("width")? as f32 / 2.,
        y + value["h"].as_f64().context("height")? as f32 / 2.,
    ))
}

fn menu(fx: &Fx) -> Vec<(String, bool)> {
    fx.read(|root, cx| {
        root.diff_panel()
            .unwrap()
            .read(cx)
            .menu_items()
            .map(|(label, enabled)| (label.to_owned(), enabled))
            .collect()
    })
}

fn clipboard(fx: &mut Fx) -> Option<String> {
    fx.cx
        .update(|cx| cx.read_from_clipboard().and_then(|item| item.text()))
}

fn choose(fx: &mut Fx, label: &str) -> Result<()> {
    let items = menu(fx);
    let index = items
        .iter()
        .filter(|(_, enabled)| *enabled)
        .position(|(text, _)| *text == label)
        .context("enabled menu item")?;
    fx.keys("home")?;
    for _ in 0..index {
        fx.keys("down")?;
    }
    fx.keys("enter")
}

#[test]
fn diff_selection_and_hover_replay() -> Result<()> {
    if !cfg!(target_os = "macos") {
        eprintln!("S027 requires the macOS headless renderer");
        return Ok(());
    }
    let screenshots = tempfile::tempdir()?;
    let recording: Value =
        serde_json::from_str(include_str!("../../../refs/interactions/s027.json"))?;
    let mut fx = Fx::captured("dark", (1280., 832.), common::script(None, |_| {})?)?;
    fx.click(fx.at("sidebar", "Add a subtract")?)?;
    fx.wait_thread(common::SUBTRACT)?;
    fx.click(fx.at("thread", "View changes")?)?;
    let requests_before = fx.sent()?.len();
    let steps = recording["steps"].as_array().context("steps")?;
    let header = steps
        .iter()
        .find(|s| s["target"]["text"] == "File actions")
        .context("file actions")?;
    let trigger = center(&header["target"]["bbox"])?;
    fx.click(trigger)?;
    let expected: Vec<_> = header["expect"]["menu"]
        .as_array()
        .context("header menu")?
        .iter()
        .map(|item| {
            (
                item["text"].as_str().unwrap().to_owned(),
                item["disabled"].as_bool() != Some(true),
            )
        })
        .collect();
    ensure!(menu(&fx) == expected, "header menu: {:?}", menu(&fx));
    fx.shot()?
        .save(screenshots.path().join("s027-header-menu.png"))?;
    let copy = steps
        .iter()
        .find(|s| s["target"]["text"] == "Copy path")
        .context("copy path")?;
    fx.click(center(&copy["target"]["bbox"])?)?;
    ensure!(
        clipboard(&mut fx).as_deref() == copy["expect"]["clipboard"].as_str(),
        "header path copy"
    );
    ensure!(menu(&fx).is_empty(), "copy must close the menu");
    fx.click(trigger)?;
    fx.keys("down down down enter")?;
    ensure!(
        !fx.read(|root, cx| root.diff_panel().unwrap().read(cx).files[0].open),
        "the third enabled item collapses the file"
    );
    fx.click(trigger)?;
    choose(&mut fx, "Expand file")?;
    ensure!(
        fx.read(|root, cx| root.diff_panel().unwrap().read(cx).files[0].open),
        "expand file"
    );
    fx.right_click((800., 229.))?;
    ensure!(
        menu(&fx)[0] == ("Copy selection".into(), false),
        "copy needs a selection"
    );
    fx.keys("escape")?;
    ensure!(menu(&fx).is_empty(), "Escape must close the menu");
    let hover = steps
        .iter()
        .find(|s| s["type"] == "hover")
        .context("hover")?;
    ensure!(
        hover["expect"]["hover"]["before"] == hover["expect"]["hover"]["after"],
        "the original hover changed"
    );
    fx.hover((640., 22.))?;
    let before = fx.shot()?;
    fx.hover(point(&hover["portAt"])?)?;
    let after = fx.shot()?;
    ensure!(
        common::changed(&before, &after, [662., 218., 614., 22.]) == 0,
        "diff hover must not change the row"
    );
    let drag = steps.iter().find(|s| s["type"] == "drag").context("drag")?;
    fx.drag(point(&drag["portFrom"])?, point(&drag["portTo"])?, 10)?;
    let selected = fx.read(|root, cx| root.diff_panel().unwrap().read(cx).selected_text());
    ensure!(
        selected == drag["expect"]["selection"].as_str().context("selection")?,
        "selected text: {selected:?}"
    );
    let key = steps.iter().find(|s| s["type"] == "key").context("copy")?;
    fx.keys(key["portKey"].as_str().context("port key")?)?;
    let copied = clipboard(&mut fx);
    ensure!(
        copied.as_deref() == key["expect"]["clipboard"].as_str(),
        "clipboard: {copied:?}"
    );
    fx.shot()?
        .save(screenshots.path().join("s027-selection.png"))?;
    fx.drag(point(&drag["portTo"])?, point(&drag["portFrom"])?, 10)?;
    fx.keys("cmd-c")?;
    let reverse = clipboard(&mut fx);
    ensure!(
        reverse == copied,
        "reverse selection must copy the same text"
    );
    let context = steps
        .iter()
        .find(|s| s["type"] == "context-menu")
        .context("context menu")?;
    fx.right_click(point(&context["portAt"])?)?;
    let expected: Vec<_> = context["expect"]["items"]
        .as_array()
        .context("context items")?
        .iter()
        .filter(|item| {
            matches!(
                item["id"].as_str(),
                Some("copy-selection" | "copy-path" | "copy-relative-path" | "toggle-wrap")
            )
        })
        .map(|item| {
            (
                item["label"].as_str().unwrap().to_owned(),
                item["enabled"].as_bool().unwrap(),
            )
        })
        .collect();
    ensure!(menu(&fx) == expected, "context menu: {:?}", menu(&fx));
    fx.shot()?
        .save(screenshots.path().join("s027-context-menu.png"))?;
    choose(&mut fx, "Copy selection")?;
    ensure!(
        clipboard(&mut fx).as_deref()
            == recording["selectionNative"]["clipboard"][0]["args"][0].as_str(),
        "native selection copy"
    );
    for action in recording["contextActions"]
        .as_array()
        .context("context actions")?
    {
        fx.right_click(point(&action["portAt"])?)?;
        let label = match action["id"].as_str().context("action id")? {
            "copy-path" => "Copy path",
            "copy-relative-path" => "Copy relative path",
            "toggle-wrap" => "Toggle line wrap",
            other => anyhow::bail!("unknown menu action {other}"),
        };
        choose(&mut fx, label)?;
        if let Some(text) = action["expect"]["clipboard"][0]["args"][0].as_str() {
            ensure!(
                clipboard(&mut fx).as_deref() == Some(text),
                "{label} clipboard"
            );
        } else {
            ensure!(
                action["expect"]["wrapMode"] == "wrap",
                "original wrap state"
            );
            ensure!(
                fx.read(|root, cx| root.diff_panel().unwrap().read(cx).wraps_lines()),
                "wrap must turn on"
            );
        }
    }
    let whitespace = &recording["whitespaceSelection"];
    fx.drag(
        point(&whitespace["portFrom"])?,
        point(&whitespace["portTo"])?,
        10,
    )?;
    fx.keys("cmd-c")?;
    ensure!(
        clipboard(&mut fx).as_deref() == whitespace["keyboardClipboard"].as_str(),
        "keyboard copy preserves whitespace"
    );
    fx.right_click((800., 164.))?;
    choose(&mut fx, "Copy selection")?;
    ensure!(
        clipboard(&mut fx).as_deref() == whitespace["menuClipboard"].as_str(),
        "native menu copy trims whitespace"
    );
    fx.right_click((800., 229.))?;
    fx.click((640., 22.))?;
    ensure!(menu(&fx).is_empty(), "outside click must close the menu");
    let sent = fx.sent()?;
    ensure!(
        serde_json::to_value(&sent[requests_before..])? == recording["result"]["requests"],
        "diff actions must match the recorded app-server traffic"
    );
    let thread = common::recorded_thread(common::SUBTRACT)?;
    let turn = thread["turns"]
        .as_array()
        .context("turns")?
        .last()
        .context("last turn")?;
    let long_line = format!("    {}end", "abcdefghij ".repeat(12));
    fx.root.update(&mut fx.cx, |root, cx| {
        root.on_event(bridge::Event::Notification {
            method: "turn/diff/updated".into(),
            params: serde_json::json!({
                "threadId": common::SUBTRACT,
                "turnId": turn["id"],
                "diff": format!("diff --git a/src/math.py b/src/math.py\n--- a/src/math.py\n+++ b/src/math.py\n@@ -0,0 +1 @@\n+{long_line}\n"),
            }),
        }, cx);
    });
    fx.cx.run_until_parked();
    fx.drag((714., 143.), (1270., 174.), 10)?;
    fx.keys("cmd-c")?;
    fx.shot()?
        .save(screenshots.path().join("s027-wrapped-selection.png"))?;
    let wrapped = clipboard(&mut fx);
    ensure!(
        wrapped.as_deref() == Some(long_line.as_str()),
        "wrapped text: {wrapped:?}"
    );
    fx.click((1000., 300.))?;
    ensure!(
        fx.read(|root, cx| root.diff_panel().unwrap().read(cx).selected_text())
            .is_empty(),
        "background click clears the selection"
    );
    fx.keys("cmd-c")?;
    ensure!(
        clipboard(&mut fx) == wrapped,
        "copy with no selection leaves the clipboard unchanged"
    );
    Ok(())
}

use app::{host::LocalHost, host_settings::Settings};
use serde_json::json;
use std::{fs, os::unix::fs::PermissionsExt as _, path::Path, process::Command, sync::Arc};

const FAILED: &str = "Unable to open in editor";
/// Header icons of the first file in the Unstaged list, as in `git_revert_ui`.
const OPEN_ICON: (f32, f32) = (1234., 110.);
const FILE_ACTIONS: (f32, f32) = (1258., 110.);

fn git(root: &Path, args: &[&str]) -> Result<()> {
    let output = Command::new("git").current_dir(root).args(args).output()?;
    ensure!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(())
}

fn toast(fx: &Fx, expected: &str) -> bool {
    fx.read(|root, cx| root.toasts(cx).first().is_some_and(|t| t == expected))
}

fn clear_toasts(fx: &mut Fx) -> Result<()> {
    fx.hover((1000., 22.))?;
    fx.advance(ui::toast::LIFETIME + std::time::Duration::from_secs(1))
}

fn launches(log: &Path) -> Vec<String> {
    fs::read_to_string(log)
        .unwrap_or_default()
        .lines()
        .map(str::to_owned)
        .collect()
}

#[test]
fn open_in_editor_native_input_reaches_the_host_opener() -> Result<()> {
    if !cfg!(target_os = "macos") {
        eprintln!("S027 requires the macOS headless renderer");
        return Ok(());
    }
    let scratch = std::env::var_os("CGX_OPEN_IN_EDITOR_EVIDENCE")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join("cgx-open-in-editor"));
    fs::create_dir_all(&scratch)?;
    let repo = tempfile::tempdir_in(&scratch)?;
    let root = repo.path().canonicalize()?;
    git(&root, &["init", "-q"])?;
    git(&root, &["config", "user.email", "proof@example.invalid"])?;
    git(&root, &["config", "user.name", "Native proof"])?;
    fs::write(root.join("a.txt"), "one\ntwo\n")?;
    git(&root, &["add", "a.txt"])?;
    git(&root, &["commit", "-qm", "base"])?;
    fs::write(root.join("a.txt"), "one\nchanged\n")?;

    // The controlled opener records its arguments, or fails while `fail` exists.
    let log = scratch.join(format!("opener-{}.log", std::process::id()));
    let fail = scratch.join(format!("opener-{}.fail", std::process::id()));
    let opener = scratch.join(format!("opener-{}.sh", std::process::id()));
    fs::write(
        &opener,
        format!(
            "#!/bin/sh\nif [ -e '{}' ]; then echo 'editor refused' >&2; exit 3; fi\nprintf '%s\\n' \"$*\" >> '{}'\n",
            fail.display(),
            log.display()
        ),
    )?;
    fs::set_permissions(&opener, fs::Permissions::from_mode(0o755))?;

    let mut settings =
        Settings::load(scratch.join(format!("host-settings-{}.json", std::process::id())))?;
    for (key, value) in common::recorded_host_settings()?.read_all() {
        settings.write(key, value.clone())?;
    }
    settings.write("open-in-target-preferences", json!({"global": "zed"}))?;
    let mut lines = common::script(None, |_| {})?;
    for line in &mut lines {
        if let Some(thread) = line.pointer_mut("/msg/result/thread") {
            thread["cwd"] = root.to_string_lossy().into_owned().into();
        }
    }
    let mut fx = common::Fx::captured_with_host_settings("dark", (1280., 832.), lines, settings)?;
    let host = LocalHost::new(root.join(".git/proof-profile")).with_opener(&opener);
    fx.root
        .update(&mut fx.cx, |root, cx| root.set_host(Arc::new(host), cx));
    fx.keys("cmd-1")?;
    fx.wait_thread(common::GREETING)?;
    fx.keys("alt-cmd-b")?;
    fx.hover((1000., 22.))?;
    fx.advance(ui::toast::LIFETIME + std::time::Duration::from_secs(1))?;
    fx.click((735., 70.))?;
    choose(&mut fx, "Unstaged")?;
    fx.wait("repository changes", |fx| {
        fx.read(|root, cx| !root.diff_panel().unwrap().read(cx).pending())
    })?;
    fx.shot()?.save(scratch.join("header-icon.png"))?;

    let expected = format!("-a Zed {}", root.join("a.txt").display());
    fx.click(OPEN_ICON)?;
    fx.wait("file tab", |fx| {
        fx.read(|root, cx| {
            root.file_view().is_some_and(|file| {
                file.read(cx)
                    .contents
                    .as_ref()
                    .is_some_and(|contents| contents.as_deref() == Ok("one\nchanged\n"))
            })
        })
    })?;
    ensure!(
        launches(&log).is_empty(),
        "the file tab launched an external editor"
    );
    fx.action(Box::new(ui::screens::diff::panel::CloseSideTab(1)))?;

    // The file menu reaches the same opener, and its other actions stay enabled.
    fx.click(FILE_ACTIONS)?;
    ensure!(
        ["Open in editor", "Stage file", "Revert file"]
            .iter()
            .all(|label| menu(&fx).contains(&(label.to_string(), true)))
    );
    fx.shot()?.save(scratch.join("file-menu.png"))?;
    choose(&mut fx, "Open in editor")?;
    fx.wait("menu launch", |_| launches(&log).len() == 1)?;
    ensure!(launches(&log)[0] == expected);
    ensure!(!fx.read(|root, cx| root.diff_panel().unwrap().read(cx).pending()));
    ensure!(fx.read(|root, cx| root.diff_panel().unwrap().read(cx).files[0].open));

    // An opener failure is reported, and the changes list stays as it was.
    fs::write(&fail, "")?;
    fx.click(FILE_ACTIONS)?;
    choose(&mut fx, "Open in editor")?;
    fx.wait("failure toast", |fx| toast(fx, FAILED))?;
    fx.shot()?.save(scratch.join("failure-toast.png"))?;
    ensure!(launches(&log).len() == 1, "a refused launch was recorded");
    ensure!(!fx.read(|root, cx| root.diff_panel().unwrap().read(cx).pending()));
    fs::remove_file(&fail)?;
    clear_toasts(&mut fx)?;

    // A changed file that no longer exists is not opened.
    fs::remove_file(root.join("a.txt"))?;
    fx.click(FILE_ACTIONS)?;
    choose(&mut fx, "Open in editor")?;
    fx.wait("missing file toast", |fx| toast(fx, FAILED))?;
    ensure!(
        launches(&log).len() == 1,
        "a missing file reached the opener"
    );
    Ok(())
}

#[test]
fn host_open_file_refuses_paths_outside_the_project_and_unknown_targets() -> Result<()> {
    let scratch = tempfile::tempdir()?;
    fs::create_dir(scratch.path().join("repo"))?;
    let root = scratch.path().join("repo").canonicalize()?;
    git(&root, &["init", "-q"])?;
    fs::create_dir(root.join("src"))?;
    fs::write(root.join("src/a.txt"), "inside")?;
    fs::write(root.join("top.txt"), "top")?;
    let outside = scratch.path().join("outside.txt");
    fs::write(&outside, "outside")?;
    std::os::unix::fs::symlink(&outside, root.join("escape.txt"))?;
    let log = scratch.path().join("log");
    let opener = scratch.path().join("opener.sh");
    fs::write(
        &opener,
        format!("#!/bin/sh\nprintf '%s\\n' \"$*\" >> '{}'\n", log.display()),
    )?;
    fs::set_permissions(&opener, fs::Permissions::from_mode(0o755))?;
    let host = LocalHost::new(scratch.path().join("home")).with_opener(&opener);
    let open = |cwd: &Path, path: &str, target: Option<&str>| {
        use app::host::{Host as _, Request};
        host.fetch(&Request::post(
            "open-file",
            Some(json!({"hostId": "local", "cwd": cwd, "path": path, "target": target})),
        ))
    };
    let inside = root.join("src/a.txt");
    let outside_path = outside.to_string_lossy().into_owned();
    let traversal = root.join("src/../../outside.txt");
    for (cwd, path, target, reason) in [
        (&root, outside_path.as_str(), None, "outside file"),
        (&root, "src/a.txt", None, "relative path"),
        (&root, traversal.to_str().unwrap(), None, "dot-dot escape"),
        (
            &root,
            root.join("escape.txt").to_str().unwrap(),
            None,
            "symlink escape",
        ),
        (&root, root.join("src").to_str().unwrap(), None, "directory"),
        (
            &root,
            root.join("gone.txt").to_str().unwrap(),
            None,
            "missing file",
        ),
        (
            &root,
            inside.to_str().unwrap(),
            Some("notAnEditor"),
            "unknown target",
        ),
    ] {
        ensure!(open(cwd, path, target).is_err(), "{reason} was opened");
    }
    ensure!(
        launches(&log).is_empty(),
        "a refused request ran the opener"
    );
    // A thread in a subfolder still opens repository files outside that subfolder.
    let top = root.join("top.txt");
    open(&root.join("src"), top.to_str().unwrap(), Some("cursor"))?;
    ensure!(launches(&log) == [format!("-a Cursor {}", top.display())]);
    Ok(())
}
