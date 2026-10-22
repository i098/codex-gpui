use super::*;
use rustix::fs::{AtFlags, Dir, FileType, Mode, OFlags, openat, statat};
use std::{
    os::unix::{ffi::OsStrExt, fs::MetadataExt},
    path::Component,
};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Params<'a> {
    workspace_root: &'a str,
    #[serde(default)]
    directory_path: Option<&'a str>,
    #[serde(default)]
    directories_only: bool,
    #[serde(default)]
    include_hidden: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Entry {
    name: String,
    path: String,
    #[serde(rename = "type")]
    kind: &'static str,
    is_symlink: bool,
}

pub(super) fn read(params: &Value) -> Result<Value> {
    let params = Params::deserialize(params)?;
    ensure!(
        !params.workspace_root.trim().is_empty(),
        "workspaceRoot is required"
    );
    let root = resolve(Path::new(""), Path::new(params.workspace_root))?;
    let root_file = fs::File::open(&root)?;
    ensure!(
        root_file.metadata()?.is_dir(),
        "workspaceRoot must be a directory"
    );
    let directory = resolve(&root, Path::new(params.directory_path.unwrap_or("")))?;
    ensure!(
        directory.starts_with(&root),
        "directoryPath must stay within workspaceRoot"
    );
    let directory_file = fs::File::open(&directory)?;
    ensure!(
        within_root(&directory_file, &root_file)?,
        "directoryPath must stay within workspaceRoot"
    );
    ensure!(
        directory_file.metadata()?.is_dir(),
        "directoryPath must point to a directory"
    );
    let relative = directory.strip_prefix(&root)?;
    let entries = directory_entries(&directory_file, &root_file, relative, &params)?;
    let relative = path_string(relative);
    let parent =
        (!relative.is_empty()).then(|| relative.rsplit_once('/').map_or("", |(parent, _)| parent));
    Ok(json!({
        "workspaceRoot": params.workspace_root,
        "directoryPath": relative,
        "parentPath": parent,
        "entries": sorted(entries),
    }))
}

fn directory_entries(
    directory: &fs::File,
    root: &fs::File,
    relative: &Path,
    params: &Params<'_>,
) -> Result<Vec<Entry>> {
    let mut entries = Vec::new();
    for entry in Dir::read_from(directory)? {
        let entry = entry?;
        let name = entry.file_name();
        if matches!(name.to_bytes(), b"." | b"..") {
            continue;
        }
        let name = std::ffi::OsStr::from_bytes(name.to_bytes());
        let display_name = name.to_string_lossy().into_owned();
        if !params.include_hidden && display_name.starts_with('.') {
            continue;
        }
        let kind =
            FileType::from_raw_mode(statat(directory, name, AtFlags::SYMLINK_NOFOLLOW)?.st_mode);
        let is_symlink = kind == FileType::Symlink;
        let is_directory = if is_symlink {
            match openat(
                directory,
                name,
                OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC,
                Mode::empty(),
            ) {
                Ok(target) => {
                    if !within_root(&fs::File::from(target), root)? {
                        continue;
                    }
                    true
                }
                Err(_) => false,
            }
        } else {
            kind == FileType::Directory
        };
        if params.directories_only && !is_directory {
            continue;
        }
        entries.push(Entry {
            name: display_name,
            path: path_string(&relative.join(name)),
            kind: if is_directory { "directory" } else { "file" },
            is_symlink,
        });
    }
    Ok(entries)
}

fn within_root(directory: &fs::File, root: &fs::File) -> Result<bool> {
    let identity = |file: &fs::File| -> std::io::Result<_> {
        let metadata = file.metadata()?;
        Ok((metadata.dev(), metadata.ino()))
    };
    let root_id = identity(root)?;
    let mut previous_id = identity(directory)?;
    if previous_id == root_id {
        return Ok(true);
    }
    let parent_of = |file: &fs::File| {
        openat(
            file,
            c"..",
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map(fs::File::from)
    };
    let mut ancestor = parent_of(directory)?;
    loop {
        let id = identity(&ancestor)?;
        if id == root_id {
            return Ok(true);
        }
        if id == previous_id {
            return Ok(false);
        }
        previous_id = id;
        ancestor = parent_of(&ancestor)?;
    }
}

fn resolve(base: &Path, path: &Path) -> Result<PathBuf> {
    let mut resolved = if path.is_absolute() {
        PathBuf::new()
    } else if base.is_absolute() {
        base.to_path_buf()
    } else {
        std::env::current_dir()?.join(base)
    };
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                resolved.pop();
            }
            other => resolved.push(other.as_os_str()),
        }
    }
    Ok(resolved)
}

fn path_string(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

fn sorted(entries: Vec<Entry>) -> Vec<Entry> {
    #[cfg(target_os = "macos")]
    {
        use objc2_foundation::NSString;
        let mut keyed: Vec<_> = entries
            .into_iter()
            .map(|entry| {
                let name = NSString::from_str(&entry.name);
                (entry, name)
            })
            .collect();
        keyed.sort_by(|(a, a_name), (b, b_name)| {
            a.kind
                .cmp(b.kind)
                .then_with(|| a_name.localizedCompare(b_name).into())
        });
        keyed.into_iter().map(|(entry, _)| entry).collect()
    }
    #[cfg(not(target_os = "macos"))]
    {
        let mut entries = entries;
        entries.sort_by(|a, b| a.kind.cmp(b.kind).then_with(|| a.name.cmp(&b.name)));
        entries
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn init() {
        #[cfg(target_os = "macos")]
        std::panic::set_hook(Box::new(|info| {
            eprintln!("{info}");
            std::process::exit(101);
        }));
    }

    #[test]
    fn directory_requests_filter_entries_and_preserve_relative_paths() -> Result<()> {
        init();
        let root = tempfile::tempdir()?;
        fs::create_dir(root.path().join("src"))?;
        fs::write(root.path().join("z.txt"), "z")?;
        fs::write(root.path().join("a.txt"), "a")?;
        fs::write(root.path().join(".hidden"), "hidden")?;
        fs::write(root.path().join("src/lib.rs"), "pub fn value() {}")?;
        let host = LocalHost::new(root.path());
        let request = |directory: &str, hidden, directories| {
            Request::post(
                "workspace-directory-entries",
                Some(json!({
                    "workspaceRoot": root.path(), "directoryPath": directory,
                    "includeHidden": hidden, "directoriesOnly": directories,
                })),
            )
        };
        let response = json_body(&host.fetch(&request("", false, false))?)?;
        assert_eq!(response["directoryPath"], "");
        assert!(response["parentPath"].is_null());
        assert_eq!(
            response["entries"]
                .as_array()
                .unwrap()
                .iter()
                .map(|entry| entry["name"].as_str().unwrap())
                .collect::<Vec<_>>(),
            ["src", "a.txt", "z.txt"]
        );
        assert_eq!(
            json_body(&host.fetch(&request("src/../src/", false, false))?)?["entries"][0]["path"],
            "src/lib.rs"
        );
        let nested = json_body(&host.fetch(&request("src", false, false))?)?;
        assert_eq!(nested["parentPath"], "");
        let hidden = json_body(&host.fetch(&request("", true, true))?)?;
        assert_eq!(hidden["entries"].as_array().unwrap().len(), 1);
        assert_eq!(hidden["entries"][0]["type"], "directory");
        let all = json_body(&host.fetch(&request("", true, false))?)?;
        assert!(
            all["entries"]
                .as_array()
                .unwrap()
                .iter()
                .any(|entry| entry["name"] == ".hidden")
        );
        for name in [" padded ", r"back\slash"] {
            fs::create_dir(root.path().join(name))?;
            fs::write(root.path().join(name).join("note.txt"), "note")?;
            let listing = json_body(&host.fetch(&request("", true, false))?)?;
            let returned = listing["entries"]
                .as_array()
                .unwrap()
                .iter()
                .find(|entry| entry["name"] == name)
                .unwrap()["path"]
                .as_str()
                .unwrap();
            let nested = json_body(&host.fetch(&request(returned, false, false))?)?;
            assert_eq!(nested["entries"][0]["path"], format!("{name}/note.txt"));
        }
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn directory_requests_reject_lexical_and_symlink_escapes() -> Result<()> {
        init();
        use std::os::unix::fs::symlink;
        let parent = tempfile::tempdir()?;
        let root = parent.path().join("workspace");
        let outside = parent.path().join("outside");
        fs::create_dir_all(root.join("src"))?;
        fs::create_dir(&outside)?;
        fs::write(outside.join("secret"), "private")?;
        symlink(&outside, root.join("outside-link"))?;
        symlink(root.join("src"), root.join("inside-link"))?;
        let host = LocalHost::new(parent.path());
        let request = |directory| {
            Request::post(
                "workspace-directory-entries",
                Some(json!({
                    "workspaceRoot": root, "directoryPath": directory, "includeHidden": true,
                })),
            )
        };
        for directory in ["../outside", "outside-link"] {
            assert!(host.fetch(&request(directory)).is_err());
        }
        let response = json_body(&host.fetch(&request(""))?)?;
        assert!(
            !response["entries"]
                .as_array()
                .unwrap()
                .iter()
                .any(|entry| entry["name"] == "outside-link")
        );
        assert!(
            response["entries"]
                .as_array()
                .unwrap()
                .iter()
                .any(|entry| entry["name"] == "inside-link" && entry["isSymlink"] == true)
        );
        assert!(
            host.fetch(&Request::post(
                "workspace-directory-entries",
                Some(json!({
                    "workspaceRoot": root, "hostId": "unavailable-remote",
                }))
            ))
            .is_err()
        );
        Ok(())
    }

    #[test]
    fn opened_directory_does_not_follow_a_replaced_symlink() -> Result<()> {
        init();
        use std::os::unix::fs::symlink;
        let parent = tempfile::tempdir()?;
        let root = parent.path().join("workspace");
        let inside = root.join("inside");
        let outside = parent.path().join("outside");
        let link = root.join("moving");
        fs::create_dir_all(&inside)?;
        fs::create_dir(&outside)?;
        fs::write(inside.join("safe.txt"), "safe")?;
        fs::write(outside.join("secret.txt"), "private")?;
        symlink(&inside, &link)?;
        let directory = fs::File::open(&link)?;
        let root_file = fs::File::open(&root)?;
        assert!(within_root(&directory, &root_file)?);
        fs::remove_file(&link)?;
        symlink(&outside, &link)?;
        let params = json!({ "workspaceRoot": root, "includeHidden": true });
        let params = Params::deserialize(&params)?;
        let entries = directory_entries(&directory, &root_file, Path::new("moving"), &params)?;
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].path, "moving/safe.txt");
        assert!(
            LocalHost::new(parent.path())
                .fetch(&Request::post(
                    "workspace-directory-entries",
                    Some(json!({ "workspaceRoot": root, "directoryPath": "moving" })),
                ))
                .is_err()
        );
        Ok(())
    }
}
