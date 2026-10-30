use super::*;

pub(super) async fn list_profiles(bridge: &Bridge, cwd: &str) -> Result<Value> {
    let mut data = Vec::new();
    let mut cursor = Value::Null;
    loop {
        let page: Value = bridge
            .request(
                "permissionProfile/list",
                json!({"cwd": cwd, "cursor": cursor}),
            )
            .await?;
        data.extend(
            page["data"]
                .as_array()
                .context("Missing permission profiles")?
                .iter()
                .cloned(),
        );
        cursor = page["nextCursor"].clone();
        if cursor.is_null() {
            break;
        }
    }
    Ok(json!({"data": data}))
}

fn assessment(params: &Value) -> Value {
    let mut action = params["action"].clone();
    if let Some(fields) = action.as_object_mut() {
        for (wire, protocol) in [
            ("approvalId", "approval_id"),
            ("processId", "process_id"),
            ("toolName", "tool_name"),
            ("toolTitle", "tool_title"),
            ("connectorId", "connector_id"),
            ("connectorName", "connector_name"),
        ] {
            if let Some(value) = fields.remove(wire) {
                fields.insert(protocol.into(), value);
            }
        }
        for key in ["type", "source", "protocol"] {
            if let Some(value) = fields.get_mut(key) {
                let mapped = match value.as_str() {
                    Some("writeStdin") => Some("write_stdin"),
                    Some("applyPatch") => Some("apply_patch"),
                    Some("networkAccess") => Some("network_access"),
                    Some("mcpToolCall") => Some("mcp_tool_call"),
                    Some("requestPermissions") => Some("request_permissions"),
                    Some("unifiedExec") => Some("unified_exec"),
                    Some("socks5Tcp") => Some("socks5_tcp"),
                    Some("socks5Udp") => Some("socks5_udp"),
                    _ => None,
                };
                if let Some(mapped) = mapped {
                    *value = mapped.into();
                }
            }
        }
        if let Some(permissions) = fields.get_mut("permissions").and_then(Value::as_object_mut)
            && let Some(files) = permissions.remove("fileSystem")
        {
            permissions.insert("file_system".into(), files);
        }
    }
    json!({
        "id": params["reviewId"], "turn_id": params["turnId"],
        "target_item_id": params["targetItemId"], "started_at_ms": params["startedAtMs"],
        "completed_at_ms": params["completedAtMs"], "status": params["review"]["status"],
        "risk_level": params["review"]["riskLevel"],
        "user_authorization": params["review"]["userAuthorization"],
        "rationale": params["review"]["rationale"], "decision_source": params["decisionSource"],
        "action": action,
    })
}

impl Root {
    pub(super) fn guardian_review(&mut self, method: &str, params: &Value, cx: &mut Context<Self>) {
        let Some(thread) = params["threadId"].as_str() else {
            return;
        };
        if method == "item/autoApprovalReview/started" {
            self.toast(Toast::info("Reviewing action permissions"), cx);
        } else if params["review"]["status"] == "denied" {
            self.guardian_denials
                .insert(thread.to_owned(), assessment(params));
            self.toast(
                Toast::danger("Action denied. Use /approve to allow one retry."),
                cx,
            );
        } else {
            let status = params["review"]["status"].as_str().unwrap_or("completed");
            self.toast(Toast::info(format!("Permission review: {status}")), cx);
        }
    }

    pub(super) fn approve_denied_action(&mut self, cx: &mut Context<Self>) {
        let Route::Thread(thread) = &self.state.route else {
            return;
        };
        let thread = thread.clone();
        let Some(event) = self.guardian_denials.remove(&thread) else {
            self.toast(Toast::info("No denied action is available to retry"), cx);
            return;
        };
        let bridge = self.bridge.clone();
        cx.spawn(async move |this, cx| {
            let result = bridge
                .request::<_, Value>(
                    "thread/approveGuardianDeniedAction",
                    json!({
                        "threadId": thread, "event": event,
                    }),
                )
                .await;
            this.update(cx, |this, cx| match result {
                Ok(_) => this.toast(Toast::info("One retry approved"), cx),
                Err(error) => {
                    this.guardian_denials.entry(thread).or_insert(event);
                    this.toast(
                        Toast::danger(format!("Could not approve retry: {error}")),
                        cx,
                    );
                }
            })
            .ok();
        })
        .detach();
    }
}
