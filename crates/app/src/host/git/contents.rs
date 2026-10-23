use super::*;

const MAX_OBJECT_BYTES: usize = 5_242_880;

fn error(kind: &str) -> Value {
    json!({"type":"error","error":{"type":kind}})
}

fn root(cwd: &Path) -> Result<PathBuf> {
    ensure!(cwd.is_absolute(), "Git cwd must be absolute");
    let path = String::from_utf8(checked(run(cwd, &["rev-parse", "--show-toplevel"])?)?)?;
    Ok(PathBuf::from(path.trim_end_matches('\n')))
}

fn relative_path(root: &Path, path: &str) -> Result<PathBuf> {
    let path = Path::new(path);
    let path = if path.is_absolute() {
        let resolved = path.canonicalize().or_else(|_| {
            let parent = path
                .parent()
                .ok_or_else(|| std::io::Error::other("path has no parent"))?;
            Ok::<_, std::io::Error>(
                parent.canonicalize()?.join(
                    path.file_name()
                        .ok_or_else(|| std::io::Error::other("path has no file name"))?,
                ),
            )
        })?;
        resolved
            .strip_prefix(root.canonicalize()?)
            .context("file is outside the repository")?
            .to_path_buf()
    } else {
        path.to_path_buf()
    };
    ensure!(
        !path.as_os_str().is_empty()
            && path
                .components()
                .all(|part| matches!(part, Component::Normal(_))),
        "Git file path must be repository-relative"
    );
    Ok(path)
}

pub(in crate::host) fn read_file_version(params: &Value) -> Result<Option<Vec<u8>>> {
    let cwd = Path::new(params["cwd"].as_str().context("missing Git cwd")?);
    let root = root(cwd)?;
    let path = relative_path(
        &root,
        params["path"].as_str().context("missing Git file path")?,
    )?;
    let reference = match params.get("ref") {
        None => "head",
        Some(Value::Null) => return Ok(None),
        Some(value) => value.as_str().context("Git ref must be a string")?,
    };
    let revision = match reference {
        "index" | ":" => String::new(),
        "head" | "HEAD" => {
            let Some(head) = commit_id(&root, "HEAD")? else {
                return Ok(None);
            };
            head
        }
        reference => {
            ensure!(
                (40..=64).contains(&reference.len())
                    && reference.bytes().all(|byte| byte.is_ascii_hexdigit()),
                "Git file ref must be head, index, or a commit ID"
            );
            let Some(commit) = commit_id(&root, reference)? else {
                return Ok(None);
            };
            commit
        }
    };
    let spec = format!(
        "{revision}:{}",
        path.to_str().context("Git file path must be UTF-8")?
    );
    if let Some(limit) = params.get("maxBytes") {
        let limit = limit
            .as_u64()
            .context("maxBytes must be an unsigned integer")?;
        let size = command(&root, &["cat-file", "-s", &spec])
            .env("GIT_NO_LAZY_FETCH", "1")
            .output()?;
        if !size.status.success() {
            return Ok(None);
        }
        let bytes = std::str::from_utf8(&size.stdout)?.trim().parse::<u64>()?;
        if bytes > limit {
            return Ok(None);
        }
    }
    let output = command(&root, &["show", &spec])
        .env("GIT_NO_LAZY_FETCH", "1")
        .output()?;
    Ok(output.status.success().then_some(output.stdout))
}

fn batch(root: &Path, mode: &str, objects: &[String]) -> Result<Vec<u8>> {
    let mut child = command(root, &["cat-file", mode])
        .env("GIT_NO_LAZY_FETCH", "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let mut input = child.stdin.take().context("Git object input")?;
    for object in objects {
        writeln!(input, "{object}")?;
    }
    drop(input);
    checked(child.wait_with_output()?)
}

fn disk_lines(root: &Path, path: &str, limit: usize) -> Value {
    let result = (|| -> Result<Value> {
        let relative = relative_path(root, path)?;
        let file = root.join(relative).canonicalize()?;
        ensure!(
            file.starts_with(root.canonicalize()?),
            "file is outside the repository"
        );
        let file = fs::File::open(file)?;
        if file.metadata()?.len() > limit as u64 {
            return Ok(json!({"type":"error","error":{"type":"too-large","limitBytes":limit}}));
        }
        let mut bytes = Vec::new();
        file.take(limit as u64 + 1).read_to_end(&mut bytes)?;
        if bytes.len() > limit {
            return Ok(json!({"type":"error","error":{"type":"too-large","limitBytes":limit}}));
        }
        Ok(lines(&bytes))
    })();
    match result {
        Ok(value) => value,
        Err(error)
            if error
                .downcast_ref::<std::io::Error>()
                .is_some_and(|error| error.kind() == ErrorKind::NotFound) =>
        {
            self::error("not-found")
        }
        Err(_) => self::error("unknown"),
    }
}

fn lines(bytes: &[u8]) -> Value {
    let text = String::from_utf8_lossy(bytes);
    json!({"type":"success","lines":text.split_inclusive('\n').collect::<Vec<_>>()})
}

pub(super) fn cat_file(root: &Path, params: &Value) -> Result<Value> {
    let requests = params["requests"]
        .as_array()
        .context("missing Git object requests")?;
    if requests.len() > 4 {
        return Ok(Value::Array(
            requests.iter().map(|_| error("unknown")).collect(),
        ));
    }
    let limit = params
        .get("maxObjectBytes")
        .map(|value| {
            value
                .as_u64()
                .context("maxObjectBytes must be an unsigned integer")
        })
        .transpose()?
        .unwrap_or(MAX_OBJECT_BYTES as u64)
        .min(MAX_OBJECT_BYTES as u64) as usize;
    let mut objects = Vec::new();
    for request in requests {
        if let Some(oid) = request["oid"].as_str().filter(|oid| {
            !oid.is_empty()
                && oid.len() <= 64
                && oid.bytes().all(|byte| byte.is_ascii_hexdigit())
                && oid.bytes().any(|byte| byte != b'0')
        }) && !objects.iter().any(|object| object == oid)
        {
            objects.push(oid.to_owned());
        }
    }
    let mut values = HashMap::new();
    let mut readable = Vec::new();
    if !objects.is_empty() {
        let metadata = batch(root, "--batch-check", &objects)?;
        for (oid, line) in objects
            .iter()
            .zip(String::from_utf8_lossy(&metadata).lines())
        {
            let columns = line.split_whitespace().collect::<Vec<_>>();
            match columns.as_slice() {
                [_, "blob", size] => {
                    match size.parse::<usize>() {
                        Ok(size) if size <= limit => readable.push(oid.clone()),
                        Ok(_) => {
                            values.insert(oid.clone(), json!({"type":"error","error":{"type":"too-large","limitBytes":limit}}));
                        }
                        Err(_) => {
                            values.insert(oid.clone(), error("unknown"));
                        }
                    }
                }
                [_, "missing"] => {
                    values.insert(oid.clone(), error("not-found"));
                }
                _ => {
                    values.insert(oid.clone(), error("unknown"));
                }
            }
        }
    }
    if !readable.is_empty() {
        let content = batch(root, "--batch", &readable)?;
        let mut offset = 0usize;
        for oid in readable {
            let Some(end) = content[offset..]
                .iter()
                .position(|byte| *byte == b'\n')
                .map(|end| offset + end)
            else {
                break;
            };
            let size = std::str::from_utf8(&content[offset..end])
                .ok()
                .and_then(|line| line.split_whitespace().last()?.parse::<usize>().ok());
            offset = end + 1;
            let Some(end) = size
                .and_then(|size| offset.checked_add(size))
                .filter(|end| *end < content.len())
            else {
                break;
            };
            if content[end] != b'\n' {
                break;
            }
            values.insert(oid, lines(&content[offset..end]));
            offset = end + 1;
        }
    }
    Ok(Value::Array(
        requests
            .iter()
            .map(|request| {
                let value = request["oid"]
                    .as_str()
                    .and_then(|oid| values.get(oid))
                    .cloned()
                    .unwrap_or_else(|| error("not-found"));
                if value["error"]["type"] == "not-found" && request["fallbackToDisk"] == true {
                    disk_lines(root, request["path"].as_str().unwrap_or(""), limit)
                } else {
                    value
                }
            })
            .collect(),
    ))
}
