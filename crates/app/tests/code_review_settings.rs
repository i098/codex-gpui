//! Settings ▸ Code Review through the real `Root` against a stateful fake of the ChatGPT
//! backend's `/wham/settings` review service: turn on automatic review, open a repository and
//! turn off its automatic review.
#[path = "common/mod.rs"]
mod common;

use anyhow::{Context as _, Result, ensure};
use app::host::{Host, Identity, Request};
use common::Fx;
use serde_json::{Value, json};
use std::sync::Mutex;

#[test]
fn proxy() {
    common::proxy_child();
}

const REPO: &str = "github-acme/widgets";

struct Backend {
    prefs: Mutex<Value>,
    repo: Mutex<Value>,
    log: Mutex<Vec<(String, String, Value)>>,
}

impl Default for Backend {
    fn default() -> Self {
        Self {
            prefs: Mutex::new(json!({
                "code_review_preference": "repo_default",
                "code_review_trigger_policy": "pr_open",
                "allow_credits_for_code_reviews": false,
            })),
            repo: Mutex::new(json!({
                "repository": {
                    "id": REPO,
                    "repository_full_name": "acme/widgets",
                    "clone_url": "https://github.com/acme/widgets.git",
                    "default_branch": "main",
                    "permissions": {"admin": true, "push": true},
                },
                "review_preference": "team",
                "global_review_preference": "review_if_enabled",
                "funding_generation": 3,
            })),
            log: Mutex::default(),
        }
    }
}

impl Backend {
    fn answer(&self, method: &str, url: &str, body: &Value) -> (u16, Value) {
        let detail = format!("/wham/settings/code_review/{}", REPO.replace('/', "%2F"));
        match method {
            "GET" if url == "/wham/settings/user" => (200, self.prefs.lock().unwrap().clone()),
            "PATCH" if url == "/wham/settings/user" => {
                let mut prefs = self.prefs.lock().unwrap();
                for (key, value) in body.as_object().into_iter().flatten() {
                    prefs[key] = value.clone();
                }
                (200, prefs.clone())
            }
            "GET" if url.starts_with("/wham/settings/code_review_search") => (
                200,
                json!({"repo_review_settings": [self.repo.lock().unwrap().clone()]}),
            ),
            "GET" if url == detail => (200, self.repo.lock().unwrap().clone()),
            "PATCH" if url == "/wham/settings/code_review" => {
                let mut repo = self.repo.lock().unwrap();
                for (key, value) in body["repo_review_settings"][0]
                    .as_object()
                    .into_iter()
                    .flatten()
                {
                    if key != "repository" {
                        repo[key] = value.clone();
                    }
                }
                (200, json!({}))
            }
            _ => (404, json!({"detail": "not found"})),
        }
    }

    fn patches(&self) -> Vec<(String, Value)> {
        self.log
            .lock()
            .unwrap()
            .iter()
            .filter(|(m, _, _)| m == "PATCH")
            .map(|(_, url, body)| (url.clone(), body.clone()))
            .collect()
    }
}

impl Host for Backend {
    fn fetch(&self, request: &Request) -> Result<Value> {
        let Request::Http(request) = request else {
            anyhow::bail!("not an HTTP request");
        };
        let method = request["method"].as_str().context("method")?.to_owned();
        let url = request["url"].as_str().context("url")?.to_owned();
        let body = request["body"]
            .as_str()
            .map(serde_json::from_str)
            .transpose()?
            .unwrap_or(Value::Null);
        let (status, answer) = self.answer(&method, &url, &body);
        self.log.lock().unwrap().push((method, url, body));
        Ok(json!({"status": status, "bodyJsonString": answer.to_string()}))
    }

    fn identity(&self) -> Option<Identity> {
        Some(Identity {
            account_id: "00000000-0000-4000-8000-000000000000".into(),
            user_id: "user-1".into(),
            device_id: "device".into(),
        })
    }
}

fn click(fx: &mut Fx, selector: &str) -> Result<()> {
    let bounds = fx
        .cx
        .debug_bounds(fx.window, selector)?
        .with_context(|| format!("{selector} is not on screen"))?;
    let center = bounds.center();
    fx.click((f32::from(center.x), f32::from(center.y)))
}

fn visible(fx: &mut Fx, selector: &str) -> bool {
    fx.cx
        .debug_bounds(fx.window, selector)
        .is_ok_and(|b| b.is_some())
}

fn shot(fx: &mut Fx, name: &str) -> Result<()> {
    fx.capture_evidence("CGX_CODE_REVIEW_EVIDENCE", name)
}

#[test]
fn personal_and_repository_review_settings() -> Result<()> {
    let mut fx = Fx::new()?;
    let backend = std::sync::Arc::new(Backend::default());
    fx.root
        .update(&mut fx.cx, |root, cx| root.set_host(backend.clone(), cx));
    let link = app::deep_link::parse("codex://settings/code-review").context("link")?;
    let root = fx.root.clone();
    fx.cx.update_window(fx.window, |_, window, cx| {
        root.update(cx, |root, cx| root.open_deep_link(&link, window, cx));
    })?;
    fx.wait("repository list", |fx| visible(fx, "code-review-repo-0"))
        .with_context(|| format!("requests: {:?}", backend.log.lock().unwrap()))?;
    shot(&mut fx, "page")?;

    click(&mut fx, "code-review-auto")?;
    fx.wait("personal auto review saved", |_| {
        backend.prefs.lock().unwrap()["code_review_preference"] == "always"
    })?;
    ensure!(
        backend.patches().first()
            == Some(&(
                "/wham/settings/user".into(),
                json!({"code_review_preference": "always"})
            ))
    );

    click(&mut fx, "code-review-repo-0")?;
    fx.wait("repository detail", |fx| {
        visible(fx, "code-review-repo-auto")
    })?;
    shot(&mut fx, "repository")?;

    click(&mut fx, "code-review-repo-auto")?;
    fx.wait("repository auto review off", |_| {
        backend.repo.lock().unwrap()["global_review_preference"] == "never_review"
    })?;
    let (url, body) = backend
        .patches()
        .last()
        .cloned()
        .context("no repository PATCH")?;
    ensure!(url == "/wham/settings/code_review");
    let item = &body["repo_review_settings"][0];
    ensure!(
        item["repository"] == REPO && item["funding_generation"] == 3,
        "{body}"
    );
    shot(&mut fx, "repository-off")?;

    click(&mut fx, "code-review-back")?;
    fx.wait("back to the list", |fx| visible(fx, "code-review-repo-0"))?;
    Ok(())
}
