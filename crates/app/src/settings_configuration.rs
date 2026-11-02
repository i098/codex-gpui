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

/// Gives the page the catalog efforts, reads the layers, and opens `config.toml` on request.
pub(super) fn attach(
    root: &mut Root,
    page: &Entity<settings::SettingsPage>,
    cx: &mut Context<Root>,
) {
    page.update(cx, |page, cx| page.set_models(&root.models, cx));
    cx.subscribe(page, |this, _, _: &settings::OpenConfigToml, cx| {
        read_layers(this, true, cx)
    })
    .detach();
    if !root.bridge.is_acp() {
        read_layers(root, false, cx);
    }
}

/// `config/read` with layers on this window's host. `open` then opens the user layer's
/// file, the one native settings write, rather than a locally guessed `$CODEX_HOME`.
pub(crate) fn read_layers(root: &mut Root, open: bool, cx: &mut Context<Root>) {
    let bridge = root.bridge.clone();
    let host = (root.host_id != "local").then(|| root.host_id.clone());
    cx.spawn(async move |this, cx| {
        let read = bridge
            .request::<_, Value>("config/read", json!({"includeLayers": true}))
            .await;
        this.update(cx, |this, cx| {
            let read = match read {
                Ok(read) => read,
                Err(error) => {
                    eprintln!("codex-gpui: config/read layers: {error:#}");
                    if open {
                        this.toast(Toast::danger("Unable to read config.toml"), cx);
                    }
                    return;
                }
            };
            let file = user_file(&read).map(str::to_owned);
            if let Some(page) = &this.settings_page {
                page.update(cx, |page, cx| page.set_config_layers(read, cx));
            }
            if !open {
                return;
            }
            match (file, host) {
                (Some(file), None) => cx.open_with_system(std::path::Path::new(&file)),
                // shortcut: a remote host's file cannot open in a local editor; name it until
                // remote editing exists.
                (Some(file), Some(host)) => this.toast(
                    Toast::danger(format!("config.toml is on {host}: {file}")),
                    cx,
                ),
                (None, _) => this.toast(Toast::danger("No user config.toml on this host"), cx),
            }
        })
        .ok();
    })
    .detach();
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
