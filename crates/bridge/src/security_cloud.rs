//! Codex Security Cloud (`codex_security_cloud`, the `codex_apps` connector behind the Explore
//! menu's `Security Cloud`): the read tools its MCP app (`ui://defense-factory/workflows.html`)
//! calls through `mcpServer/tool/call`, decoded the way its `defense_factory_*` client does.
//! Reads only: scans, triage and monitoring changes stay in the original.

use crate::Bridge;
use serde::Deserialize;
use serde_json::{Value, json};

const SERVER: &str = "codex_apps";
/// The statuses of the original's `Open findings` view (`OPEN_FINDING_STATUSES`).
pub const OPEN_STATUSES: &str = "new,triaged,in_progress";

/// One `findings_list` (v2) item: the fields the original's findings table shows.
#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct Finding {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub criticality: Option<String>,
    pub status: String,
    #[serde(default)]
    pub created_at: Option<String>,
    #[serde(default)]
    pub repo_url: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Findings {
    pub items: Vec<Finding>,
    pub total: u64,
}

/// One repository from `workflow_repositories` or `github_repositories`.
#[derive(Clone, Debug, PartialEq)]
pub struct Repository {
    pub name: String,
    pub owner: String,
}

/// One `workflow_list` run.
#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct Scan {
    pub id: String,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub repo_id: String,
    #[serde(default)]
    pub repo_url: Option<String>,
    #[serde(default)]
    pub created_at: Option<String>,
}

/// One page of Security Cloud data; each section fails on its own, as the original's queries do.
#[derive(Clone, Debug, PartialEq)]
pub struct Snapshot {
    /// The header's `{remaining} free credits` amount in USD, when the allowance applies.
    pub free_credits: Option<f64>,
    pub findings: Result<Findings, String>,
    /// `findings_summary` total for the same statuses.
    pub summary: Result<u64, String>,
    pub repositories: Result<Vec<Repository>, String>,
    pub scans: Result<Vec<Scan>, String>,
}
impl Snapshot {
    /// Every section failed with `error`, as when the app-server cannot start the tool thread.
    pub fn failed(error: String) -> Self {
        Self {
            free_credits: None,
            findings: Err(error.clone()),
            summary: Err(error.clone()),
            repositories: Err(error.clone()),
            scans: Err(error),
        }
    }
}

/// The original's `ZC`: an error result raises its text, a result without `data` is incomplete.
fn decode(result: Value) -> Result<Value, String> {
    let structured = &result["structuredContent"];
    if result["isError"] == true || structured["error"].is_object() {
        let text = result["content"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|part| part["type"] == "text")
            .filter_map(|part| part["text"].as_str())
            .collect::<Vec<_>>()
            .join("\n");
        return Err(if text.is_empty() {
            "The Codex Security Cloud request failed.".into()
        } else {
            text
        });
    }
    structured
        .get("data")
        .cloned()
        .ok_or_else(|| "The Codex Security Cloud host returned an incomplete response.".into())
}

/// The original's `lot`: an eligible allowance with `0 < remaining <= total`.
fn free_credits(usage: &Value) -> Option<f64> {
    let allowance = &usage["allowance"];
    if allowance["eligible"] != true {
        return None;
    }
    let amount = |key: &str| allowance[key].as_str()?.trim().parse::<f64>().ok();
    let (total, remaining) = (amount("total_usd")?, amount("remaining_usd")?);
    (total.is_finite() && total >= 0. && remaining > 0. && remaining <= total).then_some(remaining)
}

/// `owner/name` from a GitHub URL, like the original's `repositoryLabel`.
pub fn repository_label(url: &str) -> &str {
    let url = url.strip_prefix("https://github.com/").unwrap_or(url);
    url.strip_suffix(".git").unwrap_or(url)
}

fn repositories(workflow: &Value, github: Option<&Value>) -> Vec<Repository> {
    let mut list: Vec<Repository> = Vec::new();
    let items = workflow["items"]
        .as_array()
        .into_iter()
        .chain(github.and_then(|github| github["items"].as_array()))
        .flatten();
    for item in items {
        let name = item["repository_full_name"]
            .as_str()
            .or_else(|| item["full_name"].as_str())
            .or_else(|| item["repo_url"].as_str().map(repository_label))
            .unwrap_or_default();
        if name.is_empty() || list.iter().any(|repo| repo.name == name) {
            continue;
        }
        let owner = item["owner"]["login"]
            .as_str()
            .or_else(|| item["owner"].as_str())
            .or_else(|| name.split_once('/').map(|(owner, _)| owner))
            .unwrap_or_default();
        list.push(Repository {
            name: name.to_owned(),
            owner: owner.to_owned(),
        });
    }
    list
}

pub struct Client<'a> {
    bridge: &'a Bridge,
    thread_id: String,
}

impl<'a> Client<'a> {
    /// An ephemeral read-only thread, as the host gives an MCP app's tool calls.
    pub async fn connect(bridge: &'a Bridge) -> anyhow::Result<Self> {
        let reply: Value = bridge
            .request(
                "thread/start",
                json!({"ephemeral": true, "permissions": ":read-only", "threadSource": "mcp_extension_host"}),
            )
            .await?;
        let thread_id = reply["thread"]["id"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("thread/start returned no thread id"))?
            .to_owned();
        Ok(Self { bridge, thread_id })
    }

    pub async fn close(self) {
        let _ = self
            .bridge
            .request::<_, Value>("thread/unsubscribe", json!({"threadId": self.thread_id}))
            .await;
    }

    /// `defense_factory_<operation>`, the original's `W.request`.
    pub async fn call(&self, operation: &str, arguments: Value) -> Result<Value, String> {
        let result: Value = self
            .bridge
            .request(
                "mcpServer/tool/call",
                json!({
                    "server": SERVER,
                    "tool": format!("codex_security_cloud.defense_factory_{operation}"),
                    "arguments": arguments,
                    "threadId": self.thread_id,
                }),
            )
            .await
            .map_err(|error| error.to_string())?;
        decode(result)
    }

    /// The first page of each section the original's three tabs read. `statuses` is the
    /// findings status filter (`OPEN_STATUSES`, or empty for all findings).
    pub async fn snapshot(&self, statuses: &str) -> Snapshot {
        let bootstrap = self.call("bootstrap", json!({})).await.unwrap_or_default();
        let billing = bootstrap["billingEnabled"] == true;
        let usage = self.call("usage_get", json!({})).await.ok();
        let status = (!statuses.is_empty()).then_some(statuses);
        let findings = self
            .call(
                "findings_list",
                json!({"parameters": {"query": {
                    "api_version": "v2", "status": status, "sort": "sev_desc,date_desc", "limit": 50
                }}}),
            )
            .await
            .and_then(|data| {
                Ok(Findings {
                    items: Vec::<Finding>::deserialize(&data["items"])
                        .map_err(|error| error.to_string())?,
                    total: data["total"].as_u64().unwrap_or_default(),
                })
            });
        let summary = self
            .call(
                "findings_summary",
                json!({"parameters": {"query": {"api_version": "v2", "status": status}}}),
            )
            .await
            .map(|data| data["total"].as_u64().unwrap_or_default());
        let workflow = self.call("workflow_repositories", json!({})).await;
        let github = match bootstrap["githubConnectorIds"][0].as_str() {
            Some(connector) => Some(
                self.call(
                    "github_repositories",
                    json!({"parameters": {"query": {"connector_id": connector, "page": 1, "per_page": 50}}}),
                )
                .await,
            ),
            None => None,
        };
        let repositories = match (workflow, github) {
            (Err(error), _) | (_, Some(Err(error))) => Err(error),
            (Ok(workflow), github) => Ok(repositories(
                &workflow,
                github.and_then(Result::ok).as_ref(),
            )),
        };
        let scans = self
            .call(
                "workflow_list",
                json!({"parameters": {"query": {"limit": 50, "include_ownership": true}}}),
            )
            .await
            .and_then(|data| {
                Vec::<Scan>::deserialize(&data["items"]).map_err(|error| error.to_string())
            });
        Snapshot {
            free_credits: billing
                .then(|| usage.as_ref().and_then(free_credits))
                .flatten(),
            findings,
            summary,
            repositories,
            scans,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn errors_and_incomplete_results_fail_like_the_original_client() {
        let ok = json!({"structuredContent": {"data": {"total": 0}}, "isError": false});
        assert_eq!(decode(ok), Ok(json!({"total": 0})));
        // Recorded `github_repositories` refusal (no repository access).
        let refused = json!({
            "content": [{"type": "text", "text": "GitHub could not complete the request."}],
            "structuredContent": {"error": {"message": "GitHub could not complete the request."}, "error_code": "FORBIDDEN"},
            "isError": true
        });
        assert_eq!(
            decode(refused),
            Err("GitHub could not complete the request.".into())
        );
        assert_eq!(
            decode(json!({"structuredContent": {"error": {}}})),
            Err("The Codex Security Cloud request failed.".into())
        );
        assert!(decode(json!({"structuredContent": {}})).is_err());
    }

    #[test]
    fn free_credits_need_an_eligible_allowance_within_its_total() {
        let usage = |eligible, remaining: &str| json!({"allowance": {"eligible": eligible, "remaining_usd": remaining, "total_usd": "500"}});
        assert_eq!(free_credits(&usage(true, "500")), Some(500.));
        assert_eq!(free_credits(&usage(false, "500")), None);
        assert_eq!(free_credits(&usage(true, "0")), None);
        assert_eq!(free_credits(&usage(true, "501")), None);
        assert_eq!(free_credits(&usage(true, " ")), None);
    }

    #[test]
    fn repositories_merge_both_lists_once() {
        let workflow = json!({"items": [{"repo_url": "https://github.com/acme/api.git"}]});
        let github = json!({"items": [
            {"full_name": "acme/api", "owner": {"login": "acme"}},
            {"full_name": "acme/web", "owner": {"login": "acme"}}
        ]});
        let names: Vec<_> = repositories(&workflow, Some(&github))
            .into_iter()
            .map(|repo| repo.name)
            .collect();
        assert_eq!(names, ["acme/api", "acme/web"]);
    }
}
