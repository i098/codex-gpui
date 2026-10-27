use super::*;
use base64::{Engine as _, engine::general_purpose::STANDARD};
use gpui::ClipboardItem;

impl Root {
    pub(super) fn copy_diff_patch(&mut self, panel: Entity<DiffPanel>, cx: &mut Context<Self>) {
        let Some(params) = panel.read(cx).patch_params() else {
            return;
        };
        let host = self.host.clone();
        let read = cx.background_spawn(async move {
            let body = host::json_body(
                &host.fetch(&host::Request::post("git-review-patch", Some(params)))?,
            )?;
            let encoded = body["contentsBase64"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("patch response has no bytes"))?;
            Ok(String::from_utf8(STANDARD.decode(encoded)?)?)
        });
        cx.spawn(async move |this, cx| {
            let result: anyhow::Result<String> = read.await;
            let _ = this.update(cx, |this, cx| {
                match result {
                    Ok(patch) => {
                        cx.write_to_clipboard(ClipboardItem::new_string(format!(
                            " (cd \"$(git rev-parse --show-toplevel)\" && git apply --3way <<'EOF' \n{patch} \nEOF\n)"
                        )));
                        this.toast(Toast::info("Copied git apply command to the clipboard"), cx);
                    }
                    Err(error) => {
                        eprintln!("codex-gpui: cannot export patch: {error:#}");
                        this.toast(Toast::danger("Couldn’t generate git apply command"), cx);
                    }
                }
            });
        }).detach();
    }
}
