use super::*;
use serde_json::{Value, json};
use std::cell::RefCell;
use std::sync::Arc;
use std::sync::LazyLock;

#[derive(Clone, Debug)]
pub struct ComputerUseApproval {
    pub app: String,
    pub display_name: String,
    pub connector_name: String,
    pub subtitle: Option<String>,
    pub high_risk: bool,
    pub allow_session: bool,
    pub allow_always: bool,
}

#[derive(Clone, Copy, Debug)]
pub enum ComputerUseChoice {
    Once,
    Conversation,
    Always,
    Decline,
    Cancel,
}

impl ComputerUseApproval {
    pub fn response(&self, choice: ComputerUseChoice) -> Option<Value> {
        match choice {
            ComputerUseChoice::Once => Some(json!({"action": "accept", "content": {}})),
            ComputerUseChoice::Conversation if self.allow_session => Some(json!({
                "action": "accept", "content": {}, "_meta": {"persist": "session"}
            })),
            ComputerUseChoice::Always if self.allow_always => Some(json!({
                "action": "accept", "content": {}, "_meta": {"persist": "always"}
            })),
            ComputerUseChoice::Decline => Some(json!({"action": "decline"})),
            ComputerUseChoice::Cancel => Some(json!({"action": "cancel"})),
            _ => None,
        }
    }
}

const DISCLOSURE: &str = "Computer Use lets ChatGPT use apps on your computer. It may take screenshots of app contents while working. You can choose which apps ChatGPT can access, stop actions at any time, and control whether screenshots are used for training.";

pub(super) type IconCache =
    Rc<RefCell<Option<(Bounds<Pixels>, Hsla, f32, Arc<gpui::RenderImage>)>>>;

const ICON_BYTES: &[u8] = include_bytes!("../../../../assets/ui/computer-use.png");

#[cfg(not(target_os = "macos"))]
static ICON: LazyLock<Arc<gpui::Image>> = LazyLock::new(|| {
    Arc::new(gpui::Image::from_bytes(
        gpui::ImageFormat::Png,
        ICON_BYTES.to_vec(),
    ))
});

#[cfg(target_os = "macos")]
static ICON_PIXELS: LazyLock<image::RgbaImage> = LazyLock::new(|| {
    let mut pixels = image::load_from_memory(ICON_BYTES)
        .expect("Computer Use icon PNG")
        .to_rgba8();
    shapes::premultiply(&mut pixels);
    pixels
});

#[cfg(target_os = "macos")]
fn app_icon(cache: IconCache, page: Hsla) -> AnyElement {
    gpui::canvas(
        |_, _, _| (),
        move |bounds, (), window, _| {
            let mut cache = cache.borrow_mut();
            let scale = window.scale_factor();
            let image = if let Some((rect, color, cached_scale, image)) = &*cache
                && *rect == bounds
                && *color == page
                && *cached_scale == scale
            {
                image.clone()
            } else {
                let snap = |v: Pixels| px((f32::from(v) * scale).round() / scale);
                let destination = Bounds::new(
                    point(snap(bounds.left()), snap(bounds.top())),
                    size(snap(bounds.size.width), snap(bounds.size.height)),
                );
                let image = Arc::new(
                    shapes::raster_images(
                        &[(&ICON_PIXELS, destination)],
                        bounds,
                        Corners::all(px(3.)),
                        shapes::Corner::Round,
                        shapes::ImageComposition::Composited(page),
                        window,
                    )
                    .expect("macOS image rasterizer"),
                );
                *cache = Some((bounds, page, scale, image.clone()));
                image
            };
            drop(cache);
            window
                .paint_image(bounds, bounds, Corners::default(), image, 0, false)
                .ok();
        },
    )
    .size(px(18.))
    .into_any_element()
}

#[cfg(not(target_os = "macos"))]
fn app_icon(_: IconCache, _: Hsla) -> AnyElement {
    gpui::img(ICON.clone())
        .size(px(18.))
        .rounded(px(3.))
        .into_any_element()
}

impl Approval {
    pub fn computer_use(
        id: Value,
        thread: String,
        request: ComputerUseApproval,
        show_disclosure: bool,
    ) -> Self {
        let reason = match (show_disclosure, request.subtitle.as_deref()) {
            (true, Some(subtitle)) => Some(format!("{DISCLOSURE}\n\n{subtitle}")),
            (true, None) => Some(DISCLOSURE.to_owned()),
            (false, subtitle) => subtitle.map(str::to_owned),
        };
        Self {
            title: format!("Allow ChatGPT to use {}?", request.display_name).into(),
            detail: None,
            reason,
            scoped_label: "Always allow".into(),
            decisions: None,
            permissions: None,
            form: None,
            computer_use: Some(request),
            identity: Rc::new(RequestIdentity { id, thread }),
        }
    }
}

impl Composer {
    pub(super) fn computer_use_card(
        &self,
        approval: &Approval,
        t: &Theme,
        cx: &mut Context<Self>,
    ) -> Div {
        let request = approval.computer_use.as_ref().unwrap();
        let narrow = self.footer_width < 448.;
        let entity = cx.entity();
        let header = div()
            .flex()
            .items_center()
            .gap(px(8.))
            .child(if request.high_risk {
                // The shield retains this space but paints outside the rounded clip.
                div().size(px(16.)).into_any_element()
            } else {
                app_icon(self.computer_use_icon.clone(), t.surface_elevated_secondary)
            })
            .child(
                text(
                    if request.high_risk {
                        "Elevated Risk".to_owned()
                    } else {
                        request.connector_name.clone()
                    },
                    13.,
                    20.,
                    if request.high_risk { 500. } else { 400. },
                    if request.high_risk {
                        t.text_warning
                    } else {
                        t.text_secondary
                    },
                )
                .top_layer(),
            );
        let body = div()
            .px(px(16.))
            .pt(px(16.))
            .pb(px(12.))
            .flex()
            .flex_col()
            .gap(px(8.))
            .cc_tiles()
            .child(header)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .child(
                        text(approval.title.clone(), 14., 20., 500., t.text)
                            .whitespace_normal()
                            .top_layer(),
                    )
                    .when_some(approval.reason.clone(), |d, reason| {
                        d.child(
                            text(reason, 13., 19.5, 430., t.text_tertiary)
                                .whitespace_normal()
                                .top_layer(),
                        )
                    }),
            );
        let actions = div()
            .flex()
            .gap(px(8.))
            .when(narrow, |d| d.flex_col().w_full())
            .child(
                self.request_button(
                    "computer-use-deny",
                    "Deny".into(),
                    ApprovalDecision::Decline,
                    false,
                    t,
                    cx,
                )
                .when(narrow, |d| d.w_full()),
            )
            .child(
                self.request_button(
                    "computer-use-allow",
                    if request.allow_session {
                        "Allow this conversation"
                    } else {
                        "Allow once"
                    }
                    .into(),
                    ApprovalDecision::Accept,
                    true,
                    t,
                    cx,
                )
                .when(narrow, |d| d.w_full()),
            );
        let footer = div()
            .px(px(16.))
            .pb(px(16.))
            .pt(px(8.))
            .flex()
            .items_center()
            .justify_end()
            .gap(px(8.))
            .when(narrow, |d| d.flex_col())
            .when(request.allow_always, |d| {
                d.child(
                    self.request_button(
                        "computer-use-always",
                        "Always allow".into(),
                        ApprovalDecision::AcceptForSession,
                        false,
                        t,
                        cx,
                    )
                    .when(narrow, |d| d.w_full()),
                )
            })
            .when(!narrow, |d| d.child(div().flex_1()))
            .child(actions);
        div()
            .relative()
            .p(px(1.))
            .flex()
            .flex_col()
            .child(shapes::decoration(shapes::Decoration {
                radius: Corners::all(px(25.)),
                corner: shapes::Corner::Squircle,
                fill: Some(t.surface_elevated_secondary),
                border: Some((px(1.), t.border)),
                ..Default::default()
            }))
            .child(
                gpui::canvas(
                    move |bounds, window, cx| {
                        let width = f32::from(bounds.size.width);
                        let changed = entity.update(cx, |this, _| {
                            let was_narrow = this.footer_width < 448.;
                            this.footer_width = width;
                            was_narrow != (width < 448.)
                        });
                        if changed {
                            let entity = entity.clone();
                            window.defer(cx, move |_, cx| entity.update(cx, |_, cx| cx.notify()));
                        }
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .size_full(),
            )
            .child(
                div()
                    .relative()
                    .flex()
                    .flex_col()
                    .child(crate::rounded_clip(
                        px(24.),
                        shapes::Corner::Squircle,
                        div().flex().flex_col().child(body).child(footer),
                    ))
                    .when(request.high_risk, |d| {
                        d.child(
                            div()
                                .absolute()
                                .top_0()
                                .left_0()
                                .size_full()
                                .cc_tiles()
                                .child(
                                    icon("computer-use-risk", 16., t.text_warning)
                                        .absolute()
                                        .left(px(16.))
                                        .top(px(18.)),
                                ),
                        )
                    }),
            )
    }
}
