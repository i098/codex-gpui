//! Space ▸ `Files`: the original Library page's `/files/library/*` requests.

use super::{MAX_PAGES, request};
use crate::*;
use ui::screens::space::{
    SpacePage,
    library::{Action, Items, Library, Node, Tab},
};
use ui::toast::Toast;

/// The original reads Library lists and favorite status in pages of 50.
const PAGE: usize = 50;

pub(super) fn on_action(
    root: &mut Root,
    page: Entity<SpacePage>,
    action: &Action,
    cx: &mut Context<Root>,
) {
    let host = root.host.clone();
    if *action == Action::Load {
        let Some(view) = page.read(cx).library.clone() else {
            return;
        };
        let page = page.downgrade();
        cx.spawn(async move |_, cx| {
            let query = view.clone();
            let items = cx
                .background_spawn(async move { list(&*host, &query) })
                .await;
            let items = items.map_or_else(
                |error| {
                    eprintln!("codex-gpui: cannot load library files: {error:#}");
                    Items::Failed
                },
                Items::Ready,
            );
            page.update(cx, |page, cx| {
                // A later tab, folder or Trash owns the list now.
                let current = page.library.as_ref().is_some_and(|l| {
                    (l.tab, l.trash, &l.path) == (view.tab, view.trash, &view.path)
                });
                if current {
                    page.set_items(items, cx);
                }
            })
            .ok();
        })
        .detach();
        return;
    }
    let action = action.clone();
    let page = page.downgrade();
    cx.spawn(async move |root, cx| {
        let change = action.clone();
        let result = cx
            .background_spawn(async move { apply(&*host, &change) })
            .await;
        let toast = match (&action, result) {
            (Action::Favorite(node), Ok(_)) => {
                Some(Toast::success(if node.favorite_id.is_some() {
                    "Removed from Favorites"
                } else {
                    "Added to Favorites"
                }))
            }
            (Action::Favorite(_), Err(_)) => Some(Toast::danger("Could not update favorite")),
            (Action::EmptyTrash, Ok(0)) => Some(Toast::success("Trash emptied")),
            (Action::EmptyTrash, Ok(_)) => {
                Some(Toast::danger("Some items couldn't be deleted. Try again."))
            }
            (Action::EmptyTrash, Err(_)) => Some(Toast::danger("Failed to empty trash")),
            (Action::Delete(_), Err(_)) => Some(Toast::danger("Could not delete file")),
            (Action::Restore(_), Err(_)) => Some(Toast::danger("Could not restore item")),
            _ => None,
        };
        root.update(cx, |root, cx| {
            if let Some(toast) = toast {
                root.toast(toast, cx);
            }
            if let Some(page) = page.upgrade() {
                page.update(cx, |page, cx| page.reload_library(cx));
            }
        })
        .ok();
    })
    .detach();
}

/// The rows [`Library`] describes, every page of them.
fn list(host: &dyn host::Host, library: &Library) -> Result<Vec<Node>> {
    if library.trash {
        // `Gyr`: `POST /files/library` with `trashed_only` lists the file Trash.
        return paged(|cursor| {
            let body = json!({"cursor": cursor, "limit": PAGE, "trashed_only": true});
            get(host, "POST", "/files/library".into(), Some(body))
        })
        .map(|items| items.iter().filter_map(|item| node(item, None)).collect());
    }
    if library.tab == Tab::Favorites && library.path.is_empty() {
        return paged(|cursor| {
            let mut query = form();
            query.append_pair("limit", &PAGE.to_string());
            if let Some(cursor) = cursor {
                query.append_pair("cursor", cursor);
            }
            get(
                host,
                "GET",
                format!("/files/library/favorites?{}", query.finish()),
                None,
            )
        })
        .map(|entries| {
            entries
                .iter()
                .filter(|entry| entry["availability"] == "available")
                .filter_map(|entry| node(&entry["item"], entry["favorite_id"].as_str()))
                .collect()
        });
    }
    let folder = library.folder();
    let mut nodes: Vec<Node> = paged(|cursor| {
        let mut query = form();
        query.append_pair("entry_surface", "spaces");
        if let Some(folder) = folder {
            query.append_pair("parent_directory_id", folder);
        }
        if let Some(cursor) = cursor {
            query.append_pair("cursor", cursor);
        }
        // The root of `Folders` lists only folders.
        let files = library.tab != Tab::Folders || folder.is_some();
        query.append_pair("include_files", if files { "true" } else { "false" });
        query.append_pair("include_saved_entities", "true");
        get(
            host,
            "GET",
            format!("/files/library/nodes?{}", query.finish()),
            None,
        )
    })?
    .iter()
    .filter_map(|item| node(item, None))
    .collect();
    // The list stays usable without favorite marks, as in the original.
    if let Err(error) = favorite_status(host, &mut nodes) {
        eprintln!("codex-gpui: cannot read library favorites: {error:#}");
    }
    Ok(nodes)
}

/// `POST /files/library/favorites/status` for the file rows, 50 at a time.
fn favorite_status(host: &dyn host::Host, nodes: &mut [Node]) -> Result<()> {
    let files: Vec<String> = nodes
        .iter()
        .filter(|node| !node.folder)
        .map(|node| node.id.clone())
        .collect();
    for chunk in files.chunks(PAGE) {
        let refs: Vec<Value> = chunk
            .iter()
            .map(|id| json!({"type": "library_file", "library_file_id": id}))
            .collect();
        let body = json!({"target_refs": refs});
        let status = get(
            host,
            "POST",
            "/files/library/favorites/status".into(),
            Some(body),
        )?;
        for item in status["items"].as_array().into_iter().flatten() {
            let Some(id) = item["target_ref"]["library_file_id"].as_str() else {
                continue;
            };
            let favorite_id = (item["availability"] == "available")
                .then(|| item["favorite"]["favorite_id"].as_str())
                .flatten();
            if let Some(node) = nodes.iter_mut().find(|node| node.id == id) {
                node.favorite_id = favorite_id.map(str::to_owned);
            }
        }
    }
    Ok(())
}

/// Sends a change; returns the Empty trash `failed_count`, else 0.
fn apply(host: &dyn host::Host, action: &Action) -> Result<u64> {
    match action {
        Action::Load => Ok(0),
        Action::Favorite(node) => {
            match &node.favorite_id {
                Some(favorite) => get(
                    host,
                    "DELETE",
                    format!("/files/library/favorites/{}", path(favorite)),
                    None,
                )?,
                None => get(
                    host,
                    "POST",
                    "/files/library/favorites".into(),
                    Some(
                        json!({"target_ref": {"type": "library_file", "library_file_id": node.id}}),
                    ),
                )?,
            };
            Ok(0)
        }
        Action::Delete(node) => {
            let mut query = form();
            if let Some(file_id) = &node.file_id {
                query.append_pair("file_id", file_id);
            }
            if let Some(parent) = &node.parent {
                query.append_pair("parent_directory_id", parent);
            }
            query.append_pair("file_name", &node.name);
            query.append_pair("soft_delete", "true");
            let url = format!("/files/library/files/{}?{}", path(&node.id), query.finish());
            let body = get(host, "DELETE", url, None)?;
            ensure!(
                body["success"] == true,
                "Library deletion was not completed"
            );
            Ok(0)
        }
        Action::Restore(node) => {
            let url = format!("/files/library/files/{}/restore", path(&node.id));
            get(host, "POST", url, None)?;
            Ok(0)
        }
        Action::EmptyTrash => {
            let body = get(host, "DELETE", "/files/library/trash".into(), None)?;
            Ok(body["failed_count"].as_u64().unwrap_or(0))
        }
    }
}

/// A `/files/library/nodes` node, or a library file (`file_name`, `directory_id`) from
/// Favorites and Trash, as the original's `Kyr` maps it.
fn node(item: &Value, favorite_id: Option<&str>) -> Option<Node> {
    let text = |key: &str| {
        item[key]
            .as_str()
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
    };
    Some(Node {
        id: text("id")?,
        name: text("name")
            .or_else(|| text("file_name"))
            .unwrap_or_default(),
        folder: item["kind"] == "directory",
        parent: text("parent_directory_id").or_else(|| text("directory_id")),
        file_id: text("file_id"),
        favorite_id: favorite_id.map(str::to_owned),
    })
}

/// Reads `items` from each page until `cursor` is null.
fn paged(mut read: impl FnMut(Option<&str>) -> Result<Value>) -> Result<Vec<Value>> {
    let mut items = Vec::new();
    let mut cursor: Option<String> = None;
    for _ in 0..MAX_PAGES {
        let body = read(cursor.as_deref())?;
        items.extend(body["items"].as_array().cloned().unwrap_or_default());
        cursor = body["cursor"].as_str().map(str::to_owned);
        if cursor.is_none() {
            break;
        }
    }
    Ok(items)
}

fn get(host: &dyn host::Host, method: &str, url: String, body: Option<Value>) -> Result<Value> {
    host::json_body(&host.fetch(&request(method, &url, body))?)
}

fn form() -> url::form_urlencoded::Serializer<'static, String> {
    url::form_urlencoded::Serializer::new(String::new())
}

/// `id` percent-encoded as one URL path segment.
fn path(id: &str) -> String {
    let mut url = url::Url::parse("http://h/").expect("a valid base URL");
    url.path_segments_mut().expect("a base URL").push(id);
    url.path()[1..].to_owned()
}
