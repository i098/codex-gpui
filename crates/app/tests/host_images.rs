mod common;

use anyhow::{Result, ensure};
use common::{Fx, GREETING, exchange, proxy_child, recorded_thread, script};
use serde_json::{Value, json};
use std::{path::PathBuf, sync::Arc};
use ui::screens::settings::AppServer;

const PNG: &str =
    "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAIAAACQd1PeAAAADElEQVR4nGP4z8AAAAMBAQDJ/pLvAAAAAElFTkSuQmCC";

#[test]
fn proxy() {
    proxy_child();
}

#[test]
fn selected_host_images_keep_the_draft_after_a_missing_file() -> Result<()> {
    let mut lines = script(None, |_| {})?;
    lines.extend(exchange("fs/readFile", json!({"dataBase64": PNG})));
    let mut missing = exchange("fs/readFile", json!({}));
    missing[1]["msg"].as_object_mut().unwrap().remove("result");
    missing[1]["msg"]["error"] = json!({"code": -32000, "message": "image not found"});
    lines.extend(missing);
    lines.extend(exchange("fs/readFile", json!({"dataBase64": PNG})));
    let mut thread = recorded_thread(GREETING)?;
    thread["turns"] = json!([]);
    lines.extend(exchange(
        "thread/start",
        json!({"thread": thread, "model": "gpt-6-astra", "cwd": common::CWD}),
    ));
    lines.extend(exchange(
        "turn/start",
        json!({"turn": {"id": "image-turn", "items": [], "status": "completed", "error": null}}),
    ));
    let mut fx = Fx::captured("dark", (1280., 832.), lines)?;
    fx.type_text("Describe the selected host image.")?;
    let composer = fx.composer();
    let selected = fx.cx.update(|cx| cx.global::<AppServer>().0.clone());
    let (other, _) = bridge::Bridge::failed();
    fx.cx.update(|cx| cx.set_global(AppServer(Arc::new(other))));
    let remote = PathBuf::from("/selected-host/only.png");
    composer.update(&mut fx.cx, |composer, cx| {
        composer.drop_files(std::slice::from_ref(&remote), cx)
    });
    fx.cx.update(|cx| cx.set_global(AppServer(selected)));
    fx.advance(std::time::Duration::from_millis(100))?;
    fx.capture_evidence("CGX_IMAGE_EVIDENCE", "image-after-drop")?;
    fx.wait("selected host image read", |fx| {
        fx.sent()
            .is_ok_and(|rows| rows.iter().any(|row| row["method"] == "fs/readFile"))
    })?;
    fx.advance(std::time::Duration::from_millis(100))?;
    fx.capture_evidence("CGX_IMAGE_EVIDENCE", "selected-host-image")?;

    composer.update(&mut fx.cx, |composer, cx| {
        composer.drop_files(&["/selected-host/missing.png".into()], cx)
    });
    fx.wait("missing image read", |fx| {
        fx.sent().is_ok_and(|rows| {
            rows.iter()
                .filter(|row| row["method"] == "fs/readFile")
                .count()
                == 2
        })
    })?;
    fx.advance(std::time::Duration::from_millis(100))?;
    fx.keys("enter")?;
    ensure!(
        fx.composer_text()? == "Describe the selected host image.",
        "a failed image read discarded the draft"
    );
    ensure!(!fx.sent()?.iter().any(|row| row["method"] == "turn/start"));
    fx.capture_evidence("CGX_IMAGE_EVIDENCE", "missing-image-draft")?;
    fx.keys("escape")?;

    let retry = PathBuf::from("/selected-host/retry.png");
    composer.update(&mut fx.cx, |composer, cx| {
        composer.drop_files(std::slice::from_ref(&retry), cx)
    });
    fx.wait("image retry", |fx| {
        fx.sent().is_ok_and(|rows| {
            rows.iter()
                .filter(|row| row["method"] == "fs/readFile")
                .count()
                == 3
        })
    })?;
    fx.advance(std::time::Duration::from_millis(100))?;
    fx.keys("enter")?;
    fx.wait_thread(GREETING)?;
    let mut sent: Vec<Value> = Vec::new();
    fx.wait("image turn", |fx| {
        sent = fx.sent().unwrap_or_default();
        sent.iter().any(|row| row["method"] == "turn/start")
    })?;
    let turn = sent
        .iter()
        .find(|row| row["method"] == "turn/start")
        .unwrap();
    ensure!(
        turn["params"]["input"]
            == json!([
                {"type": "text", "text": "Describe the selected host image.", "text_elements": []},
                {"type": "localImage", "path": remote},
                {"type": "localImage", "path": retry}
            ])
    );
    Ok(())
}

#[test]
fn pasted_images_are_written_under_the_selected_hosts_codex_home() -> Result<()> {
    use base64::Engine as _;
    let mut lines = script(None, |_| {})?;
    lines.extend(exchange("fs/createDirectory", json!({})));
    lines.extend(exchange("fs/writeFile", json!({})));
    let mut fx = Fx::captured("dark", (1280., 832.), lines)?;
    fx.type_text("Keep this draft while the image uploads.")?;
    let composer = fx.composer();
    composer.update(&mut fx.cx, |composer, cx| {
        composer.attach_png(
            base64::engine::general_purpose::STANDARD
                .decode(PNG)
                .unwrap(),
            cx,
        );
    });
    let mut requests = Vec::new();
    fx.wait("image saved on selected host", |fx| {
        requests = fx.sent().unwrap_or_default();
        requests.iter().any(|row| row["method"] == "fs/writeFile")
    })?;
    let saved = requests
        .iter()
        .find(|row| row["method"] == "fs/writeFile")
        .unwrap();
    ensure!(saved["params"]["dataBase64"] == PNG);
    ensure!(
        saved["params"]["path"]
            .as_str()
            .unwrap()
            .starts_with("/private/tmp/cgx-mac/home/attachments/")
    );
    ensure!(fx.composer_text()? == "Keep this draft while the image uploads.");
    Ok(())
}
