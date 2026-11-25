use crate::*;
use gpui::Div;
use ui::screens::space::{self, Retry, Space, SpacePage, Spaces, library::Action};

mod library;

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
    cx.subscribe(&page, |root, page, _: &space::PageRetry, cx| {
        load_pages(root, page, cx)
    })
    .detach();
    cx.subscribe(&page, |root, page, action: &Action, cx| {
        library::on_action(root, page, action, cx)
    })
    .detach();
    load(root, page.clone(), cx);
    load_pages(root, page.clone(), cx);
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

/// `GET /pages?surface=private&limit=50`, following `next_cursor`, newest `last_used_at` first.
fn load_pages(root: &Root, page: Entity<SpacePage>, cx: &mut Context<Root>) {
    page.update(cx, |page, cx| page.set_pages(space::Pages::Loading, cx));
    let host = root.host.clone();
    let page = page.downgrade();
    cx.spawn(async move |_, cx| {
        let pages = cx.background_spawn(async move { pages(&*host) }).await;
        let pages = pages.map_or_else(
            |error| {
                eprintln!("codex-gpui: cannot load pages: {error:#}");
                space::Pages::Failed
            },
            space::Pages::Ready,
        );
        page.update(cx, |page, cx| page.set_pages(pages, cx)).ok();
    })
    .detach();
}

fn pages(host: &dyn host::Host) -> Result<Vec<space::Page>> {
    let mut items = Vec::new();
    let mut cursor: Option<String> = None;
    for _ in 0..MAX_PAGES {
        let mut query = url::form_urlencoded::Serializer::new(String::new());
        query.append_pair("surface", "private");
        if let Some(cursor) = &cursor {
            query.append_pair("cursor", cursor);
        }
        let url = format!("/pages?{}", query.append_pair("limit", "50").finish());
        let body = host::json_body(&host.fetch(&request("GET", &url, None))?)?;
        items.extend(body["items"].as_array().cloned().unwrap_or_default());
        cursor = body["next_cursor"].as_str().map(str::to_owned);
        if cursor.is_none() {
            break;
        }
    }
    // shortcut: RFC 3339 strings in one format sort by time; a page with no `last_used_at` sorts last.
    items.sort_by(|a, b| b["last_used_at"].as_str().cmp(&a["last_used_at"].as_str()));
    Ok(items
        .iter()
        .filter_map(|item| {
            Some(space::Page {
                id: item["page_id"].as_str().or(item["id"].as_str())?.to_owned(),
                title: item["title"]
                    .as_str()
                    .filter(|title| !title.is_empty())
                    .unwrap_or("Untitled")
                    .to_owned(),
                document_type: item["document_type"].as_str().map(str::to_owned),
            })
        })
        .collect())
}

/// `POST /pages` as the original's create flow sends it (`app-initial-6350d04672d0.js`, the
/// `idempotency_key:S,initial_blocks:[]` literal): a private, untitled, empty Page. The server
/// answers `metadata.page_id`; `idempotency_key` is the client's local page id.
// shortcut: no parent Page and no realtime edge headers; landing wiring and sub-pages come with #1598.
#[allow(dead_code)]
fn create_page(host: &dyn host::Host, idempotency_key: &str) -> Result<space::Page> {
    let body = json!({
        "destination": "personal_file_drive",
        "idempotency_key": idempotency_key,
        "initial_blocks": [],
        "title": "",
    });
    let created = host::json_body(&host.fetch(&request("POST", "/pages", Some(body)))?)?;
    let id = created["metadata"]["page_id"]
        .as_str()
        .context("POST /pages answered no metadata.page_id")?;
    Ok(space::Page {
        id: id.to_owned(),
        title: "Untitled".to_owned(),
        document_type: created["metadata"]["document_type"].as_str().map(str::to_owned),
    })
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
        let body = host::json_body(&host.fetch(&request("GET", &url, None))?)?;
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

fn request(method: &str, url: &str, body: Option<Value>) -> host::Request {
    let mut request = json!({
        "url": url,
        "method": method,
        "headers": {
            "OAI-Language": "en-US",
            "X-OpenAI-Attach-Auth": "1",
            "X-OpenAI-Attach-Desktop-Surface": "1",
            "X-OpenAI-Attach-Integrity-State": "1",
            "originator": "Codex Desktop"
        },
        "returnErrorResponse": true
    });
    if let Some(body) = body {
        request["body"] = body.to_string().into();
        request["headers"]["Content-Type"] = "application/json".into();
    }
    host::Request::Http(request)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    struct Recorder(Mutex<Vec<Value>>);

    impl host::Host for Recorder {
        fn fetch(&self, request: &host::Request) -> Result<Value> {
            let host::Request::Http(http) = request else {
                panic!("unexpected local request");
            };
            self.0.lock().unwrap().push(http.clone());
            Ok(json!({"status": 200, "body": r#"{"metadata":{"page_id":"p1"}}"#}))
        }
    }

    #[test]
    fn create_page_posts_the_original_body() -> Result<()> {
        let host = Recorder(Mutex::default());
        let page = create_page(&host, "local-1")?;
        let sent = host.0.lock().unwrap();
        assert_eq!(sent[0]["method"], "POST");
        assert_eq!(sent[0]["url"], "/pages");
        let body: Value = serde_json::from_str(sent[0]["body"].as_str().unwrap())?;
        assert_eq!(
            body,
            json!({
                "destination": "personal_file_drive",
                "idempotency_key": "local-1",
                "initial_blocks": [],
                "title": "",
            })
        );
        assert_eq!(page.id, "p1");
        Ok(())
    }
}
