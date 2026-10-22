use anyhow::{Context as _, Result, bail, ensure};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, HashSet},
    fs,
    io::{Read as _, Seek as _},
    path::{Component, Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

pub(super) fn recommended(home: &Path, params: &Value) -> Value {
    let repo = home.join("vendor_imports/skills");
    let cache_path = home.join("vendor_imports/skills-curated-cache.json");
    let cache = fs::read(&cache_path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
        .filter(|value| value["fetchedAt"].is_u64() && value["skills"].is_array());
    let result = (|| -> Result<Value> {
        let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis() as u64;
        let stale = params["refresh"] == true
            || cache.as_ref().is_none_or(|cache| {
                now.saturating_sub(cache["fetchedAt"].as_u64().unwrap_or(0)) > 600_000
            });
        let mut refreshed = false;
        if stale && (repo.join(".git").exists() || !repo.join("skills/.curated").exists()) {
            refresh(&repo)?;
            refreshed = true;
        }
        let skills = catalogue(&repo)?;
        let fetched_at = if refreshed {
            now
        } else {
            cache
                .as_ref()
                .and_then(|cache| cache["fetchedAt"].as_u64())
                .unwrap_or(now)
        };
        let cache = json!({"skills":skills,"fetchedAt":fetched_at});
        fs::create_dir_all(cache_path.parent().context("Skill cache has no parent")?)?;
        let mut file = tempfile::NamedTempFile::new_in(
            cache_path.parent().context("Skill cache has no parent")?,
        )?;
        serde_json::to_writer(&mut file, &cache)?;
        file.persist(&cache_path)?;
        Ok(
            json!({"skills":cache["skills"],"fetchedAt":fetched_at,"source":if refreshed {"git"} else {"cache"},"repoRoot":repo,"error":null}),
        )
    })();
    result.unwrap_or_else(|error| json!({"skills":cache.as_ref().map(|cache| &cache["skills"]).unwrap_or(&json!([])),
        "fetchedAt":cache.as_ref().map(|cache| &cache["fetchedAt"]),"source":"cache","repoRoot":repo,"error":error.to_string()}))
}

fn refresh(repo: &Path) -> Result<()> {
    let parent = repo.parent().context("Skill repository has no parent")?;
    if !repo.join(".git").exists() {
        ensure!(
            !repo.exists(),
            "Expected {} to be a git checkout but found an existing directory",
            repo.display()
        );
        fs::create_dir_all(parent)?;
        git(
            parent,
            &[
                "clone",
                "--depth",
                "1",
                "--filter=blob:none",
                "--sparse",
                "https://github.com/openai/skills.git",
                repo.to_str()
                    .context("Skill repository path is not UTF-8")?,
            ],
        )?;
    }
    git(repo, &["fetch", "--depth", "1", "origin", "main"])?;
    git(repo, &["reset", "--hard", "FETCH_HEAD"])?;
    git(
        repo,
        &[
            "sparse-checkout",
            "set",
            "skills/.curated",
            "skills/.experimental",
        ],
    )
}

fn git(root: &Path, args: &[&str]) -> Result<()> {
    let mut output = tempfile::tempfile()?;
    let mut child = Command::new("git")
        .args(args)
        .current_dir(root)
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(Stdio::null())
        .stdout(Stdio::from(output.try_clone()?))
        .stderr(Stdio::from(output.try_clone()?))
        .spawn()?;
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        if let Some(status) = child.try_wait()? {
            output.rewind()?;
            let mut message = String::new();
            output.read_to_string(&mut message)?;
            ensure!(status.success(), "Git: {}", message.trim());
            return Ok(());
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            bail!("Recommended skill Git request timed out");
        }
        std::thread::sleep(Duration::from_millis(25));
    }
}

fn catalogue(repo: &Path) -> Result<Vec<Value>> {
    let mut seen = HashSet::new();
    let mut skills = Vec::new();
    for group in [".curated", ".experimental"] {
        let root = repo.join("skills").join(group);
        if !root.exists() {
            continue;
        }
        for entry in fs::read_dir(root)? {
            let entry = entry?;
            let name = entry.file_name();
            let name = name.to_str().context("Skill name is not UTF-8")?;
            if name.starts_with('.') {
                continue;
            }
            let path = entry.path();
            let directory = path.is_dir();
            let markdown = if directory {
                path.join("SKILL.md")
            } else {
                path.clone()
            };
            if !markdown.exists() {
                continue;
            }
            let id = if directory {
                name
            } else {
                Path::new(name)
                    .file_stem()
                    .and_then(|name| name.to_str())
                    .context("Skill file has no name")?
            };
            if !seen.insert(id.to_owned()) {
                continue;
            }
            let fields = frontmatter(&fs::read_to_string(&markdown)?);
            let interface = fs::read_to_string(path.join("agents/openai.yaml"))
                .ok()
                .map(|text| interface_fields(&text))
                .unwrap_or_default();
            let icon = |size: &str, candidates: &[String]| {
                fields
                    .get(size)
                    .cloned()
                    .or_else(|| interface.get(size).cloned())
                    .or_else(|| {
                        directory
                            .then(|| {
                                candidates
                                    .iter()
                                    .map(|name| path.join("assets").join(name))
                                    .find(|path| path.exists())
                            })
                            .flatten()
                            .map(|path| path.to_string_lossy().into_owned())
                    })
            };
            let small = icon(
                "iconSmall",
                &[
                    format!("{id}-small.svg"),
                    format!("{id}-small.png"),
                    "small.svg".into(),
                    "small.png".into(),
                    format!("{id}.svg"),
                    format!("{id}.png"),
                    "icon-small.svg".into(),
                    "icon-small.png".into(),
                ],
            );
            let large = icon(
                "iconLarge",
                &[
                    format!("{id}.png"),
                    format!("{id}.svg"),
                    "icon.png".into(),
                    "icon.svg".into(),
                    format!("{id}-large.png"),
                    format!("{id}-large.svg"),
                ],
            );
            skills.push(json!({"id":id,"name":fields.get("name").map(String::as_str).unwrap_or(id),
                "description":fields.get("description").or_else(||fields.get("shortDescription")).map(String::as_str).unwrap_or(id),
                "shortDescription":fields.get("shortDescription").or_else(||interface.get("shortDescription")),
                "iconSmall":small,"iconLarge":large,"repoPath":path.strip_prefix(repo)?.to_string_lossy().replace('\\',"/")}));
        }
    }
    skills.sort_by_cached_key(|skill| skill["name"].as_str().unwrap_or_default().to_lowercase());
    Ok(skills)
}

fn field(key: &str) -> Option<&'static str> {
    match key {
        "name" => Some("name"),
        "description" => Some("description"),
        "short-description" | "short_description" | "shortDescription" => Some("shortDescription"),
        "icon-small" | "icon_small" | "iconSmall" => Some("iconSmall"),
        "icon-large" | "icon_large" | "iconLarge" => Some("iconLarge"),
        _ => None,
    }
}

fn unquote(value: &str) -> String {
    let value = value.trim();
    let value =
        if value.len() >= 2 && value.starts_with(['\'', '"']) && value.ends_with(['\'', '"']) {
            &value[1..value.len() - 1]
        } else {
            value
        };
    let mut chars = value.chars().peekable();
    let mut result = String::with_capacity(value.len());
    while let Some(ch) = chars.next() {
        if ch == '\\' && chars.peek().is_some_and(|ch| matches!(ch, '\\' | '"')) {
            if let Some(ch) = chars.next() {
                result.push(ch);
            }
        } else {
            result.push(ch);
        }
    }
    result
}

fn frontmatter(text: &str) -> BTreeMap<&'static str, String> {
    let mut fields = BTreeMap::new();
    let mut lines = text.lines();
    if lines.next().map(str::trim_end) != Some("---") {
        return fields;
    }
    let mut nested = false;
    for line in lines.take_while(|line| !line.starts_with("---")) {
        if line.trim().is_empty() {
            continue;
        }
        let Some((key, value)) = line.trim().split_once(':') else {
            continue;
        };
        let indented = line.starts_with(char::is_whitespace);
        if !indented {
            nested = matches!(key.trim(), "metadata" | "interface");
        }
        if let Some(key) = field(key.trim())
            && (!indented || (nested && key != "name" && key != "description"))
        {
            fields.insert(key, unquote(value));
        }
    }
    fields
}

fn interface_fields(text: &str) -> BTreeMap<&'static str, String> {
    let mut fields = BTreeMap::new();
    let mut lines = text.lines().skip_while(|line| line.trim() != "interface:");
    lines.next();
    for line in lines.take_while(|line| line.starts_with(char::is_whitespace)) {
        if let Some((key, value)) = line.trim().split_once(':')
            && let Some(key) = field(key.trim())
            && key != "name"
            && key != "description"
            && !value.trim().is_empty()
        {
            fields.insert(key, unquote(value));
        }
    }
    fields
}

pub(super) fn install(home: &Path, params: &Value) -> Value {
    match install_skill(home, params) {
        Ok(destination) => json!({"success":true,"destination":destination,"error":null}),
        Err(error) => json!({"success":false,"destination":null,"error":error.to_string()}),
    }
}

fn install_skill(home: &Path, params: &Value) -> Result<PathBuf> {
    let id = params["skillId"]
        .as_str()
        .context("Recommended skill has no ID")?;
    let root = match params["installRoot"].as_str() {
        Some(root) => {
            ensure!(
                Path::new(root).is_absolute(),
                "Skill install root must be absolute"
            );
            Path::new(root).join(".codex/skills")
        }
        None => home.join("skills"),
    };
    let destination = confined(&root, id)?;
    let markdown = params["skillStatsigOverride"].as_str();
    if let Some(markdown) = markdown {
        ensure!(
            !markdown.trim().is_empty(),
            "Recommended skill markdown override is empty: {id}"
        );
    }
    fs::create_dir_all(&root)?;
    ensure_contained(&root, &destination)?;
    ensure_contained(&root, &destination.join("SKILL.md"))?;
    if markdown.is_some() && params["source"].is_null() {
        let staging = stage_skill(
            &destination,
            destination.exists().then_some(destination.as_path()),
        )?;
        finish_install(staging, &destination, markdown)?;
        return Ok(destination);
    }
    install_from_repository(home, params, &destination, markdown)?;
    Ok(destination)
}

fn install_from_repository(
    home: &Path,
    params: &Value,
    destination: &Path,
    markdown: Option<&str>,
) -> Result<()> {
    let repo = home.join("vendor_imports/skills");
    let source = confined(
        &repo,
        params["repoPath"]
            .as_str()
            .context("Recommended skill has no repository path")?,
    )?;
    if !source.exists() {
        recommended(home, &json!({"refresh":true}));
    }
    ensure!(
        source.exists(),
        "Recommended skill not found at {}",
        source.display()
    );
    ensure_contained(&repo, &source)?;
    if destination.exists() && params["forceReinstall"] != true && markdown.is_none() {
        return Ok(());
    }
    let staging = stage_skill(
        destination,
        (destination.exists() && params["forceReinstall"] != true).then_some(destination),
    )?;
    if source.is_dir() {
        copy_directory(&source, staging.path())?;
    } else {
        fs::copy(source, staging.path().join("SKILL.md"))?;
    }
    finish_install(staging, destination, markdown)?;
    Ok(())
}

fn stage_skill(destination: &Path, existing: Option<&Path>) -> Result<tempfile::TempDir> {
    let parent = destination
        .parent()
        .context("Skill destination has no parent")?;
    fs::create_dir_all(parent)?;
    let staging = tempfile::tempdir_in(parent)?;
    if let Some(existing) = existing {
        copy_directory(existing, staging.path())?;
    }
    Ok(staging)
}

fn finish_install(
    staging: tempfile::TempDir,
    destination: &Path,
    markdown: Option<&str>,
) -> Result<()> {
    validate_links(staging.path(), staging.path())?;
    if let Some(markdown) = markdown {
        fs::write(staging.path().join("SKILL.md"), markdown)?;
    }
    publish_skill(staging.path(), destination)
}

fn publish_skill(staging: &Path, destination: &Path) -> Result<()> {
    if destination.symlink_metadata().is_err() {
        return Ok(fs::rename(staging, destination)?);
    }
    let parent = destination
        .parent()
        .context("Skill destination has no parent")?;
    let backup = tempfile::tempdir_in(parent)?;
    let previous = backup.path().join("previous");
    fs::rename(destination, &previous)?;
    if let Err(error) = fs::rename(staging, destination) {
        if let Err(restore) = fs::rename(&previous, destination) {
            let saved = backup.keep().join("previous");
            bail!(
                "Skill publication failed: {error}; restore failed: {restore}. Previous skill is preserved at {}",
                saved.display()
            );
        }
        return Err(error.into());
    }
    Ok(())
}

fn confined(root: &Path, relative: &str) -> Result<PathBuf> {
    let mut path = root.to_owned();
    for component in Path::new(relative).components() {
        match component {
            Component::Normal(part) => path.push(part),
            Component::CurDir => {}
            Component::ParentDir if path != root => {
                path.pop();
            }
            _ => bail!("Invalid skill path: {relative}"),
        }
    }
    ensure!(path != root, "Invalid skill path: {relative}");
    Ok(path)
}

fn ensure_contained(root: &Path, path: &Path) -> Result<()> {
    let root = root.canonicalize()?;
    let mut parent = path;
    while parent.symlink_metadata().is_err() {
        parent = parent
            .parent()
            .context("Skill path has no existing parent")?;
    }
    ensure!(
        parent.canonicalize()?.starts_with(&root),
        "Skill path leaves its root"
    );
    Ok(())
}

fn copy_directory(source: &Path, destination: &Path) -> Result<()> {
    fs::create_dir_all(destination)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let target = destination.join(entry.file_name());
        ensure_contained(destination, &target)?;
        let kind = entry.file_type()?;
        if kind.is_dir() {
            copy_directory(&entry.path(), &target)?;
        } else if kind.is_symlink() {
            #[cfg(unix)]
            std::os::unix::fs::symlink(fs::read_link(entry.path())?, target)?;
            #[cfg(windows)]
            if entry.path().is_dir() {
                std::os::windows::fs::symlink_dir(fs::read_link(entry.path())?, target)?;
            } else {
                std::os::windows::fs::symlink_file(fs::read_link(entry.path())?, target)?;
            }
        } else {
            fs::copy(entry.path(), target)?;
        }
    }
    Ok(())
}

fn validate_links(root: &Path, directory: &Path) -> Result<()> {
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let kind = entry.file_type()?;
        if kind.is_symlink() {
            ensure_contained(root, &entry.path())?;
        } else if kind.is_dir() {
            validate_links(root, &entry.path())?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn install_preserves_existing_skill_and_rejects_escaping_paths() -> Result<()> {
        let home = tempfile::tempdir()?;
        let source = home
            .path()
            .join("vendor_imports/skills/skills/.curated/test");
        fs::create_dir_all(&source)?;
        fs::write(source.join("SKILL.md"), "upstream")?;
        let params = json!({"skillId":"test","repoPath":"skills/.curated/test"});
        let destination = install_skill(home.path(), &params)?;
        assert_eq!(
            fs::read_to_string(destination.join("SKILL.md"))?,
            "upstream"
        );
        fs::write(destination.join("SKILL.md"), "local changes")?;
        install_skill(home.path(), &params)?;
        assert_eq!(
            fs::read_to_string(destination.join("SKILL.md"))?,
            "local changes"
        );
        assert!(
            install_skill(
                home.path(),
                &json!({"skillId":"../outside","repoPath":"skills/.curated/test"})
            )
            .is_err()
        );
        assert!(
            install_skill(
                home.path(),
                &json!({"skillId":"test","repoPath":"../../outside"})
            )
            .is_err()
        );
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn install_rejects_existing_file_and_directory_symlink_escapes() -> Result<()> {
        let home = tempfile::tempdir()?;
        let outside = tempfile::tempdir()?;
        let protected = outside.path().join("SKILL.md");
        fs::write(&protected, "Keep this file")?;
        let root = home.path().join("skills");
        let override_path = root.join("override");
        fs::create_dir_all(&override_path)?;
        std::os::unix::fs::symlink(&protected, override_path.join("SKILL.md"))?;
        assert!(
            install_skill(
                home.path(),
                &json!({"skillId":"override","skillStatsigOverride":"New instructions"})
            )
            .is_err()
        );
        assert_eq!(fs::read_to_string(&protected)?, "Keep this file");

        fs::remove_file(override_path.join("SKILL.md"))?;
        let missing = outside.path().join("missing.md");
        std::os::unix::fs::symlink(&missing, override_path.join("SKILL.md"))?;
        assert!(
            install_skill(
                home.path(),
                &json!({"skillId":"override","skillStatsigOverride":"New instructions"})
            )
            .is_err()
        );
        assert!(!missing.exists());

        let source = home
            .path()
            .join("vendor_imports/skills/skills/.curated/merge");
        fs::create_dir_all(source.join("nested"))?;
        fs::write(source.join("nested/SKILL.md"), "Upstream instructions")?;
        let merge_path = root.join("merge");
        fs::create_dir_all(&merge_path)?;
        std::os::unix::fs::symlink(outside.path(), merge_path.join("nested"))?;
        assert!(
            install_skill(
                home.path(),
                &json!({"skillId":"merge","repoPath":"skills/.curated/merge",
                    "skillStatsigOverride":"New instructions","source":"catalog"})
            )
            .is_err()
        );
        assert_eq!(fs::read_to_string(&protected)?, "Keep this file");
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn source_links_cannot_escape_the_installed_skill() -> Result<()> {
        let home = tempfile::tempdir()?;
        let outside = tempfile::tempdir()?;
        let protected = outside.path().join("instructions.md");
        fs::write(&protected, "Keep these instructions")?;
        let source = home
            .path()
            .join("vendor_imports/skills/skills/.curated/linked");
        fs::create_dir_all(&source)?;
        std::os::unix::fs::symlink(&protected, source.join("SKILL.md"))?;
        let params = json!({"skillId":"linked","repoPath":"skills/.curated/linked",
            "skillStatsigOverride":"New instructions","source":"catalog"});
        assert!(install_skill(home.path(), &params).is_err());
        assert_eq!(fs::read_to_string(&protected)?, "Keep these instructions");
        assert!(!home.path().join("skills/linked").exists());

        fs::remove_file(source.join("SKILL.md"))?;
        fs::write(source.join("instructions.md"), "Internal instructions")?;
        std::os::unix::fs::symlink("instructions.md", source.join("SKILL.md"))?;
        let destination = install_skill(home.path(), &params)?;
        assert_eq!(
            fs::read_to_string(destination.join("SKILL.md"))?,
            "New instructions"
        );
        assert_eq!(
            fs::read_to_string(source.join("instructions.md"))?,
            "Internal instructions"
        );
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn copy_failures_preserve_installed_skills_and_never_publish_partial_skills() -> Result<()> {
        let home = tempfile::tempdir()?;
        let source = home
            .path()
            .join("vendor_imports/skills/skills/.curated/failing");
        fs::create_dir_all(&source)?;
        fs::write(source.join("SKILL.md"), "New instructions")?;
        let socket_path = source.join("socket");
        let socket = std::os::unix::net::UnixListener::bind(&socket_path)?;
        let destination = home.path().join("skills/existing");
        fs::create_dir_all(&destination)?;
        fs::write(destination.join("SKILL.md"), "Old instructions")?;
        let replacement = json!({"skillId":"existing","repoPath":"skills/.curated/failing","forceReinstall":true});
        assert!(install_skill(home.path(), &replacement).is_err());
        assert_eq!(
            fs::read_to_string(destination.join("SKILL.md"))?,
            "Old instructions"
        );
        let fresh = json!({"skillId":"fresh","repoPath":"skills/.curated/failing"});
        for _ in 0..2 {
            assert!(install_skill(home.path(), &fresh).is_err());
            assert!(!home.path().join("skills/fresh").exists());
        }
        drop(socket);
        fs::remove_file(socket_path)?;
        install_skill(home.path(), &replacement)?;
        assert_eq!(
            fs::read_to_string(destination.join("SKILL.md"))?,
            "New instructions"
        );
        let fresh_path = install_skill(home.path(), &fresh)?;
        assert_eq!(
            fs::read_to_string(fresh_path.join("SKILL.md"))?,
            "New instructions"
        );
        Ok(())
    }

    #[test]
    fn catalogue_preserves_metadata_and_curated_precedence() -> Result<()> {
        let root = tempfile::tempdir()?;
        for group in [".curated", ".experimental"] {
            let skill = root.path().join("skills").join(group).join("test");
            fs::create_dir_all(skill.join("agents"))?;
            fs::write(
                skill.join("SKILL.md"),
                format!(
                    "---\nname: Test\ndescription: {group}\nmetadata:\n  short-description: 'Short text'\n---\nInstructions"
                ),
            )?;
            fs::write(
                skill.join("agents/openai.yaml"),
                "interface:\n  icon_small: './assets/icon.svg'\n",
            )?;
        }
        let skills = catalogue(root.path())?;
        assert_eq!(
            skills,
            vec![
                json!({"id":"test","name":"Test","description":".curated","shortDescription":"Short text",
            "iconSmall":"./assets/icon.svg","iconLarge":null,"repoPath":"skills/.curated/test"})
            ]
        );
        Ok(())
    }
}
