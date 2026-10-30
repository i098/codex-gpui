mod common;

use anyhow::{Result, bail};
use common::Fx;
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};
use ui::screens::home::{CHAT_HEADLINES, WORK_HEADLINE};

/// Serves `GET /conversations` in the desktop fetch wire format; `fail` answers 500 instead.
struct ConversationsHost {
    fail: Mutex<bool>,
    urls: Mutex<Vec<String>>,
}

impl app::host::Host for ConversationsHost {
    fn fetch(&self, request: &app::host::Request) -> Result<Value> {
        let app::host::Request::Http(http) = request else {
            bail!("unexpected host request: {request:?}");
        };
        let url = http["url"].as_str().unwrap_or_default().to_owned();
        if !url.starts_with("/conversations?") {
            bail!("unexpected host request: {url}");
        }
        self.urls.lock().unwrap().push(url);
        let (status, body) = if *self.fail.lock().unwrap() {
            (500, json!({"detail": "error"}))
        } else {
            (
                200,
                json!({"items": [
                {"id": "c-1", "title": "Trip plan"},
                {"id": "c-2", "title": ""},
            ], "total": 2, "limit": 20, "offset": 0}),
            )
        };
        Ok(json!({"value": {"response": {"__response": {
            "status": status,
            "statusText": "",
            "headers": [["content-type", "application/json"]],
            "body": body.to_string(),
        }}}}))
    }

    fn identity(&self) -> Option<app::host::Identity> {
        Some(app::host::Identity {
            account_id: "00000000-0000-4000-8000-000000000000".into(),
            user_id: "user-000000000000000000000000".into(),
            device_id: "device".into(),
        })
    }

    fn projects(&self) -> Result<Vec<ui::screens::sidebar::Project>> {
        Ok(Vec::new())
    }
}

fn surface(app: &Fx) -> (usize, Option<&'static str>, String) {
    app.read(|root, cx| {
        let home = root.home().read(cx);
        (
            root.sidebar().read(cx).mode(),
            home.headline,
            home.composer.read(cx).placeholder().to_string(),
        )
    })
}

/// Whether Recents lists this ChatGPT conversation.
fn shows(app: &Fx, title: &str) -> bool {
    app.read(|root, cx| {
        let sidebar = root.sidebar().read(cx);
        sidebar.chatgpt_surface()
            && sidebar
                .chatgpt_chats()
                .iter()
                .any(|c| c.title.as_ref() == title)
    })
}

#[test]
fn mode_switch_changes_sidebar_and_home() -> Result<()> {
    let mut app = Fx::new()?;
    let host = Arc::new(ConversationsHost {
        fail: Mutex::new(false),
        urls: Mutex::new(Vec::new()),
    });
    app.root
        .update(&mut app.cx, |root, cx| root.set_host(host.clone(), cx));
    assert_eq!(surface(&app), (1, None, "Do anything".into()));
    app.capture_evidence("CGX_MODES_SHOTS", "codex-home")?;

    app.keys("ctrl-1")?;
    app.wait("ChatGPT rows in Recents", |app| shows(app, "Trip plan"))?;
    app.capture_evidence("CGX_MODES_SHOTS", "chatgpt-home")?;
    let titles: Vec<_> = app.read(|root, cx| {
        let sidebar = root.sidebar().read(cx);
        sidebar
            .chatgpt_chats()
            .iter()
            .map(|c| (c.title.to_string(), c.url.clone()))
            .collect()
    });
    assert_eq!(
        titles,
        [
            ("Trip plan".into(), "https://chatgpt.com/c/c-1".to_owned()),
            ("New chat".into(), "https://chatgpt.com/c/c-2".to_owned()),
        ],
        "an untitled conversation shows `New chat`"
    );
    let (_, headline, placeholder) = surface(&app);
    assert!(CHAT_HEADLINES.contains(&headline.unwrap()));
    assert_eq!(placeholder, "Ask ChatGPT");
    assert_eq!(
        host.urls.lock().unwrap()[0],
        "/conversations?exclude_conversation_origin=tpp&expand=false&hide_snorlax=false\
         &is_archived=false&is_starred=false&limit=20&order=updated&offset=0",
        "the original renderer's recorded request"
    );

    // A failed page keeps the shown rows, without a message, like the original.
    *host.fail.lock().unwrap() = true;
    app.keys("ctrl-2")?;
    app.wait("Work page request", |_| {
        host.urls.lock().unwrap().len() == 2
    })?;
    app.cx.run_until_parked();
    assert_eq!(
        surface(&app),
        (2, Some(WORK_HEADLINE), "Work with ChatGPT".into())
    );
    assert!(shows(&app, "Trip plan"));
    app.capture_evidence("CGX_MODES_SHOTS", "work-home")?;

    app.keys("ctrl-3")?;
    assert_eq!(surface(&app), (1, None, "Do anything".into()));
    assert!(!shows(&app, "Trip plan"));
    Ok(())
}
