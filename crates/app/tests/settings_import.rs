use anyhow::Result;
use app::settings_import::{detect, import_items};
use bridge::{
    Bridge,
    imports::{DetectedItem, ItemType, MigrationItem, Policy, Provider},
};
use serde_json::{Value, json};
use std::io::Write as _;

fn replay(rows: &[(&str, Value)]) -> Result<(Bridge, tempfile::NamedTempFile)> {
    let mut file = tempfile::NamedTempFile::new()?;
    for (direction, message) in rows {
        writeln!(file, "{}", json!({"dir":direction,"msg":message}))?;
    }
    let (bridge, _) = Bridge::replay(file.path())?;
    Ok((bridge, file))
}

#[test]
fn completion_identity_wins_even_when_the_notification_precedes_the_reply() -> Result<()> {
    let completed = |id| {
        json!({"method":"externalAgentConfig/import/completed", "params":{
            "importId":id,"itemTypeResults":[{"itemType":"AGENTS_MD","successes":[],"failures":[]}]
        }})
    };
    let (bridge, _file) = replay(&[
        (
            "send",
            json!({"id":1,"method":"externalAgentConfig/import","params":{}}),
        ),
        ("recv", completed("another-import")),
        ("recv", completed("matching-import")),
        (
            "recv",
            json!({"id":1,"result":{"importId":"matching-import"}}),
        ),
    ])?;
    let item = DetectedItem {
        provider_id: Provider::ClaudeCode,
        item: MigrationItem {
            item_type: ItemType::AgentsMd,
            description: "fixture instructions".into(),
            cwd: Some("/owned/fixture".into()),
            details: None,
        },
    };
    let host = app::host::ReplayHost::from_recording(r#"{"exchanges":[]}"#)?;
    let result = pollster::block_on(import_items(&bridge, &host, &[item]))?;
    assert_eq!(
        result
            .completions
            .iter()
            .map(|result| result["importId"].as_str())
            .collect::<Vec<_>>(),
        [Some("matching-import")]
    );
    Ok(())
}

#[test]
fn an_empty_provider_does_not_hide_a_detection_failure() -> Result<()> {
    let (bridge, _file) = replay(&[
        (
            "send",
            json!({"id":1,"method":"externalAgentConfig/detect","params":{"includeHome":true}}),
        ),
        ("recv", json!({"id":1,"result":{"items":[]}})),
        (
            "send",
            json!({"id":2,"method":"externalAgentConfig/detect","params":{"includeHome":true,"migrationSource":"cursor"}}),
        ),
        (
            "recv",
            json!({"id":2,"error":{"code":-32000,"message":"fixture provider scan failed"}}),
        ),
    ])?;
    let error = pollster::block_on(detect(
        &bridge,
        Policy {
            enabled: true,
            sync: false,
            cursor: true,
        },
    ))
    .unwrap_err();
    assert!(error.to_string().contains("fixture provider scan failed"));
    Ok(())
}

#[test]
fn cursor_projectless_import_records_the_workspace_without_creating_a_project() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let root = directory.path().join("workspaces");
    let source = "/owned/.cursor/projects/empty-window/agent-transcripts/imported.jsonl";
    let (bridge, _file) = replay(&[
        (
            "send",
            json!({"id":1,"method":"config/read","params":{"includeLayers":false}}),
        ),
        (
            "recv",
            json!({"id":1,"result":{"config":{"desktop":{"projectlessWorkspaceRoot":root}}}}),
        ),
        (
            "send",
            json!({"id":2,"method":"externalAgentConfig/import","params":{}}),
        ),
        (
            "recv",
            json!({"id":2,"result":{"importId":"projectless-import"}}),
        ),
        (
            "recv",
            json!({"method":"externalAgentConfig/import/completed","params":{
                "importId":"projectless-import",
                "itemTypeResults":[{
                    "itemType":"SESSIONS",
                    "successes":[{
                        "itemType":"SESSIONS",
                        "source":"C:\\Users\\fixture\\.cursor\\projects\\empty-window\\agent-transcripts\\imported.jsonl",
                        "target":"imported-thread"
                    }],
                    "failures":[]
                }]
            }}),
        ),
    ])?;
    let bridge = std::sync::Arc::new(bridge);
    let host =
        app::host::LocalHost::new(directory.path().join("codex")).with_bridge(bridge.clone());
    let item = DetectedItem {
        provider_id: Provider::Cursor,
        item: MigrationItem {
            item_type: ItemType::Sessions,
            description: "Projectless chat".into(),
            cwd: None,
            details: Some(json!({"sessions":[{
                "workspaceKind":"projectless", "path":source, "title":"Imported chat"
            }]})),
        },
    };

    let imported = pollster::block_on(import_items(&bridge, &host, &[item]))?;

    assert!(imported.project_roots.is_empty());
    let state: Value = serde_json::from_slice(&std::fs::read(
        directory.path().join("codex/.codex-global-state.json"),
    )?)?;
    assert_eq!(state["projectless-thread-ids"], json!(["imported-thread"]));
    assert_eq!(
        state["thread-workspace-root-hints"]["imported-thread"],
        json!(root)
    );
    let output = std::path::Path::new(
        state["thread-projectless-output-directories"]["imported-thread"]
            .as_str()
            .expect("imported output directory"),
    );
    assert!(output.is_dir());
    assert!(output.parent().expect("workspace").join("work").is_dir());
    Ok(())
}
