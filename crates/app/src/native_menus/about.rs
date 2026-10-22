use gpui::{
    App, AppContext as _, Bounds, Context, FocusHandle, Global, Image, InteractiveElement as _,
    IntoElement, ParentElement as _, Render, Styled as _, TitlebarOptions, Window, WindowBounds,
    WindowHandle, WindowOptions, div, img, px, rgb, size,
};
use std::sync::Arc;

#[derive(Default)]
struct AboutWindow(Option<WindowHandle<About>>);
impl Global for AboutWindow {}

struct About {
    focus: FocusHandle,
    icon: Option<Arc<Image>>,
}

pub(super) fn open(cx: &mut App) {
    if let Some(handle) = cx.default_global::<AboutWindow>().0
        && handle
            .update(cx, |_, window, _| window.activate_window())
            .is_ok()
    {
        return;
    }
    let bounds = Bounds::centered(None, size(px(380.), px(256.)), cx);
    match cx.open_window(
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            titlebar: Some(TitlebarOptions {
                title: Some("About Codex".into()),
                appears_transparent: false,
                ..Default::default()
            }),
            is_resizable: false,
            is_minimizable: false,
            ..Default::default()
        },
        |window, cx| {
            let about = cx.new(|cx| About {
                focus: cx.focus_handle(),
                icon: application_icon(),
            });
            let focus = about.read(cx).focus.clone();
            window.focus(&focus, cx);
            about
        },
    ) {
        Ok(handle) => cx.default_global::<AboutWindow>().0 = Some(handle),
        Err(error) => eprintln!("codex-gpui: cannot open About: {error:#}"),
    }
}

impl Render for About {
    fn render(&mut self, window: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let dark = matches!(
            window.appearance(),
            gpui::WindowAppearance::Dark | gpui::WindowAppearance::VibrantDark
        );
        let muted = if dark { rgb(0xc8c8c8) } else { rgb(0x4f4f4f) };
        div()
            .size_full()
            .p(px(32.))
            .flex()
            .flex_col()
            .items_center()
            .font_family(ui::SANS)
            .text_size(px(12.))
            .line_height(px(17.4))
            .text_center()
            .bg(if dark { rgb(0x202020) } else { rgb(0xffffff) })
            .text_color(if dark { rgb(0xf5f5f5) } else { rgb(0x1f1f1f) })
            .track_focus(&self.focus)
            .on_key_down(|event, window, cx| {
                if event.keystroke.key == "escape" {
                    window.remove_window();
                    cx.stop_propagation();
                }
            })
            .on_action(|_: &crate::CloseWindow, window, _| window.remove_window())
            .children(
                self.icon
                    .as_ref()
                    .map(|icon| img(icon.clone()).size(px(72.)).mb(px(14.))),
            )
            .child(
                div()
                    .mb(px(18.))
                    .text_size(px(17.))
                    .line_height(px(21.25))
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .child("Codex"),
            )
            .child(div().w_full().text_color(muted).child(concat!(
                "Powered by Codex & GPUI\nVersion ",
                env!("CARGO_PKG_VERSION")
            )))
            .child(div().mt(px(14.)).text_color(muted).child("© OpenAI"))
    }
}

#[cfg(target_os = "macos")]
fn application_icon() -> Option<Arc<Image>> {
    use objc2::{AnyThread as _, rc::autoreleasepool};
    use objc2_app_kit::{NSBitmapImageFileType, NSBitmapImageRep, NSImage};
    use objc2_foundation::{NSData, NSDictionary};

    autoreleasepool(|_| {
        let bytes = theme::Assets::get("app/AppIcon.icns")?;
        let image = NSImage::initWithData(NSImage::alloc(), &NSData::with_bytes(&bytes.data))?;
        let tiff = image.TIFFRepresentation()?;
        let bitmap = NSBitmapImageRep::imageRepWithData(&tiff)?;
        let png = unsafe {
            bitmap.representationUsingType_properties(
                NSBitmapImageFileType::PNG,
                &NSDictionary::new(),
            )
        }?;
        Some(Arc::new(Image::from_bytes(
            gpui::ImageFormat::Png,
            unsafe { png.as_bytes_unchecked().to_vec() },
        )))
    })
}

#[cfg(not(target_os = "macos"))]
fn application_icon() -> Option<Arc<Image>> {
    None
}
