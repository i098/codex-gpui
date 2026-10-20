use gpui::BorrowAppContext as _;
use serde_json::Value;
use ui::composer::computer_use::ComputerUseApproval;

fn native_app(params: &Value) -> Option<&str> {
    let meta = &params["_meta"];
    let schema = &params["requestedSchema"];
    let connector = meta["connector_id"].as_str()?;
    let mut connector = connector
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|part| !part.is_empty());
    let native = connector
        .next()
        .is_some_and(|part| part.eq_ignore_ascii_case("computer"))
        && connector
            .next()
            .is_some_and(|part| part.eq_ignore_ascii_case("use"));
    if params["mode"] != "form"
        || params["message"].as_str()?.trim().is_empty()
        || schema["type"] != "object"
        || !schema["properties"].as_object()?.is_empty()
        || schema
            .get("required")
            .is_some_and(|required| required.as_array().is_none_or(|fields| !fields.is_empty()))
        || meta["codex_approval_kind"] != "mcp_tool_call"
        || !native
    {
        return None;
    }
    let app = meta["tool_params"]["app"].as_str()?.trim();
    if app.is_empty() {
        return None;
    }
    Some(app)
}

pub fn approval(params: &Value) -> Option<ComputerUseApproval> {
    let app = native_app(params)?;
    let meta = &params["_meta"];
    let display_name = meta["tool_params_display"]
        .as_array()
        .and_then(|fields| fields.iter().find(|field| field["name"] == "app"))
        .and_then(|field| field["value"].as_str())
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .unwrap_or(app);
    let offered = |scope: &str| {
        meta["persist"] == scope
            || meta["persist"]
                .as_array()
                .is_some_and(|scopes| scopes.iter().any(|value| value == scope))
    };
    Some(ComputerUseApproval {
        app: app.to_owned(),
        display_name: display_name.to_owned(),
        connector_name: meta["connector_name"]
            .as_str()
            .unwrap_or("Computer Use")
            .to_owned(),
        subtitle: meta["subtitle"].as_str().map(str::to_owned),
        high_risk: meta["riskLevel"] == "high",
        allow_session: offered("session"),
        allow_always: offered("always"),
    })
}

const APPROVED: &str = "has-approved-computer-use-app";

impl crate::Root {
    pub(super) fn computer_use_approval(
        &self,
        pending: &crate::PendingApproval,
        cx: &gpui::App,
    ) -> Option<ui::composer::Approval> {
        let request = approval(&pending.params)?;
        let approved = cx
            .try_global::<crate::host_settings::Settings>()
            .is_some_and(|settings| settings.read_all().get(APPROVED) == Some(&Value::Bool(true)));
        Some(ui::composer::Approval::computer_use(
            pending.id.clone(),
            pending.thread().to_owned(),
            request,
            !approved,
        ))
    }
}

pub(crate) fn accepted(method: &str, params: &Value, response: &Value) -> bool {
    method == "mcpServer/elicitation/request"
        && response["action"] == "accept"
        && native_app(params).is_some()
}

pub(crate) fn remember_acceptance(cx: &mut gpui::App) {
    if !cx.has_global::<crate::host_settings::Settings>() {
        eprintln!(
            "codex-gpui: could not save Computer Use disclosure: host settings are unavailable"
        );
        return;
    }
    cx.update_global::<crate::host_settings::Settings, _>(|settings, _| {
        if let Err(error) = settings.write(APPROVED, Value::Bool(true)) {
            eprintln!("codex-gpui: could not save Computer Use disclosure: {error:#}");
        }
    });
}
