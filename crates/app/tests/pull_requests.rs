mod common;

use anyhow::Result;
use app::{Route, host::ReplayHost};
use common::Fx;
use gpui::Focusable as _;
use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;
use ui::screens::pull_requests::{Availability, Event};

#[test]
fn missing_host_response_keeps_retry_and_chat_available() -> Result<()> {
    let mut fx = Fx::new()?;
    let host = Arc::new(ReplayHost::from_recording(r#"{"exchanges":[]}"#)?);
    fx.root
        .update(&mut fx.cx, |root, cx| root.set_host(host, cx));
    fx.cx.update_window(fx.window, |_, window, cx| {
        window.dispatch_action(Box::new(ui::OpenPullRequests), cx);
    })?;
    fx.wait("GitHub access failure", |fx| {
        fx.read(|root, cx| {
            root.pull_requests_page().is_some_and(|page| {
                matches!(page.read(cx).availability, Availability::Unavailable(_))
            })
        })
    })?;
    assert_eq!(fx.route(), Route::PullRequests(None));
    capture(&mut fx, "access-error")?;
    let page = fx.read(|root, _| root.pull_requests_page().unwrap().clone());
    let generation = fx.read(|_, cx| page.read(cx).generation);
    page.update(&mut fx.cx, |_, cx| cx.emit(Event::Retry));
    fx.wait("GitHub retry failure", |fx| {
        fx.read(|_, cx| {
            page.read(cx).generation > generation
                && matches!(page.read(cx).availability, Availability::Unavailable(_))
        })
    })?;
    assert_eq!(fx.route(), Route::PullRequests(None));
    page.update(&mut fx.cx, |_, cx| {
        cx.emit(Event::AskCodex(
            ui::screens::pull_requests::INSTALL_PROMPT.into(),
        ))
    });
    fx.wait("installation draft", |fx| fx.route() == Route::Home)?;
    assert_eq!(
        fx.composer_text()?,
        ui::screens::pull_requests::INSTALL_PROMPT
    );
    capture(&mut fx, "install-draft")?;
    Ok(())
}

#[test]
fn original_cursor_page_preserves_the_existing_rows() -> Result<()> {
    use ui::screens::pull_requests::model::View;
    let mut fx = Fx::new()?;
    let recording =
        std::fs::read_to_string(format!("{}/fixtures/pull-requests.json", common::ROOT))?;
    let host = Arc::new(ReplayHost::from_recording(&recording)?);
    fx.root
        .update(&mut fx.cx, |root, cx| root.set_host(host, cx));
    fx.cx.update_window(fx.window, |_, window, cx| {
        window.dispatch_action(Box::new(ui::OpenPullRequests), cx);
    })?;
    fx.wait("recorded GitHub identity", |fx| {
        fx.read(|root, cx| {
            root.pull_requests_page()
                .is_some_and(|page| page.read(cx).account().is_some())
        })
    })?;
    let page = fx.read(|root, _| root.pull_requests_page().unwrap().clone());
    fx.cx.update_window(fx.window, |_, window, cx| {
        page.focus_handle(cx).focus(window, cx)
    })?;
    fx.cx.run_until_parked();
    fx.type_text("repo:i098/codex-gpui")?;
    fx.advance(Duration::from_millis(250))?;
    let loaded = fx.wait("recorded open queries", |fx| {
        fx.read(|_, cx| {
            let page = page.read(cx);
            !page.streams.is_empty()
                && page
                    .streams
                    .iter()
                    .all(|stream| stream.loaded && !stream.loading && stream.error.is_none())
        })
    });
    if let Err(error) = loaded {
        let (query, states) = fx.read(|_, cx| {
            let page = page.read(cx);
            (
                page.query.clone(),
                page.streams
                    .iter()
                    .map(|stream| {
                        (
                            stream.key,
                            stream.loaded,
                            stream.loading,
                            stream.error.clone(),
                        )
                    })
                    .collect::<Vec<_>>(),
            )
        });
        capture(&mut fx, "open-query-error")?;
        anyhow::bail!("{error:#}; query={query:?}; states={states:?}");
    }
    capture(&mut fx, "open-all")?;
    page.update(&mut fx.cx, |page, cx| {
        page.view = View::Authored;
        page.lifecycle = "merged".into();
        cx.emit(Event::Query);
    });
    fx.wait("first recorded merged page", |fx| {
        fx.read(|_, cx| {
            let page = page.read(cx);
            page.streams.len() == 1
                && page.streams[0].loaded
                && !page.streams[0].loading
                && page.streams[0].cursor.is_some()
        })
    })?;
    let first_ids = fx.read(|_, cx| {
        page.read(cx).streams[0]
            .items
            .iter()
            .map(|item| item.node_id.clone())
            .collect::<Vec<_>>()
    });
    capture(&mut fx, "merged-first-page")?;
    page.update(&mut fx.cx, |_, cx| cx.emit(Event::More));
    fx.wait("second recorded merged page", |fx| {
        fx.read(|_, cx| {
            let stream = &page.read(cx).streams[0];
            !stream.loading
                && stream
                    .items
                    .iter()
                    .any(|item| !first_ids.contains(&item.node_id))
        })
    })?;
    let all_ids = fx.read(|_, cx| {
        page.read(cx).streams[0]
            .items
            .iter()
            .map(|item| item.node_id.clone())
            .collect::<Vec<_>>()
    });
    assert_eq!(&all_ids[..first_ids.len()], first_ids);
    assert_eq!(all_ids.iter().collect::<HashSet<_>>().len(), all_ids.len());
    capture(&mut fx, "merged-second-page")?;
    let selected = fx.read(|_, cx| page.read(cx).streams[0].items[0].pull_request.clone());
    page.update(&mut fx.cx, |_, cx| cx.emit(Event::Select(Some(selected))));
    fx.wait("recorded detail response", |fx| {
        fx.read(|_, cx| matches!(page.read(cx).detail, Some(Ok(_))))
    })?;
    capture(&mut fx, "merged-detail")?;
    Ok(())
}

fn capture(fx: &mut Fx, name: &str) -> Result<()> {
    if let Some(directory) = std::env::var_os("CGX_PULL_REQUEST_EVIDENCE") {
        std::fs::create_dir_all(&directory)?;
        fx.shot()?
            .save(std::path::Path::new(&directory).join(format!("{name}.png")))?;
    }
    Ok(())
}
