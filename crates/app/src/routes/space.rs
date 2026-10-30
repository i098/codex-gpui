use crate::*;
use gpui::Div;
use ui::screens::space::{self, Retry, Space, SpacePage, Spaces};

/// The original's `space-list-v2` query reads at most this many pages (`FDr`).
const MAX_PAGES: usize = 20;

pub(super) fn actions(root: Div, cx: &mut Context<Root>) -> Div {
    root.on_action(
        cx.listener(|this, _: &ui::OpenSpace, window, cx| this.navigate(Route::Space, window, cx)),
    )
}

pub(super) fn page(root: &mut Root, cx: &mut Context<Root>) -> Entity<Shell> {
    let (shell, page) = space::shell(cx);
    cx.subscribe(&page, |root, page, _: &Retry, cx| load(root, page, cx))
        .detach();
    load(root, page.clone(), cx);
    root.space_page = Some(page);
    shell
}

fn load(root: &Root, page: Entity<SpacePage>, cx: &mut Context<Root>) {
    page.update(cx, |page, cx| page.set_spaces(Spaces::Loading, cx));
    let host = root.host.clone();
    let page = page.downgrade();
    cx.spawn(async move |_, cx| {
        let spaces = cx.background_spawn(async move { list(&*host) }).await;
        let spaces = spaces.map_or_else(
            |error| {
                eprintln!("codex-gpui: cannot load spaces: {error:#}");
                Spaces::Failed
            },
            Spaces::Ready,
        );
        page.update(cx, |page, cx| page.set_spaces(spaces, cx)).ok();
    })
    .detach();
}

/// `GET /spaces/v2?limit=100`, following `next_cursor`, newest `updated_at` first.
fn list(host: &dyn host::Host) -> Result<Vec<Space>> {
    let mut items = Vec::new();
    let mut cursor: Option<String> = None;
    for _ in 0..MAX_PAGES {
        let mut query = url::form_urlencoded::Serializer::new(String::new());
        if let Some(cursor) = &cursor {
            query.append_pair("cursor", cursor);
        }
        let url = format!("/spaces/v2?{}", query.append_pair("limit", "100").finish());
        let body = host::json_body(&host.fetch(&request(&url))?)?;
        items.extend(body["items"].as_array().cloned().unwrap_or_default());
        cursor = body["next_cursor"].as_str().map(str::to_owned);
        if cursor.is_none() {
            break;
        }
    }
    // shortcut: RFC 3339 strings in one format sort by time; parse them if the backend mixes offsets.
    items.sort_by(|a, b| b["updated_at"].as_str().cmp(&a["updated_at"].as_str()));
    Ok(items
        .iter()
        .filter_map(|item| {
            Some(Space {
                id: item["id"].as_str()?.to_owned(),
                name: [&item["title"], &item["name"]]
                    .into_iter()
                    .find_map(|v| v.as_str().filter(|s| !s.is_empty()))
                    .unwrap_or_default()
                    .to_owned(),
            })
        })
        .collect())
}

fn request(url: &str) -> host::Request {
    host::Request::Http(json!({
        "url": url,
        "method": "GET",
        "headers": {
            "OAI-Language": "en-US",
            "X-OpenAI-Attach-Auth": "1",
            "X-OpenAI-Attach-Desktop-Surface": "1",
            "X-OpenAI-Attach-Integrity-State": "1",
            "originator": "Codex Desktop"
        },
        "returnErrorResponse": true
    }))
}
