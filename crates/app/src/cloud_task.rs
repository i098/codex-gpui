use crate::{
    Root, Route,
    host::{Host, Identity, Request, json_body, response_wire},
};
use anyhow::{Context as _, Result, bail, ensure};
use base64::Engine as _;
use bridge::types::{Thread, ThreadItem, ThreadStatus, Turn, TurnStatus, UserInput};
use gpui::{AppContext as _, Context, Entity, SharedString, Task, Window};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, HashSet},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use ui::{
    composer::{Composer, Submission},
    screens::cloud_task::{
        ApplyResult, CloudAction, CloudTask, CloudTaskPage, CloudTurn, LocalTarget, Phase,
    },
    toast::Toast,
};

pub(crate) struct Session {
    pub page: Entity<CloudTaskPage>,
    pub header: Entity<ui::screens::cloud_task::CloudTaskHeader>,
    cancelled: Arc<AtomicBool>,
    _task: Task<()>,
}

impl Drop for Session {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::Release);
    }
}

impl Route {
    pub fn remote_path(path: &str) -> Option<Self> {
        let (id, hotkey) = path
            .strip_prefix("/hotkey-window/remote/")
            .map(|id| (id, true))
            .or_else(|| path.strip_prefix("/remote/").map(|id| (id, false)))?;
        let id = id.split(['?', '#']).next()?;
        if id.is_empty() || id.contains('/') {
            return None;
        }
        Some(Self::Remote {
            task_id: id.to_owned(),
            hotkey,
        })
    }
}

enum Update {
    Snapshot(CloudTask, Option<LocalTarget>),
    Event(String, Value),
    Failed(String),
    Unavailable,
}

impl Root {
    pub fn open_remote_task(
        &mut self,
        task_id: String,
        hotkey: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.navigate(Route::Remote { task_id, hotkey }, window, cx);
    }

    pub fn remote_page(&self) -> Option<&Entity<CloudTaskPage>> {
        self.cloud.as_ref().map(|session| &session.page)
    }

    pub(crate) fn load_cloud_task(
        &mut self,
        task_id: String,
        hotkey: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let root = cx.weak_entity();
        let composer = cx.new(|cx| {
            let mut composer = Composer::thread(window, cx);
            composer.on_submit(move |submission, window, cx| {
                root.update(cx, |root, cx| {
                    root.cloud_follow_up(submission.clone(), window, cx)
                })
                .ok();
            });
            composer
        });
        composer.update(cx, |composer, cx| {
            composer.set_models(&self.models, &self.config, cx)
        });
        let page = cx.new(|cx| CloudTaskPage::new(composer, hotkey, cx));
        cx.subscribe_in(
            &page,
            window,
            |root, _, action: &CloudAction, window, cx| match action {
                CloudAction::Retry => {
                    root.sync(window, cx);
                }
                CloudAction::Apply { revert } => root.apply_cloud_changes(*revert, cx),
                CloudAction::LoadImage(pointer) => root.load_cloud_image(pointer.clone(), cx),
                CloudAction::OpenMain => {
                    if let Route::Remote { task_id, .. } = &root.state.route {
                        root.open_remote_task(task_id.clone(), false, window, cx);
                    }
                }
                CloudAction::Cancel => root.cancel_cloud_task(cx),
            },
        )
        .detach();
        let (send, receive) = async_channel::bounded(128);
        let host = self.host.clone();
        let cancelled = Arc::new(AtomicBool::new(false));
        let stop = cancelled.clone();
        cx.background_spawn(async move {
            let result = run(&*host, &task_id, &stop, &send);
            if let Err(error) = result {
                let _ = send.send_blocking(Update::Failed(error.to_string()));
            }
        })
        .detach();
        let entity = page.clone();
        let task = cx.spawn_in(window, async move |root, cx| {
            while let Ok(update) = receive.recv().await {
                if root.update_in(cx, |root, window, cx| {
                    if !root.cloud.as_ref().is_some_and(|session| session.page == entity) { return; }
                    match update {
                        Update::Snapshot(data, target) => entity.update(cx, |page, cx| { page.target = target; page.set_data(data, cx); }),
                        Update::Event(id, event) => entity.update(cx, |page, cx| page.apply_event(&id, event, cx)),
                        Update::Failed(error) => entity.update(cx, |page, cx| { page.error = Some(error); cx.notify(); }),
                        Update::Unavailable => {
                            root.navigate(Route::Home, window, cx);
                            root.toast(Toast::info("You don’t have access to this Codex chat. Make sure you’re signed in to the right account and workspace."), cx);
                        }
                    }
                }).is_err() { break; }
            }
        });
        let header = cx.new(|cx| ui::screens::cloud_task::CloudTaskHeader::new(page.clone(), cx));
        self.cloud = Some(Session {
            page,
            header,
            cancelled,
            _task: task,
        });
    }

    pub(crate) fn load_cloud_environment(
        &mut self,
        config_id: String,
        repositories: Vec<Value>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let connection = Arc::new(std::sync::OnceLock::<(Arc<bridge::Bridge>, Identity)>::new());
        let send_connection = connection.clone();
        let host = self.host.clone();
        let root = cx.weak_entity();
        let composer = cx.new(|cx| {
            let mut composer = Composer::thread(window, cx);
            composer.on_submit(move |submission, window, cx| {
                let Some((bridge, identity)) = send_connection.get().cloned() else {
                    root.update(cx, |root, cx| {
                        if let Some(page) = root.remote_page().cloned() {
                            page.update(cx, |page, cx| {
                                page.composer.update(cx, |composer, cx| {
                                    composer.restore_submission(&submission.text, cx);
                                    composer.set_running(false, cx);
                                });
                            });
                        }
                        root.toast(Toast::danger("The cloud setup connection is not ready"), cx);
                    })
                    .ok();
                    return;
                };
                let host = host.clone();
                let submission = submission.clone();
                root.update(cx, |root, cx| {
                    let Some(page) = root.remote_page().cloned() else {
                        return;
                    };
                    let Some(thread_id) = page.read(cx).data.as_ref().map(|data| data.id.clone())
                    else {
                        return;
                    };
                    cx.spawn_in(window, async move |root, cx| {
                        let result = async {
                            ensure!(
                                host.identity().as_ref() == Some(&identity),
                                "Your account changed during cloud setup"
                            );
                            let input = crate::user_input(&submission);
                            bridge
                                .request::<_, Value>(
                                    "turn/start",
                                    json!({
                                        "threadId":thread_id,"input":input,
                                        "model":submission.model,"effort":submission.effort,
                                        "serviceTier":submission.service_tier,
                                    }),
                                )
                                .await?;
                            Ok::<_, anyhow::Error>(())
                        }
                        .await;
                        if let Err(error) = result {
                            page.update(cx, |page, cx| {
                                page.composer.update(cx, |composer, cx| {
                                    composer.restore_submission(&submission.text, cx);
                                    composer.set_running(false, cx);
                                });
                            });
                            root.update(cx, |root, cx| {
                                root.toast(Toast::danger(error.to_string()), cx)
                            })
                            .ok();
                        }
                    })
                    .detach();
                })
                .ok();
            });
            composer.set_models(&self.models, &self.config, cx);
            composer.set_running(true, cx);
            composer
        });
        let page = cx.new(|cx| CloudTaskPage::new(composer, false, cx));
        let actions = connection.clone();
        cx.subscribe_in(
            &page,
            window,
            move |root, page, action: &CloudAction, window, cx| match action {
                CloudAction::Retry | CloudAction::OpenMain => root.sync(window, cx),
                CloudAction::Cancel => {
                    let Some((bridge, identity)) = actions.get().cloned() else {
                        return;
                    };
                    let Some(data) = &page.read(cx).data else {
                        return;
                    };
                    let thread_id = data.id.clone();
                    let host = root.host.clone();
                    cx.spawn(async move |root, cx| {
                        let result = async {
                            ensure!(
                                host.identity().as_ref() == Some(&identity),
                                "Your account changed during cloud setup"
                            );
                            let thread = bridge.thread_read(&thread_id, true).await?.thread;
                            let turn_id = &thread
                                .turns
                                .last()
                                .context("No cloud setup turn is running")?
                                .id;
                            bridge.turn_interrupt(&thread_id, turn_id).await
                        }
                        .await;
                        if let Err(error) = result {
                            root.update(cx, |root, cx| {
                                root.toast(Toast::danger(error.to_string()), cx)
                            })
                            .ok();
                        }
                    })
                    .detach();
                }
                CloudAction::Apply { revert } => root.apply_cloud_changes(*revert, cx),
                CloudAction::LoadImage(pointer) => root.load_cloud_image(pointer.clone(), cx),
            },
        )
        .detach();
        let host = self.host.clone();
        let local = self.bridge.clone();
        let settings = self.config.clone();
        let entity = page.clone();
        let cancelled = Arc::new(AtomicBool::new(false));
        let task = cx.spawn_in(window, async move |_, cx| {
            let result = async {
                let source = host.clone();
                let (bridge, config, identity) = cx.background_executor().spawn(async move {
                    let identity = source.identity().context("Cloud setup requires a ChatGPT account")?;
                    let config = crate::cloud::execute(&*source, &identity, &ui::screens::cloud::Operation::Config(config_id))?;
                    let credentials = (crate::host::app_server_credentials(local))(false)?
                        .context("Cloud setup requires a ChatGPT account")?;
                    let (bridge, _) = bridge::Bridge::connect_cloud(
                        bridge::CLOUD_ENDPOINT, &credentials.token, Some(&identity.account_id),
                    )?;
                    ensure!(source.identity().as_ref() == Some(&identity), "Your account changed during cloud setup");
                    Ok::<_, anyhow::Error>((Arc::new(bridge), config, identity))
                }).await?;
                let events = bridge.subscribe_async();
                let thread = environment_thread(&bridge, &identity, &config, repositories, &settings).await?;
                ensure!(host.identity().as_ref() == Some(&identity), "Your account changed during cloud setup");
                let thread_id = thread.id.clone();
                let working = thread.turns.last().is_some_and(|turn| turn.status == TurnStatus::InProgress);
                let title = format!("Set up {}", config["name"].as_str().unwrap_or("cloud environment"));
                connection.set((bridge, identity.clone())).ok();
                entity.update(cx, |page, cx| page.set_data(CloudTask {
                    id: thread_id.clone(), title, active_turn: Some(thread_id.clone()),
                    turns: vec![CloudTurn { id: thread_id.clone(), thread, phase: Phase::SettingUp, error: None, logs: Vec::new(), images: Vec::new() }],
                    diff_turn: None, diff: String::new(), clone_url: None,
                    environment_label: config["name"].as_str().map(str::to_owned), working,
                }, cx));
                while let Ok(event) = events.recv().await {
                    ensure!(host.identity().as_ref() == Some(&identity), "Your account changed during cloud setup");
                    match event {
                        bridge::Event::Notification { method, params }
                            if params["threadId"].as_str() == Some(&thread_id) => {
                                entity.update(cx, |page, cx| page.apply_event(
                                    &thread_id, json!({"item_type":"thread_event","event":{"method":method,"params":params}}), cx,
                                ));
                            }
                        bridge::Event::Closed => bail!("The cloud setup connection closed"),
                        _ => {}
                    }
                }
                Ok::<_, anyhow::Error>(())
            }.await;
            if let Err(error) = result {
                entity.update(cx, |page, cx| {
                    page.error = Some(error.to_string());
                    page.composer.update(cx, |composer, cx| composer.set_running(false, cx));
                    cx.notify();
                });
            }
        });
        let header = cx.new(|cx| ui::screens::cloud_task::CloudTaskHeader::new(page.clone(), cx));
        self.cloud = Some(Session {
            page,
            header,
            cancelled,
            _task: task,
        });
    }

    fn apply_cloud_changes(&mut self, revert: bool, cx: &mut Context<Self>) {
        let Some(session) = &self.cloud else {
            return;
        };
        let page = session.page.clone();
        let state = page.read(cx);
        let Some(data) = &state.data else {
            return;
        };
        let (Some(turn), Some(target)) = (&data.diff_turn, &state.target) else {
            return;
        };
        let (turn, diff, cwd) = (turn.clone(), data.diff.clone(), target.cwd.clone());
        let host = self.host.clone();
        let work = cx.background_spawn(async move {
            let result = json_body(&host.fetch(&Request::post("apply-patch", Some(json!({
                "cwd":cwd,"diff":diff,"revert":revert,"operationSource":"remote_conversation_apply_diff"
            }))))?)?;
            Ok::<_, anyhow::Error>(serde_json::from_value::<ApplyResult>(result)?)
        });
        cx.spawn(async move |root, cx| {
            let result = work.await.unwrap_or_else(|error| ApplyResult {
                status: "error".into(),
                error: Some(error.to_string()),
                ..Default::default()
            });
            root.update(cx, |root, cx| {
                let message = if result.error_code.as_deref() == Some("not-git-repo") {
                    if revert {
                        "Revert requires a Git repository"
                    } else {
                        "Apply requires a Git repository"
                    }
                } else {
                    match (result.status.as_str(), revert) {
                        ("success", false) => "Changes applied",
                        ("success", true) => "Changes reverted",
                        ("partial-success", false) => "Changes partially applied",
                        ("partial-success", true) => "Changes partially reverted",
                        (_, false) => "Failed to apply changes",
                        (_, true) => "Failed to revert changes",
                    }
                };
                root.toast(
                    if result.status == "success" {
                        Toast::success(message)
                    } else {
                        Toast::danger(message)
                    },
                    cx,
                );
                page.update(cx, |page, cx| page.finish_apply(&turn, revert, result, cx));
            })
            .ok();
        })
        .detach();
    }

    fn load_cloud_image(&self, pointer: String, cx: &mut Context<Self>) {
        let Some(session) = &self.cloud else {
            return;
        };
        let page = session.page.clone();
        let host = self.host.clone();
        let key = pointer.clone();
        let work = cx.background_spawn(async move {
            let identity = host
                .identity()
                .context("Cloud images require a ChatGPT account")?;
            let id = key
                .strip_prefix("file-service://")
                .or_else(|| key.strip_prefix("sediment://"))
                .unwrap_or(&key);
            let result = fetch_json(
                &*host,
                &identity,
                &format!("/files/download/{}", segment(id)),
            )?;
            let url = result
                .get("download_url")
                .or_else(|| result.pointer("/data/download_url"))
                .and_then(Value::as_str)
                .context("missing image download URL")?;
            let response = host.fetch(&Request::Http(
                json!({"url":url,"method":"GET","returnErrorResponse":true}),
            ))?;
            let wire = response_wire(&response);
            ensure!(
                wire["status"]
                    .as_u64()
                    .is_some_and(|status| (200..300).contains(&status)),
                "image download failed"
            );
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(wire["base64"].as_str().context("missing image bytes")?)?;
            ui::screens::cloud_task::decode_image(&bytes)
        });
        cx.spawn(async move |_, cx| {
            let image = work.await.ok();
            page.update(cx, |page, cx| page.set_image(pointer, image, cx));
        })
        .detach();
    }

    fn cloud_follow_up(
        &mut self,
        submission: Submission,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(session) = &self.cloud else {
            return;
        };
        let page = session.page.clone();
        let selection = page.read(cx).data.as_ref().and_then(|data| {
            data.active_turn
                .as_ref()
                .map(|turn| (data.id.clone(), turn.clone()))
        });
        let Some((id, turn)) = selection else {
            page.update(cx, |page, cx| {
                page.composer.update(cx, |composer, cx| {
                    composer.restore_submission(&submission.text, cx)
                })
            });
            self.toast(
                Toast::danger("This cloud chat is not ready for a follow-up"),
                cx,
            );
            return;
        };
        let host = self.host.clone();
        let prompt = submission.text.clone();
        let context_prompt =
            crate::native_menus::context_files::prompt(submission.text, &submission.context_files);
        let work = cx.background_spawn(async move {
            let identity = host.identity().context("Cloud tasks require a ChatGPT account")?;
            let mut input = vec![json!({"type":"message","role":"user","content":[{"content_type":"text","text":context_prompt}]})];
            for attachment in &submission.attachments {
                input.push(upload_image(&*host, &identity, attachment)?);
            }
            let body = json!({"follow_up":{"task_id":id,"turn_id":turn,"environment_mode":"code"},"metadata":{"model_slug":submission.model},"input_items":input});
            json_body(&host.fetch(&request(&identity, "/wham/tasks", "POST", Some(body)))?)
        });
        cx.spawn_in(window, async move |root, cx| {
            let result = work.await;
            root.update_in(cx, |root, window, cx| {
                if !root
                    .cloud
                    .as_ref()
                    .is_some_and(|session| session.page == page)
                {
                    return;
                }
                match result {
                    Ok(_) => root.sync(window, cx),
                    Err(error) => {
                        page.update(cx, |page, cx| {
                            page.composer.update(cx, |composer, cx| {
                                composer.restore_submission(&prompt, cx)
                            });
                        });
                        root.toast(Toast::danger(error.to_string()), cx);
                    }
                }
            })
            .ok();
        })
        .detach();
    }

    fn cancel_cloud_task(&self, cx: &mut Context<Self>) {
        let Some(page) = self.cloud.as_ref().map(|session| session.page.clone()) else {
            return;
        };
        let Some(id) = page.read(cx).data.as_ref().map(|data| data.id.clone()) else {
            return;
        };
        let host = self.host.clone();
        let work = cx.background_spawn(async move {
            let identity = host
                .identity()
                .context("Cloud tasks require a ChatGPT account")?;
            json_body(&host.fetch(&request(
                &identity,
                &format!("/wham/tasks/{}/cancel", segment(&id)),
                "POST",
                None,
            ))?)
        });
        cx.spawn(async move |root, cx| {
            if let Err(error) = work.await {
                root.update(cx, |root, cx| {
                    root.toast(Toast::danger(error.to_string()), cx)
                })
                .ok();
            }
        })
        .detach();
    }
}

async fn environment_thread(
    bridge: &bridge::Bridge,
    identity: &Identity,
    config: &Value,
    repositories: Vec<Value>,
    settings: &Value,
) -> Result<Thread> {
    if let Some(id) = config["thread_id"].as_str() {
        let resumed: Value = bridge
            .request("thread/resume", json!({"threadId":id}))
            .await?;
        return Ok(serde_json::from_value(resumed["thread"].clone())?);
    }
    let config_id = config["id"]
        .as_str()
        .context("Missing cloud environment configuration ID")?;
    ensure!(
        !repositories.is_empty()
            && repositories.iter().all(|repository| repository["name"]
                .as_str()
                .is_some_and(|name| !name.is_empty())
                && repository["ref"]
                    .as_str()
                    .is_some_and(|reference| !reference.is_empty())),
        "Missing cloud repository names or branches"
    );
    let started = bridge
        .start_cloud_environment_onboarding(
            &identity.account_id,
            config_id,
            json!({
                "model":settings["model"],"serviceTier":settings["service_tier"],
                "serviceName":"codex_cloud","mode":"default","cloudThreadPrototype":true,
                "useAppServerPermissionDefault":true,
                "collaborationMode":{"mode":"default","settings":{
                    "model":settings["model"],"reasoning_effort":settings["model_reasoning_effort"],
                    "developer_instructions":null,
                }},
            }),
        )
        .await?;
    let mut thread: Thread = serde_json::from_value(started["thread"].clone())?;
    let title = format!(
        "Set up {}",
        config["name"]
            .as_str()
            .context("Missing cloud environment name")?
    );
    bridge
        .request::<_, Value>(
            "thread/name/set",
            json!({"threadId":thread.id,"name":title}),
        )
        .await?;
    thread.name = Some(title);
    let skill = config["onboarding_skill"]
        .as_str()
        .unwrap_or("cloud-environment-onboarding:setup");
    let input = vec![
        json!({"type":"text","text":format!("{}\n\n## My request:", json!({"repositories":repositories})),"text_elements":[]}),
        json!({"type":"text","text":format!("Use ${skill} to set up this cloud environment"),"text_elements":[]}),
    ];
    let started: Value = bridge
        .request(
            "turn/start",
            json!({
                "threadId":thread.id,"input":input,"model":settings["model"],
                "effort":settings["model_reasoning_effort"],"serviceTier":settings["service_tier"],
            }),
        )
        .await?;
    thread
        .turns
        .push(serde_json::from_value(started["turn"].clone())?);
    Ok(thread)
}

fn upload_image(host: &dyn Host, identity: &Identity, path: &std::path::Path) -> Result<Value> {
    let bytes = std::fs::read(path).context("read cloud attachment")?;
    let size_bytes = bytes.len();
    let (width, height) = ui::screens::cloud_task::image_dimensions(&bytes)?;
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .context("attachment filename must be UTF-8")?;
    let created = json_body(&host.fetch(&request(
        identity,
        "/files",
        "POST",
        Some(json!({
            "file_name":name, "file_size":bytes.len(), "use_case":"codex",
            "timezone_offset_min":-crate::utc_offset()/60, "reset_rate_limits":false
        })),
    ))?)?;
    let id = created["file_id"]
        .as_str()
        .context("missing cloud attachment id")?;
    let url = created["upload_url"]
        .as_str()
        .context("missing cloud upload URL")?;
    let parsed = url::Url::parse(url).context("invalid cloud upload URL")?;
    let estuary = parsed.path().ends_with("/estuary/upload_content_bytes");
    let mime = match path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        _ => "application/octet-stream",
    };
    let mut upload = if estuary {
        let boundary = format!("codex-{}", crate::host::device_id());
        let name = name.replace(['\r', '\n', '"'], "_");
        let mut body = format!("--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"{name}\"\r\nContent-Type: {mime}\r\n\r\n").into_bytes();
        body.extend_from_slice(&bytes);
        body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
        json!({"url":url,"method":"POST","headers":{"Content-Type":format!("multipart/form-data; boundary={boundary}"),"X-OpenAI-Attach-Auth":"1"},
            "bodyBase64":base64::engine::general_purpose::STANDARD.encode(body),"returnErrorResponse":true})
    } else {
        let url = parsed
            .query_pairs()
            .find_map(|(key, value)| (key == "upload_url").then(|| value.into_owned()))
            .unwrap_or_else(|| url.to_owned());
        json!({"url":url,"method":"PUT","headers":{"Content-Type":mime,"x-ms-blob-type":"BlockBlob","x-ms-version":"2020-04-08","x-ms-blob-content-type":mime},
            "bodyBase64":base64::engine::general_purpose::STANDARD.encode(bytes),"returnErrorResponse":true})
    };
    if estuary {
        upload["expectedIdentity"] =
            json!({"accountId":identity.account_id,"userId":identity.user_id});
    }
    let uploaded = host.fetch(&Request::Http(upload))?;
    let uploaded = response_wire(&uploaded);
    ensure!(
        uploaded["status"]
            .as_u64()
            .is_some_and(|status| (200..300).contains(&status)),
        "cloud attachment upload failed"
    );
    let finalized = json_body(&host.fetch(&request(
        identity,
        &format!("/files/{}/uploaded", segment(id)),
        "POST",
        Some(json!({})),
    ))?);
    if estuary {
        finalized?;
    } else if let Err(error) = finalized {
        eprintln!("codex-gpui: finalize cloud attachment: {error}");
    }
    let pointer = if id.starts_with("file_") {
        format!("sediment://{id}")
    } else {
        format!("file-service://{id}")
    };
    Ok(
        json!({"type":"image_asset_pointer","asset_pointer":pointer,"width":width,"height":height,"size_bytes":size_bytes}),
    )
}

pub(crate) fn segment(value: &str) -> String {
    url::form_urlencoded::byte_serialize(value.as_bytes()).collect()
}

pub(crate) fn request(
    identity: &Identity,
    path: &str,
    method: &str,
    body: Option<Value>,
) -> Request {
    let mut request = json!({"url":path,"method":method,"headers":{"OAI-Language":"en-US","X-OpenAI-Attach-Auth":"1","X-OpenAI-Attach-Integrity-State":"1","originator":"Codex Desktop"},"expectedIdentity":{"accountId":identity.account_id,"userId":identity.user_id},"returnErrorResponse":true});
    if let Some(body) = body {
        request["body"] = body.to_string().into();
    }
    Request::Http(request)
}

fn fetch_json(host: &dyn Host, identity: &Identity, path: &str) -> Result<Value> {
    json_body(&host.fetch(&request(identity, path, "GET", None))?)
}

fn run(
    host: &dyn Host,
    task_id: &str,
    cancelled: &AtomicBool,
    send: &async_channel::Sender<Update>,
) -> Result<()> {
    let identity = host
        .identity()
        .context("Cloud tasks require a ChatGPT account")?;
    let base = format!("/wham/tasks/{}", segment(task_id));
    let mut cached_roots = Vec::new();
    let mut cached_url = None;
    let mut target: Option<LocalTarget> = None;
    while !cancelled.load(Ordering::Acquire) {
        let result = (|| -> Result<bool> {
            let detail_wire = host.fetch(&request(&identity, &base, "GET", None))?;
            if response_wire(&detail_wire)["status"] == 404 {
                send.send_blocking(Update::Unavailable).ok();
                return Ok(false);
            }
            let detail = json_body(&detail_wire)?;
            let turns = fetch_json(host, &identity, &format!("{base}/turns"))?;
            let mut data = parse_task_response(&detail, &turns)?;
            for turn in &mut data.turns {
                if turn.phase == Phase::SetupFailed
                    && let Ok(logs) = fetch_json(
                        host,
                        &identity,
                        &format!("{base}/turns/{}/logs", segment(&turn.id)),
                    )
                {
                    turn.logs = logs["logs"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter(|log| {
                            log.pointer("/key/type").and_then(Value::as_str)
                                == Some("UserSetupScript")
                        })
                        .filter_map(|log| log["line"].as_str().map(str::to_owned))
                        .collect();
                }
            }
            let roots: Vec<_> = host
                .projects()
                .unwrap_or_default()
                .into_iter()
                .filter_map(|project| project.root)
                .collect();
            if cached_roots != roots || cached_url != data.clone_url {
                target = local_target(&roots, data.clone_url.as_deref());
                cached_roots = roots;
                cached_url.clone_from(&data.clone_url);
            }
            if let Some(target) = &mut target {
                target.branch = local_branch(&target.cwd);
            }
            let active = data.active_turn.clone();
            let working = data.working;
            if send
                .send_blocking(Update::Snapshot(data, target.clone()))
                .is_err()
                || !working
            {
                return Ok(false);
            }
            let turn = active.context("running cloud task has no assistant turn")?;
            let path = format!(
                "{base}/turns/{}/stream?item_type=thread_event&item_type=log",
                segment(&turn)
            );
            let mut seen = HashSet::new();
            host.stream(
                &request(&identity, &path, "GET", None),
                cancelled,
                &mut |event| {
                    if cancelled.load(Ordering::Acquire) {
                        return false;
                    }
                    if let Some(id) = event["id"].as_str()
                        && !seen.insert(id.to_owned())
                    {
                        return true;
                    }
                    send.send_blocking(Update::Event(turn.clone(), event))
                        .is_ok()
                },
            )?;
            Ok(true)
        })();
        match result {
            Ok(false) => return Ok(()),
            Err(error) => {
                if send
                    .send_blocking(Update::Failed(error.to_string()))
                    .is_err()
                {
                    return Ok(());
                }
            }
            Ok(true) => {}
        }
        if !cancelled.load(Ordering::Acquire) {
            std::thread::sleep(Duration::from_secs(5));
        }
    }
    Ok(())
}

fn local_target(roots: &[SharedString], clone_url: Option<&str>) -> Option<LocalTarget> {
    let key = repository_key(clone_url?);
    for root in roots {
        let Ok(origins) = std::process::Command::new("git")
            .args(["-C", root.as_ref(), "remote", "-v"])
            .output()
        else {
            continue;
        };
        if origins.status.success()
            && String::from_utf8_lossy(&origins.stdout)
                .lines()
                .any(|line| {
                    line.split_whitespace()
                        .nth(1)
                        .is_some_and(|url| repository_key(url) == key)
                })
        {
            return Some(LocalTarget {
                cwd: root.to_string(),
                branch: None,
            });
        }
    }
    None
}

fn local_branch(root: &str) -> Option<String> {
    std::process::Command::new("git")
        .args(["-C", root, "branch", "--show-current"])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

fn repository_key(url: &str) -> String {
    let url = url.trim_end_matches('/').trim_end_matches(".git");
    url.strip_prefix("git@")
        .map(|url| url.replace(':', "/"))
        .unwrap_or_else(|| {
            url.strip_prefix("https://")
                .or_else(|| url.strip_prefix("http://"))
                .unwrap_or(url)
                .to_owned()
        })
        .to_ascii_lowercase()
}

fn turn_status(value: &Value) -> Result<TurnStatus> {
    match value["turn_status"].as_str() {
        Some("completed") => Ok(TurnStatus::Completed),
        Some("cancelled") => Ok(TurnStatus::Interrupted),
        Some("failed") => Ok(TurnStatus::Failed),
        Some("pending" | "in_progress") => Ok(if value["cancellation_requested_at"].is_null() {
            TurnStatus::InProgress
        } else {
            TurnStatus::Interrupted
        }),
        _ => bail!("unknown cloud turn status"),
    }
}

fn content(items: &Value) -> String {
    items
        .as_array()
        .into_iter()
        .flatten()
        .filter(|item| item["type"] == "message")
        .flat_map(|item| item["content"].as_array().into_iter().flatten())
        .filter(|part| part["content_type"] == "text")
        .filter_map(|part| part["text"].as_str())
        .collect()
}

pub fn parse_task_response(detail: &Value, turns: &Value) -> Result<CloudTask> {
    let task = &detail["task"];
    let id = task["id"]
        .as_str()
        .context("cloud task has no id")?
        .to_owned();
    let title = task["title"]
        .as_str()
        .context("cloud task has no title")?
        .to_owned();
    let mut records = BTreeMap::new();
    for value in turns["turn_mapping"]
        .as_object()
        .into_iter()
        .flat_map(|mapping| mapping.values())
        .map(|entry| &entry["turn"])
        .chain([
            &detail["current_user_turn"],
            &detail["current_assistant_turn"],
            &detail["current_diff_task_turn"],
        ])
    {
        if let Some(id) = value["id"].as_str() {
            records.insert(id.to_owned(), value);
        }
    }
    let active_turn = detail
        .pointer("/current_assistant_turn/id")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let active = active_turn.as_ref().and_then(|id| records.get(id)).copied();
    let mut users: Vec<_> = records
        .values()
        .copied()
        .filter(|record| record["input_items"].is_array())
        .collect();
    users.sort_by_key(|user| user["created_at"].as_i64().unwrap_or_default());
    let mut projected = Vec::new();
    for user in users {
        let user_id = user["id"].as_str().context("cloud user turn has no id")?;
        let assistant = records
            .values()
            .copied()
            .filter(|record| {
                record["previous_turn_id"] == user_id && record["output_items"].is_array()
            })
            .max_by_key(|record| {
                (
                    Some(record["id"].as_str().unwrap_or_default()) == active_turn.as_deref(),
                    record["created_at"].as_i64().unwrap_or_default(),
                )
            });
        let assistant_id = assistant
            .and_then(|value| value["id"].as_str())
            .unwrap_or(user_id);
        let status = assistant
            .map(turn_status)
            .transpose()?
            .unwrap_or(TurnStatus::InProgress);
        let events = assistant
            .and_then(|value| value.pointer("/thread_events/events"))
            .and_then(Value::as_array);
        let setup_failed = assistant.is_some_and(|value| {
            value["turn_status"] == "failed"
                && value.pointer("/error/code").and_then(Value::as_str)
                    == Some("startup_script_failed")
        });
        let phase = if setup_failed {
            Phase::SetupFailed
        } else if assistant.is_some_and(|value| value["turn_status"] == "cancelled") {
            Phase::Cancelled
        } else if status == TurnStatus::Failed {
            Phase::Failed
        } else if status == TurnStatus::InProgress && events.is_none_or(Vec::is_empty) {
            Phase::SettingUp
        } else {
            Phase::Chat
        };
        let mut turn = Turn {
            id: assistant_id.to_owned(),
            items: vec![],
            status,
            error: None,
            started_at: None,
            completed_at: None,
            duration_ms: None,
        };
        let text = content(&user["input_items"]);
        if !text.is_empty() {
            turn.items.push(ThreadItem::UserMessage {
                id: user_id.to_owned(),
                content: vec![UserInput::Text { text }],
            });
        }
        if let Some(events) = events {
            for event in events {
                reduce(&mut turn, event);
            }
        }
        if !matches!(phase, Phase::SettingUp | Phase::SetupFailed)
            && !turn
                .items
                .iter()
                .any(|item| matches!(item, ThreadItem::AgentMessage { .. }))
            && let Some(assistant) = assistant
        {
            let text = content(&assistant["output_items"]);
            if !text.trim().is_empty() {
                turn.items.push(ThreadItem::AgentMessage {
                    id: format!("cloud-assistant-message:{assistant_id}"),
                    text,
                    phase: None,
                });
            }
        }
        let images = user["input_items"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|item| item["asset_pointer"].as_str().map(str::to_owned))
            .collect();
        projected.push(CloudTurn {
            id: assistant_id.to_owned(),
            thread: Thread {
                id: id.clone(),
                preview: String::new(),
                name: Some(title.clone()),
                model: None,
                reasoning_effort: None,
                model_provider: String::new(),
                cwd: String::new(),
                path: None,
                created_at: 0,
                updated_at: 0,
                section: None,
                status: ThreadStatus::Idle,
                git_info: None,
                turns: vec![turn],
            },
            phase,
            error: assistant
                .and_then(|value| value.pointer("/error/message"))
                .and_then(Value::as_str)
                .map(str::to_owned),
            logs: vec![],
            images,
        });
    }
    let diff_turn = detail
        .get("current_diff_task_turn")
        .filter(|value| value.is_object())
        .or(active);
    let diff = diff_turn
        .and_then(|value| value["output_items"].as_array())
        .into_iter()
        .flatten()
        .find_map(|item| item.pointer("/output_diff/diff").and_then(Value::as_str))
        .unwrap_or("")
        .to_owned();
    let environment = active.and_then(|value| value.get("environment"));
    let repo = environment
        .and_then(|value| value["repos"].as_array())
        .and_then(|repos| repos.first())
        .and_then(Value::as_str);
    let clone_url = repo
        .and_then(|repo| {
            environment?
                .get("repo_map")?
                .get(repo)?
                .get("clone_url")?
                .as_str()
        })
        .map(str::to_owned);
    Ok(CloudTask {
        id,
        title,
        turns: projected,
        active_turn,
        diff_turn: diff_turn
            .and_then(|value| value["id"].as_str())
            .map(str::to_owned),
        diff,
        clone_url,
        environment_label: environment
            .and_then(|value| value["label"].as_str())
            .map(str::to_owned),
        working: active.is_some_and(|value| {
            matches!(
                value["turn_status"].as_str(),
                Some("pending" | "in_progress")
            )
        }),
    })
}

fn reduce(turn: &mut Turn, event: &Value) {
    let params = &event["params"];
    match event["method"].as_str() {
        Some("item/started" | "item/completed") => {
            if let Ok(item) = serde_json::from_value::<ThreadItem>(params["item"].clone()) {
                if matches!(item, ThreadItem::UserMessage { .. } | ThreadItem::Other) {
                    return;
                }
                let id = params["item"]["id"].as_str();
                if let Some(existing) = turn.items.iter_mut().find(|item| item_id(item) == id) {
                    *existing = item;
                } else {
                    turn.items.push(item);
                }
            }
        }
        Some(
            "item/agentMessage/delta" | "item/commandExecution/outputDelta" | "item/plan/delta",
        ) => {
            let Some(delta) = params["delta"].as_str() else {
                return;
            };
            if let Some(item) = turn
                .items
                .iter_mut()
                .find(|item| item_id(item) == params["itemId"].as_str())
            {
                match item {
                    ThreadItem::AgentMessage { text, .. } | ThreadItem::Plan { text, .. } => {
                        text.push_str(delta)
                    }
                    ThreadItem::CommandExecution {
                        aggregated_output, ..
                    } => aggregated_output.get_or_insert_default().push_str(delta),
                    _ => {}
                }
            }
        }
        Some("turn/started" | "turn/completed") => {
            if let Ok(status) = serde_json::from_value(params["turn"]["status"].clone()) {
                turn.status = status;
            }
        }
        _ => {}
    }
}

fn item_id(item: &ThreadItem) -> Option<&str> {
    match item {
        ThreadItem::UserMessage { id, .. }
        | ThreadItem::AgentMessage { id, .. }
        | ThreadItem::Plan { id, .. }
        | ThreadItem::Reasoning { id, .. }
        | ThreadItem::CommandExecution { id, .. }
        | ThreadItem::FileChange { id, .. }
        | ThreadItem::McpToolCall { id, .. }
        | ThreadItem::ContextCompaction { id }
        | ThreadItem::WebSearch { id, .. } => Some(id),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reopening_completed_setup_resumes_without_another_model_turn() {
        let thread = json!({
            "id":"setup-thread","preview":"Environment ready","model":null,
            "modelProvider":"openai","cwd":"/workspace","path":null,
            "createdAt":1,"updatedAt":2,"status":{"type":"idle"},
            "turns":[{"id":"setup-turn","status":"completed","items":[
                {"type":"agentMessage","id":"answer","text":"Environment ready"}
            ]}]
        });
        let fixture = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(fixture.path(), format!("{}\n{}\n",
            json!({"dir":"send","msg":{"id":1,"method":"thread/resume","params":{"threadId":"setup-thread"}}}),
            json!({"dir":"recv","msg":{"id":1,"result":{"thread":thread}}}),
        )).unwrap();
        let (bridge, _) = bridge::Bridge::replay(fixture.path()).unwrap();
        let resumed = pollster::block_on(environment_thread(
            &bridge,
            &Identity {
                account_id: "account".into(),
                user_id: "user".into(),
                device_id: "device".into(),
            },
            &json!({"id":"config","thread_id":"setup-thread"}),
            Vec::new(),
            &Value::Null,
        ))
        .unwrap();
        assert_eq!(resumed.turns[0].id, "setup-turn");
        assert_eq!(resumed.turns[0].status, TurnStatus::Completed);
        assert!(matches!(&resumed.turns[0].items[..],
            [ThreadItem::AgentMessage { text, .. }] if text == "Environment ready"));
    }
    #[test]
    fn a_cloud_task_never_invents_an_answer_while_setup_is_pending() {
        let detail = json!({"task":{"id":"task","title":"Build"},"current_user_turn":{"id":"user","created_at":1,"input_items":[{"type":"message","content":[{"content_type":"text","text":"Build it"}]}]},"current_assistant_turn":{"id":"assistant","previous_turn_id":"user","turn_status":"pending","output_items":[]}});
        let data = parse_task_response(&detail, &json!({})).unwrap();
        assert_eq!(data.turns[0].phase, Phase::SettingUp);
        assert!(data.working);
        assert_eq!(data.turns[0].thread.turns[0].items.len(), 1);
    }
    #[test]
    fn completed_events_win_over_duplicate_output_messages() {
        let detail = json!({"task":{"id":"task","title":"Build"},"current_user_turn":{"id":"user","input_items":[]},"current_assistant_turn":{"id":"assistant","previous_turn_id":"user","turn_status":"completed","output_items":[{"type":"message","content":[{"content_type":"text","text":"fallback"}]}],"thread_events":{"events":[{"method":"item/completed","params":{"item":{"type":"agentMessage","id":"answer","text":"recorded"}}}]}}});
        let data = parse_task_response(&detail, &json!({})).unwrap();
        assert!(
            matches!(&data.turns[0].thread.turns[0].items[..],[ThreadItem::AgentMessage{text,..}] if text=="recorded")
        );
        assert!(!data.working);
    }
    #[test]
    fn startup_failure_takes_precedence_over_stored_events_and_cancelled_is_terminal() {
        let mut detail = json!({
            "task":{"id":"task","title":"Build"},
            "current_user_turn":{"id":"user","input_items":[]},
            "current_assistant_turn":{
                "id":"assistant","previous_turn_id":"user","turn_status":"failed","output_items":[],
                "error":{"code":"startup_script_failed","message":"setup exited 1"},
                "thread_events":{"events":[{"method":"turn/started","params":{"turn":{"status":"inProgress"}}}]}
            }
        });
        let failed = parse_task_response(&detail, &json!({})).unwrap();
        assert_eq!(failed.turns[0].phase, Phase::SetupFailed);
        assert!(!failed.working);
        detail["current_assistant_turn"]["turn_status"] = "cancelled".into();
        let cancelled = parse_task_response(&detail, &json!({})).unwrap();
        assert_eq!(cancelled.turns[0].phase, Phase::Cancelled);
        assert!(!cancelled.working);
        detail["current_assistant_turn"]["turn_status"] = "unknown".into();
        assert!(parse_task_response(&detail, &json!({})).is_err());
    }
    #[test]
    fn malformed_task_and_nested_route_ids_are_rejected() {
        assert!(parse_task_response(&json!({}), &json!({})).is_err());
        assert!(Route::remote_path("/remote/a/b").is_none());
        assert_eq!(
            Route::remote_path("/hotkey-window/remote/task"),
            Some(Route::Remote {
                task_id: "task".into(),
                hotkey: true
            })
        );
    }
}

#[cfg(test)]
mod recovery_tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    struct FlakyHost(AtomicUsize);

    impl Host for FlakyHost {
        fn identity(&self) -> Option<Identity> {
            Some(Identity {
                account_id: "account".into(),
                user_id: "user".into(),
                device_id: "device".into(),
            })
        }

        fn fetch(&self, request: &Request) -> Result<Value> {
            if self.0.fetch_add(1, Ordering::Relaxed) == 0 {
                bail!("temporary network failure");
            }
            let Request::Http(request) = request else {
                bail!("expected HTTP");
            };
            let body = if request["url"]
                .as_str()
                .is_some_and(|url| url.ends_with("/turns"))
            {
                json!({})
            } else {
                json!({
                    "task":{"id":"task","title":"Recovered"},
                    "current_user_turn":{"id":"user","input_items":[]},
                    "current_assistant_turn":{"id":"assistant","previous_turn_id":"user","turn_status":"completed","output_items":[]}
                })
            };
            Ok(json!({"value":{"response":{"__response":{"status":200,"body":body.to_string()}}}}))
        }
    }

    #[test]
    fn failed_snapshot_read_recovers_without_reopening_the_page() {
        let (send, receive) = async_channel::unbounded();
        run(
            &FlakyHost(AtomicUsize::new(0)),
            "task",
            &AtomicBool::new(false),
            &send,
        )
        .unwrap();
        assert!(
            matches!(receive.try_recv().unwrap(), Update::Failed(error) if error == "temporary network failure")
        );
        assert!(
            matches!(receive.try_recv().unwrap(), Update::Snapshot(data, _) if data.title == "Recovered" && !data.working)
        );
    }

    #[test]
    fn missing_setup_logs_do_not_hide_the_failed_task() {
        struct MissingLogs<'a>(&'a AtomicBool);
        impl Host for MissingLogs<'_> {
            fn identity(&self) -> Option<Identity> {
                FlakyHost(AtomicUsize::new(1)).identity()
            }
            fn fetch(&self, request: &Request) -> Result<Value> {
                let Request::Http(request) = request else {
                    bail!("expected HTTP");
                };
                let url = request["url"].as_str().unwrap();
                if url.ends_with("/logs") {
                    self.0.store(true, Ordering::Release);
                    bail!("setup logs unavailable");
                }
                let body = if url.ends_with("/turns") {
                    json!({})
                } else {
                    json!({
                        "task":{"id":"task","title":"Failed setup"},
                        "current_user_turn":{"id":"user","input_items":[]},
                        "current_assistant_turn":{
                            "id":"assistant","previous_turn_id":"user","turn_status":"failed","output_items":[],
                            "error":{"code":"startup_script_failed"}
                        }
                    })
                };
                Ok(
                    json!({"value":{"response":{"__response":{"status":200,"body":body.to_string()}}}}),
                )
            }
        }
        let cancelled = AtomicBool::new(false);
        let (send, receive) = async_channel::unbounded();
        run(&MissingLogs(&cancelled), "task", &cancelled, &send).unwrap();
        assert!(
            matches!(receive.try_recv().unwrap(), Update::Snapshot(data, _) if data.turns[0].phase == Phase::SetupFailed)
        );
    }
}
