use super::*;
use gpui::SystemNotification;
use regex::Regex;
use std::sync::LazyLock;

static HEARTBEAT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?is)(?:```(?:xml)?\s*)?<heartbeat>(.*?)</heartbeat>(?:\s*```)?").unwrap()
});
static DECISION: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)<decision>\s*(NOTIFY|DONT_NOTIFY)\s*</decision>").unwrap());
static MESSAGE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?is)<message>\s*(.*?)\s*</message>").unwrap());
static SKIPPED: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^::\s*SKIP_COMPLETION\s*::$").unwrap());

fn body(message: &str, completed: bool, failed_only: bool) -> Option<String> {
    if failed_only && completed {
        return None;
    }
    let heartbeat = HEARTBEAT.captures_iter(message).last();
    let decision = heartbeat
        .as_ref()
        .and_then(|h| DECISION.captures(h.get(1)?.as_str()));
    if !failed_only
        && decision
            .as_ref()
            .is_some_and(|d| d[1].eq_ignore_ascii_case("DONT_NOTIFY"))
    {
        return None;
    }
    if decision.is_none() {
        return completion_body(message, completed);
    }
    if let Some(text) = heartbeat
        .as_ref()
        .and_then(|h| MESSAGE.captures(h.get(1)?.as_str()))
    {
        let text = unpad(text.get(1).unwrap().as_str()).trim();
        if !text.is_empty() {
            return completion_body(text, completed);
        }
    }
    completion_body(HEARTBEAT.replace_all(message, "").trim(), completed)
}

fn unpad(text: &str) -> &str {
    text.strip_prefix("<![CDATA[")
        .and_then(|text| text.strip_suffix("]]>"))
        .unwrap_or(text)
}

fn completion_body(text: &str, completed: bool) -> Option<String> {
    let skipped = SKIPPED.is_match(unpad(text.trim()).trim());
    if skipped && completed {
        return None;
    }
    Some(
        if skipped || text.trim().is_empty() {
            "ChatGPT finished a turn."
        } else {
            text
        }
        .to_owned(),
    )
}

impl Root {
    pub(super) fn refresh_automation_unreads(&mut self, cx: &mut Context<Self>) {
        let host = self.host.clone();
        let source = host.clone();
        let read = cx.background_spawn(async move {
            host.fetch(&host::Request::post(
                "inbox-items",
                Some(json!({"limit": 0})),
            ))
            .and_then(|response| host::json_body(&response))
        });
        self.inbox_load = Some(cx.spawn(async move |this, cx| {
            let result = read.await;
            this.update(cx, |this, cx| {
                if !Arc::ptr_eq(&source, &this.host) {
                    return;
                }
                match result {
                    Ok(inbox) => {
                        this.unread_automation_threads = inbox["unreadRunCounts"]["unreadRuns"]
                            .as_array()
                            .into_iter()
                            .flatten()
                            .filter_map(|run| run["threadId"].as_str().map(str::to_owned))
                            .collect();
                        this.sync_automation_attention(cx);
                    }
                    Err(error) => {
                        eprintln!("codex-gpui: failed to read automation unreads: {error:#}")
                    }
                }
            })
            .ok();
        }));
    }

    pub(super) fn sync_automation_attention(&self, cx: &mut Context<Self>) {
        let unread = !self.unread_automation_threads.is_empty();
        for shell in std::iter::once(&self.shell).chain(self.page.as_ref()) {
            shell.update(cx, |shell, cx| {
                shell.scheduled_attention = unread;
                cx.notify();
            });
        }
    }

    pub(super) fn read_automation_run(&mut self, id: &str, cx: &mut Context<Self>) {
        cx.dismiss_system_notification(&format!("automation:{id}"));
        let host = self.host.clone();
        let id = id.to_owned();
        let read = cx.background_spawn({
            let host = host.clone();
            async move { host.mark_automation_run_read(&id) }
        });
        cx.spawn(async move |this, cx| {
            let result = read.await;
            this.update(cx, |this, cx| {
                if !Arc::ptr_eq(&host, &this.host) {
                    return;
                }
                if let Err(error) = result {
                    eprintln!("codex-gpui: failed to mark automation run read: {error:#}");
                }
                this.refresh_automation_unreads(cx);
            })
            .ok();
        })
        .detach();
    }

    pub(super) fn archive_automation_run(&mut self, id: &str, cx: &mut Context<Self>) {
        cx.dismiss_system_notification(&format!("automation:{id}"));
        let host = self.host.clone();
        let id = id.to_owned();
        let write = cx.background_spawn({
            let host = host.clone();
            async move { host.archive_automation_run(&id) }
        });
        cx.spawn(async move |this, cx| {
            let result = write.await;
            this.update(cx, |this, cx| {
                if !Arc::ptr_eq(&host, &this.host) {
                    return;
                }
                if let Err(error) = result {
                    eprintln!("codex-gpui: failed to archive automation run metadata: {error:#}");
                }
                this.refresh_automation_unreads(cx);
            })
            .ok();
        })
        .detach();
    }

    pub(super) fn complete_automation_run(&mut self, params: &Value, cx: &mut Context<Self>) {
        let Some(id) = params["threadId"].as_str() else {
            return;
        };
        let turn = params["turn"].clone();
        let cached = self
            .last_agent_messages
            .remove(id)
            .filter(|(turn_id, _)| turn["id"] == turn_id.as_str());
        let message = turn["items"]
            .as_array()
            .and_then(|items| {
                items
                    .iter()
                    .rev()
                    .find(|item| item["type"] == "agentMessage")
                    .and_then(|item| item["text"].as_str())
                    .map(str::to_owned)
            })
            .or_else(|| cached.map(|(_, message)| message))
            .unwrap_or_default();
        let title = self
            .threads
            .iter()
            .find(|thread| thread.id == id)
            .map(thread_title);
        let host = self.host.clone();
        let id = id.to_owned();
        let read = cx.background_spawn({
            let (host, id) = (host.clone(), id.clone());
            async move {
                // The run owner persists completion and archive results before this read.
                let Some(failed_only) = host.finish_automation_run(&id, &turn)? else {
                    return Ok(None);
                };
                let inbox = host::json_body(&host.fetch(&host::Request::post(
                    "inbox-items",
                    Some(json!({"threadId": id})),
                ))?)?;
                let Some(_) = inbox["items"].as_array().and_then(|items| items.first()) else {
                    return Ok(None);
                };
                let notice =
                    body(&message, turn["status"] == "completed", failed_only).map(|body| {
                        SystemNotification {
                            tag: format!("automation:{id}").into(),
                            title: title.unwrap_or_else(|| "Turn complete".into()),
                            body: body.into(),
                            actions: Vec::new(),
                            reply_placeholder: None,
                        }
                    });
                Ok::<_, anyhow::Error>(Some(notice))
            }
        });
        cx.spawn(async move |this, cx| {
            let result = read.await;
            this.update(cx, |this, cx| {
                if !Arc::ptr_eq(&host, &this.host) {
                    return;
                }
                let notice = match result {
                    Ok(Some(notice)) => {
                        this.refresh_automation_unreads(cx);
                        this.refresh_threads(cx);
                        let Some(notice) = notice else { return };
                        notice
                    }
                    Ok(None) => return,
                    Err(error) => {
                        eprintln!(
                            "codex-gpui: failed to finish automation notification: {error:#}"
                        );
                        return;
                    }
                };
                let focused = this
                    .context_window
                    .update(cx, |_, window, _| window.is_window_active())
                    .unwrap_or(true);
                let enabled = cx
                    .try_global::<host_settings::Settings>()
                    .map(|settings| {
                        match settings
                            .read_all()
                            .get("notifications-turn-mode")
                            .and_then(Value::as_str)
                        {
                            Some("off") => false,
                            Some("always") => true,
                            _ => !focused,
                        }
                    })
                    .unwrap_or(!focused);
                if enabled {
                    cx.show_system_notification(notice);
                }
            })
            .ok();
        })
        .detach();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(target_os = "macos")]
    use std::io::Write as _;
    #[cfg(target_os = "macos")]
    use std::sync::atomic::{AtomicBool, Ordering};

    #[cfg(target_os = "macos")]
    struct NoticeFixture {
        finished: AtomicBool,
        unread: AtomicBool,
    }

    #[cfg(target_os = "macos")]
    impl host::Host for NoticeFixture {
        fn finish_automation_run(&self, id: &str, _: &Value) -> Result<Option<bool>> {
            Ok((id == "owned" && !self.finished.swap(true, Ordering::SeqCst)).then_some(false))
        }

        fn mark_automation_run_read(&self, _: &str) -> Result<bool> {
            Ok(self.unread.swap(false, Ordering::SeqCst))
        }

        fn fetch(&self, _: &host::Request) -> Result<Value> {
            let unread = self.finished.load(Ordering::SeqCst) && self.unread.load(Ordering::SeqCst);
            let body = json!({
                "items": [{"threadId": "owned", "automationId": "task", "automationName": "Review", "status": "PENDING_REVIEW"}],
                "unreadRunCounts": {"unreadRuns": if unread { json!([{"threadId": "owned"}]) } else { json!([]) }}
            });
            Ok(
                json!({"responseType": "success", "status": 200, "bodyJsonString": body.to_string()}),
            )
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn background_completion_notifies_once_and_click_reads_the_run() {
        let mut cx = gpui::HeadlessAppContext::with_asset_source(
            textsys::text_system(),
            Arc::new(theme::Assets),
        );
        cx.allow_parking();
        cx.update(|cx| {
            theme::load_fonts(cx);
            cx.set_global(Theme::dark());
            init(cx);
        });
        let script = ["readback.jsonl", "sessions/thread-settings.jsonl"]
            .map(|file| {
                fs::read_to_string(
                    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                        .join("../../fixtures")
                        .join(file),
                )
                .unwrap()
            })
            .join("\n");
        let mut script: Vec<Value> = script
            .lines()
            .filter(|line| !line.trim().is_empty())
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        let mut owned = script
            .iter()
            .find_map(|line| line["msg"]["result"].get("thread").cloned())
            .unwrap();
        owned["id"] = json!("owned");
        owned["sessionId"] = json!("owned");
        owned["name"] = json!("Review");
        owned["preview"] = json!("Review changes");
        owned["turns"] = json!([]);
        let mut resumed = script
            .iter()
            .find_map(|line| {
                let result = &line["msg"]["result"];
                result["collaborationMode"]
                    .is_object()
                    .then(|| result.clone())
            })
            .unwrap();
        resumed["thread"] = owned.clone();
        let list = script
            .iter()
            .find(|line| line["msg"]["method"] == "thread/list")
            .unwrap()
            .clone();
        let mut listed = script
            .iter()
            .find(|line| line["dir"] == "recv" && line["msg"]["id"] == list["msg"]["id"])
            .unwrap()
            .clone();
        listed["msg"]["result"]["data"]
            .as_array_mut()
            .unwrap()
            .push(owned.clone());
        let index = script
            .iter()
            .position(|line| line["dir"] == "recv" && line["msg"]["id"] == list["msg"]["id"])
            .unwrap();
        script[index] = listed.clone();
        for _ in 0..3 {
            script.extend([list.clone(), listed.clone()]);
        }
        script.extend([
            json!({"dir": "send", "msg": {"id": 90001, "method": "thread/read", "params": {"threadId": "owned", "includeTurns": true}}}),
            json!({"dir": "recv", "msg": {"id": 90001, "result": {"thread": owned}}}),
            json!({"dir": "send", "msg": {"id": 90002, "method": "thread/resume", "params": {"threadId": "owned", "excludeTurns": true}}}),
            json!({"dir": "recv", "msg": {"id": 90002, "result": resumed}}),
        ]);
        let mut recording = tempfile::NamedTempFile::new().unwrap();
        for line in script {
            writeln!(recording, "{line}").unwrap();
        }
        let (bridge, events) = Bridge::replay(recording.path()).unwrap();
        let window = cx
            .open_window(gpui::size(px(1280.), px(832.)), |window, cx| {
                cx.new(|cx| Root::new(State::default(), bridge, events, "/tmp".into(), window, cx))
            })
            .unwrap();
        let root = cx.read_window(&window, |root, _| root).unwrap();
        let pump = |cx: &mut gpui::HeadlessAppContext,
                    done: &dyn Fn(&gpui::HeadlessAppContext) -> bool| {
            let started = std::time::Instant::now();
            loop {
                cx.run_until_parked();
                if done(cx) {
                    break;
                }
                assert!(
                    started.elapsed() < Duration::from_secs(30),
                    "owned runtime timed out"
                );
                std::thread::sleep(Duration::from_millis(10));
            }
        };
        let host = Arc::new(NoticeFixture {
            finished: AtomicBool::new(false),
            unread: AtomicBool::new(true),
        });
        root.update(&mut cx, |root, cx| root.set_host(host, cx));
        pump(&mut cx, &|cx| root.read_with(cx, |root, _| root.ready()));
        cx.update(|cx| {
            cx.remove_global::<host_settings::Settings>();
        });
        let completion = json!({"threadId": "owned", "turn": {"id": "first", "status": "completed", "items": []}});
        root.update(&mut cx, |root, cx| {
            root.on_event(Event::Notification {
                method: "item/completed".into(),
                params: json!({"threadId": "owned", "turnId": "first", "item": {"type": "agentMessage", "text": "Review is ready."}}),
            }, cx);
            root.complete_automation_run(&completion, cx);
        });
        pump(&mut cx, &|cx| {
            root.read_with(cx, |root, cx| root.shell().read(cx).scheduled_attention)
        });
        let notices = cx.shown_system_notifications();
        assert_eq!(
            notices
                .iter()
                .map(|notice| notice.body.as_ref())
                .collect::<Vec<_>>(),
            ["Review is ready."]
        );
        assert!(root.read_with(&cx, |root, cx| root.shell().read(cx).scheduled_attention));
        assert!(!cx.update(|cx| window.is_active(cx)).unwrap_or(false));
        root.update(&mut cx, |root, cx| {
            root.complete_automation_run(&completion, cx)
        });
        cx.run_until_parked();
        assert_eq!(cx.shown_system_notifications(), notices);
        cx.simulate_system_notification_response(gpui::SystemNotificationResponse {
            tag: notices[0].tag.clone(),
            action_id: None,
        });
        pump(&mut cx, &|cx| {
            root.read_with(cx, |root, cx| {
                root.route() == &Route::Thread("owned".into())
                    && root
                        .thread
                        .as_ref()
                        .is_some_and(|view| view.read(cx).id() == "owned")
                    && !root.shell().read(cx).scheduled_attention
            })
        });
        let target = root.read_with(&cx, |root, _| root.route().clone());
        assert_eq!(target, Route::Thread("owned".into()));
        assert!(cx.delivered_system_notifications().is_empty());
        let other = cx
            .open_window(gpui::size(px(640.), px(600.)), |_, cx| {
                cx.new(|_| ui::EmptyMain)
            })
            .unwrap();
        cx.update_window(other.into(), |_, window, _| window.activate_window())
            .unwrap();
        cx.run_until_parked();
        assert!(!cx.update(|cx| window.is_active(cx)).unwrap_or(false));
        for mode in ["off", "always"] {
            cx.update(|cx| {
                let mut settings = host_settings::Settings::default();
                settings
                    .write("notifications-turn-mode", json!(mode))
                    .unwrap();
                cx.set_global(settings);
            });
            if mode == "always" {
                cx.update_window(window.into(), |_, window, _| window.activate_window())
                    .unwrap();
                cx.run_until_parked();
                assert!(cx.update(|cx| window.is_active(cx)).unwrap_or(false));
            }
            root.update(&mut cx, |root, cx| {
                root.set_host(Arc::new(NoticeFixture {
                    finished: AtomicBool::new(false), unread: AtomicBool::new(true),
                }), cx);
                root.on_event(Event::Notification {
                    method: "item/completed".into(),
                    params: json!({"threadId": "owned", "turnId": "stale", "item": {"type": "agentMessage", "text": "Wrong turn"}}),
                }, cx);
                root.complete_automation_run(&json!({"threadId": "owned", "turn": {"id": "next", "status": "completed", "items": []}}), cx);
            });
            pump(&mut cx, &|cx| {
                root.read_with(cx, |root, cx| root.shell().read(cx).scheduled_attention)
            });
            let shown = cx.shown_system_notifications();
            if mode == "off" {
                assert_eq!(shown, notices);
            } else {
                assert_eq!(shown.len(), 2);
                assert_eq!(shown[1].body.as_ref(), "ChatGPT finished a turn.");
            }
        }
    }

    #[test]
    fn quiet_completions_and_failure_policy_follow_the_original() {
        let quiet = "Visible text\n```xml\n<heartbeat><decision>DONT_NOTIFY</decision><message>Hidden</message></heartbeat>\n```";
        assert_eq!(body(quiet, true, false), None);
        assert_eq!(body(quiet, false, true).as_deref(), Some("Hidden"));
        assert_eq!(body("Success", true, true), None);
        assert_eq!(body(":: SKIP_COMPLETION ::", true, false), None);
        assert_eq!(body("<![CDATA[ ::SKIP_COMPLETION:: ]]>", true, false), None);
        assert_eq!(
            body("::SKIP_COMPLETION::", false, false).as_deref(),
            Some("ChatGPT finished a turn.")
        );
        assert_eq!(
            body("  ", true, false).as_deref(),
            Some("ChatGPT finished a turn.")
        );
        assert_eq!(body("Done.", true, false).as_deref(), Some("Done."));
        assert_eq!(body("Visible\n<HEARTBEAT><DECISION>NOTIFY</DECISION><MESSAGE><![CDATA[ Alert ]]></MESSAGE></HEARTBEAT>", true, false).as_deref(), Some("Alert"));
    }
}
