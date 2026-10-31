//! Settings > Configuration: the window host's config layers and `model/list` efforts.

use crate::*;
use ui::composer::EnabledEfforts;

/// The host setting the `Available reasoning efforts` menu writes.
pub(super) const ENABLED_EFFORTS: &str = "enabled-reasoning-efforts";

impl Root {
    /// The open settings page, for flows that drive it without pixel coordinates.
    pub fn settings_page(&self) -> Option<Entity<settings::SettingsPage>> {
        self.settings_page.clone()
    }
}

/// Gives the page the catalog efforts and the host filter, reads the layers, and opens
/// `config.toml` on request.
pub(super) fn attach(
    root: &mut Root,
    page: &Entity<settings::SettingsPage>,
    cx: &mut Context<Root>,
) {
    page.update(cx, |page, cx| page.set_models(&root.models, cx));
    let hosts = filter_hosts(root, cx);
    page.update(cx, |page, cx| page.set_hosts(hosts, cx));
    cx.subscribe(page, |this, _, _: &settings::OpenConfigToml, cx| {
        read_layers(this, true, cx)
    })
    .detach();
    cx.subscribe(page, |this, page, _: &settings::SelectConfigHost, cx| {
        if page.read(cx).config_host() == this.host_id {
            let config = this.config.clone();
            page.update(cx, |page, cx| page.set_config_values(config, cx));
        }
        read_layers(this, false, cx)
    })
    .detach();
    if !root.bridge.is_acp() {
        read_layers(root, false, cx);
    }
}

/// `Filter settings by host` choices: this window's host, then every other connected host.
fn filter_hosts(root: &Root, cx: &App) -> Vec<(SharedString, SharedString)> {
    let rows = cx
        .try_global::<ui::screens::connections::ConnectionsState>()
        .and_then(|state| state.connections.clone())
        .or_else(|| root.host.remote_connections().ok().map(|saved| saved.rows))
        .unwrap_or_default();
    let name = |id: &str| {
        rows.iter()
            .find(|row| row.host_id == id)
            .map_or(id, |row| row.display_name.as_str())
            .to_owned()
    };
    let own = match root.host_id.as_str() {
        "local" => "All settings".to_owned(),
        id => name(id),
    };
    let mut hosts = vec![(root.host_id.clone().into(), own.into())];
    hosts.extend(
        rows.iter()
            .filter(|row| row.host_id != root.host_id)
            .filter(|row| crate::connections::bridge(&row.host_id, cx).is_some())
            .map(|row| (row.host_id.clone().into(), row.display_name.clone().into())),
    );
    hosts
}

/// The host the settings page's filter selects; this window's host without a page or filter.
fn page_host(root: &Root, cx: &App) -> String {
    root.settings_page
        .as_ref()
        .map(|page| page.read(cx).config_host().to_owned())
        .filter(|host| !host.is_empty())
        .unwrap_or_else(|| root.host_id.clone())
}

/// [`page_host`] and its bridge; `None` while that host is disconnected.
fn selected_host(root: &Root, cx: &App) -> Option<(String, Arc<Bridge>)> {
    let host = page_host(root, cx);
    if host == root.host_id {
        return Some((host, root.bridge.clone()));
    }
    let bridge = crate::connections::bridge(&host, cx)?;
    Some((host, bridge))
}

/// `config/read` with layers on the page's selected host. `open` then opens the user layer's
/// file, the one native settings write, rather than a locally guessed `$CODEX_HOME`.
fn read_layers(root: &mut Root, open: bool, cx: &mut Context<Root>) {
    let Some((host, bridge)) = selected_host(root, cx) else {
        return root.toast(Toast::danger("This host is no longer connected"), cx);
    };
    let other = host != root.host_id;
    cx.spawn(async move |this, cx| {
        let read = bridge
            .request::<_, Value>("config/read", json!({"includeLayers": true}))
            .await;
        this.update(cx, |this, cx| {
            let read = match read {
                Ok(read) => read,
                Err(error) => {
                    eprintln!("codex-gpui: config/read layers on {host}: {error:#}");
                    if open || other {
                        this.toast(Toast::danger("Unable to read config.toml"), cx);
                    }
                    return;
                }
            };
            let file = user_file(&read).map(str::to_owned);
            // A late read for a host the filter has since left is dropped.
            if page_host(this, cx) != host {
                return;
            }
            if let Some(page) = &this.settings_page {
                page.update(cx, |page, cx| {
                    if other {
                        page.set_config_values(read["config"].clone(), cx);
                    }
                    page.set_config_layers(read, cx)
                });
            }
            if !open {
                return;
            }
            match file {
                Some(file) if host == "local" => cx.open_with_system(std::path::Path::new(&file)),
                // shortcut: a remote host's file cannot open in a local editor; name it until
                // remote editing exists.
                Some(file) => this.toast(
                    Toast::danger(format!("config.toml is on {host}: {file}")),
                    cx,
                ),
                None => this.toast(Toast::danger("No user config.toml on this host"), cx),
            }
        })
        .ok();
    })
    .detach();
}

/// Sends a persisted config write to the filter's host when it is not this window's host;
/// `false` leaves the write to this window's bridge.
pub(super) fn write_on_selected_host(
    root: &mut Root,
    change: &SettingChanged,
    cx: &mut Context<Root>,
) -> bool {
    let Some((host, bridge)) = selected_host(root, cx) else {
        root.toast(Toast::danger("This host is no longer connected"), cx);
        return true;
    };
    if host == root.host_id {
        return false;
    }
    let params = json!({
        "keyPath": change.path,
        "value": change.value,
        "mergeStrategy": "upsert",
        "filePath": null,
        "expectedVersion": null,
    });
    cx.spawn(async move |this, cx| {
        let write = bridge
            .request::<_, Value>("config/value/write", params)
            .await;
        this.update(cx, |this, cx| match write {
            Ok(_) => read_layers(this, false, cx),
            Err(error) => {
                eprintln!("codex-gpui: config/value/write on {host}: {error:#}");
                this.toast(Toast::danger("Couldn't save your changes. Try again"), cx);
                // Re-read so the page drops the value the host refused.
                read_layers(this, false, cx);
            }
        })
        .ok();
    })
    .detach();
    true
}

/// The user layer's file in a `config/read` response with layers.
fn user_file(read: &Value) -> Option<&str> {
    read["layers"]
        .as_array()?
        .iter()
        .find(|layer| layer["name"]["type"] == "user")?["name"]["file"]
        .as_str()
}

/// Publishes the host's `enabled-reasoning-efforts` to composers.
pub(super) fn sync_enabled_efforts(cx: &mut App) {
    let enabled = cx
        .try_global::<host_settings::Settings>()
        .and_then(|settings| settings.read_all().get(ENABLED_EFFORTS).cloned())
        .and_then(|value| serde_json::from_value(value).ok());
    cx.set_global(EnabledEfforts(enabled));
}

/// A saved `enabled-reasoning-efforts` re-filters every composer's effort choices.
pub(super) fn efforts_saved(root: &mut Root, cx: &mut Context<Root>) {
    sync_enabled_efforts(cx);
    let mut composers = root.thread_composers();
    composers.push(root.home.read(cx).composer.clone());
    for composer in composers {
        composer.update(cx, |c, cx| c.refresh_efforts(&root.models, cx));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_file_reads_the_user_layer() {
        let read = json!({"layers": [
            {"name": {"type": "system", "file": "/etc/codex/config.toml"}},
            {"name": {"type": "user", "file": "/srv/home/.codex/config.toml", "profile": null}},
        ]});
        assert_eq!(user_file(&read), Some("/srv/home/.codex/config.toml"));
        assert_eq!(user_file(&json!({"layers": null})), None);
    }
}
