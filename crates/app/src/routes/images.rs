use crate::host::{Identity, Request};
use crate::*;
use anyhow::Context as _;
use gpui::Div;
use ui::OpenImages;
use ui::screens::images::{self, Load, Page, Refresh};

pub(super) fn actions(root: Div, cx: &mut Context<Root>) -> Div {
    root.on_action(
        cx.listener(|this, _: &OpenImages, window, cx| this.navigate(Route::Images, window, cx)),
    )
}

pub(super) fn page(root: &mut Root, cx: &mut Context<Root>) -> Entity<Shell> {
    let page = cx.new(|_| Page::new());
    cx.subscribe(&page, |this, page, _: &Refresh, cx| {
        this.load_images(page, cx)
    })
    .detach();
    root.load_images(page.clone(), cx);
    root.images_page = Some(page.clone());
    images::shell(page, cx)
}

/// A Library read with the original's request encoding (`fixtures/images-library.json`).
fn request(identity: &Identity, url: &str) -> Request {
    Request::Http(json!({
        "url": url,
        "method": "GET",
        "headers": {
            "OAI-Language": "en-US",
            "X-OpenAI-Attach-Auth": "1",
            "X-OpenAI-Attach-Integrity-State": "1",
            "originator": "Codex Desktop",
            "oai-did": identity.device_id,
        },
        "expectedIdentity": {"accountId": identity.account_id, "userId": identity.user_id},
        "returnErrorResponse": true,
    }))
}

/// The Library's image tab (`categories=image`), first page.
const NODES: &str = "/files/library/nodes?entry_surface=library&categories=image&include_files=true&include_saved_entities=true";

/// `GET /files/library/files/{id}/thumbnail_url` (`{url}`), then the image at that URL.
fn thumbnail(host: &dyn Host, identity: &Identity, id: &str) -> Result<Arc<gpui::Image>> {
    let url = format!("/files/library/files/{id}/thumbnail_url");
    let body = host::json_body(&host.fetch(&request(identity, &url))?)?;
    super::maps::image(host, body["url"].as_str().context("no thumbnail URL")?)
}

impl Root {
    pub(crate) fn load_images(&self, page: Entity<Page>, cx: &mut Context<Self>) {
        let generation = page.update(cx, |page, cx| {
            page.generation += 1;
            page.load = Load::Loading;
            cx.notify();
            page.generation
        });
        let host = self.host.clone();
        let read = cx.background_spawn(async move {
            let identity = host.identity().ok_or_else(|| "not signed in".to_owned())?;
            let list = host
                .fetch(&request(&identity, NODES))
                .and_then(|response| host::json_body(&response))
                .map_err(|error| error.to_string())
                .and_then(|body| images::images(&body))?;
            Ok::<_, String>((identity, list))
        });
        let host = self.host.clone();
        cx.spawn(async move |_, cx| {
            let result = read.await;
            let list = result.as_ref().map(|(_, list)| list.clone());
            let current = page.update(cx, |page, cx| {
                let current = page.generation == generation;
                if current {
                    page.load = list.map_or_else(Load::Failed, Load::Ready);
                    cx.notify();
                }
                current
            });
            let Ok((identity, list)) = result else { return };
            if !current {
                return;
            }
            let thumbnails = cx
                .background_spawn(async move {
                    list.iter()
                        .filter_map(|image| {
                            let thumbnail = thumbnail(host.as_ref(), &identity, &image.id).ok()?;
                            Some((image.id.clone(), thumbnail))
                        })
                        .collect()
                })
                .await;
            page.update(cx, |page, cx| {
                if page.generation == generation {
                    page.thumbnails = thumbnails;
                    cx.notify();
                }
            });
        })
        .detach();
    }
}
