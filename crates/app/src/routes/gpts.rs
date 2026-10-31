use crate::host::{Host, Identity, Request};
use crate::*;
use gpui::Div;
use ui::OpenGpts;
use ui::screens::gpts::{self, Load, Open, Page, Refresh, View};

pub(super) fn actions(root: Div, cx: &mut Context<Root>) -> Div {
    root.on_action(
        cx.listener(|this, _: &OpenGpts, window, cx| this.navigate(Route::Gpts(None), window, cx)),
    )
}

pub(super) fn page(
    root: &mut Root,
    id: Option<String>,
    window: &mut Window,
    cx: &mut Context<Root>,
) -> Entity<Shell> {
    let page = cx.new(|_| Page::new(id));
    cx.subscribe(&page, |this, page, _: &Refresh, cx| {
        this.load_gpts(page, cx)
    })
    .detach();
    cx.subscribe_in(&page, window, |this, _, Open(id): &Open, window, cx| {
        this.navigate(Route::Gpts(Some(id.clone())), window, cx)
    })
    .detach();
    root.load_gpts(page.clone(), cx);
    root.gpts_page = Some(page.clone());
    gpts::shell(page, cx)
}

/// A gizmos read with the original's request encoding (`fixtures/gpts.json`).
fn request(identity: &Identity, url: &str) -> Request {
    Request::Http(json!({
        "url": url,
        "method": "GET",
        "headers": {
            "x-openai-web-frontend": "codex_webview",
            "x-oai-mcp-form-version": "2",
            "x-openai-codex-window-type": "electron",
            "OAI-Language": "en-US",
            "oai-did": identity.device_id,
            "X-OpenAI-Attach-Auth": "1",
            "X-OpenAI-Attach-Desktop-Surface": "1",
            "X-OpenAI-Attach-Integrity-State": "1",
        },
        "returnErrorResponse": true,
    }))
}

fn read(host: &dyn Host, identity: &Identity, url: &str) -> Result<Value, String> {
    host.fetch(&request(identity, url))
        .and_then(|response| host::json_body(&response))
        .map_err(|error| error.to_string())
}

fn lists(host: &dyn Host, identity: &Identity) -> Result<gpts::Lists, String> {
    Ok(gpts::Lists {
        mine: gpts::mine(&read(host, identity, "/gizmos/bootstrap?limit=20")?)?,
        catalog: gpts::catalog(&read(host, identity, "/public-api/gizmos/discovery")?)?,
    })
}

fn ready<T>(result: Result<T, String>) -> Load<T> {
    result.map_or_else(Load::Failed, Load::Ready)
}

/// The list's two reads, else the GPT's `/gizmos/{id}`.
fn load(host: &dyn Host, view: &View) -> View {
    let identity = host.identity().ok_or_else(|| "not signed in".to_owned());
    match view {
        View::List(_) => View::List(ready(identity.and_then(|identity| lists(host, &identity)))),
        View::Gpt(id, _) => {
            let url = format!(
                "/gizmos/{}",
                url::form_urlencoded::byte_serialize(id.as_bytes()).collect::<String>()
            );
            let gpt = identity.and_then(|identity| {
                read(host, &identity, &url).and_then(|body| gpts::detail(&body))
            });
            View::Gpt(id.clone(), ready(gpt))
        }
    }
}

impl Root {
    pub(crate) fn load_gpts(&self, page: Entity<Page>, cx: &mut Context<Self>) {
        let (generation, view) = page.update(cx, |page, cx| {
            page.generation += 1;
            page.view = match &page.view {
                View::List(_) => View::List(Load::Loading),
                View::Gpt(id, _) => View::Gpt(id.clone(), Load::Loading),
            };
            cx.notify();
            (page.generation, page.view.clone())
        });
        let host = self.host.clone();
        let read = cx.background_spawn(async move { load(host.as_ref(), &view) });
        cx.spawn(async move |_, cx| {
            let view = read.await;
            page.update(cx, |page, cx| {
                if page.generation == generation {
                    page.view = view;
                    cx.notify();
                }
            });
        })
        .detach();
    }
}
