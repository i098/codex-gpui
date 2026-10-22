use gpui::{AnyView, AppContext as _, HeadlessAppContext, Render, Window, px, size};
use serde_json::Value;
use std::sync::Arc;
use ui::screens::{
    cloud::{Access, CloudPage, Operation, Request},
    settings,
};

struct Root(AnyView);
impl Render for Root {
    fn render(&mut self, _: &mut Window, _: &mut gpui::Context<Self>) -> impl gpui::IntoElement {
        self.0.clone()
    }
}

fn main() -> anyhow::Result<()> {
    std::panic::set_hook(Box::new(|info| {
        eprintln!("{info}");
        std::process::exit(101);
    }));
    let output = std::env::args().nth(1).expect("output PNG path");
    let legacy = std::env::args().nth(2).as_deref() == Some("legacy-editor");
    let mut cx = HeadlessAppContext::with_platform(
        textsys::text_system(),
        Arc::new(theme::Assets),
        gpui_platform::current_headless_renderer,
    );
    cx.update(|cx| {
        cx.set_global(theme::Theme::dark());
        theme::load_fonts(cx);
        ui::composer::bind_keys(cx);
        if legacy {
            let font = gpui::Font {
                weight: gpui::FontWeight(430.),
                ..gpui::font(ui::SANS)
            };
            let id = cx.text_system().resolve_font(&font);
            eprintln!(
                "Inline font metrics: ascent={} descent={} x_height={}",
                f32::from(cx.text_system().ascent(id, px(12.))),
                f32::from(cx.text_system().descent(id, px(12.))),
                f32::from(cx.text_system().x_height(id, px(12.))),
            );
        }
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
            }
        });
        cx.new(|_| Root(shell.into()))
    })?;
    cx.run_until_parked();
    cx.capture_screenshot(window.into())?.save(&output)?;
    println!("{output}");
    Ok(())
}
