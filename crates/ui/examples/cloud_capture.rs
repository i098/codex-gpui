use gpui::{
    AnyView, AppContext as _, Entity, HeadlessAppContext, ParentElement, Render, Styled, Window,
    div, px, size,
};
use serde_json::Value;
use std::sync::Arc;
use ui::screens::{
    cloud::{Access, CloudModal, CloudPage, Operation, Request},
    settings,
};

struct Root {
    shell: AnyView,
    modal: Entity<CloudModal>,
}
impl Render for Root {
    fn render(&mut self, _: &mut Window, _: &mut gpui::Context<Self>) -> impl gpui::IntoElement {
        div()
            .size_full()
            .relative()
            .child(self.shell.clone())
            .child(self.modal.clone())
    }
}

fn main() -> anyhow::Result<()> {
    std::panic::set_hook(Box::new(|info| {
        eprintln!("{info}");
        std::process::exit(101);
    }));
    let output = std::env::args().nth(1).expect("output PNG path");
    let mode = std::env::args().nth(2);
    let legacy = mode.as_deref() == Some("legacy-editor");
    let create = mode.as_deref() == Some("current-create");
    let mut cx = HeadlessAppContext::with_platform(
        textsys::text_system(),
        Arc::new(theme::Assets),
        gpui_platform::current_headless_renderer,
    );
    cx.update(|cx| {
        cx.set_global(theme::Theme::dark());
        theme::load_fonts(cx);
        ui::composer::bind_keys(cx);
    });
    let window = cx.open_window(size(px(1280.), px(832.)), move |_, cx| {
        let config: bridge::types::ConfigReadResponse =
            ui::fixture::response("readback.jsonl", "config/read", 0);
        let skills: Value = ui::fixture::response("readback.jsonl", "skills/list", 0);
        let section = if legacy {
            "Legacy Codex Cloud"
        } else {
            "Codex Cloud"
        };
        let (shell, settings) = settings::shell(&config.config, &skills, Some(section), cx);
        shell.update(cx, |shell, _| shell.can_go_back = true);
        let cloud = cx.new(CloudPage::new);
        cloud.update(cx, |cloud, cx| {
            cloud.set_access(
                Ok(Access {
                    cloud: true,
                    legacy: true,
                    ..Default::default()
                }),
                cx,
            )
        });
        settings.update(cx, |settings, cx| settings.set_cloud(cloud.clone(), cx));
        cloud.update(cx, |cloud, cx| {
            if legacy {
                cloud.open_path("/settings/cloud-environments/new", cx);
                cloud.apply(
                    &Request {
                        serial: 2,
                        operation: Operation::Machines,
                    },
                    Err("Unable to load container images".into()),
                    cx,
                );
            } else {
                cloud.apply(
                    &Request {
                        serial: 1,
                        operation: Operation::Configs,
                    },
                    Err("Couldn’t load saved environments".into()),
                    cx,
                );
                if create {
                    cloud.open_path("/cloud-environments/new", cx);
                    cloud.apply(
                        &Request {
                            serial: 1,
                            operation: Operation::Connections,
                        },
                        Err("GitHub connections couldn’t load".into()),
                        cx,
                    );
                }
            }
        });
        let modal = cx.new(|cx| CloudModal::new(cloud, cx));
        cx.new(|_| Root {
            shell: shell.into(),
            modal,
        })
    })?;
    cx.run_until_parked();
    if create {
        println!(
            "dialog bounds: {:?}",
            cx.debug_bounds(window.into(), "cloud-create-dialog")?
        );
        println!(
            "form bounds: {:?}",
            cx.debug_bounds(window.into(), "cloud-create-form")?
        );
    }
    cx.capture_screenshot(window.into())?.save(&output)?;
    println!("{output}");
    Ok(())
}
