use super::*;
use std::io::Read as _;
use ui::screens::diff::panel::ReviewRepository;

const OWNER_LIMIT: u64 = 65_536;

fn output(cwd: &Path, args: &[&str]) -> Result<Option<Vec<u8>>> {
    let output = git::command(cwd, args).output().context("read Git repository metadata")?;
    Ok(output.status.success().then_some(output.stdout))
}

fn git_root(cwd: &Path) -> Result<Option<PathBuf>> {
    if !cwd.is_dir() { return Ok(None); }
    let Some(root) = output(cwd, &["rev-parse", "--show-toplevel"])? else { return Ok(None) };
    let root = String::from_utf8(root).context("Git root must be UTF-8")?;
    Ok(Some(Path::new(root.trim_end_matches('\n')).canonicalize()?))
}

fn choice(root: &Path, cwd: &Path, name: String) -> ReviewRepository {
    ReviewRepository { root: root.to_string_lossy().into_owned(), cwd: cwd.to_string_lossy().into_owned(), name }
}

pub(super) fn discover(home: &Path, thread: &str, cwd: &str) -> Result<Vec<ReviewRepository>> {
    let cwd = Path::new(cwd);
    ensure!(cwd.is_absolute(), "review cwd must be absolute");
    let resolved = match cwd.canonicalize() {
        Ok(path) => path,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => cwd.to_path_buf(),
        Err(error) => return Err(error).context("resolve review workspace"),
    };
    let cwd = resolved.as_path();
    let managed = home.canonicalize().unwrap_or_else(|_| home.to_path_buf()).join("worktrees");
    let state: Value = match fs::read(home.join(".codex-global-state.json")) {
        Ok(bytes) => serde_json::from_slice(&bytes).context("read review project assignments")?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => json!({}),
        Err(error) => return Err(error).context("read review project state"),
    };
    let projects = state["local-projects"].as_object();
    let assignment = &state["thread-project-assignments"][thread];
    let project = projects.and_then(|projects| {
        if assignment["projectKind"] == "local" {
            assignment["projectId"].as_str().and_then(|id| projects.get(id))
        } else {
            projects.values().filter_map(|project| {
                let score = project["rootPaths"].as_array()?.iter().filter_map(Value::as_str)
                    .filter_map(|root| Path::new(root).canonicalize().ok())
                    .filter(|root| cwd.starts_with(root)).map(|root| root.as_os_str().len()).max()?;
                Some((project, score))
            }).max_by_key(|(_, score)| *score).map(|(project, _)| project)
        }
    });
    let roots = project.and_then(|project| project["rootPaths"].as_array());
    let mut candidates = roots.into_iter().flatten().filter_map(Value::as_str).map(PathBuf::from).collect::<Vec<_>>();
    if !candidates.iter().any(|root| root.canonicalize().is_ok_and(|root| cwd.starts_with(root))) { candidates.push(cwd.to_path_buf()); }
    let mut seen = HashSet::new();
    let mut choices = Vec::new();
    for candidate in candidates {
        ensure!(candidate.is_absolute(), "project roots must be absolute");
        let Some(root) = git_root(&candidate)? else { continue };
        if !seen.insert(root.clone()) { continue; }
        let name = root.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_else(|| root.display().to_string());
        let directory = if cwd.starts_with(&root) { cwd } else { candidate.as_path() };
        choices.push(choice(&root, directory, name));
    }
    let mut worktrees = Vec::new();
    for repository in &choices {
        let root = Path::new(&repository.root);
        let Some(list) = output(root, &["worktree", "list", "--porcelain", "-z"])? else { continue };
        for row in list.split(|byte| *byte == 0) {
            let Some(path) = row.strip_prefix(b"worktree ") else { continue };
            let path = Path::new(std::str::from_utf8(path).context("worktree path must be UTF-8")?);
            let Ok(path) = path.canonicalize() else { continue };
            if !seen.insert(path.clone()) { continue; }
            let Some(owner) = output(&path, &["rev-parse", "--git-path", "codex-thread.json"])? else { continue };
            let owner = String::from_utf8(owner).context("worktree owner path must be UTF-8")?;
            let owner = path.join(owner.trim_end_matches('\n'));
            let Ok(owner_file) = fs::File::open(owner) else { continue };
            let mut bytes = Vec::new();
            if owner_file.take(OWNER_LIMIT + 1).read_to_end(&mut bytes).is_err() || bytes.len() as u64 > OWNER_LIMIT { continue; }
            let Ok(owner) = serde_json::from_slice::<Value>(&bytes) else { continue };
            if owner["version"] != 1 || owner["ownerThreadId"] != thread { continue; }
            let relative = Path::new(&repository.cwd).strip_prefix(root).unwrap_or_else(|_| Path::new(""));
            let directory = path.join(relative);
            let directory = if directory.is_dir() { directory } else { path.clone() };
            let name = if path.starts_with(&managed) {
                format!("Worktree {}", path.parent().and_then(Path::file_name).unwrap_or_default().to_string_lossy())
            } else { path.file_name().unwrap_or_default().to_string_lossy().into_owned() };
            worktrees.push(choice(&path, &directory, name));
        }
    }
    choices.extend(worktrees);
    Ok(choices)
}
