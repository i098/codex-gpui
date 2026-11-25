//! One `GET /accounts/{account_id}/teams/{team_id}/activities` item, worded as the original's
//! `settings.teams.activity.*` messages (`team-settings-page` `In`, `Yn`, `qn`, `Wn`).
use serde_json::Value;

#[derive(Clone, Debug, PartialEq)]
pub struct Activity {
    pub id: String,
    /// The server's RFC 3339 time.
    pub created_at: String,
    pub message: String,
    /// The change lines under the message: team or automation field changes, added members.
    pub details: Vec<String>,
}

impl Activity {
    /// `None` for an item without an id or time, or of a type the original does not show (`Vn`).
    pub fn from_value(item: &Value) -> Option<Self> {
        let data = &item["activity_data"];
        let kind = data["type"].as_str()?;
        let actor = name(&item["actor"], "id");
        let member = if data.get("principal_id").is_some() {
            name(data, "principal_id")
        } else {
            name(&data["members"][0], "principal_id")
        }
        .unwrap_or_default();
        let title = data["title"].as_str().unwrap_or_default();
        let plugin = data["plugin_name"].as_str().unwrap_or_default();
        let app = data["connector_name"].as_str().unwrap_or_default();
        let connection = data["connection_name"].as_str().unwrap_or_default();
        // `{hasActor, select, true {…} other {…}}`.
        let by = |with: String, without: String| match actor {
            Some(_) => with,
            None => without,
        };
        let a = actor.unwrap_or_default();
        let message = match kind {
            "TEAM_CREATE" => by(
                format!("{a} created the team"),
                "Someone created the team".into(),
            ),
            "TEAM_UPDATE" => by(
                format!("{a} updated the team"),
                "Someone updated the team".into(),
            ),
            "TEAM_DELETE" => by(
                format!("{a} deleted the team"),
                "Someone deleted the team".into(),
            ),
            "TEAM_MEMBER_ADD" => {
                let self_join = data["principal_type"] == "user"
                    && item["actor"]["type"] == "user"
                    && item["actor"]["id"] == data["principal_id"];
                if self_join {
                    format!("{a} joined the team")
                } else {
                    by(
                        format!("{a} added {member}"),
                        format!("{member} was added to the team"),
                    )
                }
            }
            "TEAM_MEMBERS_ADD" => {
                let more = data["total_count"].as_u64().unwrap_or(1).saturating_sub(1);
                let others = match more {
                    0 => String::new(),
                    1 => " and 1 other".into(),
                    n => format!(" and {n} others"),
                };
                let were = if more == 0 { "was" } else { "were" };
                by(
                    format!("{a} added {member}{others}"),
                    format!("{member}{others} {were} added to the team"),
                )
            }
            "TEAM_MEMBER_UPDATE" if data["role"] == "owner" => by(
                format!("{a} transferred team ownership to {member}"),
                format!("Team ownership was transferred to {member}"),
            ),
            "TEAM_MEMBER_UPDATE" => by(
                format!("{a} updated membership for {member}"),
                format!("Membership was updated for {member}"),
            ),
            "TEAM_MEMBER_REMOVE" => by(
                format!("{a} removed {member}"),
                format!("{member} was removed from the team"),
            ),
            "AUTOMATION_CREATE" => by(
                format!("{a} created automation “{title}”"),
                format!("Someone created automation “{title}”"),
            ),
            "AUTOMATION_UPDATE" => by(
                format!("{a} updated automation “{title}”"),
                format!("Automation “{title}” updated"),
            ),
            "AUTOMATION_DELETE" => by(
                format!("{a} deleted automation “{title}”"),
                format!("Someone deleted automation “{title}”"),
            ),
            "PLUGIN_INSTALL" => by(
                format!("{a} installed plugin “{plugin}”"),
                format!("Someone installed plugin “{plugin}”"),
            ),
            "PLUGIN_UNINSTALL" => by(
                format!("{a} uninstalled plugin “{plugin}”"),
                format!("Someone uninstalled plugin “{plugin}”"),
            ),
            "CONNECTION_CREATE" => by(
                format!("{a} added a team connection for {app}"),
                format!("Someone added a team connection for {app}"),
            ),
            "CONNECTION_DELETE" => by(
                format!("{a} removed connection “{connection}” for {app} from the team"),
                format!("Connection “{connection}” for {app} was removed from the team"),
            ),
            "WORKSPACE_CONNECTION_ENABLE" => by(
                format!("{a} enabled a workspace connection for {app}"),
                format!("Someone enabled a workspace connection for {app}"),
            ),
            "WORKSPACE_CONNECTION_DISABLE" => by(
                format!("{a} removed workspace connection “{connection}” for {app} from the team"),
                format!("Workspace connection “{connection}” for {app} was removed from the team"),
            ),
            _ => return None,
        };
        let changes = data["changes"].as_array().into_iter().flatten();
        let details = match kind {
            "TEAM_MEMBERS_ADD" => {
                let names: Vec<&str> = data["members"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|m| name(m, "principal_id"))
                    .collect();
                vec![conjunction(&names)]
            }
            "TEAM_UPDATE" => changes.filter_map(team_change).collect(),
            "AUTOMATION_UPDATE" => changes
                .filter(|c| c["field"] != "timing_mode")
                .map(automation_change)
                .collect(),
            _ => Vec::new(),
        };
        Some(Self {
            id: item["id"].as_str()?.to_owned(),
            created_at: item["created_at"].as_str()?.to_owned(),
            message,
            details,
        })
    }
}

/// `display_name || <fallback>`: the first non-empty one.
fn name<'a>(value: &'a Value, fallback: &str) -> Option<&'a str> {
    let text = |v: &'a Value| v.as_str().filter(|s| !s.is_empty());
    text(&value["display_name"]).or_else(|| text(&value[fallback]))
}

/// `formatList(…, {type: "conjunction"})` in English: `A`, `A and B`, `A, B, and C`.
fn conjunction(names: &[&str]) -> String {
    match names {
        [] => String::new(),
        [one] => (*one).to_owned(),
        [a, b] => format!("{a} and {b}"),
        [rest @ .., last] => format!("{}, and {last}", rest.join(", ")),
    }
}

/// `qn`: a name, description or avatar change of the team.
fn team_change(change: &Value) -> Option<String> {
    let (old, new) = (&change["old_value"], &change["new_value"]);
    let (label, old, new) = match change["field"].as_str()? {
        "avatar" => {
            let id = |v: &Value| match v {
                Value::Null => Some("null".to_owned()),
                v => v["avatar_id"].as_str().map(str::to_owned),
            };
            ("Avatar", id(old)?, id(new)?)
        }
        field @ ("name" | "description") => (
            if field == "name" {
                "Name"
            } else {
                "Description"
            },
            old.as_str()?.to_owned(),
            new.as_str()?.to_owned(),
        ),
        _ => return None,
    };
    Some(format!("{label}: “{old}” → “{new}”"))
}

/// `Wn` with `automationChange` / `promptChange`.
// shortcut: schedule changes print the raw values; the original's localized cadence (`Cn`) is a later slice.
fn automation_change(change: &Value) -> String {
    let (old, new) = (short(&change["old_value"]), short(&change["new_value"]));
    let field = change["field"].as_str().unwrap_or_default();
    if field == "prompt" {
        return format!("Prompt: {old} → {new}");
    }
    let label = match field {
        "title" => "Title",
        "schedule" => "Schedule",
        "default_timezone" => "Time zone",
        "is_enabled" => "Enabled",
        "executor" => "Execution environment",
        "thread_mode" => "Thread mode",
        "model" => "Model",
        "reasoning_effort" => "Reasoning effort",
        "notification_policy" => "Notification policy",
        "notifications_enabled" => "Notifications enabled",
        "email_enabled" => "Email enabled",
        "target_thread_id" => "Target thread",
        "safe_urls" => "URLs",
        _ => "Setting",
    };
    format!("{label}: “{old}” → “{new}”")
}

/// `Kn`: the value as text with whitespace collapsed, cut to 160 characters with `…`.
fn short(value: &Value) -> String {
    let text = match value {
        Value::String(s) => s.clone(),
        v => v.to_string(),
    };
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if text.chars().count() <= 160 {
        return text;
    }
    text.chars().take(159).chain(['…']).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn message(actor: Value, data: Value) -> Option<Activity> {
        Activity::from_value(&json!({
            "id": "a1", "created_at": "2026-11-25T09:00:00Z", "actor": actor, "activity_data": data,
        }))
    }

    #[test]
    fn members_add_counts_others_and_lists_names() {
        let a = message(
            json!({"id": "u1", "display_name": "Ada", "type": "user"}),
            json!({"type": "TEAM_MEMBERS_ADD", "total_count": 3, "members": [
                {"principal_id": "p1", "display_name": "Bo"},
                {"principal_id": "p2"},
                {"principal_id": "p3", "display_name": "Cy"},
            ]}),
        )
        .unwrap();
        assert_eq!(a.message, "Ada added Bo and 2 others");
        assert_eq!(a.details, ["Bo, p2, and Cy"]);
        let a = message(
            Value::Null,
            json!({"type": "TEAM_MEMBERS_ADD", "total_count": 1,
            "members": [{"principal_id": "p1", "display_name": "Bo"}]}),
        )
        .unwrap();
        assert_eq!(a.message, "Bo was added to the team");
    }

    #[test]
    fn self_join_owner_transfer_and_unknown_types() {
        let joined = message(
            json!({"id": "u1", "display_name": "Ada", "type": "user"}),
            json!({"type": "TEAM_MEMBER_ADD", "principal_type": "user", "principal_id": "u1"}),
        )
        .unwrap();
        assert_eq!(joined.message, "Ada joined the team");
        let owner = message(
            Value::Null,
            json!({"type": "TEAM_MEMBER_UPDATE", "role": "owner", "principal_id": "u2", "display_name": ""}),
        )
        .unwrap();
        assert_eq!(owner.message, "Team ownership was transferred to u2");
        assert_eq!(message(Value::Null, json!({"type": "SOMETHING_NEW"})), None);
    }

    #[test]
    fn team_and_automation_changes() {
        let a = message(
            json!({"id": "u1"}),
            json!({"type": "TEAM_UPDATE", "changes": [
                {"field": "name", "old_value": "A", "new_value": "B"},
                {"field": "avatar", "old_value": null, "new_value": {"avatar_id": "fox"}},
                {"field": "other", "old_value": 1, "new_value": 2},
            ]}),
        )
        .unwrap();
        assert_eq!(a.message, "u1 updated the team");
        assert_eq!(a.details, ["Name: “A” → “B”", "Avatar: “null” → “fox”"]);
        let a = message(
            Value::Null,
            json!({"type": "AUTOMATION_UPDATE", "title": "Nightly", "changes": [
                {"field": "timing_mode", "old_value": "a", "new_value": "b"},
                {"field": "is_enabled", "old_value": true, "new_value": false},
                {"field": "prompt", "old_value": "x\n  y", "new_value": "z".repeat(200)},
            ]}),
        )
        .unwrap();
        assert_eq!(a.message, "Automation “Nightly” updated");
        assert_eq!(a.details[0], "Enabled: “true” → “false”");
        assert!(a.details[1].starts_with("Prompt: x y → zzz"));
        assert_eq!(
            a.details[1].chars().count(),
            "Prompt: x y → ".chars().count() + 160
        );
    }
}
