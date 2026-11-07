//! Space ▸ `Files`: the Library lists, opens folders, toggles Favorites, and moves files
//! through Trash with the original's `/files/library/*` requests.

mod common;

use anyhow::{Context as _, Result, bail, ensure};
use app::host::{Host, Request};
use common::Fx;
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};
use ui::confirmation::Outcome;
use ui::screens::space::{
    SpacePage,
    library::{Action, Items, Node, Tab},
};

#[derive(Clone)]
struct File {
    id: &'static str,
    name: &'static str,
    parent: Option<&'static str>,
    trashed: bool,
    favorite: Option<String>,
}

/// An in-memory Library account that records each request line.
#[derive(Default)]
struct Backend {
    files: Mutex<Vec<File>>,
    log: Mutex<Vec<String>>,
}

const FOLDER: &str = "dir-walks";

fn node(file: &File) -> Value {
    json!({"id": file.id, "kind": "file", "name": file.name, "file_id": format!("f-{}", file.id),
        "parent_directory_id": file.parent})
}

impl Host for Backend {
    fn fetch(&self, request: &Request) -> Result<Value> {
        let Request::Http(http) = request else {
            bail!("unexpected local request");
        };
        let method = http["method"].as_str().context("method")?;
        let url = http["url"].as_str().context("url")?;
        let body: Value = http["body"]
            .as_str()
            .map(serde_json::from_str)
            .transpose()?
            .unwrap_or(Value::Null);
        self.log.lock().unwrap().push(format!("{method} {url}"));
        let (path, query) = url.split_once('?').unwrap_or((url, ""));
        let param = |key: &str| {
            url::form_urlencoded::parse(query.as_bytes())
                .find(|(k, _)| k == key)
                .map(|(_, v)| v.into_owned())
        };
        let mut files = self.files.lock().unwrap();
        let reply = match (method, path) {
            ("GET", "/spaces/v2") => json!({"items": [], "next_cursor": null}),
            ("GET", "/files/library/nodes") => {
                ensure!(param("entry_surface").as_deref() == Some("spaces"));
                let parent = param("parent_directory_id");
                let mut items = Vec::new();
                if parent.is_none() {
                    items.push(json!({"id": FOLDER, "kind": "directory", "name": "Walks"}));
                }
                if param("include_files").as_deref() == Some("true") {
                    items.extend(
                        files
                            .iter()
                            .filter(|f| !f.trashed && f.parent.map(str::to_owned) == parent)
                            .map(node),
                    );
                }
                json!({"items": items, "cursor": null})
            }
            ("POST", "/files/library/favorites/status") => {
                let items: Vec<Value> = body["target_refs"]
                    .as_array()
                    .context("target_refs")?
                    .iter()
                    .map(|target| {
                        let id = target["library_file_id"].as_str().unwrap_or_default();
                        let favorite = files
                            .iter()
                            .find(|f| f.id == id)
                            .and_then(|f| f.favorite.clone());
                        json!({"target_ref": target, "availability": "available",
                            "favorite": favorite.map(|id| json!({"favorite_id": id}))})
                    })
                    .collect();
                json!({"items": items})
            }
            ("GET", "/files/library/favorites") => {
                let items: Vec<Value> = files
                    .iter()
                    .filter(|f| f.favorite.is_some() && !f.trashed)
                    .map(|f| {
                        json!({"favorite_id": f.favorite, "availability": "available",
                            "item": {"id": f.id, "file_name": f.name, "directory_id": f.parent}})
                    })
                    .collect();
                json!({"items": items, "cursor": null})
            }
            ("POST", "/files/library/favorites") => {
                let id = body["target_ref"]["library_file_id"]
                    .as_str()
                    .context("id")?;
                let file = files.iter_mut().find(|f| f.id == id).context("file")?;
                file.favorite = Some(format!("fav-{id}"));
                json!({"favorite_id": file.favorite})
            }
            ("POST", "/files/library") => {
                ensure!(body["trashed_only"] == true, "{body}");
                let items: Vec<Value> = files
                    .iter()
                    .filter(|f| f.trashed)
                    .map(|f| json!({"id": f.id, "file_name": f.name, "directory_id": f.parent}))
                    .collect();
                json!({"items": items, "cursor": null})
            }
            ("DELETE", "/files/library/trash") => {
                let before = files.len();
                files.retain(|f| !f.trashed);
                json!({"deleted_count": before - files.len(), "failed_count": 0})
            }
            _ => {
                let id = path.trim_start_matches("/files/library/");
                if let ("DELETE", Some(favorite)) = (method, id.strip_prefix("favorites/")) {
                    let file = files
                        .iter_mut()
                        .find(|f| f.favorite.as_deref() == Some(favorite));
                    file.context("favorite")?.favorite = None;
                    json!({})
                } else if let (Some(id), "DELETE") = (id.strip_prefix("files/"), method) {
                    ensure!(param("soft_delete").as_deref() == Some("true"));
                    files
                        .iter_mut()
                        .find(|f| f.id == id)
                        .context("file")?
                        .trashed = true;
                    json!({"success": true})
                } else if let Some(id) = id
                    .strip_prefix("files/")
                    .and_then(|id| id.strip_suffix("/restore"))
                {
                    files
                        .iter_mut()
                        .find(|f| f.id == id)
                        .context("file")?
                        .trashed = false;
                    json!({})
                } else {
                    bail!("unexpected {method} {url}");
                }
            }
        };
        Ok(json!({"status": 200, "body": reply.to_string()}))
    }
}

fn names(fx: &Fx) -> Option<Vec<(String, bool)>> {
    fx.read(|root, cx| {
        let Items::Ready(nodes) = &root.space_page()?.read(cx).library.as_ref()?.items else {
            return None;
        };
        Some(
            nodes
                .iter()
                .map(|n| (n.name.clone(), n.favorite_id.is_some()))
                .collect(),
        )
    })
}

fn rows(list: &[(&str, bool)]) -> Option<Vec<(String, bool)>> {
    Some(list.iter().map(|(n, f)| (n.to_string(), *f)).collect())
}

fn node_named(fx: &Fx, name: &str) -> Node {
    fx.read(|root, cx| {
        let Some(Items::Ready(nodes)) = root
            .space_page()
            .and_then(|p| p.read(cx).library.as_ref().map(|l| l.items.clone()))
        else {
            panic!("no Library list");
        };
        nodes.into_iter().find(|n| n.name == name).expect(name)
    })
}

#[test]
fn library_lists_favorites_and_trash() -> Result<()> {
    let mut fx = Fx::new()?;
    let backend = Arc::new(Backend::default());
    *backend.files.lock().unwrap() = vec![
        File {
            id: "notes",
            name: "walking-notes.txt",
            parent: None,
            trashed: false,
            favorite: None,
        },
        File {
            id: "route",
            name: "walking-route.svg",
            parent: Some(FOLDER),
            trashed: false,
            favorite: Some("fav-route".into()),
        },
    ];
    fx.root
        .update(&mut fx.cx, |root, cx| root.set_host(backend.clone(), cx));
    fx.action(Box::new(ui::OpenSpace))?;
    let page = fx
        .read(|root, _| root.space_page().cloned())
        .context("space page")?;
    fx.capture_evidence("CGX_LIBRARY_EVIDENCE", "before-space")?;
    let update = |fx: &mut Fx, f: &dyn Fn(&mut SpacePage, &mut gpui::Context<SpacePage>)| {
        fx.cx.update(|cx| page.update(cx, |p, cx| f(p, cx)));
    };

    // `Files`: the root lists folders, then files, with their Favorites marks.
    update(&mut fx, &|p, cx| p.open_library(cx));
    fx.wait("the Library root", |fx| names(fx).is_some())?;
    ensure!(names(&fx) == rows(&[("Walks", false), ("walking-notes.txt", false)]));
    fx.capture_evidence("CGX_LIBRARY_EVIDENCE", "after-library-all")?;

    // A folder row opens the folder; its breadcrumb returns to the root.
    let walks = node_named(&fx, "Walks");
    update(&mut fx, &|p, cx| p.open_folder(&walks, cx));
    fx.wait("the folder", |fx| {
        names(fx) == rows(&[("walking-route.svg", true)])
    })?;
    update(&mut fx, &|p, cx| p.open_crumb(0, cx));
    fx.wait("the root again", |fx| {
        names(fx).is_some_and(|n| n.len() == 2)
    })?;

    // `Folders` lists only folders.
    update(&mut fx, &|p, cx| p.set_tab(Tab::Folders, cx));
    fx.wait("the Folders tab", |fx| {
        names(fx) == rows(&[("Walks", false)])
    })?;

    // `Add to Favorites`, then the `Favorites` tab shows it beside the older favorite.
    update(&mut fx, &|p, cx| p.set_tab(Tab::All, cx));
    fx.wait("All", |fx| names(fx).is_some_and(|n| n.len() == 2))?;
    let notes = node_named(&fx, "walking-notes.txt");
    update(&mut fx, &|_, cx| cx.emit(Action::Favorite(notes.clone())));
    fx.wait("the favorite mark", |fx| {
        names(fx) == rows(&[("Walks", false), ("walking-notes.txt", true)])
    })?;
    update(&mut fx, &|p, cx| p.set_tab(Tab::Favorites, cx));
    fx.wait("Favorites", |fx| {
        names(fx) == rows(&[("walking-notes.txt", true), ("walking-route.svg", true)])
    })?;
    fx.capture_evidence("CGX_LIBRARY_EVIDENCE", "after-library-favorites")?;

    // `Remove from Favorites` drops it from the tab.
    let notes = node_named(&fx, "walking-notes.txt");
    update(&mut fx, &|_, cx| cx.emit(Action::Favorite(notes.clone())));
    fx.wait("one favorite", |fx| {
        names(fx) == rows(&[("walking-route.svg", true)])
    })?;

    // `Delete` moves the file to Trash; `Restore` brings it back.
    update(&mut fx, &|p, cx| p.set_tab(Tab::All, cx));
    fx.wait("All", |fx| names(fx).is_some_and(|n| n.len() == 2))?;
    let notes = node_named(&fx, "walking-notes.txt");
    update(&mut fx, &|_, cx| cx.emit(Action::Delete(notes.clone())));
    fx.wait("the deleted file gone", |fx| {
        names(fx) == rows(&[("Walks", false)])
    })?;
    update(&mut fx, &|p, cx| p.open_trash(cx));
    fx.wait("Trash", |fx| {
        names(fx) == rows(&[("walking-notes.txt", false)])
    })?;
    fx.capture_evidence("CGX_LIBRARY_EVIDENCE", "after-library-trash")?;
    let trashed = node_named(&fx, "walking-notes.txt");
    update(&mut fx, &|_, cx| cx.emit(Action::Restore(trashed.clone())));
    fx.wait("an empty Trash", |fx| names(fx) == rows(&[]))?;

    // `Empty file trash` asks first, then deletes every trashed file.
    update(&mut fx, &|p, cx| p.set_tab(Tab::All, cx));
    fx.wait("All", |fx| names(fx).is_some_and(|n| n.len() == 2))?;
    let notes = node_named(&fx, "walking-notes.txt");
    update(&mut fx, &|_, cx| cx.emit(Action::Delete(notes.clone())));
    fx.wait("the deleted file gone", |fx| {
        names(fx) == rows(&[("Walks", false)])
    })?;
    update(&mut fx, &|p, cx| p.open_trash(cx));
    fx.wait("Trash", |fx| names(fx).is_some_and(|n| n.len() == 1))?;
    fx.cx.update_window(fx.window, |_, window, cx| {
        page.update(cx, |p, cx| p.confirm_empty_trash(window, cx))
    })?;
    fx.cx.run_until_parked();
    fx.capture_evidence("CGX_LIBRARY_EVIDENCE", "after-library-empty-trash-confirm")?;
    let confirmation = fx
        .read(|_, cx| page.read(cx).confirmation().cloned())
        .context("Delete forever? dialog")?;
    fx.cx
        .update(|cx| confirmation.update(cx, |_, cx| cx.emit(Outcome::Confirm)));
    fx.wait("Trash emptied", |fx| names(fx) == rows(&[]))?;
    ensure!(backend.files.lock().unwrap().len() == 1);

    let log = backend.log.lock().unwrap();
    for line in [
        "GET /files/library/nodes?entry_surface=spaces&include_files=true&include_saved_entities=true",
        "GET /files/library/nodes?entry_surface=spaces&parent_directory_id=dir-walks&include_files=true&include_saved_entities=true",
        "GET /files/library/nodes?entry_surface=spaces&include_files=false&include_saved_entities=true",
        "POST /files/library/favorites/status",
        "POST /files/library/favorites",
        "GET /files/library/favorites?limit=50",
        "DELETE /files/library/favorites/fav-notes",
        "DELETE /files/library/files/notes?file_id=f-notes&file_name=walking-notes.txt&soft_delete=true",
        "POST /files/library",
        "POST /files/library/files/notes/restore",
        "DELETE /files/library/trash",
    ] {
        ensure!(log.iter().any(|l| l == line), "no request {line}: {log:#?}");
    }
    Ok(())
}
