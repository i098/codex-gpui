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

impl Root {
    #[cfg(target_os = "windows")]
    pub(super) fn check_windows_sandbox_readiness(&mut self, cx: &mut Context<Self>) {
        let bridge = self.bridge.clone();
        cx.spawn(async move |root, cx| {
            let result = bridge
                .request::<_, Value>("windowsSandbox/readiness", json!({}))
                .await;
            root.update(cx, |root, cx| match result {
                Ok(response) => match windows_sandbox_setup_needed(response["status"].as_str()) {
                    Some(needed) => root.set_windows_sandbox_state(needed, false, cx),
                    None => root.toast(
                        Toast::danger(format!(
                            "Unknown Windows sandbox readiness: {}",
                            response["status"]
                        )),
                        cx,
                    ),
                },
                Err(error) => root.toast(
                    Toast::danger(format!(
                        "Could not check Windows sandbox readiness: {error}"
                    )),
                    cx,
                ),
            })
            .ok();
        })
        .detach();
    }

    #[cfg(target_os = "windows")]
    pub(super) fn start_windows_sandbox_setup(
        &mut self,
        mode: &'static str,
        cx: &mut Context<Self>,
    ) {
        if self.home.read(cx).windows_sandbox_setup_pending {
            return;
        }
        self.set_windows_sandbox_state(true, true, cx);
        let bridge = self.bridge.clone();
        let cwd = self.state.project_root.clone();
        cx.spawn(async move |root, cx| {
            let result = bridge
                .request::<_, Value>(
                    "windowsSandbox/setupStart",
                    json!({"mode": mode, "cwd": cwd}),
                )
                .await;
            root.update(cx, |root, cx| match result {
                Ok(response) if response["started"] == true => {}
                Ok(_) => {
                    root.set_windows_sandbox_state(true, false, cx);
                    root.toast(Toast::danger("Windows sandbox setup did not start"), cx);
                }
                Err(error) => {
                    root.set_windows_sandbox_state(true, false, cx);
                    root.toast(
                        Toast::danger(format!("Could not start Windows sandbox setup: {error}")),
                        cx,
                    );
                }
            })
            .ok();
        })
        .detach();
    }

    #[cfg(target_os = "windows")]
    pub(super) fn windows_sandbox_setup_completed(
        &mut self,
        params: &Value,
        cx: &mut Context<Self>,
    ) {
        if !self.home.read(cx).windows_sandbox_setup_pending {
            return;
        }
        let success = params["success"].as_bool().unwrap_or(false);
        self.set_windows_sandbox_state(!success, false, cx);
        if success {
            self.toast(Toast::info("Windows sandbox setup completed"), cx);
        } else {
            let error = params["error"]
                .as_str()
                .unwrap_or("No error details were provided");
            self.toast(
                Toast::danger(format!("Windows sandbox setup failed: {error}")),
                cx,
            );
        }
    }

    #[cfg(target_os = "windows")]
    fn set_windows_sandbox_state(&mut self, needed: bool, pending: bool, cx: &mut Context<Self>) {
        self.home.update(cx, |home, cx| {
            home.windows_sandbox_setup_needed = needed;
            home.windows_sandbox_setup_pending = pending;
            cx.notify();
        });
    }
}

#[cfg(any(target_os = "windows", test))]
fn windows_sandbox_setup_needed(status: Option<&str>) -> Option<bool> {
    match status {
        Some("ready") => Some(false),
        Some("notConfigured" | "updateRequired") => Some(true),
        _ => None,
    }
}

#[cfg(test)]
mod windows_sandbox_tests {
    use super::*;

    #[test]
    fn readiness_only_shows_setup_for_incomplete_states() {
        assert_eq!(windows_sandbox_setup_needed(Some("ready")), Some(false));
        assert_eq!(
            windows_sandbox_setup_needed(Some("notConfigured")),
            Some(true)
        );
        assert_eq!(
            windows_sandbox_setup_needed(Some("updateRequired")),
            Some(true)
        );
        assert_eq!(windows_sandbox_setup_needed(None), None);
        assert_eq!(windows_sandbox_setup_needed(Some("unknown")), None);
    }
}
