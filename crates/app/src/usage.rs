use crate::{
    Root,
    host::{self, Host, Identity, Request},
};
use anyhow::{Context as _, Result, bail, ensure};
use bridge::Bridge;
use gpui::{App, BorrowAppContext as _, Context, Global, WeakEntity};
use serde_json::{Value, json};
use std::{collections::HashSet, sync::Arc};
use ui::screens::{
    computer_use::Load,
    settings::usage::{self as view, Access, Action, BalanceHistory, Limits, Reload, State},
};

#[path = "usage_analytics.rs"]
mod analytics_api;

#[derive(Default)]
struct Runtime {
    generation: u64,
    identity: Option<Identity>,
    reset: Option<ResetAttempt>,
}
impl Global for Runtime {}

#[derive(Clone)]
struct ResetAttempt {
    credit_id: Option<String>,
    request_id: String,
}

impl Runtime {
    fn begin_reset(&mut self, credit_id: Option<String>) -> Result<ResetAttempt> {
        if let Some(attempt) = &self.reset
            && attempt.credit_id != credit_id
        {
            bail!("Retry the previous reset first");
        }
        Ok(self
            .reset
            .get_or_insert_with(|| ResetAttempt {
                credit_id,
                request_id: host::device_id(),
            })
            .clone())
    }
}

pub fn install(root: WeakEntity<Root>, cx: &mut App) {
    cx.set_global(Runtime::default());
    let agent_reset_enabled = cx
        .try_global::<crate::host_settings::Settings>()
        .and_then(|settings| {
            settings
                .read_all()
                .get("agent-usage-reset-enabled")
                .and_then(Value::as_bool)
        })
        .unwrap_or(false);
    cx.set_global(State {
        agent_reset_enabled,
        dispatch: Some(Arc::new(move |action, cx| {
            root.update(cx, |root, cx| root.usage_action(action, cx))
                .ok();
        })),
        ..Default::default()
    });
}

pub fn invalidate(cx: &mut App) {
    cx.update_global::<Runtime, _>(|runtime, _| {
        runtime.generation += 1;
        runtime.identity = None;
        runtime.reset = None;
    });
    let current = cx.global::<State>();
    let dispatch = current.dispatch.clone();
    let page_open = current.page_open;
    let agent_reset_enabled = current.agent_reset_enabled;
    cx.set_global(State {
        dispatch: dispatch.clone(),
        page_open,
        agent_reset_enabled,
        ..Default::default()
    });
    if page_open && let Some(dispatch) = dispatch {
        cx.defer(move |cx| dispatch(Action::Refresh, cx));
    }
}

pub fn on_event(event: &bridge::Event, cx: &mut App) {
    let bridge::Event::Notification { method, params } = event else {
        return;
    };
    if method == "account/updated" {
        invalidate(cx);
    } else if method == "account/rateLimits/updated"
        && let Ok(limit) = serde_json::from_value(params["rateLimits"].clone())
    {
        cx.update_global::<State, _>(|state, _| {
            if let Load::Ready(limits) = &mut state.limits {
                let limits = Arc::make_mut(limits);
                limits.rate_limits = limit;
                if let Some(by_id) = params.get("rateLimitsByLimitId")
                    && let Ok(by_id) = serde_json::from_value(by_id.clone())
                {
                    limits.rate_limits_by_limit_id = by_id;
                }
            }
        });
    }
}

fn load<T>(result: Result<T>) -> Load<Arc<T>> {
    match result {
        Ok(value) => Load::Ready(Arc::new(value)),
        Err(error) => Load::Failed(error.to_string()),
    }
}

fn load_history(result: Result<Option<BalanceHistory>>) -> Load<Option<Arc<BalanceHistory>>> {
    match result {
        Ok(value) => Load::Ready(value.map(Arc::new)),
        Err(error) => Load::Failed(error.to_string()),
    }
}

fn http(
    host: &dyn Host,
    identity: &Identity,
    method: &str,
    url: &str,
    body: Option<Value>,
) -> Result<Value> {
    let mut request = json!({
        "url": url, "method": method,
        "headers": {
            "OAI-Language": "en-US", "oai-did": identity.device_id,
            "X-OpenAI-Attach-Auth": "1", "X-OpenAI-Attach-Integrity-State": "1",
            "ChatGPT-Account-ID": identity.account_id,
        },
        "expectedIdentity": { "accountId": identity.account_id, "userId": identity.user_id },
        "returnErrorResponse": true,
    });
    if let Some(body) = body {
        request["body"] = body.to_string().into();
    }
    let response = host.fetch(&Request::Http(request))?;
    let wire = response.get("value").unwrap_or(&response);
    if wire
        .pointer("/response/__response/status")
        .and_then(Value::as_u64)
        == Some(204)
    {
        return Ok(Value::Null);
    }
    host::json_body(&response)
}

fn read_reload(host: &dyn Host, identity: &Identity) -> Result<Reload> {
    Ok(serde_json::from_value(http(
        host,
        identity,
        "GET",
        "/subscriptions/auto_top_up/settings?include_payment_method=true",
        None,
    )?)?)
}

fn read_app_limits(host: &dyn Host, identity: &Identity) -> Result<Value> {
    let value = match http(host, identity, "GET", "/wham/usage/chatpass/apps", None) {
        Err(error)
            if [
                "host HTTP status 401",
                "host HTTP status 403",
                "host HTTP status 404",
            ]
            .contains(&error.to_string().as_str()) =>
        {
            Value::Null
        }
        result => result?,
    };
    if !value.is_null() {
        let items = value["items"]
            .as_array()
            .context("App limits have no items")?;
        ensure!(
            items.iter().all(|item| item["id"].is_string()
                && item["name"].is_string()
                && item["allowed_usage_percent"]
                    .as_u64()
                    .is_some_and(|value| value <= 100)),
            "Invalid app limit response"
        );
    }
    Ok(value)
}

fn reconcile_reload(state: &mut State, result: Option<Result<Reload>>) -> bool {
    match result {
        Some(Ok(reload)) => {
            state.reload = Load::Ready(Arc::new(reload));
            true
        }
        Some(Err(error)) => {
            if state.error.is_none() {
                state.error = Some(error.to_string());
            }
            false
        }
        None => false,
    }
}

fn access(plan: &str, account: Option<&Value>, gates: &Value, has_balance: bool) -> Access {
    let gate = |key: &str| gates["feature_gates"][key]["value"] == true;
    let consumer = matches!(plan, "plus" | "prolite" | "pro" | "promax");
    let free = matches!(plan, "free" | "go");
    let business = matches!(
        plan,
        "team" | "self_serve_business_prolite" | "self_serve_business_usage_based"
    );
    let enterprise = matches!(
        plan,
        "business"
            | "enterprise"
            | "enterprise_cbp_usage_based"
            | "enterprise_cbp_automation"
            | "enterprise_cbp_trial"
            | "enterprise_cbp_view_only"
            | "deprecated_enterprise"
            | "hc"
            | "finserv"
            | "law"
            | "ent26"
            | "education"
            | "edu_plus"
            | "edu_pro"
            | "edu"
            | "deprecated_edu"
            | "k12"
    );
    let free_purchase =
        gates["dynamic_configs"]["3291298264"]["value"]["allow_free_users_to_use_credits"] == true;
    let manage = consumer || (free && free_purchase);
    let analytics = (free || consumer || business) && gate("4027330399");
    let visible = usage_visible(
        consumer,
        free,
        business,
        enterprise,
        has_balance || free_purchase,
        analytics,
        gates,
    );
    let workspace_owner = account.is_some_and(|a| {
        a["structure"] == "workspace" && a["account_user_role"] == "account-owner"
    });
    Access {
        visible,
        manage_credits: manage,
        view_credits: manage || free && has_balance || business && workspace_owner,
        analytics,
        balance_history: gate("2125321167") && gate("393259752"),
        app_limits: gate("185496087"),
        logged_in_pricing: gates["dynamic_configs"]["337040058"]["value"]["show_logged_in_pricing_page"]
            == true,
        enterprise,
        maximum_credits:
            gates["dynamic_configs"]["2414703414"]["value"]["credit_purchase_quantity_max"]
                .as_u64()
                .unwrap_or(250_000),
    }
}

fn usage_visible(
    consumer: bool,
    free: bool,
    business: bool,
    enterprise: bool,
    credits_available: bool,
    analytics: bool,
    gates: &Value,
) -> bool {
    let gate = |key: &str| gates["feature_gates"][key]["value"] == true;
    consumer
        || free && (credits_available || gate("3909937021"))
        || enterprise && gate("3909937021")
        || business && gate("1014047933")
        || analytics
}

fn payload_gates(value: Value) -> Result<Value> {
    if let Some(payload) = value["payload"].as_str() {
        Ok(serde_json::from_str(payload)?)
    } else if value.get("feature_gates").is_some() {
        Ok(value)
    } else {
        bail!("Statsig response has no feature gates")
    }
}

async fn snapshot(bridge: Arc<Bridge>, host: Arc<dyn Host>) -> (State, Option<Identity>) {
    let account = bridge
        .request::<_, Value>("account/read", json!({"refreshToken": false}))
        .await;
    let mut state = State::default();
    let account = match account {
        Ok(account) if account["account"]["type"] == "chatgpt" => account,
        Ok(_) => {
            state.limits = Load::Failed("Usage limits require a ChatGPT account".into());
            state.unavailable = true;
            return (state, None);
        }
        Err(error) => {
            state.limits = Load::Failed(error.to_string());
            return (state, None);
        }
    };
    state.plan = account["account"]["planType"]
        .as_str()
        .unwrap_or_default()
        .into();
    let limits = bridge
        .request::<_, Limits>("account/rateLimits/read", json!({}))
        .await;
    if let Ok(limits) = &limits
        && let Some(plan) = &limits.rate_limits.plan_type
    {
        state.plan = plan.clone();
    }
    state.limits = load(limits);
    let identity = host.identity();
    let has_balance = matches!(&state.limits, Load::Ready(limits) if limits.rate_limits.credits.as_ref().is_some_and(|credits| credits.balance.is_some()));
    state.access = access(&state.plan, None, &Value::Null, has_balance);
    let Some(identity) = identity else {
        state.access.manage_credits = false;
        state.reload = Load::Failed("Automatic reload requires a ChatGPT account".into());
        return (state, None);
    };
    let accounts = http(
        host.as_ref(),
        &identity,
        "GET",
        "/accounts/check/v4-2023-04-27",
        None,
    )
    .ok();
    let entry = accounts
        .as_ref()
        .map(|accounts| &accounts["accounts"][&identity.account_id]);
    let current = entry.map(|entry| &entry["account"]);
    state.plan_origin = entry
        .and_then(|entry| entry["last_active_subscription"]["purchase_origin_platform"].as_str())
        .map(str::to_owned);
    if let Some(plan) = current.and_then(|account| account["plan_type"].as_str()) {
        state.plan = plan.into();
    }
    let gates = http(host.as_ref(), &identity, "POST", "/wham/statsig/bootstrap", Some(json!({
        "app_session_id": host::device_id(), "app_version": "26.1007.30912", "brand_name": "chatgpt",
        "build_flavor": "prod", "locale": "en-US", "stable_id": identity.device_id,
        "system_name": std::env::consts::OS, "window_type": "electron",
    }))).and_then(payload_gates).unwrap_or(Value::Null);
    state.access = access(&state.plan, current, &gates, has_balance);
    if state.access.app_limits {
        state.app_limits = load(read_app_limits(host.as_ref(), &identity));
        if matches!(&state.app_limits, Load::Ready(apps) if !apps.is_null())
            && state.access.view_credits
        {
            state.app_credit_preference = load(http(
                host.as_ref(),
                &identity,
                "GET",
                "/settings/user",
                None,
            ));
        }
    }
    let credits: Result<view::ResetCredits> = http(
        host.as_ref(),
        &identity,
        "GET",
        "/wham/rate-limit-reset-credits",
        None,
    )
    .and_then(|value| Ok(serde_json::from_value(value)?));
    if let Load::Ready(limits) = &mut state.limits {
        Arc::make_mut(limits).rate_limit_reset_credits = credits.ok();
    }
    load_account_details(&mut state, &bridge, &host, &identity, entry).await;
    (state, Some(identity))
}

async fn load_account_details(
    state: &mut State,
    bridge: &Arc<Bridge>,
    host: &Arc<dyn Host>,
    identity: &Identity,
    entry: Option<&Value>,
) {
    if state.access.manage_credits {
        state.reload = load(read_reload(host.as_ref(), identity));
        state.has_saved_payment_method = http(
            host.as_ref(),
            identity,
            "GET",
            &format!(
                "/payments/payment_methods?account_id={}",
                identity.account_id
            ),
            None,
        )
        .ok()
        .and_then(|value| {
            value["payment_methods"]
                .as_array()
                .map(|methods| !methods.is_empty())
        })
        .unwrap_or(false);
        state.reload_pricing = load(
            entry
                .and_then(|entry| entry["entitlement"]["billing_currency"].as_str())
                .context("Billing currency is unavailable")
                .and_then(|currency| {
                    ensure!(
                        currency.bytes().all(|byte| byte.is_ascii_alphabetic()),
                        "Invalid billing currency"
                    );
                    http(
                        host.as_ref(),
                        identity,
                        "GET",
                        &format!("/checkout_pricing_config/configs/{currency}"),
                        None,
                    )
                }),
        );
    }
    if state.access.enterprise {
        let id = &identity.account_id;
        state.monthly = load(http(
            host.as_ref(),
            identity,
            "GET",
            &format!(
                "/accounts/{id}/spend-controls/current-user/monthly-usage?supports_usage_limit_modes=true"
            ),
            None,
        ));
        state.request_policy = load(
            http(
                host.as_ref(),
                identity,
                "GET",
                &format!("/accounts/{id}/settings"),
                None,
            )
            .map(|settings| {
                settings
                    .get("usage_limit_increase_request")
                    .filter(|value| !value.is_null())
                    .cloned()
                    .unwrap_or_else(|| json!({"kind":"openai_native"}))
            }),
        );
        if matches!(&state.request_policy, Load::Ready(policy) if policy["kind"] == "openai_native")
        {
            state.requests = load(requests(host.as_ref(), identity));
        }
    }
    if matches!(&state.limits, Load::Ready(limits) if limits.rate_limit_reset_credits.as_ref().is_some_and(|credits| credits.history_enabled))
    {
        state.reset_history = load(http(
            host.as_ref(),
            identity,
            "GET",
            "/wham/rate-limit-reset-credits/history",
            None,
        ));
    }
    if state.access.analytics {
        state.analytics = analytics_api::snapshot(
            bridge.as_ref(),
            host.as_ref(),
            identity,
            state.reference_time,
        )
        .await;
    }
    if state.access.balance_history {
        let minutes = match &state.limits {
            Load::Ready(limits) => limits
                .rate_limits
                .primary
                .iter()
                .chain(limits.rate_limits.secondary.iter())
                .filter_map(|window| window.window_duration_mins)
                .filter(|minutes| matches!(minutes, 300 | 10080))
                .min(),
            _ => None,
        };
        if let Some(minutes) = minutes {
            state.history = load_history(history(host.as_ref(), identity, minutes));
        } else {
            state.access.balance_history = false;
        }
    }
}

fn requests(host: &dyn Host, identity: &Identity) -> Result<Value> {
    let mut items = Vec::new();
    let mut cursor: Option<String> = None;
    let mut seen = HashSet::new();
    loop {
        let mut url = url::Url::parse(&format!(
            "https://chatgpt.com/accounts/{}/workspace_admin_requests",
            identity.account_id
        ))?;
        url.query_pairs_mut().append_pair("limit", "100");
        if let Some(cursor) = &cursor {
            url.query_pairs_mut().append_pair("cursor", cursor);
        }
        let value = http(
            host,
            identity,
            "GET",
            &format!("{}?{}", url.path(), url.query().unwrap_or_default()),
            None,
        )?;
        items.extend(
            value["items"]
                .as_array()
                .context("Request response has no items")?
                .iter()
                .cloned(),
        );
        let next = value["cursor"].as_str().map(str::to_owned);
        if next.is_none() {
            return Ok(json!({"items":items}));
        }
        ensure!(
            seen.insert(next.clone()),
            "Repeated workspace request cursor"
        );
        cursor = next;
    }
}

fn history(host: &dyn Host, identity: &Identity, minutes: i64) -> Result<Option<BalanceHistory>> {
    ensure!(matches!(minutes, 300 | 10080), "Unsupported plan window");
    let mut result = BalanceHistory::default();
    let mut seen = HashSet::new();
    loop {
        let mut url = url::Url::parse("https://chatgpt.com/wham/usage/plan_limit_balance_history")?;
        url.query_pairs_mut()
            .append_pair("window_minutes", &minutes.to_string());
        if let Some(cursor) = &result.next_cursor {
            url.query_pairs_mut().append_pair("cursor", cursor);
        }
        let request = http(
            host,
            identity,
            "GET",
            &format!("{}?{}", url.path(), url.query().unwrap_or_default()),
            None,
        );
        let page: BalanceHistory = match request {
            Ok(value) => serde_json::from_value(value)?,
            Err(error) if error.to_string() == "host HTTP status 404" => return Ok(None),
            Err(error) => return Err(error),
        };
        ensure!(
            page.next_cursor.is_none() || seen.insert(page.next_cursor.clone()),
            "Repeated plan history cursor"
        );
        result.points.extend(page.points);
        result.data_as_of = page.data_as_of;
        result.next_cursor = page.next_cursor;
        if result.next_cursor.is_none() {
            return Ok(Some(result));
        }
    }
}

async fn usage_operation(
    action: Action,
    snapshot: &State,
    attempt: Option<ResetAttempt>,
    host: &dyn Host,
    bridge: &Bridge,
    identity: &Identity,
) -> Result<(Action, Value)> {
    ensure!(
        host.identity().as_ref() == Some(identity),
        "Account changed before usage operation"
    );
    let value = match &action {
        Action::History(minutes) => history(host, identity, *minutes)?
            .map(serde_json::to_value)
            .transpose()?
            .unwrap_or(Value::Null),
        Action::Reset(_) => {
            let attempt = attempt.context("Missing reset request identity")?;
            let result = bridge
                .request::<_, Value>(
                    "account/rateLimitResetCredit/consume",
                    json!({
                        "creditId": attempt.credit_id, "idempotencyKey": attempt.request_id,
                    }),
                )
                .await?;
            let limits = bridge
                .request::<_, Value>("account/rateLimits/read", json!({}))
                .await;
            json!({
                "outcome": result["outcome"],
                "limits": limits.as_ref().ok(),
                "refreshError": limits.err().map(|error| error.to_string()),
            })
        }
        Action::SaveReload(draft) => save_reload(draft, snapshot, host, identity)?,
        Action::DisableReload => {
            ensure!(
                snapshot.access.manage_credits,
                "Credit changes are unavailable"
            );
            let result = http(
                host,
                identity,
                "POST",
                "/subscriptions/auto_top_up/disable",
                None,
            )?;
            ensure!(
                result["is_enabled"] == false,
                "Auto-reload was not disabled"
            );
            result
        }
        Action::SaveAppLimits(values) => {
            ensure!(snapshot.access.app_limits, "App limits are unavailable");
            let Load::Ready(apps) = &snapshot.app_limits else {
                bail!("App limits have not loaded")
            };
            let items = apps["items"]
                .as_array()
                .context("App limits have no items")?;
            ensure!(
                values.len() == items.len()
                    && items.iter().all(|item| item["id"]
                        .as_str()
                        .is_some_and(|id| values.contains_key(id))),
                "App limits changed; reload before saving"
            );
            ensure!(
                values.values().all(|value| *value <= 100),
                "Invalid app limit"
            );
            http(
                host,
                identity,
                "PUT",
                "/wham/usage/chatpass/limits",
                Some(json!({"global_percentage":100,"per_app_percentage":values})),
            )?;
            read_app_limits(host, identity)?
        }
        Action::SetAppCreditPreference(enabled) => {
            ensure!(
                snapshot.access.app_limits && snapshot.access.view_credits,
                "Credit preference is unavailable"
            );
            http(
                host,
                identity,
                "PATCH",
                &format!(
                    "/settings/account_user_setting?feature=can_consume_personal_credits_chatpass&value={enabled}"
                ),
                None,
            )?;
            http(host, identity, "GET", "/settings/user", None)?
        }
        Action::RequestMore(justification) => {
            request_more(justification, snapshot, host, identity)?
        }
        _ => bail!("Unsupported usage operation"),
    };
    Ok((action, value))
}

fn save_reload(
    draft: &view::ReloadDraft,
    snapshot: &State,
    host: &dyn Host,
    identity: &Identity,
) -> Result<Value> {
    ensure!(
        snapshot.access.manage_credits,
        "Credit changes are unavailable"
    );
    let settings = read_reload(host, identity)?;
    draft
        .validate(&settings, snapshot.access.maximum_credits)
        .map_err(anyhow::Error::msg)?;
    if settings.is_enabled && draft == &view::ReloadDraft::from_settings(&settings) {
        return Ok(serde_json::to_value(settings)?);
    }
    ensure!(
        snapshot.has_saved_payment_method,
        "A saved payment method is required"
    );
    let mut payload = draft.payload(&settings);
    if !settings.is_enabled {
        let Load::Ready(pricing) = &snapshot.reload_pricing else {
            bail!("Could not load credit pricing. Try again to continue.")
        };
        let config = &pricing["currency_config"];
        ensure!(
            config["amount_per_credit"]
                .as_f64()
                .is_some_and(|amount| amount > 0.),
            "Credit price is unavailable"
        );
        let currency = config["symbol_code"]
            .as_str()
            .context("Billing currency is unavailable")?;
        payload["billing_details"] = json!({"currency": currency});
    }
    let endpoint = if settings.is_enabled {
        "update"
    } else {
        "enable"
    };
    let result = http(
        host,
        identity,
        "POST",
        &format!("/subscriptions/auto_top_up/{endpoint}"),
        Some(payload),
    )?;
    ensure!(
        result["mandate_setup"].is_null(),
        "Auto-reload mandate confirmation requires a supported client"
    );
    ensure!(result["is_enabled"] == true, "Auto-reload was not enabled");
    if let Some(status) = result["immediate_top_up_status"].as_str() {
        ensure!(
            matches!(status, "succeeded" | "not_required"),
            "Initial top-up was not confirmed: {status}"
        );
    }
    Ok(result)
}

fn request_more(
    justification: &str,
    snapshot: &State,
    host: &dyn Host,
    identity: &Identity,
) -> Result<Value> {
    ensure!(
        snapshot.access.enterprise,
        "Workspace usage requests are unavailable"
    );
    ensure!(
        matches!(&snapshot.request_policy, Load::Ready(policy) if policy["kind"] == "openai_native"),
        "Workspace requests are not enabled"
    );
    ensure!(
        !justification.trim().is_empty(),
        "Enter a justification to submit your request"
    );
    let mut path = format!("/accounts/{}/workspace_admin_requests", identity.account_id);
    let latest = requests(host, identity)?;
    let method = if let Some(id) = view::pending_request_in(&latest).and_then(|v| v["id"].as_str())
    {
        path.push('/');
        path.push_str(&url::form_urlencoded::byte_serialize(id.as_bytes()).collect::<String>());
        "PATCH"
    } else {
        "POST"
    };
    http(
        host,
        identity,
        method,
        &path,
        Some(json!({"justification":justification.trim()})),
    )?;
    requests(host, identity)
}

fn apply_reset_response(state: &mut State, value: &Value) -> bool {
    let completed = match value["outcome"].as_str() {
        Some("reset" | "alreadyRedeemed") => {
            state.notice = Some("Usage limits reset".into());
            true
        }
        Some("nothingToReset") => {
            state.error = Some("No current usage limit is eligible for a reset.".into());
            true
        }
        Some("noCredit") => {
            state.error = Some("No resets available".into());
            true
        }
        _ => {
            state.error = Some("Couldn’t confirm the usage reset. Retry the same reset.".into());
            false
        }
    };
    if !value["limits"].is_null() {
        state.limits = load(serde_json::from_value(value["limits"].clone()).map_err(Into::into));
    } else if let Some(error) = value["refreshError"].as_str() {
        state.error = Some(error.into());
    }
    completed
}

fn apply_usage_response(
    state: &mut State,
    action: Action,
    value: Value,
    reload_confirmed: bool,
) -> bool {
    match action {
        Action::History(_) => {
            state.history = load_history(if value.is_null() {
                Ok(None)
            } else {
                serde_json::from_value(value).map(Some).map_err(Into::into)
            })
        }
        Action::Reset(_) => return apply_reset_response(state, &value),
        Action::SaveReload(_) | Action::DisableReload => {
            if reload_confirmed {
                let enabled = matches!(&state.reload, Load::Ready(reload) if reload.is_enabled);
                state.notice = Some(
                    if enabled {
                        "Updated auto reload settings"
                    } else {
                        "Disabled auto reload"
                    }
                    .into(),
                );
            }
        }
        Action::RequestMore(_) => {
            state.requests = Load::Ready(Arc::new(value));
            state.notice = Some("Request submitted".into());
        }
        Action::SaveAppLimits(_) => {
            state.app_limits = Load::Ready(Arc::new(value));
            state.notice = Some("App limits updated".into());
        }
        Action::SetAppCreditPreference(_) => {
            state.app_credit_preference = Load::Ready(Arc::new(value));
            state.notice = Some("Credit preference updated".into());
        }
        _ => {}
    }
    false
}

impl Root {
    fn usage_action(&mut self, action: Action, cx: &mut Context<Self>) {
        if let Action::SetAgentResetEnabled(enabled) = action {
            let result = if cx.has_global::<crate::host_settings::Settings>() {
                cx.global_mut::<crate::host_settings::Settings>()
                    .write("agent-usage-reset-enabled", Value::Bool(enabled))
            } else {
                Err(anyhow::anyhow!("Host settings are unavailable"))
            };
            cx.update_global::<State, _>(|state, _| match result {
                Ok(()) => state.agent_reset_enabled = enabled,
                Err(error) => state.error = Some(error.to_string()),
            });
            return;
        }
        if let Action::OpenUrl(url) = action {
            if let Ok(url) = url::Url::parse(&url)
                && url.scheme() == "https"
                && url.username().is_empty()
                && url.password().is_none()
            {
                cx.open_url(url.as_str());
            }
            return;
        }
        if cx.global::<State>().busy {
            return;
        }
        match action {
            Action::Analytics(request) => self.load_usage_analytics(request, cx),
            Action::Refresh => self.refresh_usage(cx),
            action => self.update_usage(action, cx),
        }
    }

    fn load_usage_analytics(&mut self, request: view::analytics::Query, cx: &mut Context<Self>) {
        if !cx.global::<State>().access.analytics {
            return;
        }
        let Some(identity) = cx.global::<Runtime>().identity.clone() else {
            return;
        };
        let previous = cx.global::<State>().analytics.clone();
        let now = cx.global::<State>().reference_time;
        let generation = cx.global::<Runtime>().generation;
        cx.update_global::<State, _>(|state, _| {
            state.busy = true;
            state.analytics.set(request.metric, Load::Loading);
        });
        let bridge = self.bridge.clone();
        let host = self.host.clone();
        let read = cx.background_executor().spawn(async move {
            analytics_api::query(
                bridge.as_ref(),
                host.as_ref(),
                &identity,
                request,
                now,
                previous,
            )
            .await
        });
        cx.spawn(async move |_, cx| {
            let data = read.await;
            cx.update(|cx| {
                if cx.global::<Runtime>().generation == generation {
                    cx.update_global::<State, _>(|state, _| {
                        state.analytics = data;
                        state.busy = false;
                    });
                }
            });
        })
        .detach();
    }

    fn refresh_usage(&mut self, cx: &mut Context<Self>) {
        let generation = cx.update_global::<Runtime, _>(|runtime, _| {
            runtime.generation += 1;
            runtime.generation
        });
        cx.update_global::<State, _>(|state, _| {
            state.busy = true;
            state.error = None;
            state.notice = None;
        });
        let bridge = self.bridge.clone();
        let host = self.host.clone();
        let task = cx.background_executor().spawn(snapshot(bridge, host));
        cx.spawn(async move |_, cx| {
            let (mut state, identity) = task.await;
            cx.update(|cx| {
                if cx.global::<Runtime>().generation != generation {
                    return;
                }
                state.dispatch = cx.global::<State>().dispatch.clone();
                state.page_open = cx.global::<State>().page_open;
                state.agent_reset_enabled = cx
                    .try_global::<crate::host_settings::Settings>()
                    .and_then(|settings| {
                        settings
                            .read_all()
                            .get("agent-usage-reset-enabled")
                            .and_then(Value::as_bool)
                    })
                    .unwrap_or(false);
                state.revision = cx.global::<State>().revision + 1;
                cx.update_global::<Runtime, _>(|runtime, _| {
                    if runtime.identity != identity {
                        runtime.reset = None;
                    }
                    runtime.identity = identity;
                });
                cx.set_global(state);
            });
        })
        .detach();
    }

    fn update_usage(&mut self, action: Action, cx: &mut Context<Self>) {
        let Some(identity) = cx.global::<Runtime>().identity.clone() else {
            cx.update_global::<State, _>(|state, _| {
                state.error = Some("Usage changes require a ChatGPT account".into())
            });
            return;
        };
        let snapshot = cx.global::<State>().clone();
        let attempt = if let Action::Reset(credit_id) = &action {
            match cx.global_mut::<Runtime>().begin_reset(credit_id.clone()) {
                Ok(attempt) => Some(attempt),
                Err(error) => {
                    cx.update_global::<State, _>(|state, _| state.error = Some(error.to_string()));
                    return;
                }
            }
        } else {
            None
        };
        cx.update_global::<State, _>(|state, _| {
            state.busy = true;
            state.error = None;
            state.notice = None;
        });
        let host = self.host.clone();
        let bridge = self.bridge.clone();
        let generation = cx.global::<Runtime>().generation;
        let billing = matches!(action, Action::SaveReload(_) | Action::DisableReload);
        let task = cx.background_executor().spawn(async move {
            let result = usage_operation(
                action,
                &snapshot,
                attempt,
                host.as_ref(),
                bridge.as_ref(),
                &identity,
            )
            .await;
            let reloaded = billing.then(|| read_reload(host.as_ref(), &identity));
            (result, reloaded)
        });
        cx.spawn(async move |_, cx| {
            let (result, reloaded) = task.await;
            cx.update(|cx| {
                if cx.global::<Runtime>().generation != generation {
                    return;
                }
                let mut reset_completed = false;
                cx.update_global::<State, _>(|state, _| {
                    state.busy = false;
                    let reload_confirmed = reconcile_reload(state, reloaded);
                    match result {
                        Err(error) => state.error = Some(error.to_string()),
                        Ok((action, value)) => {
                            state.revision += 1;
                            reset_completed =
                                apply_usage_response(state, action, value, reload_confirmed);
                        }
                    }
                });
                if reset_completed {
                    cx.global_mut::<Runtime>().reset = None;
                }
            });
        })
        .detach();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enterprise_and_history_require_the_original_gates() {
        assert!(!access("business", None, &Value::Null, false).visible);
        assert!(!access("free", None, &Value::Null, false).visible);
        assert!(!access("quorum", None, &Value::Null, true).visible);
        assert!(!access("promax", None, &Value::Null, true).balance_history);
        let gates = json!({"feature_gates": {
            "3909937021":{"value":true},
            "2125321167":{"value":true},
            "393259752":{"value":true}
        }});
        let enterprise = access("business", None, &gates, false);
        assert!(enterprise.visible && enterprise.enterprise);
        let consumer = access("promax", None, &gates, true);
        assert!(consumer.visible && consumer.manage_credits && consumer.balance_history);
    }

    #[test]
    fn original_recording_loads_limits_without_a_cloud_identity() -> Result<()> {
        let recording =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/readback.jsonl");
        let (bridge, _) = Bridge::replay(&recording)?;
        let home = tempfile::tempdir()?;
        let (state, identity) = pollster::block_on(snapshot(
            Arc::new(bridge),
            Arc::new(host::LocalHost::new(home.path())),
        ));
        assert!(identity.is_none());
        assert!(state.access.visible);
        let Load::Ready(limits) = state.limits else {
            bail!("Recorded limits must load");
        };
        assert_eq!(
            limits
                .rate_limits
                .primary
                .as_ref()
                .context("weekly window")?
                .remaining(),
            99.
        );
        assert_eq!(
            limits
                .rate_limit_reset_credits
                .as_ref()
                .context("reset summary")?
                .available_count,
            1
        );
        Ok(())
    }

    #[test]
    fn an_uncertain_reset_keeps_its_key_and_explains_a_different_choice() -> Result<()> {
        let mut runtime = Runtime::default();
        let first = runtime.begin_reset(Some("credit-a".into()))?;
        let retry = runtime.begin_reset(Some("credit-a".into()))?;
        assert_eq!(retry.request_id, first.request_id);
        let other = runtime.begin_reset(Some("credit-b".into()));
        assert_eq!(
            other.err().map(|error| error.to_string()).as_deref(),
            Some("Retry the previous reset first")
        );
        Ok(())
    }

    #[test]
    fn billing_error_keeps_the_error_and_refreshes_confirmed_server_state() {
        let mut state = State {
            error: Some("Initial top-up was not confirmed".into()),
            reload: Load::Ready(Arc::new(Reload::default())),
            ..Default::default()
        };
        assert!(reconcile_reload(
            &mut state,
            Some(Ok(Reload {
                is_enabled: true,
                ..Default::default()
            }))
        ));
        assert!(matches!(&state.reload, Load::Ready(reload) if reload.is_enabled));
        assert_eq!(
            state.error.as_deref(),
            Some("Initial top-up was not confirmed")
        );
        assert!(state.notice.is_none());
    }
}
