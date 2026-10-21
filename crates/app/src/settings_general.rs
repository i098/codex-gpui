use super::*;

impl Root {
    /// Window preferences keep their state storage; other host settings use the desktop service.
    pub(super) fn write_setting(&mut self, e: &SettingChanged, cx: &mut Context<Self>) {
        if e.path == crate::settings_audio::SETTING {
            if e.value == true && self.audio_capture.is_none() {
                self.start_settings_audio(cx);
                return;
            }
            if e.value == true
                && self
                    .audio_capture
                    .as_ref()
                    .is_some_and(|capture| !capture.ready)
            {
                return;
            }
            if e.value == false {
                self.audio_generation = self.audio_generation.wrapping_add(1);
                self.audio_capture = None;
            }
        }
        if e.path == settings::THEME {
            return;
        }
        if e.path == "host.confirm-before-close" {
            if let Ok(value) = serde_json::from_value(e.value.clone()) {
                self.state.confirm_before_close = value;
                self.config["host"]["confirm-before-close"] = e.value.clone();
                self.state.save();
                self.setting_finished(e.path, e.value.clone(), false, cx);
            }
            return;
        }
        if e.path == "desktop.followUpQueueMode" {
            let queue = e.value == "queue";
            self.thread_composer
                .update(cx, |c, cx| c.set_queue_mode(queue, cx));
        }
        let (store, key) = e.path.split_once('.').unwrap_or((e.path, ""));
        if store == "host" {
            let result = if cx.has_global::<host_settings::Settings>() {
                cx.global_mut::<host_settings::Settings>()
                    .write(key, e.value.clone())
            } else {
                Err(anyhow::anyhow!("host settings could not be loaded"))
            };
            let failed = result.is_err();
            if let Err(error) = result {
                eprintln!("codex-gpui: settings.write({key}): {error:#}");
                let message = match key {
                    "git-branch-prefix" => "Failed to save branch prefix",
                    "git-pull-request-merge-method" => "Failed to save pull request merge method",
                    "git-always-force-push" => "Failed to save force push setting",
                    "git-create-pull-request-as-draft" => {
                        "Failed to save draft pull request setting"
                    }
                    "git-review-mode" => "Could not save Git diff setting",
                    "git-pr-watch-auto-merge" => "Failed to save automatic PR merging",
                    "git-commit-instructions"
                    | "git-pr-instructions"
                    | "git-pr-watch-instructions" => "Failed to save instructions",
                    _ => "Couldn't save your changes. Try again",
                };
                self.toast(Toast::danger(message), cx);
            } else {
                let message = match key {
                    "git-branch-prefix" => Some("Saved branch prefix".to_owned()),
                    "git-pull-request-merge-method" => {
                        Some("Saved pull request merge method".to_owned())
                    }
                    "git-always-force-push" => Some(format!(
                        "Always force push {}",
                        if e.value == true {
                            "enabled"
                        } else {
                            "disabled"
                        }
                    )),
                    "git-create-pull-request-as-draft" => Some(format!(
                        "Create draft pull requests {}",
                        if e.value == true {
                            "enabled"
                        } else {
                            "disabled"
                        }
                    )),
                    "git-pr-watch-auto-merge" => Some(format!(
                        "Automatic PR merging {}",
                        if e.value == true {
                            "enabled"
                        } else {
                            "disabled"
                        }
                    )),
                    _ => None,
                };
                if let Some(message) = message {
                    self.toast(Toast::info(message), cx);
                }
            }
            let value = cx
                .try_global::<host_settings::Settings>()
                .and_then(|settings| settings.read_all().get(key))
                .cloned()
                .unwrap_or(Value::Null);
            self.setting_finished(e.path, value, failed, cx);
            return;
        }
        let value = e.value.clone();
        let apply = move |config: &mut Value| {
            if key.is_empty() {
                config[store] = value;
            } else {
                config[store][key] = value;
            }
        };
        if !matches!(store, "desktop" | "features") && e.path != "service_tier" {
            apply(&mut self.config);
            self.setting_finished(e.path, e.value.clone(), false, cx);
            return;
        }
        let params = json!({
            "keyPath": e.path,
            "value": e.value,
            "mergeStrategy": "upsert",
            "filePath": null,
            "expectedVersion": null,
        });
        self.write_config("config/value/write", params, cx, apply);
    }

    /// Sends a config write and, once the server took it, applies it to the kept config.
    pub(super) fn write_config(
        &mut self,
        method: &'static str,
        params: Value,
        cx: &mut Context<Self>,
        apply: impl FnOnce(&mut Value) + 'static,
    ) {
        self.qa_work_started = true;
        let bridge = self.bridge.clone();
        let key_path = params["keyPath"].as_str().map(str::to_owned);
        let value = params["value"].clone();
        let review_delivery = params["keyPath"] == "desktop.reviewDelivery";
        let queue_mode = params["keyPath"] == "desktop.followUpQueueMode";
        let task = cx.spawn(async move |this, cx| -> Result<()> {
            if let Err(error) = bridge.request::<_, Value>(method, params).await {
                if review_delivery {
                    this.update(cx, |this, cx| {
                        this.toast(
                            Toast::danger("Could not save review delivery preference"),
                            cx,
                        );
                    })?;
                }
                if queue_mode {
                    // The composer shows the behavior that still applies.
                    this.update(cx, |this, cx| {
                        let queue = this.config["desktop"]["followUpQueueMode"] == "queue";
                        this.thread_composer
                            .update(cx, |c, cx| c.set_queue_mode(queue, cx));
                        this.toast(Toast::danger("Could not save follow-up behavior"), cx);
                    })?;
                }
                if let Some(path) = &key_path {
                    this.update(cx, |this, cx| {
                        let previous = path
                            .split_once('.')
                            .map(|(store, key)| this.config[store][key].clone())
                            .unwrap_or_else(|| this.config[path].clone());
                        this.setting_finished(path, previous, true, cx);
                        if !queue_mode && !review_delivery {
                            let message = match path.as_str() {
                                "features.local_thread_store_compression" => {
                                    "Couldn’t save the compression setting. Try again."
                                }
                                "desktop.projectlessWorkspaceRoot" if value.is_null() => {
                                    "Unable to reset the projectless task folder"
                                }
                                "desktop.projectlessWorkspaceRoot" => {
                                    "Unable to change the projectless task folder"
                                }
                                "desktop.defaultTerminalLocation" => {
                                    "Could not save terminal location preference"
                                }
                                _ => "Couldn’t save your changes. Try again",
                            };
                            this.toast(Toast::danger(message), cx);
                        }
                    })?;
                }
                return Err(error);
            }
            this.update(cx, |this, cx| {
                apply(&mut this.config);
                if let Some(path) = key_path {
                    this.setting_finished(&path, value, false, cx);
                }
            })
        });
        log_err(task, cx);
    }

    fn setting_finished(&mut self, path: &str, value: Value, failed: bool, cx: &mut Context<Self>) {
        if let Some(page) = &self.settings_page {
            page.update(cx, |page, cx| page.finish_write(path, value, failed, cx));
            if path == "desktop.projectlessWorkspaceRoot" && !failed {
                self.load_projectless_folder(page, cx);
            }
        }
        self.refresh_general_preferences(cx);
    }

    pub(super) fn refresh_general_preferences(&self, cx: &mut Context<Self>) {
        let show_full_access = cx
            .try_global::<host_settings::Settings>()
            .and_then(|settings| {
                settings
                    .read_all()
                    .get("composer-permission-mode-visibility")
            })
            .and_then(Value::as_bool)
            .unwrap_or(true);
        let show_context_usage = cx
            .try_global::<host_settings::Settings>()
            .and_then(|settings| settings.read_all().get("show-context-window-usage"))
            .and_then(Value::as_bool)
            .unwrap_or(false);
        cx.set_global(ui::composer::Preferences {
            enter: ui::composer::EnterBehavior::from_setting(
                self.config["desktop"]["composerEnterBehavior"]
                    .as_str()
                    .unwrap_or("enter"),
            ),
            show_full_access,
            show_context_usage,
        });
    }

    pub(super) fn bind_general_settings(
        &mut self,
        page: &Entity<settings::SettingsPage>,
        cx: &mut Context<Self>,
    ) {
        page.update(cx, |page, cx| {
            page.set_audio_available(crate::settings_audio::available(), cx)
        });
        self.load_projectless_folder(page, cx);
        cx.observe(page, |this, page, cx| {
            if this
                .settings_page
                .as_ref()
                .is_some_and(|active| active.entity_id() == page.entity_id())
            {
                this.settings_section = Some(page.read(cx).route().clone());
                this.load_settings_profile(&page, cx);
                this.load_settings_notices(&page, cx);
                cx.notify();
            }
        })
        .detach();
        cx.subscribe(page, |this, _, notice: &settings::SettingsNotice, cx| {
            this.toast(Toast::danger(notice.0), cx);
        })
        .detach();
        self.load_settings_profile(page, cx);
        self.load_settings_notices(page, cx);
        let bridge = self.bridge.clone();
        let requirements_page = page.clone();
        let chatgpt = self.chatgpt_account;
        cx.spawn(async move |_, cx| {
            let result = bridge
                .request::<_, Value>("configRequirements/read", json!({}))
                .await
                .map_err(|error| error.to_string());
            let _ =
                requirements_page.update(cx, |page, cx| page.set_requirements(result, chatgpt, cx));
        })
        .detach();
        let host = self.host.clone();
        let page = page.clone();
        let task = cx.background_executor().spawn(async move {
            host.fetch(&host::Request::post("open-in-targets", Some(json!({}))))
                .and_then(|response| host::json_body(&response))
                .map_err(|error| error.to_string())
        });
        cx.spawn(async move |_, cx| {
            let result = task.await;
            let _ = page.update(cx, |page, cx| page.set_targets(result, cx));
        })
        .detach();
    }

    pub fn open_settings_path(
        &mut self,
        path: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.settings_dirty(cx) {
            let path = path.to_owned();
            self.confirm_settings_leave(window, cx, move |root, window, cx| {
                root.open_settings_path(&path, window, cx);
            });
            return true;
        }
        let Some(route) = settings::SettingsRoute::parse(path) else {
            return false;
        };
        self.settings_section = Some(route);
        if self.state.route == Route::Settings {
            self.sync(window, cx);
        } else {
            self.navigate(Route::Settings, window, cx);
        }
        true
    }

    fn load_settings_profile(&self, page: &Entity<settings::SettingsPage>, cx: &mut Context<Self>) {
        if !page.update(cx, |page, _| page.request_profile()) {
            return;
        }
        let host = self.host.clone();
        let page = page.clone();
        let task = cx.background_executor().spawn(async move {
            let read = || -> Result<Option<Value>> {
                let identity = host.identity().context("No ChatGPT account")?;
                let response = host.fetch(&host::Request::Http(json!({
                    "url": "/profiles/me/page",
                    "method": "GET",
                    "headers": {
                        "X-OpenAI-Attach-Auth": "1",
                        "X-OpenAI-Attach-Integrity-State": "1",
                        "ChatGPT-Account-ID": identity.account_id,
                    },
                    "expectedIdentity": { "accountId": identity.account_id, "userId": identity.user_id },
                    "returnErrorResponse": true,
                })))?;
                let raw = response.pointer("/response/__response").unwrap_or(&response);
                if matches!(raw["status"].as_u64(), Some(403 | 404)) { return Ok(None); }
                let profile = host::json_body(&response)?;
                ensure!(profile["profile_details"].is_object() && profile["page"].is_object(), "invalid profile response");
                Ok(Some(profile))
            };
            read().map_err(|error| error.to_string())
        });
        cx.spawn(async move |_, cx| {
            let result = task.await;
            let _ = page.update(cx, |page, cx| page.set_profile(result, cx));
        })
        .detach();
    }

    fn load_projectless_folder(
        &self,
        page: &Entity<settings::SettingsPage>,
        cx: &mut Context<Self>,
    ) {
        let host = self.host.clone();
        let page = page.clone();
        let task = cx.background_executor().spawn(async move {
            host.fetch(&host::Request::post(
                "projectless-task-folder",
                Some(json!({})),
            ))
            .and_then(|response| host::json_body(&response))
            .and_then(|value| {
                value["path"]
                    .as_str()
                    .map(str::to_owned)
                    .context("Projectless task folder is unavailable")
            })
            .map_err(|error| error.to_string())
        });
        cx.spawn(async move |_, cx| {
            let result = task.await;
            let _ = page.update(cx, |page, cx| page.set_projectless_root(result, cx));
        })
        .detach();
    }

    fn load_settings_notices(&self, page: &Entity<settings::SettingsPage>, cx: &mut Context<Self>) {
        if !page.update(cx, |page, _| page.request_notices()) {
            return;
        }
        let host = self.host.clone();
        let page = page.clone();
        let task = cx.background_executor().spawn(async move {
            host.fetch(&host::Request::post("third-party-notices", Some(json!({}))))
                .and_then(|response| host::json_body(&response))
                .map(|value| value["text"].as_str().unwrap_or_default().to_owned())
                .map_err(|error| error.to_string())
        });
        cx.spawn(async move |_, cx| {
            let result = task.await;
            let _ = page.update(cx, |page, cx| page.set_notices(result, cx));
        })
        .detach();
    }

    pub(super) fn settings_dirty(&self, cx: &Context<Self>) -> bool {
        self.state.route == Route::Settings
            && self
                .settings_page
                .as_ref()
                .is_some_and(|page| page.read(cx).has_unsaved_changes())
    }

    pub(super) fn confirm_settings_leave(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
        proceed: impl Fn(&mut Self, &mut Window, &mut Context<Self>) + 'static,
    ) {
        let Some(page) = &self.settings_page else {
            return;
        };
        let root = cx.entity().downgrade();
        page.update(cx, |page, cx| {
            page.ask_discard(window, cx, move |_, window, cx| {
                let _ = root.update(cx, |root, cx| proceed(root, window, cx));
            })
        });
    }

    pub(super) fn update_context_usage(&mut self, params: &Value, cx: &mut Context<Self>) {
        let Some(thread) = params["threadId"].as_str().filter(|id| !id.is_empty()) else {
            return;
        };
        let usage = ui::composer::ContextUsage::from_notification(&params["tokenUsage"]);
        if let Some(usage) = usage {
            self.context_usage.insert(thread.to_owned(), usage);
        } else {
            self.context_usage.remove(thread);
        }
        let selected = matches!(&self.state.route, Route::Thread(id) if id == thread)
            || self
                .thread
                .as_ref()
                .is_some_and(|view| view.read(cx).id() == thread);
        if selected {
            self.thread_composer
                .update(cx, |composer, cx| composer.set_context_usage(usage, cx));
        }
    }
}
