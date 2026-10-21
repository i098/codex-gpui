use super::{Output, execute};
use crate::host::{Host, LocalHost, Request, json_body};
use anyhow::{Context as _, Result, bail, ensure};
use serde_json::{Value, json};
use std::path::Path;

pub(in crate::host) fn fetch(host: &LocalHost, endpoint: &str, params: &Value) -> Result<Value> {
    if endpoint == "github-request" {
        ensure!(
            params
                .pointer("/hostConfig/id")
                .is_none_or(|id| id == "local"),
            "remote GitHub host is unsupported"
        );
        let method = params
            .pointer("/request/method")
            .and_then(Value::as_str)
            .context("missing GitHub method")?;
        ensure!(method.starts_with("gh-"), "invalid GitHub method");
        return json_body(&host.fetch(&Request::post(
            method,
            Some(params["request"]["params"].clone()),
        ))?);
    }
    if endpoint == "gh-pr-local-state" {
        return Ok(branch_pull_request(host, params)?.unwrap_or(Value::Null));
    }
    if endpoint == "gh-pr-for-branch" {
        return Ok(branch_pull_request(host, params)?
            .and_then(|item| item.get("url").cloned())
            .unwrap_or(Value::Null));
    }
    let result = match endpoint {
        "gh-pr-create" => create(host, params),
        "gh-pr-update" => update(host, params),
        "gh-user-search" => search_users(host, params),
        _ => bail!("unsupported GitHub action: {endpoint}"),
    };
    Ok(result.unwrap_or_else(|error| json!({"status": "error", "error": error.to_string()})))
}

fn command(
    host: &LocalHost,
    program: &str,
    args: &[String],
    cwd: Option<&Path>,
    timeout_ms: u64,
) -> Result<Output> {
    let bridge = host
        .bridge
        .as_ref()
        .context("GitHub requires an app-server connection")?;
    let mut argv = Vec::with_capacity(args.len() + 1);
    argv.push(program.to_owned());
    argv.extend_from_slice(args);
    let output = execute(bridge, cwd.unwrap_or(&host.home), &argv, timeout_ms)?;
    ensure!(
        !output.stdout_cap_reached && !output.stderr_cap_reached,
        "GitHub command output exceeded the capture limit"
    );
    Ok(output)
}

fn require_github(host: &LocalHost, hostname: Option<&str>) -> Result<()> {
    let status = json_body(&host.fetch(&Request::post(
        "gh-cli-status",
        Some(json!({"hostId": "local", "hostname": hostname})),
    ))?)?;
    ensure!(
        status["isInstalled"].is_boolean() && status["isAuthenticated"].is_boolean(),
        "{}",
        status["error"]
            .as_str()
            .unwrap_or("Failed to check GitHub CLI availability")
    );
    ensure!(
        status["isInstalled"] == true,
        "GitHub CLI (gh) is not installed"
    );
    ensure!(
        status["isAuthenticated"] == true,
        "GitHub CLI (gh) is not authenticated."
    );
    Ok(())
}

fn create(host: &LocalHost, params: &Value) -> Result<Value> {
    let cwd = Path::new(field(params, "cwd")?);
    ensure!(cwd.is_absolute(), "No workspace available for PR creation");
    require_github(host, None)?;
    let head = field(params, "headBranch")?.trim();
    ensure!(!head.is_empty(), "Head branch is required");
    let root = command(
        host,
        "git",
        &strings(&["rev-parse", "--show-toplevel"]),
        Some(cwd),
        5000,
    )?;
    ensure!(
        root.exit_code == 0,
        "No git repository found for PR creation"
    );
    let root = Path::new(root.stdout.trim());
    let mut title = params["titleOverride"].as_str().unwrap_or("").to_owned();
    let mut body = params["bodyOverride"].as_str().unwrap_or("").to_owned();
    if title.is_empty() || body.is_empty() {
        let commit = command(
            host,
            "git",
            &strings(&["log", "-1", "--format=%s%n%b"]),
            Some(root),
            5000,
        )?;
        if commit.exit_code == 0 {
            let (subject, description) = commit
                .stdout
                .split_once('\n')
                .unwrap_or((&commit.stdout, ""));
            if title.is_empty() {
                title = subject.trim().to_owned();
            }
            if body.is_empty() {
                body = description.trim().to_owned();
            }
        }
    }
    if title.is_empty() {
        title = head.to_owned();
    }
    let web = params["openInBrowser"] == true;
    let mut args = strings(&["pr", "create"]);
    if web {
        args.push("--web".into());
    }
    args.extend(strings(&["--head", head, "--title", &title]));
    if let Some(base) = params["baseBranch"]
        .as_str()
        .map(str::trim)
        .filter(|base| !base.is_empty())
    {
        args.extend(strings(&["--base", base]));
    }
    if params["isDraft"] == true && !web {
        args.push("--draft".into());
    }
    args.extend(strings(&["--body", &body]));
    let output = command(host, "gh", &args, Some(root), 15000)?;
    if output.exit_code != 0 {
        return Ok(failure(
            &output,
            &args,
            if web {
                "Failed to open pull request in browser"
            } else {
                "Failed to create pull request"
            },
        ));
    }
    let created = created_url(&output.stdout).or_else(|| created_url(&output.stderr));
    Ok(
        json!({"status": "success", "url": created.map(|(url, _)| url), "number": created.map(|(_, number)| number)}),
    )
}

fn created_url(output: &str) -> Option<(&str, u64)> {
    for word in output.split_whitespace() {
        let address = word.trim_end_matches([')', ',', '.']);
        let Ok(url) = url::Url::parse(address) else {
            continue;
        };
        if !matches!(url.scheme(), "http" | "https") {
            continue;
        }
        let Some(segments) = url.path_segments() else {
            continue;
        };
        let mut follows_pull = false;
        for segment in segments {
            if follows_pull && let Ok(number) = segment.parse() {
                return Some((address, number));
            }
            follows_pull = segment == "pull";
        }
    }
    None
}

fn target(params: &Value) -> Result<(&str, &str, &str, u64)> {
    let account = &params["account"];
    let pull = &params["pullRequest"];
    let hostname = field(pull, "hostname")?;
    ensure!(
        account["hostId"] == "local" && field(account, "hostname")?.eq_ignore_ascii_case(hostname),
        "Pull request account does not match the request"
    );
    let owner = field(pull, "owner")?;
    let repository = field(pull, "repository")?;
    let number = pull["number"]
        .as_u64()
        .filter(|number| *number > 0)
        .context("invalid pull request number")?;
    for component in [hostname, owner, repository] {
        ensure!(
            !component.is_empty()
                && !component.starts_with('-')
                && !component.contains(['/', '\\', '\0', '\n', '\r']),
            "invalid GitHub repository"
        );
    }
    Ok((hostname, owner, repository, number))
}

fn update(host: &LocalHost, params: &Value) -> Result<Value> {
    let (hostname, owner, repository, number) = target(params)?;
    let number = number.to_string();
    let repo = format!("{hostname}/{owner}/{repository}");
    let action = field(params, "action")?;
    let enabled = if action == "toggle-auto-merge" {
        Some(
            params["enabled"]
                .as_bool()
                .context("missing auto-merge state")?,
        )
    } else {
        None
    };
    let mut args = match action {
        "mark-draft" => strings(&["pr", "ready", &number, "--undo"]),
        "mark-ready" => strings(&["pr", "ready", &number]),
        "close" | "reopen" => strings(&["pr", action, &number]),
        "reopen-ready" => strings(&["pr", "reopen", &number]),
        "toggle-auto-merge" if enabled == Some(true) => strings(&[
            "pr",
            "merge",
            &number,
            "--auto",
            if params["mergeMethod"] == "squash" {
                "--squash"
            } else {
                "--merge"
            },
        ]),
        "toggle-auto-merge" => strings(&["pr", "merge", &number, "--disable-auto"]),
        "update-title" => strings(&["pr", "edit", &number, "--title", field(params, "title")?]),
        "update-body" => strings(&["pr", "edit", &number, "--body", field(params, "body")?]),
        "request-reviewers" | "remove-reviewers" => {
            let reviewers = params["reviewers"]
                .as_array()
                .context("missing reviewers")?;
            let teams = if action == "remove-reviewers" {
                params["teamReviewers"]
                    .as_array()
                    .map(Vec::as_slice)
                    .unwrap_or_default()
            } else {
                &[]
            };
            ensure!(
                !reviewers.is_empty() || !teams.is_empty(),
                "At least one reviewer is required"
            );
            let mut args = vec![
                "api".into(),
                format!("repos/{owner}/{repository}/pulls/{number}/requested_reviewers"),
                "--method".into(),
                if action == "remove-reviewers" {
                    "DELETE"
                } else {
                    "POST"
                }
                .into(),
                "--hostname".into(),
                hostname.into(),
            ];
            for reviewer in reviewers {
                args.extend([
                    "-f".into(),
                    format!(
                        "reviewers[]={}",
                        reviewer.as_str().context("invalid reviewer")?
                    ),
                ]);
            }
            for team in teams {
                args.extend([
                    "-f".into(),
                    format!(
                        "team_reviewers[]={}",
                        team.as_str().context("invalid team reviewer")?
                    ),
                ]);
            }
            args
        }
        _ => bail!("unsupported pull request action: {action}"),
    };
    if !matches!(action, "request-reviewers" | "remove-reviewers") {
        args.extend(strings(&["--repo", &repo]));
    }
    require_github(host, Some(hostname))?;
    let mut output = command(
        host,
        "gh",
        &args,
        None,
        if matches!(action, "request-reviewers" | "remove-reviewers") {
            60000
        } else {
            15000
        },
    )?;
    if output.exit_code == 0 && action == "reopen-ready" {
        args = strings(&["pr", "ready", &number, "--repo", &repo]);
        output = command(host, "gh", &args, None, 15000)?;
    }
    Ok(if output.exit_code == 0 {
        json!({"status": "success"})
    } else {
        failure(&output, &args, "Failed to update pull request")
    })
}

fn strings(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).to_owned()).collect()
}

fn field<'a>(value: &'a Value, key: &str) -> Result<&'a str> {
    value[key]
        .as_str()
        .with_context(|| format!("missing {key}"))
}

fn error<'a>(output: &'a Output, fallback: &'a str) -> &'a str {
    if !output.stderr.trim().is_empty() {
        output.stderr.trim()
    } else if !output.stdout.trim().is_empty() {
        output.stdout.trim()
    } else {
        fallback
    }
}

fn failure(output: &Output, args: &[String], fallback: &str) -> Value {
    let error = error(output, fallback);
    let command = std::iter::once("gh")
        .chain(args.iter().map(String::as_str))
        .collect::<Vec<_>>()
        .join(" ");
    let output = [&output.stdout, &output.stderr]
        .into_iter()
        .filter(|text| !text.trim().is_empty())
        .map(|text| text.trim())
        .collect::<Vec<_>>()
        .join("\n");
    json!({"status": "error", "error": error, "execOutput": {"command": command, "output": if output.is_empty() { error.to_owned() } else { output }}})
}

fn repository_argument(remote: &str) -> Option<String> {
    let normalized = remote
        .strip_prefix("git@")
        .map(|remote| format!("ssh://{}", remote.replacen(':', "/", 1)));
    let url = url::Url::parse(normalized.as_deref().unwrap_or(remote)).ok()?;
    let host = url.host_str()?;
    let path = url
        .path()
        .trim_matches('/')
        .strip_suffix(".git")
        .unwrap_or(url.path().trim_matches('/'));
    (path.split('/').count() == 2).then(|| format!("{host}/{path}"))
}

fn branch_pull_request(host: &LocalHost, params: &Value) -> Result<Option<Value>> {
    let cwd = Path::new(field(params, "cwd")?);
    let branch = field(params, "headBranch")?.trim();
    if branch.is_empty() {
        return Ok(None);
    }
    require_github(host, None)?;
    let remote = |name: &str| -> Result<Option<String>> {
        let output = command(
            host,
            "git",
            &strings(&["config", "--get", &format!("remote.{name}.url")]),
            Some(cwd),
            5000,
        )?;
        Ok((output.exit_code == 0)
            .then(|| repository_argument(output.stdout.trim()))
            .flatten())
    };
    let origin = remote("origin")?;
    let upstream = remote("upstream")?;
    let mut candidates = Vec::new();
    if let (Some(origin), Some(upstream)) = (&origin, &upstream)
        && origin != upstream
    {
        let owner = origin
            .split('/')
            .nth(1)
            .context("invalid origin repository")?;
        let head = if branch.contains(':') {
            branch.to_owned()
        } else {
            format!("{owner}:{branch}")
        };
        candidates.push((Some(upstream.clone()), head));
    }
    candidates.push((origin, branch.to_owned()));
    let mut merged = None;
    for (repository, head) in candidates {
        let mut args = strings(&[
            "pr",
            "list",
            "--head",
            &head,
            "--author",
            "@me",
            "--state",
            "all",
            "--json",
            "number,url,state,headRefName",
        ]);
        if let Some(repository) = repository {
            args.extend(strings(&["--repo", &repository]));
        }
        let output = command(host, "gh", &args, Some(cwd), 5000)?;
        ensure!(
            output.exit_code == 0,
            "{}",
            error(&output, "Failed to determine pull request status")
        );
        let items: Vec<Value> = serde_json::from_str(&output.stdout)?;
        for item in items {
            if item["headRefName"].as_str() != head.rsplit(':').next() {
                continue;
            }
            if item["state"]
                .as_str()
                .is_some_and(|state| state.eq_ignore_ascii_case("OPEN"))
            {
                return Ok(Some(item));
            }
            if merged.is_none()
                && item["state"]
                    .as_str()
                    .is_some_and(|state| state.eq_ignore_ascii_case("MERGED"))
            {
                merged = Some(item);
            }
        }
    }
    Ok(merged)
}

fn search_users(host: &LocalHost, params: &Value) -> Result<Value> {
    let (hostname, owner, repository, number) = target(params)?;
    let query = field(params, "query")?.trim();
    let scope = field(params, "scope")?;
    if matches!(scope, "collaborators" | "users") && query.is_empty() {
        return Ok(json!({"status": "success", "users": []}));
    }
    let args = if scope == "users" {
        vec![
            "api".into(),
            "--method".into(),
            "GET".into(),
            "search/users".into(),
            "-f".into(),
            format!("q={query} in:login"),
            "-f".into(),
            "per_page=10".into(),
            "--hostname".into(),
            hostname.into(),
        ]
    } else {
        let graphql = match scope {
            "collaborators" => {
                "query($owner:String!,$repo:String!,$search:String!){viewer{login} repository(owner:$owner,name:$repo){collaborators(first:100,query:$search){edges{node{avatarUrl(size:48) login}}}}}"
            }
            "mentions" => {
                "query($owner:String!,$repo:String!,$number:Int!,$search:String!){repository(owner:$owner,name:$repo){mentionableUsers(first:10,query:$search){nodes{avatarUrl(size:48) login}} pullRequest(number:$number){participants(first:100){nodes{avatarUrl(size:48) login}}}}}"
            }
            _ => bail!("unsupported GitHub user search scope"),
        };
        let mut args = vec![
            "api".into(),
            "graphql".into(),
            "-f".into(),
            format!("query={graphql}"),
            "-f".into(),
            format!("search={query}"),
            "-f".into(),
            format!("owner={owner}"),
            "-f".into(),
            format!("repo={repository}"),
            "--hostname".into(),
            hostname.into(),
        ];
        if scope == "mentions" {
            args.extend(["-F".into(), format!("number={number}")]);
        }
        args
    };
    let output = command(host, "gh", &args, None, 15000)?;
    if output.exit_code != 0 {
        return Ok(failure(&output, &args, "Failed to search GitHub users"));
    }
    let mut data: Value = serde_json::from_str(&output.stdout)?;
    let mut users = Vec::new();
    if scope == "users" {
        let Value::Array(items) = data["items"].take() else {
            bail!("Unexpected GitHub CLI response");
        };
        for mut item in items.into_iter().take(10) {
            ensure!(
                item["login"].is_string() && item["avatar_url"].is_string(),
                "Unexpected GitHub CLI response"
            );
            users.push(
                json!({"login": item["login"].take(), "avatarUrl": item["avatar_url"].take()}),
            );
        }
    } else if scope == "collaborators" {
        let viewer = data
            .pointer("/data/viewer/login")
            .and_then(Value::as_str)
            .context("Unexpected GitHub CLI response")?
            .to_owned();
        let Value::Array(edges) = data["data"]["repository"]["collaborators"]["edges"].take()
        else {
            bail!("Unexpected GitHub CLI response");
        };
        for mut edge in edges {
            let user = edge["node"].take();
            let login = user["login"]
                .as_str()
                .context("Unexpected GitHub CLI response")?;
            ensure!(
                user["avatarUrl"].is_string(),
                "Unexpected GitHub CLI response"
            );
            if !login.eq_ignore_ascii_case(&viewer) {
                users.push(user);
            }
        }
    } else {
        let Value::Array(participants) =
            data["data"]["repository"]["pullRequest"]["participants"]["nodes"].take()
        else {
            bail!("Unexpected GitHub CLI response");
        };
        let Value::Array(mentionable) =
            data["data"]["repository"]["mentionableUsers"]["nodes"].take()
        else {
            bail!("Unexpected GitHub CLI response");
        };
        let query = query.to_lowercase();
        let mut seen = std::collections::HashSet::new();
        for user in participants
            .into_iter()
            .filter(|user| {
                user["login"]
                    .as_str()
                    .is_some_and(|login| login.to_lowercase().contains(&query))
            })
            .chain(mentionable.into_iter().filter(|_| !query.is_empty()))
        {
            let login = user["login"]
                .as_str()
                .context("Unexpected GitHub CLI response")?;
            ensure!(
                user["avatarUrl"].is_string(),
                "Unexpected GitHub CLI response"
            );
            if seen.insert(login.to_lowercase()) {
                users.push(user);
            }
            if !query.is_empty() && users.len() == 10 {
                break;
            }
        }
    }
    Ok(json!({"status": "success", "users": users}))
}

#[cfg(test)]
mod tests {
    use super::created_url;

    #[test]
    fn created_pull_request_urls_keep_http_and_ignore_trailing_punctuation() {
        assert_eq!(
            created_url("created http://git.example/team/repo/pull/42/files)."),
            Some(("http://git.example/team/repo/pull/42/files", 42))
        );
        assert_eq!(
            created_url("https://github.com/team/repo/pull/not-a-number"),
            None
        );
    }
}
