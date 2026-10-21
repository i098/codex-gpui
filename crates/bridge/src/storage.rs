use crate::Bridge;
use anyhow::{Context as _, Result, bail, ensure};
use serde_json::{Value, json};
use std::{
    collections::{HashMap, HashSet},
    fs,
    path::{Path, PathBuf},
    sync::Arc,
};

pub const CATEGORIES: [&str; 6] = [
    "Chats",
    "Subagents",
    "Archived chats",
    "Unclassified chats",
    "Worktrees",
    "Other",
];
const SOURCES: [&str; 10] = [
    "cli",
    "vscode",
    "exec",
    "appServer",
    "subAgent",
    "subAgentReview",
    "subAgentCompact",
    "subAgentThreadSpawn",
    "subAgentOther",
    "unknown",
];
const USER_SOURCES: [&str; 17] = [
    "user",
    "page_comment",
    "agent_created_thread",
    "agent_forked_thread",
    "ambient_suggestion_task",
    "avatar_quick_chat",
    "chatgpt_handoff",
    "code_review",
    "codex_replay",
    "conversation_digest",
    "implement_todo",
    "local_environment_configuration",
    "mcp_app_follow_up",
    "onboarding_checklist",
    "security_remediation",
    "security_scan",
    "voice_chat",
];

#[derive(Clone, Debug, Default)]
pub struct Usage {
    pub home: PathBuf,
    pub bytes: [u64; 6],
    pub thread_bytes: Arc<HashMap<String, u64>>,
}
impl Usage {
    pub fn total(&self) -> u64 {
        self.bytes.iter().sum()
    }
}

#[derive(Clone, Debug)]
pub struct Entry {
    pub thread: Value,
    pub archived: bool,
}
impl Entry {
    pub fn id(&self) -> &str {
        self.thread["id"].as_str().unwrap_or("")
    }
    pub fn parent(&self) -> Option<&str> {
        self.thread["parentThreadId"].as_str().or_else(|| {
            self.thread["source"]["subAgent"]["thread_spawn"]["parent_thread_id"].as_str()
        })
    }
    fn updated(&self) -> Option<i64> {
        self.thread["updatedAt"].as_i64()
    }
    fn inactive_before(&self, cutoff: i64) -> bool {
        self.thread["ephemeral"] != true
            && self.thread["status"]["type"] != "active"
            && self.updated().is_some_and(|time| time > 0 && time < cutoff)
    }
    pub fn title(&self) -> &str {
        self.thread["name"]
            .as_str()
            .filter(|name| !name.is_empty())
            .or_else(|| {
                self.thread["preview"]
                    .as_str()
                    .filter(|title| !title.is_empty())
            })
            .unwrap_or("Untitled chat")
    }
}

#[derive(Clone, Debug)]
pub struct Candidate {
    pub root: Entry,
    pub descendants: Vec<Entry>,
    pub bytes: u64,
}
impl Candidate {
    fn members(&self) -> impl Iterator<Item = &Entry> {
        std::iter::once(&self.root).chain(&self.descendants)
    }
    fn unchanged(&self, other: &Self) -> bool {
        let versions: HashMap<_, _> = self
            .members()
            .map(|entry| (entry.id(), entry.updated()))
            .collect();
        self.descendants.len() == other.descendants.len()
            && other
                .members()
                .all(|entry| versions.get(entry.id()) == Some(&entry.updated()))
    }
}

async fn list_all(bridge: &Bridge, mut params: Value) -> Result<Vec<Value>> {
    let mut threads = Vec::new();
    let mut cursors = HashSet::new();
    loop {
        let mut page: Value = bridge.request("thread/list", &params).await?;
        let cursor = page["nextCursor"].as_str().map(str::to_owned);
        threads.append(
            page["data"]
                .as_array_mut()
                .context("missing chat inventory")?,
        );
        let Some(cursor) = cursor else { break };
        ensure!(
            cursors.insert(cursor.clone()),
            "repeated chat inventory cursor"
        );
        params["cursor"] = json!(cursor);
    }
    Ok(threads)
}

pub async fn inventory(bridge: &Bridge) -> Result<Vec<Entry>> {
    let mut entries = Vec::new();
    let mut ids = HashSet::new();
    for archived in [false, true] {
        let threads = list_all(bridge, json!({"archived":archived,"cursor":null,"limit":100,"modelProviders":[],"sourceKinds":SOURCES,"useStateDbOnly":true})).await?;
        for thread in threads {
            let id = thread["id"].as_str().context("missing chat id")?;
            ensure!(
                ids.insert(id.to_owned()),
                "Chat inventory changed during pagination"
            );
            entries.push(Entry { thread, archived });
        }
    }
    Ok(entries)
}

pub fn candidates(
    entries: &[Entry],
    usage: &Usage,
    cutoff: i64,
    archived_only: bool,
) -> Vec<Candidate> {
    let mut children: HashMap<&str, Vec<&Entry>> = HashMap::new();
    for entry in entries {
        if let Some(parent) = entry.parent() {
            children.entry(parent).or_default().push(entry);
        }
    }
    let mut result = Vec::new();
    for root in entries {
        let source = root.thread["threadSource"].as_str();
        if (archived_only && !root.archived)
            || root.parent().is_some()
            || !root.thread["source"].is_string()
            || root.thread["source"] == "unknown"
            || source.is_some_and(|s| !USER_SOURCES.contains(&s))
        {
            continue;
        }
        let mut descendants = Vec::new();
        let mut pending = children.get(root.id()).cloned().unwrap_or_default();
        let mut seen = HashSet::from([root.id()]);
        let mut valid = root.inactive_before(cutoff);
        while let Some(child) = pending.pop() {
            if !seen.insert(child.id()) {
                valid = false;
                break;
            }
            valid &= child.inactive_before(cutoff);
            descendants.push(child.clone());
            pending.extend(children.get(child.id()).into_iter().flatten().copied());
        }
        if !valid {
            continue;
        }
        let bytes: Option<u64> = std::iter::once(root)
            .chain(descendants.iter())
            .map(|e| usage.thread_bytes.get(e.id()).copied())
            .sum();
        if let Some(bytes) = bytes {
            result.push(Candidate {
                root: root.clone(),
                descendants,
                bytes,
            });
        }
    }
    result.sort_by_key(|candidate| candidate.root.updated());
    result
}

fn rollout_id(path: &Path) -> Option<String> {
    let name = path.file_name()?.to_str()?;
    let stem = name
        .strip_suffix(".jsonl.zst")
        .or_else(|| name.strip_suffix(".jsonl"))?;
    let id = stem.get(stem.len().checked_sub(36)?..)?;
    (id.bytes().enumerate().all(|(i, c)| {
        if [8, 13, 18, 23].contains(&i) {
            c == b'-'
        } else {
            c.is_ascii_hexdigit()
        }
    }))
    .then(|| id.to_ascii_lowercase())
}

fn walk(paths: Vec<PathBuf>, mut file: impl FnMut(&Path, u64)) -> Result<()> {
    let mut pending = paths;
    while let Some(path) = pending.pop() {
        let metadata = match fs::symlink_metadata(&path) {
            Ok(value) => value,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error.into()),
        };
        if metadata.is_symlink() {
            continue;
        }
        if metadata.is_dir() {
            for entry in fs::read_dir(&path)? {
                pending.push(entry?.path());
            }
        } else if metadata.is_file() {
            file(&path, metadata.len());
        }
    }
    Ok(())
}

pub async fn measure(
    bridge: &Bridge,
    home: &Path,
    mut publish: impl FnMut(Usage, usize),
) -> Result<Usage> {
    ensure!(fs::metadata(home)?.is_dir(), "Storage root unavailable");
    let entries = inventory(bridge).await.unwrap_or_default();
    let mut classifications = HashMap::new();
    let mut paths = HashMap::new();
    for entry in &entries {
        classifications.insert(
            entry.id().to_owned(),
            usize::from(entry.thread["source"].get("subAgent").is_some()),
        );
        if let Some(path) = entry.thread["path"]
            .as_str()
            .and_then(|path| Path::new(path).file_name())
            .and_then(|name| name.to_str())
        {
            paths.insert(path.replace(".jsonl.zst", ".jsonl"), entry.id().to_owned());
        }
    }
    let mut files = Vec::new();
    walk(
        vec![home.join("sessions"), home.join("archived_sessions")],
        |path, bytes| files.push((path.to_owned(), bytes)),
    )?;
    for (path, _) in &files {
        let Some(id) = rollout_id(path) else { continue };
        if classifications.contains_key(&id) {
            continue;
        }
        if let Ok(reply) = bridge
            .request::<_, Value>("thread/read", json!({"threadId":id,"includeTurns":false}))
            .await
        {
            let thread = &reply["thread"];
            if thread["id"] == id && thread["source"] != "unknown" {
                classifications.insert(id, usize::from(thread["source"].get("subAgent").is_some()));
            }
        }
    }
    let mut usage = Usage {
        home: home.to_owned(),
        ..Default::default()
    };
    let mut thread_bytes = HashMap::new();
    for (path, bytes) in files {
        let id = rollout_id(&path).or_else(|| {
            path.file_name()
                .and_then(|name| name.to_str())
                .and_then(|name| paths.get(&name.replace(".jsonl.zst", ".jsonl")))
                .cloned()
        });
        let rollout = path
            .to_str()
            .is_some_and(|path| path.ends_with(".jsonl") || path.ends_with(".jsonl.zst"));
        let category = if rollout {
            if path.starts_with(home.join("archived_sessions")) {
                2
            } else {
                id.as_ref()
                    .and_then(|id| classifications.get(id))
                    .copied()
                    .unwrap_or(3)
            }
        } else {
            5
        };
        usage.bytes[category] += bytes;
        if let Some(id) = id.filter(|id| classifications.contains_key(id)) {
            *thread_bytes.entry(id).or_default() += bytes;
        }
    }
    usage.thread_bytes = Arc::new(thread_bytes);
    publish(usage.clone(), 4);
    let other = fs::read_dir(home)?
        .map(|entry| entry.map(|e| e.path()))
        .collect::<std::io::Result<Vec<_>>>()?
        .into_iter()
        .filter(|path| {
            !["sessions", "archived_sessions", "worktrees"]
                .iter()
                .any(|name| path.file_name().is_some_and(|n| n == *name))
        })
        .collect();
    walk(other, |_, bytes| usage.bytes[5] += bytes)?;
    publish(usage.clone(), 5);
    walk(vec![home.join("worktrees")], |_, bytes| {
        usage.bytes[4] += bytes
    })?;
    publish(usage.clone(), 6);
    Ok(usage)
}

pub async fn supports_cleanup(bridge: &Bridge) -> bool {
    if bridge.is_acp() {
        return false;
    }
    let Ok(info) = bridge
        .initialize("codex-gpui", env!("CARGO_PKG_VERSION"))
        .await
    else {
        return false;
    };
    let version = info
        .user_agent
        .split('/')
        .nth(1)
        .and_then(|v| v.split_whitespace().next())
        .unwrap_or("");
    cleanup_version(version)
}

fn cleanup_version(version: &str) -> bool {
    let version = version.split('+').next().unwrap_or("");
    let (core, prerelease) = version.split_once('-').unwrap_or((version, ""));
    let mut parts = core.split('.');
    let mut numbers = [0u32; 3];
    for number in &mut numbers {
        let Some(part) = parts.next() else {
            return false;
        };
        if part.is_empty()
            || (part.len() > 1 && part.starts_with('0'))
            || !part.bytes().all(|c| c.is_ascii_digit())
        {
            return false;
        }
        let Ok(value) = part.parse() else {
            return false;
        };
        *number = value;
    }
    if parts.next().is_some() {
        return false;
    }
    if numbers != [0, 143, 0] {
        return numbers > [0, 143, 0];
    }
    if prerelease.is_empty() {
        return true;
    }
    let mut parts = prerelease.split('.');
    match parts.next().unwrap_or("").cmp("alpha") {
        std::cmp::Ordering::Greater => true,
        std::cmp::Ordering::Less => false,
        std::cmp::Ordering::Equal => parts
            .next()
            .and_then(|n| n.parse::<u32>().ok())
            .is_some_and(|n| n >= 16),
    }
}

#[derive(Debug, Default)]
pub struct CleanupResult {
    pub deleted: usize,
    pub skipped: usize,
    pub failed: usize,
}

pub async fn delete_selected(
    bridge: &Bridge,
    reviewed: &[Candidate],
    selected: &HashSet<String>,
    usage: &Usage,
    cutoff: i64,
    archived_only: bool,
) -> Result<CleanupResult> {
    ensure!(
        supports_cleanup(bridge).await,
        "Chat cleanup requires descendant discovery support"
    );
    let mut entries = inventory(bridge).await?;
    for candidate in reviewed {
        if entries
            .iter()
            .any(|entry| entry.id() == candidate.root.id())
        {
            continue;
        }
        let reply: Value = bridge
            .request(
                "thread/read",
                json!({"threadId":candidate.root.id(),"includeTurns":false}),
            )
            .await?;
        ensure!(
            reply["thread"]["id"] == candidate.root.id(),
            "Chat inventory changed during recovery"
        );
        entries.push(Entry {
            thread: reply["thread"].clone(),
            archived: false,
        });
    }
    let current = candidates(&entries, usage, cutoff, archived_only);
    let mut result = CleanupResult::default();
    for old in reviewed
        .iter()
        .filter(|old| selected.contains(old.root.id()))
    {
        let Some(now) = current
            .iter()
            .find(|candidate| candidate.root.id() == old.root.id() && old.unchanged(candidate))
        else {
            result.skipped += 1;
            continue;
        };
        match delete_unchanged(bridge, now).await {
            Ok(true) => result.deleted += 1,
            Ok(false) => result.skipped += 1,
            Err(_) => result.failed += 1,
        }
    }
    Ok(result)
}

async fn delete_unchanged(bridge: &Bridge, candidate: &Candidate) -> Result<bool> {
    let mut parents = vec![candidate.root.id().to_owned()];
    let mut descendants = HashSet::new();
    let expected: HashMap<_, _> = candidate
        .descendants
        .iter()
        .map(|entry| (entry.id(), entry.updated()))
        .collect();
    while let Some(parent) = parents.pop() {
        for thread in list_all(bridge, json!({"limit":200,"cursor":null,"modelProviders":null,"archived":false,"parentThreadId":parent,"sourceKinds":["subAgentThreadSpawn"],"sortDirection":"desc","sortKey":"created_at","useStateDbOnly":true})).await? {
                let entry = Entry {
                    thread,
                    archived: false,
                };
                if entry.parent() != Some(parent.as_str()) {
                    continue;
                }
                if expected.get(entry.id()) != Some(&entry.updated())
                    || entry.thread["status"]["type"] == "active"
                {
                    return Ok(false);
                }
                if !descendants.insert(entry.id().to_owned()) {
                    bail!("cyclic descendant inventory")
                }
                parents.push(entry.id().to_owned());
            }
    }
    for old in candidate.members() {
        let reply: Value = bridge
            .request(
                "thread/read",
                json!({"threadId":old.id(),"includeTurns":false}),
            )
            .await?;
        let thread = &reply["thread"];
        if thread["id"] != old.id()
            || thread["updatedAt"] != old.thread["updatedAt"]
            || thread["status"]["type"] == "active"
        {
            return Ok(false);
        }
    }
    bridge
        .request::<_, Value>("thread/delete", json!({"threadId":candidate.root.id()}))
        .await?;
    Ok(true)
}

#[cfg(test)]
#[path = "storage_tests.rs"]
mod tests;
