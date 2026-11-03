#[path = "common/mod.rs"]
mod common;
use anyhow::{Result, ensure};
use base64::Engine as _;
use common::{Fx, exchange, script};
use serde_json::json;

#[test]
fn proxy() {
    common::proxy_child();
}

#[test]
fn slash_sketch_attaches_the_drawing_as_a_png() -> Result<()> {
    let mut lines = script(None, |_| {})?;
    lines.extend(exchange("fs/createDirectory", json!({})));
    lines.extend(exchange("fs/writeFile", json!({})));
    let mut fx = Fx::captured("dark", (1280., 832.), lines)?;
    fx.type_text("/sketch")?;
    fx.keys("enter")?;
    let composer = fx.composer();
    ensure!(composer.read_with(&fx.cx, |composer, _| composer.sketch_open()));
    ensure!(fx.composer_text()?.is_empty());
    composer.update(&mut fx.cx, |composer, cx| {
        composer.sketch_stroke(&[(20., 40.), (200., 40.)], cx);
        composer.attach_sketch(cx);
    });
    ensure!(!composer.read_with(&fx.cx, |composer, _| composer.sketch_open()));
    let mut requests = Vec::new();
    fx.wait("sketch saved", |fx| {
        requests = fx.sent().unwrap_or_default();
        requests.iter().any(|row| row["method"] == "fs/writeFile")
    })?;
    let saved = requests
        .iter()
        .find(|row| row["method"] == "fs/writeFile")
        .unwrap();
    let png = base64::engine::general_purpose::STANDARD
        .decode(saved["params"]["dataBase64"].as_str().unwrap())?;
    let image = image::load_from_memory(&png)?.to_rgba8();
    ensure!(image.get_pixel(100, 40).0 == [0, 0, 0, 255]);
    ensure!(image.get_pixel(100, 10).0 == [255, 255, 255, 255]);
    Ok(())
}
