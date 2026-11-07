//! What the profile avatar menu (`codex.profileDropdown`) shows: the account's name and plan,
//! the primary limit's remaining percent, the Help flyout's gates and the changelog posts.

use crate::{
    Root,
    chatgpt::Gate,
    host::{self, CHANGELOG_FEED, Host, Request},
    usage,
};
use anyhow::{Context as _, Result};
use bridge::Bridge;
use gpui::{Context, Div, InteractiveElement as _};
use serde_json::{Value, json};
use ui::screens::settings::usage::Limits;
use ui::{ChangelogPost, Profile};

pub(crate) fn actions(root: Div, cx: &mut Context<Root>) -> Div {
    root.on_action(cx.listener(|this, _: &ui::RefreshProfile, _, cx| load(this, cx)))
}

/// Reads the profile off the UI thread and gives it to every shell; the last read wins.
pub(crate) fn load(root: &mut Root, cx: &mut Context<Root>) {
    let bridge = root.bridge.clone();
    let host = root.host.clone();
    let task = cx
        .background_executor()
        .spawn(async move { read(&bridge, host.as_ref()).await });
    cx.spawn(async move |this, cx| {
        let profile = task.await;
        this.update(cx, |root, cx| {
            for shell in std::iter::once(&root.shell).chain(root.page.iter()) {
                shell.update(cx, |shell, cx| {
                    shell.profile = profile.clone();
                    cx.notify();
                });
            }
        })
        .ok();
    })
    .detach();
}

async fn read(bridge: &Bridge, host: &dyn Host) -> Profile {
    let mut profile = Profile {
        changelog: changelog(host).unwrap_or_default(),
        ..Default::default()
    };
    let Ok(account) = bridge
        .request::<_, Value>("account/read", json!({"refreshToken": false}))
        .await
    else {
        return profile;
    };
    let account = &account["account"];
    profile.name = account["email"]
        .as_str()
        .map(|email| email.to_owned().into());
    if account["type"] != "chatgpt" {
        return profile;
    }
    let limits = bridge
        .request::<_, Limits>("account/rateLimits/read", json!({}))
        .await
        .ok();
    let plan = limits
        .as_ref()
        .and_then(|limits| limits.rate_limits.plan_type.as_deref())
        .or(account["planType"].as_str())
        .unwrap_or_default();
    profile.plan = Some(plan_name(plan).into());
    profile.usage_left = limits
        .and_then(|limits| limits.rate_limits.primary)
        .map(|window| window.remaining());
    if let Some(identity) = host.identity() {
        if let Some(name) = usage::http(host, &identity, "GET", "/me", None)
            .ok()
            .and_then(|me| me["name"].as_str().map(str::to_owned))
            .filter(|name| !name.is_empty())
        {
            profile.name = Some(name.into());
        }
        let gates = usage::statsig(host, &identity);
        profile.chrome_extension_setup = Gate::ChromeExtensionSetup.enabled(&gates);
        profile.privacy_center = Gate::PrivacyCenter.enabled(&gates);
    }
    profile
}

/// `pricingPlanConstants.*.name`.
fn plan_name(plan: &str) -> String {
    match plan {
        "team" | "business" | "self_serve_business_usage_based" => "Business".into(),
        "edu" | "education" => "Edu".into(),
        _ => {
            let mut chars = plan.chars();
            chars
                .next()
                .map(|first| first.to_uppercase().chain(chars).collect())
                .unwrap_or_default()
        }
    }
}

/// The feed's newest three posts (`items.slice(0, 3)`).
fn changelog(host: &dyn Host) -> Result<Vec<ChangelogPost>> {
    let response = host.fetch(&Request::Fetch {
        method: "GET".into(),
        url: CHANGELOG_FEED.into(),
        body: String::new(),
    })?;
    let feed = host::json_body(&response)?;
    let items = feed["items"].as_array().context("changelog has no items")?;
    items
        .iter()
        .take(3)
        .map(|item| {
            let date = item["date"]
                .as_str()
                .context("changelog post has no date")?;
            Ok(ChangelogPost {
                title: item["title"].as_str().unwrap_or_default().to_owned().into(),
                date: chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d")?
                    .format("%-d %b")
                    .to_string()
                    .into(),
                url: item["url"]
                    .as_str()
                    .context("changelog post has no URL")?
                    .to_owned()
                    .into(),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plan_names_match_the_pricing_constants() {
        assert_eq!(plan_name("pro"), "Pro");
        assert_eq!(plan_name("plus"), "Plus");
        assert_eq!(plan_name("team"), "Business");
        assert_eq!(plan_name(""), "");
    }
}
