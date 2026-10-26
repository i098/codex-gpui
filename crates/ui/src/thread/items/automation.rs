//! `codex_app` `automation_update` tool calls: their cached results and native task cards.

use super::*;
use serde_json::json;

impl ThreadView {
    pub fn set_automation_card_result(
        &mut self,
        call_id: &str,
        item: &Value,
        cx: &mut Context<Self>,
    ) {
        for entry in self.turns.iter_mut().flat_map(|turn| &mut turn.items) {
            if let ThreadItem::McpToolCall {
                id,
                arguments,
                metadata,
                ..
            } = entry
                && id == call_id
            {
                **arguments = item.clone();
                arguments["mode"] = json!("view");
                metadata.remove("automation");
                metadata.remove("automationId");
                cx.notify();
                return;
            }
        }
    }

    fn open_automation_card(&mut self, call_id: &str, cx: &mut Context<Self>) {
        let Some((_, arguments, metadata)) = self
            .turns
            .iter()
            .flat_map(|turn| &turn.items)
            .filter_map(automation_item)
            .find(|(id, _, _)| *id == call_id)
        else {
            return;
        };
        let mut payload = automation_data(arguments, metadata).clone();
        payload["mode"] = arguments["mode"].clone();
        if let Some(id) = metadata.get("automationId") {
            payload["id"] = id.clone();
        }
        cx.emit(OpenAutomationCard {
            arguments: payload,
            thread_id: self.thread_id.clone(),
            call_id: call_id.to_owned(),
        });
    }

    pub(crate) fn automation_cards(
        &self,
        items: &[ThreadItem],
        window: &mut Window,
        t: &Theme,
        cx: &mut Context<Self>,
    ) -> Div {
        let mut automation_items = items.iter().filter_map(automation_item).peekable();
        if automation_items.peek().is_none() {
            return div();
        }
        let mut cards = div().flex().flex_col().mt(px(16.));
        for (id, arguments, metadata) in automation_items {
            let data = automation_data(arguments, metadata);
            let name = data["name"]
                .as_str()
                .filter(|name| !name.is_empty())
                .unwrap_or("Untitled scheduled task");
            let detail = crate::screens::automations::schedule_summary(
                data["rrule"].as_str().unwrap_or_default(),
            );
            let can_open = arguments["mode"] != "delete";
            let key: SharedString = format!("automation-card-{id}").into();
            let card = crate::screens::automations::task_card(
                name.to_owned().into(),
                detail.into(),
                can_open,
                t,
            )
            .id(key.clone())
            .mt(px(12.));
            let card = if can_open {
                let focus = crate::focus::handle(key, 0, window, cx);
                let click_id = id.to_owned();
                let key_id = click_id.clone();
                crate::focus::ring(
                    card.role(gpui::Role::Button)
                        .aria_label(format!("Open {name}"))
                        .cursor_pointer()
                        .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.open_automation_card(&click_id, cx)
                        }))
                        .on_key_down(cx.listener(
                            move |this, event: &gpui::KeyDownEvent, _, cx| {
                                if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                                    this.open_automation_card(&key_id, cx);
                                    cx.stop_propagation();
                                }
                            },
                        )),
                    &focus,
                    px(20.),
                    px(2.),
                    cx,
                )
            } else {
                card
            };
            cards = cards.child(card);
        }
        cards
    }
}

pub(crate) fn cache_automation_result(item: &mut ThreadItem) {
    let ThreadItem::McpToolCall {
        server,
        tool,
        result: Some(result),
        metadata,
        ..
    } = item
    else {
        return;
    };
    if server != "codex_app" || tool != "automation_update" {
        return;
    }
    let Some(content) = result["content"].as_array() else {
        return;
    };
    for content in content {
        let Some(text) = content["text"].as_str() else {
            continue;
        };
        if let Ok(values) = serde_json::from_str::<serde_json::Map<String, Value>>(text) {
            for (key, value) in values {
                metadata.entry(key).or_insert(value);
            }
        }
    }
}

pub(crate) fn automation_item(
    item: &ThreadItem,
) -> Option<(&str, &Value, &serde_json::Map<String, Value>)> {
    match item {
        ThreadItem::McpToolCall {
            id,
            server,
            tool,
            status,
            arguments,
            metadata,
            ..
        } if server == "codex_app" && tool == "automation_update" && status == "completed" => {
            Some((id, arguments, metadata))
        }
        _ => None,
    }
}

fn automation_data<'a>(
    arguments: &'a Value,
    metadata: &'a serde_json::Map<String, Value>,
) -> &'a Value {
    metadata.get("automation").unwrap_or(arguments)
}
