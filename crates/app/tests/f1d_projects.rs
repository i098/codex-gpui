#[cfg(target_os = "macos")]
mod common;

use anyhow::Result;
use app::host::{Host, LocalHost, ReplayHost};
use serde_json::{Value, json};
use std::fs;

const RECORDING: &str = include_str!("../../../refs/interactions/f1d_projects.json");

#[test]
fn native_project_order_and_markers_follow_the_stored_roots() -> Result<()> {
    let recording: Value = serde_json::from_str(RECORDING)?;
    let mut state = recording["project_snapshot"]["state"].clone();
    let home = tempfile::tempdir()?;
    let ordinary = home.path().join("project");
    let worktree = home.path().join("worktrees/one/project");
    let missing = home.path().join("missing");
    let outside = home.path().join("worktrees-extra/project");
    for path in [&ordinary, &worktree, &outside] {
        fs::create_dir_all(path)?;
    }
    for (id, root) in [
        ("parity-project", &ordinary),
        ("parity-worktree", &worktree),
        ("parity-missing", &missing),
    ] {
        state["local-projects"][id]["rootPaths"] = json!([root]);
    }
    state["local-projects"]["outside"] = json!({
        "id": "outside", "name": "  Outside  ", "rootPaths": [outside],
        "createdAt": 1, "updatedAt": 1,
    });
    state["local-projects"]["malformed"] = json!({"id": "malformed", "rootPaths": [7]});
    state["project-order"] = json!([
        "parity-worktree",
        "unknown",
        "parity-missing",
        "parity-project"
    ]);
    let path = home.path().join(".codex-global-state.json");
    let original = serde_json::to_vec(&state)?;
    fs::write(&path, &original)?;
    let host = LocalHost::new(home.path());
    let rows = host.projects()?;
    assert_eq!(
        rows.iter().map(|row| row.id.as_ref()).collect::<Vec<_>>(),
        [
            "parity-worktree",
            "parity-missing",
            "parity-project",
            "outside"
        ],
    );
    assert!(rows[0].worktree && !rows[0].missing && !rows[0].expanded);
    assert!(rows[1].missing && !rows[1].worktree);
    assert!(!rows[2].missing && !rows[2].worktree);
    assert!(!rows[3].worktree);
    assert_eq!(rows[3].label.as_ref(), "Outside");
    assert!(rows[3].expanded);

    fs::remove_dir(&ordinary)?;
    assert!(
        host.projects()?
            .iter()
            .find(|row| row.id == "parity-project")
            .unwrap()
            .missing
    );
    fs::create_dir(&ordinary)?;
    assert!(
        !host
            .projects()?
            .iter()
            .find(|row| row.id == "parity-project")
            .unwrap()
            .missing
    );
    assert_eq!(
        fs::read(path)?,
        original,
        "reading projects must not rewrite native storage"
    );
    Ok(())
}

#[test]
fn a_missing_path_response_does_not_mark_every_project_deleted() -> Result<()> {
    let mut recording: Value = serde_json::from_str(RECORDING)?;
    recording["project_snapshot"]["existing_paths"] = Value::Null;
    let host = ReplayHost::from_recording(&recording.to_string())?;
    assert!(host.projects()?.iter().all(|row| !row.missing));
    recording["project_snapshot"]["existing_paths"] = json!([]);
    let host = ReplayHost::from_recording(&recording.to_string())?;
    assert!(host.projects()?.iter().all(|row| row.missing));
    Ok(())
}

#[cfg(target_os = "macos")]
#[test]
fn recorded_project_marker_is_painted() -> Result<()> {
    use anyhow::Context as _;
    use base64::{Engine as _, engine::general_purpose::STANDARD};
    use common::Fx;
    use std::{sync::Arc, time::Duration};

    let recording: Value = serde_json::from_str(RECORDING)?;
    let marker = &recording["rest"]["worktreeIcon"];
    let expected = image::load_from_memory(
        &STANDARD.decode(marker["png"].as_str().context("recorded marker pixels")?)?,
    )?
    .to_rgba8();
    let bounds = &marker["bbox"];
    let coordinate = |key: &str| (bounds[key].as_f64().unwrap() * 2.) as u32;
    let host = Arc::new(ReplayHost::from_recording(RECORDING)?);
    let mut fx = Fx::new()?;
    fx.root
        .update(&mut fx.cx, |root, cx| root.set_host(host, cx));
    fx.hover((900., 10.))?;
    let expected_ids: Vec<_> = recording["rest"]["rows"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| row["id"].as_str().unwrap())
        .collect();
    fx.wait("the recorded project rows", |fx| {
        fx.read(|root, cx| {
            root.sidebar()
                .read(cx)
                .projects()
                .iter()
                .map(|project| project.id.as_ref())
                .eq(expected_ids.iter().copied())
        })
    })?;
    let rest = fx.shot()?;
    rest.save(format!(
        "{}/f1d-projects-rest.png",
        env!("CARGO_TARGET_TMPDIR")
    ))?;
    anyhow::ensure!(
        image::imageops::crop_imm(
            &rest,
            coordinate("x"),
            coordinate("y"),
            coordinate("w"),
            coordinate("h"),
        )
        .to_image()
            == expected,
        "the worktree glyph differs from the original pixels"
    );
    let rows = fx.read(|root, cx| root.sidebar().read(cx).projects().to_vec());
    for (row, expected) in rows
        .iter()
        .zip(recording["rest"]["rows"].as_array().unwrap())
    {
        assert_eq!(row.id.as_ref(), expected["id"].as_str().unwrap());
        assert_eq!(row.label.as_ref(), expected["text"].as_str().unwrap());
        assert_eq!(row.missing, expected["opacity"] == "0.7");
        assert_eq!(row.expanded, expected["expanded"] == "true");
    }
    for step in recording["steps"].as_array().unwrap() {
        let at = if step["at"].is_object() {
            (
                step["at"]["x"].as_f64().unwrap(),
                step["at"]["y"].as_f64().unwrap(),
            )
        } else {
            let box_ = &step["target"]["bbox"];
            (
                box_["x"].as_f64().unwrap() + box_["w"].as_f64().unwrap() / 2.,
                box_["y"].as_f64().unwrap() + box_["h"].as_f64().unwrap() / 2.,
            )
        };
        fx.hover((at.0 as f32, at.1 as f32))?;
        fx.advance(Duration::from_millis(step["waitMs"].as_u64().unwrap()))?;
        let image = fx.shot()?;
        for tip in step["expected"]["tooltips"]
            .as_array()
            .into_iter()
            .flatten()
        {
            let label = tip["text"].as_str().unwrap();
            let suffix = if label == "This is a git worktree" {
                "worktree"
            } else {
                "missing"
            };
            image.save(format!(
                "{}/f1d-projects-{suffix}.png",
                env!("CARGO_TARGET_TMPDIR")
            ))?;
            let actual = if label == "This is a git worktree" {
                fx.read(|root, cx| {
                    root.sidebar()
                        .read(cx)
                        .worktree_tooltip()
                        .map(|(label, bounds)| (label.to_owned(), bounds))
                })
            } else {
                fx.read(|_, cx| {
                    ui::tooltip::shown(cx).map(|tip| (tip.label.to_string(), tip.bounds))
                })
            }
            .context("recorded project tooltip did not open")?;
            assert_eq!(actual.0, label);
            for (key, actual) in [
                ("x", actual.1.origin.x),
                ("y", actual.1.origin.y),
                ("w", actual.1.size.width),
                ("h", actual.1.size.height),
            ] {
                anyhow::ensure!(
                    (f32::from(actual) - tip["bbox"][key].as_f64().unwrap() as f32).abs() < 1.,
                    "{label} {key} differs from the original: {}",
                    f32::from(actual)
                );
            }
        }
        if step["at"].is_object() {
            assert!(fx.read(|root, cx| root.sidebar().read(cx).worktree_tooltip().is_none()));
            assert!(fx.read(|_, cx| ui::tooltip::shown(cx).is_none()));
        }
    }
    Ok(())
}
