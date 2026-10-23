use crate::{
    Root,
    host::{Host, Identity, Request, json_body},
};
use anyhow::{Context as _, Result, ensure};
use bridge::Bridge;
use gpui::{AppContext as _, Context, SharedString};
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

const CUSTOMIZATION: &str = "inline-sidebar-customization-state-by-account-id-v2";
const SNAPSHOTS: &str = "sidebar-customization-snapshots-v2";
const SEEN: &str = "sidebar-more-menu-seen-mcp-destination-ids-by-account-id-v1";
const CACHE: &str = "mcp-global-entrypoints-v2";

#[derive(Debug, PartialEq)]
struct Scope {
    account: String,
    user: String,
}

fn scope(identity: Option<Identity>) -> Result<Option<Scope>> {
    identity
        .map(|identity| {
            ensure!(
                !identity.account_id.is_empty() && !identity.user_id.is_empty(),
                "navigation identity is empty"
            );
            Ok(Scope {
                account: identity.account_id,
                user: identity.user_id,
            })
        })
        .transpose()
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Customization {
    version: u8,
    #[serde(default)]
    visibility_by_account_id: HashMap<String, HashMap<String, bool>>,
}

fn pin_overrides(atoms: &mut Value, scope: Option<&Scope>) -> HashMap<String, bool> {
    if let Some(scope) = scope {
        let key = serde_json::to_string(&[scope.account.as_str(), scope.user.as_str()])
            .expect("navigation identity serializes");
        if let Some(snapshot) = atoms
            .get_mut(SNAPSHOTS)
            .and_then(|value| value.get_mut(&key))
            .filter(|value| !value.is_null())
        {
            if !snapshot["order"]
                .as_array()
                .is_some_and(|order| order.iter().all(Value::is_string))
            {
                return HashMap::new();
            }
            return snapshot
                .get_mut("pinOverrides")
                .map(Value::take)
                .and_then(|value| serde_json::from_value(value).ok())
                .unwrap_or_default();
        }
    }
    let customization: Customization = atoms
        .get_mut(CUSTOMIZATION)
        .map(Value::take)
        .and_then(|value| serde_json::from_value(value).ok())
        .unwrap_or_default();
    let mut visibility = customization.visibility_by_account_id;
    if customization.version == 2 {
        visibility
            .remove(scope.map_or("anonymous", |scope| scope.account.as_str()))
            .unwrap_or_default()
    } else {
        HashMap::new()
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CachedEntrypoint {
    tool_name: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CachedServer {
    server: String,
    plugin_id: Option<String>,
    tools: Vec<Value>,
    entrypoints: Vec<CachedEntrypoint>,
}

struct Persisted {
    scope: Option<Scope>,
    overrides: HashMap<String, bool>,
    seen: HashSet<String>,
    cached: Option<Vec<CachedServer>>,
}

fn persisted(host: &dyn Host, authenticated: bool) -> Result<Persisted> {
    let scope = scope(host.identity())?;
    ensure!(
        !authenticated || scope.is_some(),
        "authenticated navigation identity is unavailable"
    );
    let request = Request::post(
        "get-global-state",
        Some(json!({"key": "electron-persisted-atom-state"})),
    );
    let mut body = json_body(&host.fetch(&request)?)?;
    let mut atoms = body
        .get_mut("value")
        .context("missing persisted navigation state")?
        .take();
    ensure!(
        atoms.is_null() || atoms.is_object(),
        "persisted navigation state is not an object"
    );
    let account = scope
        .as_ref()
        .map_or("anonymous", |scope| scope.account.as_str());
    let overrides = pin_overrides(&mut atoms, scope.as_ref());
    let seen = match atoms
        .get_mut(SEEN)
        .map(Value::take)
        .filter(|value| !value.is_null())
    {
        Some(value) => {
            let mut accounts: HashMap<String, HashSet<String>> =
                serde_json::from_value(value).context("invalid seen-destination state")?;
            accounts.remove(account).unwrap_or_default()
        }
        None => HashSet::new(),
    };
    let cached = scope.as_ref().and_then(|scope| {
        let key = serde_json::to_string(&[scope.user.as_str(), scope.account.as_str(), "local"])
            .expect("navigation identity serializes");
        atoms
            .get_mut(CACHE)?
            .get_mut(&key)
            .map(Value::take)
            .and_then(|value| serde_json::from_value(value).ok())
    });
    Ok(Persisted {
        scope,
        overrides,
        seen,
        cached,
    })
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Server {
    name: String,
    plugin_id: Option<String>,
    tools: serde_json::Map<String, Value>,
    tools_error: Option<Value>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Catalog {
    data: Vec<Server>,
    next_cursor: Option<String>,
    coverage: Option<HashSet<String>>,
}

async fn catalog(bridge: &Bridge) -> Result<Catalog> {
    let request = |cursor: Option<&str>| json!({"cursor": cursor, "detail": "toolsAndAuthOnly", "limit": 100});
    let mut catalog: Catalog = bridge
        .request("mcpServerStatus/list", request(None))
        .await?;
    let mut cursors = HashSet::new();
    while let Some(cursor) = catalog.next_cursor.take() {
        let params = request(Some(&cursor));
        ensure!(cursors.insert(cursor), "MCP catalog repeated its cursor");
        let page: Catalog = bridge.request("mcpServerStatus/list", params).await?;
        catalog.data.extend(page.data);
        catalog.next_cursor = page.next_cursor;
        catalog.coverage = None;
    }
    Ok(catalog)
}

struct Destination {
    id: SharedString,
    legacy_id: Option<SharedString>,
    visible_by_default: bool,
}

fn resource_uri(meta: &Value) -> Option<&str> {
    if meta.get("ui").is_some_and(|value| !value.is_object()) {
        return None;
    }
    let resources = [
        meta.get("ui").and_then(|ui| ui.get("resourceUri")),
        meta.get("ui/resourceUri"),
    ];
    if resources.iter().flatten().any(|value| !value.is_string()) {
        return None;
    }
    let template = meta.get("openai/outputTemplate");
    if template.is_some_and(|value| !value.is_null() && !value.is_string()) {
        return None;
    }
    resources
        .into_iter()
        .flatten()
        .find_map(Value::as_str)
        .or_else(|| template.and_then(Value::as_str))
}

fn connector_action(meta: &Value) -> Option<(&str, &str)> {
    let uri = meta["_codex_apps"]["resource_uri"].as_str()?;
    let mut parts = uri.split('/');
    let empty = parts.next()?;
    let connector = parts.next()?;
    let target = parts.next()?;
    let action = parts.next()?;
    (empty.is_empty()
        && !connector.is_empty()
        && !target.is_empty()
        && !action.is_empty()
        && parts.next().is_none())
    .then_some((connector, action))
}

fn destination(server: &str, tool: &Value) -> Option<Destination> {
    let name = tool["name"].as_str()?;
    let meta = &tool["_meta"];
    if !resource_uri(meta)?.starts_with("ui://")
        || !meta["openai/ui"]["entrypoints"]
            .as_array()?
            .iter()
            .any(|entry| entry["type"] == "global")
    {
        return None;
    }
    let connector = if server == "codex_apps" {
        meta.get("connectorId")
            .or_else(|| meta.get("connector_id"))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|id| !id.is_empty())
            .unwrap_or(server)
    } else {
        server
    };
    let (canonical_connector, action) = if server == "codex_apps" {
        connector_action(meta).unwrap_or((connector, name))
    } else {
        (connector, name)
    };
    if ui::RAIL_DESTINATIONS.iter().any(|entry| {
        entry.mcp_entrypoints.contains(&(server, name))
            || entry
                .mcp_entrypoints
                .contains(&(canonical_connector, action))
    }) {
        return None;
    }
    Some(Destination {
        legacy_id: ((canonical_connector, action) != (connector, name))
            .then(|| destination_id(connector, name)),
        id: destination_id(canonical_connector, action),
        visible_by_default: server == "code-review",
    })
}

fn destination_id(connector: &str, action: &str) -> SharedString {
    let mut bytes = Vec::with_capacity(
        connector.len() + action.len() + b"mcp:[\"mcp-global\",\"\",\"\",\"global\"]".len(),
    );
    bytes.extend_from_slice(b"mcp:");
    serde_json::to_writer(&mut bytes, &["mcp-global", connector, action, "global"])
        .expect("destination identity serializes");
    String::from_utf8(bytes).expect("JSON is UTF-8").into()
}

fn append_cached(destinations: &mut Vec<Destination>, server: &CachedServer) {
    for tool in &server.tools {
        if server
            .entrypoints
            .iter()
            .any(|entry| tool["name"] == entry.tool_name)
            && let Some(destination) = destination(&server.server, tool)
        {
            destinations.push(destination);
        }
    }
}

fn append_server(
    destinations: &mut Vec<Destination>,
    server: &Server,
    cached: Option<&[CachedServer]>,
) {
    if server.tools_error.is_some()
        && let Some(cached) = cached
    {
        for saved in cached
            .iter()
            .filter(|saved| saved.server == server.name && saved.plugin_id == server.plugin_id)
        {
            append_cached(destinations, saved);
        }
        return;
    }
    destinations.extend(
        server
            .tools
            .values()
            .filter_map(|tool| destination(&server.name, tool)),
    );
}

fn eligible(catalog: Option<&Catalog>, cached: Option<&[CachedServer]>) -> Vec<Destination> {
    let mut destinations: Vec<_> = ui::RAIL_DESTINATIONS
        .iter()
        .map(|entry| Destination {
            id: entry.id.into(),
            legacy_id: None,
            visible_by_default: true,
        })
        .collect();
    if let Some(catalog) = catalog {
        for server in &catalog.data {
            append_server(&mut destinations, server, cached);
        }
    }
    if let Some(cached) = cached {
        for server in cached.iter().filter(|server| {
            catalog.is_none_or(|catalog| {
                catalog
                    .coverage
                    .as_ref()
                    .is_some_and(|coverage| !coverage.contains(&server.server))
            })
        }) {
            append_cached(&mut destinations, server);
        }
    }
    let mut ids = HashSet::new();
    destinations.retain(|destination| ids.insert(destination.id.clone()));
    destinations
}

fn pinned(
    destinations: &[Destination],
    overrides: &HashMap<String, bool>,
) -> HashSet<SharedString> {
    let mut pins = HashSet::new();
    for destination in destinations {
        let value = overrides.get(destination.id.as_ref()).or_else(|| {
            destination
                .legacy_id
                .as_ref()
                .and_then(|id| overrides.get(id.as_ref()))
        });
        if value == Some(&true) {
            pins.insert(destination.id.clone());
        }
    }
    for destination in destinations {
        if pins.len() >= 8 {
            break;
        }
        let overridden = overrides.contains_key(destination.id.as_ref())
            || destination
                .legacy_id
                .as_ref()
                .is_some_and(|id| overrides.contains_key(id.as_ref()));
        if destination.visible_by_default && !overridden {
            pins.insert(destination.id.clone());
        }
    }
    pins
}

fn unread(destinations: Vec<Destination>, persisted: &Persisted) -> Arc<[SharedString]> {
    let pins = pinned(&destinations, &persisted.overrides);
    destinations
        .into_iter()
        .filter(|destination| {
            !pins.contains(&destination.id)
                && destination.id.starts_with("mcp:")
                && !persisted.seen.contains(destination.id.as_ref())
        })
        .map(|destination| destination.id)
        .collect()
}

impl Root {
    pub(crate) fn set_unread_destinations(
        &mut self,
        unread: Option<Arc<[SharedString]>>,
        cx: &mut Context<Self>,
    ) {
        self.unread_destinations = unread.clone();
        for shell in std::iter::once(&self.shell).chain(self.page.iter()) {
            shell.update(cx, |shell, cx| {
                shell.unread_destinations = unread.clone();
                cx.notify();
            });
        }
    }

    pub(crate) fn load_destinations(&mut self, cx: &mut Context<Self>) {
        self.destination_load = None;
        self.set_unread_destinations(None, cx);
        let host = self.host.clone();
        let authenticated = self.chatgpt_account;
        let read = cx.background_spawn(async move { persisted(host.as_ref(), authenticated) });
        let host = self.host.clone();
        let bridge = self.bridge.clone();
        self.destination_load = Some(cx.spawn(async move |this, cx| {
            let result: Result<_> = async {
                let persisted = read.await?;
                let catalog = match catalog(&bridge).await {
                    Ok(catalog) => Some(catalog),
                    Err(error) if persisted.cached.is_some() => {
                        eprintln!("codex-gpui: using the persisted MCP catalog after a read failure: {error:#}");
                        None
                    }
                    Err(error) => return Err(error),
                };
                let current = cx.background_spawn(async move { scope(host.identity()) }).await?;
                ensure!(current == persisted.scope, "navigation account changed during the read");
                let destinations = eligible(catalog.as_ref(), persisted.cached.as_deref());
                Ok(unread(destinations, &persisted))
            }.await;
            this.update(cx, |this, cx| match result {
                Ok(unread) => this.set_unread_destinations(Some(unread), cx),
                Err(error) => eprintln!("codex-gpui: cannot read navigation destinations: {error:#}"),
            }).ok();
        }));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::LocalHost;

    fn tool(name: &str) -> Value {
        json!({
            "name": name,
            "_meta": {
                "ui": {"resourceUri": "ui://notes/app"},
                "openai/ui": {"entrypoints": [{"type": "global"}]}
            }
        })
    }

    fn empty_state() -> Persisted {
        Persisted {
            scope: None,
            overrides: HashMap::new(),
            seen: HashSet::new(),
            cached: None,
        }
    }

    #[test]
    fn pinned_and_unpinned_destinations_follow_canonical_overrides() {
        let mut tool = tool("notes.open");
        tool["_meta"]["connector_id"] = json!("connector_notes");
        tool["_meta"]["_codex_apps"] = json!({"resource_uri": "/connector_notes/link_a/open"});
        let destination = destination("codex_apps", &tool).unwrap();
        let canonical = "mcp:[\"mcp-global\",\"connector_notes\",\"open\",\"global\"]";
        let legacy = "mcp:[\"mcp-global\",\"connector_notes\",\"notes.open\",\"global\"]";
        assert_eq!(destination.id.as_ref(), canonical);
        assert_eq!(destination.legacy_id.as_deref(), Some(legacy));
        let destinations = vec![destination];
        let mut state = empty_state();
        state.overrides.insert(legacy.into(), true);
        assert!(pinned(&destinations, &state.overrides).contains(canonical));
        state.overrides.insert(canonical.into(), false);
        assert!(!pinned(&destinations, &state.overrides).contains(canonical));
        assert_eq!(
            unread(destinations, &state).as_ref(),
            &[SharedString::from(canonical)]
        );
    }

    #[test]
    fn seen_destinations_do_not_light_explore() {
        let mut state = empty_state();
        let entry = destination("notes", &tool("open")).unwrap();
        let id = entry.id.clone();
        assert_eq!(
            unread(vec![entry], &state).as_ref(),
            std::slice::from_ref(&id)
        );
        state.seen.insert(id.to_string());
        assert!(unread(vec![destination("notes", &tool("open")).unwrap()], &state).is_empty());
    }

    struct ScopedHost {
        local: LocalHost,
        account: String,
        user: String,
    }

    impl Host for ScopedHost {
        fn fetch(&self, request: &Request) -> Result<Value> {
            self.local.fetch(request)
        }
        fn identity(&self) -> Option<Identity> {
            Some(Identity {
                account_id: self.account.clone(),
                user_id: self.user.clone(),
                device_id: "test".into(),
            })
        }
    }

    #[test]
    fn cached_destinations_and_seen_state_remain_account_separated() -> Result<()> {
        let home = tempfile::tempdir()?;
        let cache = |name: &str| {
            json!([{
                "server": "notes", "pluginId": null, "pluginVersion": null, "httpOrigin": null,
                "tools": [tool(name)],
                "entrypoints": [{"toolName": name, "title": "Notes", "icon": null}]
            }])
        };
        let a = "mcp:[\"mcp-global\",\"notes\",\"a\",\"global\"]";
        let b = "mcp:[\"mcp-global\",\"notes\",\"b\",\"global\"]";
        let atoms = json!({
            (CUSTOMIZATION): {"version": 2, "orderByAccountId": {}, "visibilityByAccountId": {
                "account-a": {(b): true}, "account-b": {(a): true}
            }},
            (SEEN): {"account-a": [a]},
            (CACHE): {
                "[\"user-a\",\"account-a\",\"local\"]": cache("a"),
                "[\"user-b\",\"account-b\",\"local\"]": cache("b")
            }
        });
        std::fs::write(
            home.path().join(".codex-global-state.json"),
            json!({"electron-persisted-atom-state": atoms}).to_string(),
        )?;
        let read = |account: &str, user: &str| {
            persisted(
                &ScopedHost {
                    local: LocalHost::new(home.path()),
                    account: account.into(),
                    user: user.into(),
                },
                true,
            )
        };
        let state_a = read("account-a", "user-a")?;
        assert!(unread(eligible(None, state_a.cached.as_deref()), &state_a).is_empty());
        let state_b = read("account-b", "user-b")?;
        assert_eq!(
            unread(eligible(None, state_b.cached.as_deref()), &state_b).as_ref(),
            &[SharedString::from(b)]
        );
        let wrong_user = read("account-a", "user-b")?;
        assert!(unread(eligible(None, wrong_user.cached.as_deref()), &wrong_user).is_empty());
        Ok(())
    }

    #[test]
    fn a_complete_catalog_drops_removed_cached_destinations() {
        let cached: Vec<CachedServer> = serde_json::from_value(json!([{
            "server": "notes", "pluginId": null, "tools": [tool("old")],
            "entrypoints": [{"toolName": "old", "title": "Notes", "icon": null}]
        }]))
        .unwrap();
        let state = empty_state();
        assert_eq!(
            unread(eligible(None, Some(&cached)), &state).as_ref(),
            &[SharedString::from(
                "mcp:[\"mcp-global\",\"notes\",\"old\",\"global\"]"
            )]
        );
        let catalog = Catalog {
            data: Vec::new(),
            next_cursor: None,
            coverage: None,
        };
        assert!(unread(eligible(Some(&catalog), Some(&cached)), &state).is_empty());
    }

    #[test]
    fn only_valid_global_ui_tools_become_destinations() {
        let mut candidate = tool("open");
        candidate["_meta"]["openai/ui"]["entrypoints"] = json!([{"type": "thread"}]);
        assert!(destination("notes", &candidate).is_none());
        candidate["_meta"]["openai/ui"]["entrypoints"] = json!([{"type": "global"}]);
        candidate["_meta"]["ui"]["resourceUri"] = json!("https://example.com/app");
        assert!(destination("notes", &candidate).is_none());
        candidate["_meta"]["ui"]["resourceUri"] = Value::Null;
        candidate["_meta"]["openai/outputTemplate"] = json!("ui://notes/app");
        assert!(destination("notes", &candidate).is_none());
        candidate["_meta"]["ui"]
            .as_object_mut()
            .unwrap()
            .remove("resourceUri");
        assert_eq!(
            destination("notes", &candidate).unwrap().id.as_ref(),
            "mcp:[\"mcp-global\",\"notes\",\"open\",\"global\"]"
        );
        candidate["_meta"]["ui"] = json!(42);
        assert!(destination("notes", &candidate).is_none());
    }

    #[test]
    fn partial_catalog_does_not_reuse_a_different_plugin_or_removed_server() {
        let saved = |server: &str, plugin: Option<&str>| {
            json!({
                "server": server, "pluginId": plugin, "tools": [tool("open")],
                "entrypoints": [{"toolName": "open", "title": server, "icon": null}]
            })
        };
        let cached: Vec<CachedServer> = serde_json::from_value(json!([
            saved("notes", Some("old-plugin")),
            saved("calendar", None),
            saved("removed", None)
        ]))
        .unwrap();
        let mut catalog: Catalog = serde_json::from_value(json!({
            "data": [
                {"name": "notes", "pluginId": "new-plugin", "tools": {}, "toolsError": "unavailable"},
                {"name": "removed", "pluginId": null, "tools": {}, "toolsError": null}
            ],
            "coverage": ["notes", "removed"], "nextCursor": null
        })).unwrap();
        assert_eq!(
            unread(eligible(Some(&catalog), Some(&cached)), &empty_state()).as_ref(),
            &[SharedString::from(
                "mcp:[\"mcp-global\",\"calendar\",\"open\",\"global\"]"
            )]
        );
        catalog.data[0].plugin_id = Some("old-plugin".into());
        assert_eq!(
            unread(eligible(Some(&catalog), Some(&cached)), &empty_state()).as_ref(),
            &[
                SharedString::from("mcp:[\"mcp-global\",\"notes\",\"open\",\"global\"]"),
                SharedString::from("mcp:[\"mcp-global\",\"calendar\",\"open\",\"global\"]"),
            ]
        );
    }

    #[test]
    fn default_pins_fill_eight_slots_without_overriding_explicit_choices() {
        let destinations: Vec<_> = (0..10)
            .map(|index| Destination {
                id: format!("destination-{index}").into(),
                legacy_id: None,
                visible_by_default: true,
            })
            .collect();
        let overrides = HashMap::from([
            ("destination-0".into(), false),
            ("destination-9".into(), true),
        ]);
        let expected: HashSet<SharedString> = (1..8)
            .chain(std::iter::once(9))
            .map(|index| format!("destination-{index}").into())
            .collect();
        assert_eq!(pinned(&destinations, &overrides), expected);
    }

    #[test]
    fn native_routes_do_not_create_duplicate_unseen_mcp_destinations() {
        assert!(destination("code-review", &tool("pull_requests.open")).is_none());
        let mut connector = tool("review.pull_requests.open");
        connector["_meta"]["connector_id"] = json!("connector_openai_code_review");
        connector["_meta"]["_codex_apps"] = json!({
            "resource_uri": "/connector_openai_code_review/link/pull_requests.open"
        });
        assert!(destination("codex_apps", &connector).is_none());
        assert!(destination("notes", &tool("open")).is_some());
    }

    #[test]
    fn signed_in_snapshot_pins_are_scoped_and_precede_legacy_state() -> Result<()> {
        let home = tempfile::tempdir()?;
        let id = "mcp:[\"mcp-global\",\"notes\",\"open\",\"global\"]";
        let key = "[\"account-a\",\"user-a\"]";
        let mut atoms = json!({
            (SNAPSHOTS): {
                (key): {"order": [], "pinOverrides": {(id): true}}
            }
        });
        let save = |atoms: &Value| {
            std::fs::write(
                home.path().join(".codex-global-state.json"),
                json!({"electron-persisted-atom-state": atoms}).to_string(),
            )
        };
        let read = |account: &str, user: &str| -> Result<Arc<[SharedString]>> {
            let state = persisted(
                &ScopedHost {
                    local: LocalHost::new(home.path()),
                    account: account.into(),
                    user: user.into(),
                },
                true,
            )?;
            Ok(unread(
                vec![destination("notes", &tool("open")).unwrap()],
                &state,
            ))
        };
        save(&atoms)?;
        assert!(read("account-a", "user-a")?.is_empty());
        assert_eq!(
            read("account-b", "user-a")?.as_ref(),
            &[SharedString::from(id)]
        );
        atoms[SNAPSHOTS][key]["pinOverrides"][id] = json!(false);
        atoms[CUSTOMIZATION] = json!({
            "version": 2, "orderByAccountId": {},
            "visibilityByAccountId": {"account-a": {(id): true}}
        });
        save(&atoms)?;
        assert_eq!(
            read("account-a", "user-a")?.as_ref(),
            &[SharedString::from(id)]
        );
        assert!(read("account-a", "user-b")?.is_empty());
        atoms[SNAPSHOTS][key]["pinOverrides"] = json!({});
        save(&atoms)?;
        assert_eq!(
            read("account-a", "user-a")?.as_ref(),
            &[SharedString::from(id)]
        );
        Ok(())
    }
}
