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
    let result = pollster::block_on(import_items(&bridge, &[item]))?;
    assert_eq!(
        result
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
