use anyhow::{Context as _, Result, bail, ensure};
use base64::Engine as _;
use serde_json::{Value, json};
use std::time::Duration;
use ui::screens::pull_requests::model::{Account, Filters, Identity, valid_component};

const SEARCH: &str = "query($searchQuery:String!,$first:Int!,$after:String){search(query:$searchQuery,type:ISSUE,first:$first,after:$after){issueCount nodes{... on PullRequest{additions author{avatarUrl(size:48) login} baseRefName createdAt deletions headRefName id isDraft mergeStateStatus mergeable number repository{name owner{login}} state statusCheckRollup{contexts(first:100){totalCount nodes{__typename ... on CheckRun{conclusion name status} ... on StatusContext{context state}}}} title updatedAt url}} pageInfo{endCursor hasNextPage}}}";
const DETAIL: &str = "query($owner:String!,$repo:String!,$number:Int!){viewer{login} repository(owner:$owner,name:$repo){pullRequest(number:$number){number url title body baseRefName headRefName headRefOid state isDraft author{login avatarUrl} createdAt additions deletions changedFiles mergeable mergeStateStatus reviewDecision latestReviews(first:100){nodes{author{login avatarUrl} state comments(first:1){totalCount}} pageInfo{hasNextPage}} reviewRequests(first:100){nodes{requestedReviewer{__typename ... on User{login avatarUrl} ... on Team{name slug}}} pageInfo{hasNextPage}} statusCheckRollup{contexts(first:100){totalCount nodes{__typename ... on CheckRun{conclusion name status detailsUrl} ... on StatusContext{context state targetUrl}}}}}}}";

struct Output {
    code: Option<i32>,
    stdout: String,
    stderr: String,
    missing: bool,
}

fn run(host: &super::LocalHost, args: &[String], seconds: u64) -> Result<Output> {
    let bridge = host
        .bridge
        .as_ref()
        .context("GitHub access requires an app-server connection")?;
    match super::git_pr::execute(bridge, &host.home, args, seconds * 1000) {
        Ok(output) => {
            ensure!(
                !output.stdout_cap_reached && !output.stderr_cap_reached,
                "GitHub response is too large"
            );
            Ok(Output {
                code: Some(output.exit_code),
                stdout: output.stdout,
                stderr: output.stderr,
                missing: false,
            })
        }
        Err(error) if missing_gh(&error, &host.home) => Ok(Output {
            code: None,
            stdout: String::new(),
            stderr: String::new(),
            missing: true,
        }),
        Err(error) => Err(error).context("Failed to start GitHub CLI"),
    }
}

fn missing_gh(error: &anyhow::Error, home: &std::path::Path) -> bool {
    error
        .downcast_ref::<bridge::RpcError>()
        .is_some_and(|error| {
            error.code == -32603
                && error.message.contains("No such file or directory")
                && home.is_dir()
        })
}

fn checked(output: Output) -> Result<String> {
    if output.missing {
        bail!("GitHub CLI (gh) is not installed");
    }
    if output.code != Some(0) {
        let combined = format!("{}\n{}", output.stderr, output.stdout).to_lowercase();
        if output.code == Some(4)
            || [
                "gh auth login",
                "not logged into any github hosts",
                "gh_token",
                "gh_enterprise_token",
            ]
            .iter()
            .any(|text| combined.contains(text))
        {
            bail!("GitHub CLI (gh) is not authenticated.");
        }
        bail!(
            "{}",
            if output.stderr.trim().is_empty() {
                output.stdout.trim()
            } else {
                output.stderr.trim()
            }
        );
    }
    Ok(output.stdout)
}

fn command(host: &super::LocalHost, args: &[&str], seconds: u64) -> Result<Output> {
    run(
        host,
        &std::iter::once("gh".to_owned())
            .chain(args.iter().map(|arg| (*arg).to_owned()))
            .collect::<Vec<_>>(),
        seconds,
    )
}

fn graphql(
    host: &super::LocalHost,
    hostname: &str,
    query: &str,
    fields: &[(&str, String)],
    seconds: u64,
) -> Result<Value> {
    let mut args = vec![
        "gh".into(),
        "api".into(),
        "graphql".into(),
        "--hostname".into(),
        hostname.into(),
        "-f".into(),
        format!("query={query}"),
    ];
    for (key, value) in fields {
        args.extend([
            if *key == "first" || *key == "number" {
                "-F"
            } else {
                "-f"
            }
            .into(),
            format!("{key}={value}"),
        ]);
    }
    let text = checked(run(host, &args, seconds)?)?;
    let data: Value = serde_json::from_str(&text).context("Failed to parse GitHub CLI response")?;
    if let Some(errors) = data["errors"]
        .as_array()
        .filter(|errors| !errors.is_empty())
    {
        bail!(
            "{}",
            errors
                .iter()
                .filter_map(|error| error["message"].as_str())
                .collect::<Vec<_>>()
                .join("\n")
        );
    }
    Ok(data)
}

fn account(params: &Value) -> Result<Account> {
    let account: Account =
        serde_json::from_value(params["account"].clone()).context("Invalid GitHub account")?;
    ensure!(
        account.host_id == "local",
        "Remote GitHub execution host is unsupported"
    );
    ensure!(
        valid_component(&account.hostname) && !account.login.is_empty(),
        "Invalid GitHub account"
    );
    Ok(account)
}

fn state(value: &Value) -> Result<&'static str> {
    match value.as_str() {
        Some("OPEN") => Ok("open"),
        Some("MERGED") => Ok("merged"),
        Some("CLOSED") => Ok("closed"),
        _ => bail!("Unexpected pull request state"),
    }
}

fn ci(value: &Value) -> &'static str {
    let Some(nodes) = value["contexts"]["nodes"].as_array() else {
        return "none";
    };
    let mut status = "passing";
    for node in nodes {
        let conclusion = node["conclusion"]
            .as_str()
            .or_else(|| node["state"].as_str())
            .unwrap_or("");
        if [
            "FAILURE",
            "ERROR",
            "TIMED_OUT",
            "CANCELLED",
            "ACTION_REQUIRED",
            "STARTUP_FAILURE",
        ]
        .contains(&conclusion)
        {
            return "failing";
        }
        if node["status"].as_str().is_some_and(|s| s != "COMPLETED")
            || ["PENDING", "EXPECTED"].contains(&conclusion)
        {
            status = "pending";
        }
    }
    if value["contexts"]["totalCount"].as_u64().unwrap_or(0) > nodes.len() as u64 {
        status = "pending";
    }
    if nodes.is_empty() && status != "pending" {
        "none"
    } else {
        status
    }
}

pub(super) fn fetch(host: &super::LocalHost, endpoint: &str, params: &Value) -> Result<Value> {
    match endpoint {
        "gh-cli-status" => {
            let hostname = params["hostname"].as_str().unwrap_or("github.com");
            ensure!(valid_component(hostname), "Invalid GitHub hostname");
            let version = command(host, &["--version"], 5)?;
            if version.missing {
                return Ok(json!({"isInstalled": false, "isAuthenticated": false}));
            }
            checked(version)?;
            let mut auth = command(
                host,
                &["auth", "status", "--active", "--hostname", hostname],
                10,
            )?;
            if auth.stderr.contains("unknown flag: --active") {
                auth = command(host, &["auth", "status", "--hostname", hostname], 10)?;
            }
            Ok(json!({"isInstalled": true, "isAuthenticated": auth.code == Some(0)}))
        }
        "gh-current-user" => {
            let hostname = params["hostname"].as_str().unwrap_or("github.com");
            ensure!(valid_component(hostname), "Invalid GitHub hostname");
            match checked(command(host, &["api", "user", "--hostname", hostname], 5)?) {
                Ok(text) => {
                    let user: Value = serde_json::from_str(&text)
                        .context("Failed to parse GitHub CLI response")?;
                    let login = user["login"]
                        .as_str()
                        .context("Unexpected GitHub CLI response")?;
                    Ok(
                        json!({"status": "success", "account": {"hostId": "local", "hostname": hostname.to_lowercase(), "login": login.to_lowercase()}, "login": login, "avatarUrl": user["avatar_url"]}),
                    )
                }
                Err(error) => Ok(json!({"status": "error", "error": error.to_string()})),
            }
        }
        "gh-pr-search" => search(host, params),
        "gh-pr-detail" => detail(host, params),
        "gh-pr-patch" => patch(host, params),
        "gh-pr-media" => media(host, params),
        _ => bail!("Unsupported GitHub endpoint: {endpoint}"),
    }
}

fn search(host: &super::LocalHost, params: &Value) -> Result<Value> {
    let account = account(params)?;
    let filters: Filters = serde_json::from_value(params["filters"].clone())?;
    ensure!(
        filters.text.encode_utf16().count() <= 256
            && filters
                .raw_query
                .as_ref()
                .is_none_or(|s| s.encode_utf16().count() <= 1024),
        "Pull request search text is too long"
    );
    if let Some(repo) = &filters.repository {
        ensure!(
            valid_component(&repo.owner) && valid_component(&repo.repository),
            "Pull request repository filter is invalid"
        );
    }
    ensure!(
        [
            "all",
            "authored",
            "review_requested",
            "reviewed",
            "user_review_requested"
        ]
        .contains(&filters.relationship.as_str()),
        "Invalid relationship filter"
    );
    ensure!(
        ["all", "open", "merged", "closed"].contains(&filters.lifecycle.as_str()),
        "Invalid lifecycle filter"
    );
    let page_size = params["pageSize"].as_u64().context("Missing page size")?;
    ensure!(
        (1..=100).contains(&page_size),
        "Pull request search page size is invalid"
    );
    let mut fields = vec![
        ("searchQuery", filters.search_query()),
        ("first", page_size.to_string()),
    ];
    if let Some(cursor) = params["cursor"].as_str() {
        fields.push(("after", cursor.to_owned()));
    }
    let response = graphql(host, &account.hostname, SEARCH, &fields, 30)?;
    let search = &response["data"]["search"];
    let nodes = search["nodes"]
        .as_array()
        .context("Unexpected GitHub CLI response")?;
    let mut ids = std::collections::HashSet::new();
    let mut items = Vec::new();
    for node in nodes.iter().filter(|node| !node.is_null()) {
        let id = node["id"].as_str().context("Pull request has no node ID")?;
        if !ids.insert(id) {
            continue;
        }
        let life = state(&node["state"])?;
        items.push(json!({
            "nodeId": id, "pullRequest": {"hostname": account.hostname, "number": node["number"], "owner": node["repository"]["owner"]["login"], "repository": node["repository"]["name"]},
            "title": node["title"], "state": life, "headBranch": node["headRefName"], "baseBranch": node["baseRefName"],
            "authorLogin": node["author"]["login"], "authorAvatarUrl": node["author"]["avatarUrl"], "updatedAt": node["updatedAt"],
            "additions": node["additions"], "deletions": node["deletions"], "hasDiffStats": true, "isDraft": node["isDraft"],
            "canMerge": life == "open" && node["isDraft"] == false && node["mergeable"] == "MERGEABLE" && matches!(node["mergeStateStatus"].as_str(), Some("CLEAN" | "HAS_HOOKS")),
            "mergeBlocker": if node["mergeable"] == "CONFLICTING" || node["mergeStateStatus"] == "DIRTY" { Some("conflicts") } else if node["mergeable"] == "UNKNOWN" { Some("unknown") } else { None },
            "ciStatus": if life == "open" { ci(&node["statusCheckRollup"]) } else { "none" }
        }));
    }
    let next = search["pageInfo"]["endCursor"].as_str();
    let has_next = search["pageInfo"]["hasNextPage"] == true
        && next.is_some()
        && search["pageInfo"]["endCursor"] != params["cursor"]
        && !items.is_empty();
    Ok(
        json!({"items": items, "hasNextPage": has_next, "endCursor": if has_next { next } else { None }, "totalCount": search["issueCount"]}),
    )
}

fn request_identity(params: &Value) -> Result<(Account, Identity)> {
    let account = account(params)?;
    let identity: Identity = serde_json::from_value(params["pullRequest"].clone())?;
    ensure!(
        identity.valid() && identity.hostname.eq_ignore_ascii_case(&account.hostname),
        "Pull request account does not match the request"
    );
    Ok((account, identity))
}

fn detail(host: &super::LocalHost, params: &Value) -> Result<Value> {
    let (account, identity) = request_identity(params)?;
    let response = graphql(
        host,
        &account.hostname,
        DETAIL,
        &[
            ("owner", identity.owner),
            ("repo", identity.repository),
            ("number", identity.number.to_string()),
        ],
        15,
    )?;
    verify_viewer(&response, &account)?;
    let request = &response["data"]["repository"]["pullRequest"];
    ensure!(!request.is_null(), "Pull request not found");
    let mut request = request.clone();
    request["ciStatus"] = ci(&request["statusCheckRollup"]).into();
    Ok(request)
}

fn verify_viewer(response: &Value, account: &Account) -> Result<()> {
    let viewer = response["data"]["viewer"]["login"]
        .as_str()
        .context("GitHub viewer is missing")?;
    ensure!(
        viewer.eq_ignore_ascii_case(&account.login),
        "Pull request account does not match the signed-in user"
    );
    Ok(())
}

fn patch(host: &super::LocalHost, params: &Value) -> Result<Value> {
    let (account, identity) = request_identity(params)?;
    let viewer = graphql(host, &account.hostname, "query{viewer{login}}", &[], 5)?;
    verify_viewer(&viewer, &account)?;
    let endpoint = format!(
        "repos/{}/{}/pulls/{}",
        identity.owner, identity.repository, identity.number
    );
    let diff = checked(command(
        host,
        &[
            "api",
            &endpoint,
            "--hostname",
            &account.hostname,
            "--header",
            "Accept: application/vnd.github.diff",
        ],
        30,
    )?)?;
    Ok(json!({"diff": diff}))
}

fn media_url(raw: &str) -> Result<url::Url> {
    let url = url::Url::parse(raw).context("Invalid GitHub media URL")?;
    ensure!(
        url.scheme() == "https"
            && url.port_or_known_default() == Some(443)
            && url.username().is_empty()
            && url.password().is_none()
            && matches!(
                url.host_str(),
                Some(
                    "github.com"
                        | "raw.githubusercontent.com"
                        | "user-images.githubusercontent.com"
                        | "private-user-images.githubusercontent.com"
                )
            ),
        "GitHub media is unavailable"
    );
    Ok(url)
}

fn media(host: &super::LocalHost, params: &Value) -> Result<Value> {
    let account = account(params)?;
    ensure!(
        account.hostname == "github.com",
        "GitHub media is unavailable for this account"
    );
    let identity: Identity = serde_json::from_value(params["pullRequest"].clone())?;
    ensure!(
        identity.valid() && identity.hostname == account.hostname,
        "GitHub media is unavailable"
    );
    let mut url = media_url(params["url"].as_str().context("Missing GitHub media URL")?)?;
    if url.host_str() == Some("github.com") && url.path().starts_with("/user-attachments/assets/") {
        let markdown = format!("![attachment]({url})");
        let text = checked(run(
            host,
            &[
                "gh".into(),
                "api".into(),
                "--hostname".into(),
                account.hostname,
                "markdown".into(),
                "--method".into(),
                "POST".into(),
                "-f".into(),
                format!("text={markdown}"),
                "-f".into(),
                "mode=gfm".into(),
                "-f".into(),
                format!("context={}/{}", identity.owner, identity.repository),
            ],
            30,
        )?)?;
        let source = text
            .split("src=\"")
            .nth(1)
            .and_then(|s| s.split('"').next())
            .context("GitHub media is unavailable")?;
        url = media_url(&source.replace("&amp;", "&"))?;
        ensure!(
            matches!(
                url.host_str(),
                Some(
                    "private-user-images.githubusercontent.com"
                        | "user-images.githubusercontent.com"
                )
            ),
            "GitHub media is unavailable"
        );
    }
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .max_redirects(0)
        .http_status_as_error(false)
        .timeout_global(Some(Duration::from_secs(30)))
        .build()
        .into();
    for _ in 0..6 {
        let mut response = agent
            .get(url.as_str())
            .call()
            .context("GitHub media is unavailable")?;
        if response.status().is_redirection() {
            let next = response
                .headers()
                .get("location")
                .and_then(|v| v.to_str().ok())
                .context("GitHub media redirect is missing")?;
            url = media_url(url.join(next)?.as_str())?;
            continue;
        }
        ensure!(
            response.status().is_success(),
            "GitHub media is unavailable"
        );
        let mime = response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.split(';').next())
            .unwrap_or("")
            .to_owned();
        ensure!(mime.starts_with("image/"), "GitHub media is unavailable");
        let bytes = response
            .body_mut()
            .with_config()
            .limit(10 * 1024 * 1024)
            .read_to_vec()?;
        return Ok(
            json!({"status":"success","mimeType":mime,"contentsBase64":base64::engine::general_purpose::STANDARD.encode(bytes)}),
        );
    }
    bail!("GitHub media redirected too many times")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn patch_identity_and_viewer_must_match_the_active_account() {
        let params = json!({
            "account":{"hostId":"local","hostname":"github.com","login":"alice"},
            "pullRequest":{"hostname":"elsewhere.example","owner":"org","repository":"repo","number":1}
        });
        assert!(request_identity(&params).is_err());
        let account = account(&params).unwrap();
        assert!(verify_viewer(&json!({"data":{"viewer":{"login":"bob"}}}), &account).is_err());
        assert!(verify_viewer(&json!({"data":{"viewer":{"login":"ALICE"}}}), &account).is_ok());
    }

    #[test]
    fn missing_executable_does_not_hide_a_missing_working_directory() {
        let home = tempfile::tempdir().unwrap();
        let error = anyhow::Error::new(bridge::RpcError {
            code: -32603,
            message: "could not start gh: No such file or directory".into(),
            data: Value::Null,
        });
        assert!(missing_gh(&error, home.path()));
        assert!(!missing_gh(&error, &home.path().join("missing")));
        assert!(!missing_gh(
            &anyhow::anyhow!("No such file or directory"),
            home.path()
        ));
    }

    #[test]
    fn check_status_preserves_pending_failure_and_partial_results() {
        for (rollup, expected) in [
            (json!({"contexts":{"nodes":[],"totalCount":0}}), "none"),
            (
                json!({"contexts":{"nodes":[{"status":"QUEUED","conclusion":null}],"totalCount":1}}),
                "pending",
            ),
            (
                json!({"contexts":{"nodes":[{"status":"COMPLETED","conclusion":"SUCCESS"}],"totalCount":1}}),
                "passing",
            ),
            (
                json!({"contexts":{"nodes":[{"status":"COMPLETED","conclusion":"SUCCESS"}],"totalCount":2}}),
                "pending",
            ),
            (
                json!({"contexts":{"nodes":[{"status":"QUEUED"},{"conclusion":"FAILURE"}],"totalCount":2}}),
                "failing",
            ),
        ] {
            assert_eq!(ci(&rollup), expected, "{rollup}");
        }
    }

    #[test]
    fn authentication_errors_preserve_the_setup_contract() {
        for (code, stderr) in [(4, ""), (1, "Run gh auth login to continue")] {
            let error = checked(Output {
                code: Some(code),
                stdout: String::new(),
                stderr: stderr.into(),
                missing: false,
            })
            .unwrap_err();
            assert_eq!(error.to_string(), "GitHub CLI (gh) is not authenticated.");
        }
    }

    #[test]
    fn media_rejects_credentials_local_hosts_and_non_https() {
        for url in [
            "http://github.com/a.png",
            "https://github.com.evil.test/a",
            "https://127.0.0.1/a",
            "https://token@github.com/a",
            "https://github.com:444/a",
        ] {
            assert!(media_url(url).is_err(), "{url}");
        }
        assert!(media_url("https://user-images.githubusercontent.com/image.png").is_ok());
    }
}
