pub mod model;

use crate::screens::diff::panel::DiffPanel;
use crate::screens::thread::view::Clock;
use crate::{EmptySidebar, Shell, composer::Composer, icon::icon, text};
use base64::Engine as _;
use gpui::{
    App, AppContext as _, ClipboardItem, Context, Div, Entity, EventEmitter, FocusHandle,
    Focusable, InteractiveElement, IntoElement, ParentElement, Render, ScrollHandle, SharedString,
    Stateful, StatefulInteractiveElement, Styled, StyledImage, Window, div, point,
    prelude::FluentBuilder as _, px,
};
use image::ImageDecoder as _;
use model::{Account, Filters, Identity, Repository, SearchPage, Stream, View};
use serde_json::Value;
use std::{
    cell::RefCell,
    collections::{HashMap, VecDeque},
    sync::Arc,
    time::Duration,
};
use theme::ActiveTheme as _;

fn review_icon(name: &'static str, side: f32, color: gpui::Hsla) -> impl IntoElement {
    let (key, source): (&'static str, &'static [u8]) = match name { "search" => ("pull-request-toolbar-search", br##"<svg width="16" height="16" viewBox="0 0 16 16" fill="none" xmlns="http://www.w3.org/2000/svg"><path fill-rule="evenodd" clip-rule="evenodd" d="M7.33057 1.98535C10.2484 1.98535 12.6136 4.3508 12.6138 7.26855C12.6138 8.58031 12.1346 9.77942 11.3433 10.7031L13.9897 13.3496C14.1655 13.5253 14.1655 13.8106 13.9897 13.9863C13.814 14.1621 13.5288 14.1621 13.353 13.9863L10.7017 11.335C9.78678 12.0942 8.61243 12.5518 7.33057 12.5518C4.41281 12.5516 2.04736 10.1864 2.04736 7.26855C2.04754 4.35091 4.41292 1.98553 7.33057 1.98535ZM7.33057 2.88574C4.90998 2.88592 2.94793 4.84796 2.94775 7.26855C2.94775 9.68929 4.90987 11.6522 7.33057 11.6523C9.75141 11.6523 11.7144 9.6894 11.7144 7.26855C11.7142 4.84786 9.75131 2.88574 7.33057 2.88574Z" fill="currentColor"/></svg>"##), "clear" => ("pull-request-toolbar-clear", br##"<svg height="16" viewBox="0 0 16 16" width="16" xmlns="http://www.w3.org/2000/svg"><path fill-rule="evenodd" clip-rule="evenodd" d="M8 1.25C11.7279 1.25 14.75 4.27208 14.75 8C14.75 11.7279 11.7279 14.75 8 14.75C4.27208 14.75 1.25 11.7279 1.25 8C1.25 4.27208 4.27208 1.25 8 1.25ZM10.5332 5.46582C10.2403 5.17333 9.76544 5.17317 9.47266 5.46582L8 6.93848L6.52734 5.46582C6.23456 5.17317 5.75968 5.17333 5.4668 5.46582C5.1739 5.75871 5.1739 6.23445 5.4668 6.52734L6.93848 7.99902L5.46582 9.47363C5.17319 9.76655 5.17302 10.2414 5.46582 10.5342C5.75864 10.8269 6.23349 10.8268 6.52637 10.5342L8 9.06055L9.47363 10.5342C9.76651 10.8268 10.2414 10.8269 10.5342 10.5342C10.827 10.2414 10.8268 9.76655 10.5342 9.47363L9.06055 7.99902L10.5332 6.52734C10.8261 6.23445 10.8261 5.75871 10.5332 5.46582Z" fill="currentColor"/></svg>"##), "filter" => ("pull-request-toolbar-filter", br##"<svg width="20" height="20" viewBox="0 0 20 20" fill="none" xmlns="http://www.w3.org/2000/svg"><path d="M16.0013 5.00024C16.0013 4.44715 15.5534 3.99847 15.0003 3.99829H5.00031C4.4471 3.99829 3.99835 4.44704 3.99835 5.00024V5.97583C3.99835 6.24147 4.10349 6.49697 4.29132 6.68481L7.48175 9.87524L7.63702 10.0461C7.97691 10.461 8.16534 10.9826 8.16534 11.5237V15.1663C8.16534 15.3015 8.24656 15.4238 8.3714 15.4758L11.3714 16.7258C11.592 16.8176 11.8353 16.6553 11.8353 16.4163V11.5237C11.8353 10.9054 12.0808 10.3125 12.5179 9.87524L15.7083 6.68481L15.7747 6.6106C15.9206 6.43239 16.0013 6.20818 16.0013 5.97583V5.00024ZM17.3314 5.97583C17.3314 6.51694 17.144 7.03943 16.804 7.45435L16.6487 7.62524L13.4583 10.8157C13.2706 11.0035 13.1653 11.2581 13.1653 11.5237V16.4163C13.1653 17.6044 11.9564 18.4103 10.8597 17.9534L7.85968 16.7034C7.23925 16.4448 6.83527 15.8384 6.83527 15.1663V11.5237C6.83527 11.258 6.72917 11.0035 6.54132 10.8157L3.35089 7.62524C2.91364 7.18798 2.66827 6.59421 2.66827 5.97583V5.00024C2.66827 3.7125 3.71256 2.66821 5.00031 2.66821H15.0003C16.2879 2.66839 17.3314 3.71261 17.3314 5.00024V5.97583Z" fill="currentColor"/></svg>"##), "filter-active" => ("pull-request-toolbar-filter-active", br##"<svg width="20" height="20" viewBox="0 0 20 20" fill="none" xmlns="http://www.w3.org/2000/svg"><path d="M20 5A4 4 0 1 1 12 5A4 4 0 0 1 20 5Z" fill="#0285FF"/><path fill="currentColor" d="M12.5 14.005a.665.665 0 0 1 0 1.33h-5a.665.665 0 0 1 0-1.33h5Zm-.646-4.67a5.996 5.996 0 0 0 2.169 1.33H5a.665.665 0 1 1 0-1.33h6.854ZM10 5c0 .34.03.671.084.995H2.5a.665.665 0 0 1 0-1.33h7.51c-.006.11-.01.223-.01.335Z"/></svg>"##), "fullscreen" => ("pull-request-fullscreen", br##"<svg height="16" viewBox="0 0 16 16" width="16" xmlns="http://www.w3.org/2000/svg"><path d="M6.31909 8.95312C6.52412 8.74814 6.85627 8.74811 7.06128 8.95312C7.26625 9.15814 7.26625 9.4903 7.06128 9.69531L4.61108 12.1455H6.66284C6.95279 12.1455 7.18823 12.3809 7.18823 12.6709C7.18797 12.9606 6.95263 13.1963 6.66284 13.1963H3.4314C3.08669 13.1961 2.80687 12.9159 2.8064 12.5713V9.34082C2.8064 9.05093 3.04191 8.81552 3.33179 8.81543C3.62166 8.81552 3.8562 9.05093 3.8562 9.34082V11.416L6.31909 8.95312Z" fill="currentColor"/><path d="M12.5681 2.81055C12.9131 2.81067 13.1929 3.09062 13.1931 3.43555V6.66602C13.1931 6.95597 12.9577 7.19141 12.6677 7.19141C12.3779 7.19123 12.1433 6.95586 12.1433 6.66602V4.58984L9.68042 7.05273C9.47539 7.25774 9.14325 7.25775 8.93823 7.05273C8.7336 6.84769 8.73336 6.51544 8.93823 6.31055L11.3875 3.86133H9.33667C9.0468 3.86124 8.81128 3.62583 8.81128 3.33594C8.81128 3.04604 9.0468 2.81064 9.33667 2.81055H12.5681Z" fill="currentColor"/></svg>"##), "branch" => ("pull-request-branch", br##"<svg width="20" height="20" viewBox="0 0 20 20" fill="none" xmlns="http://www.w3.org/2000/svg"><path d="M7.2915 5A1.875 1.875 0 1 1 3.5415 5A1.875 1.875 0 1 1 7.2915 5Z" stroke="currentColor" stroke-width="1.33"/><path d="M7.2915 15A1.875 1.875 0 1 1 3.5415 15A1.875 1.875 0 1 1 7.2915 15Z" stroke="currentColor" stroke-width="1.33"/><path d="M16.4583 5A1.875 1.875 0 1 1 12.7083 5A1.875 1.875 0 1 1 16.4583 5Z" stroke="currentColor" stroke-width="1.33"/><path d="M5.4165 6.66664V13.3333" stroke="currentColor" stroke-width="1.33" stroke-linejoin="round"/><path d="M5.41658 12.5V11.6667C5.41658 10.7462 6.16278 10 7.08325 10H12.9166C13.8371 10 14.5833 9.25381 14.5833 8.33333V7.5" stroke="currentColor" stroke-width="1.33" stroke-linejoin="round"/></svg>"##), "reviewers" => ("pull-request-reviewers", br##"<svg width="20" height="20" viewBox="0 0 20 20" fill="none" xmlns="http://www.w3.org/2000/svg"><path d="M16.585 10C16.585 6.3632 13.6368 3.41504 10 3.41504C6.3632 3.41504 3.41504 6.3632 3.41504 10C3.41504 11.9528 4.26592 13.7062 5.61621 14.9121C6.6544 13.6452 8.23235 12.835 10 12.835C11.7674 12.835 13.3447 13.6454 14.3828 14.9121C15.7334 13.7062 16.585 11.9531 16.585 10ZM10 14.165C8.67626 14.165 7.49115 14.7585 6.69531 15.6953C7.66679 16.2602 8.79525 16.585 10 16.585C11.2041 16.585 12.3316 16.2597 13.3027 15.6953C12.5069 14.759 11.3233 14.1651 10 14.165ZM11.835 8.5C11.835 7.48656 11.0134 6.66504 10 6.66504C8.98656 6.66504 8.16504 7.48656 8.16504 8.5C8.16504 9.51344 8.98656 10.335 10 10.335C11.0134 10.335 11.835 9.51344 11.835 8.5ZM17.915 10C17.915 14.3713 14.3713 17.915 10 17.915C5.62867 17.915 2.08496 14.3713 2.08496 10C2.08496 5.62867 5.62867 2.08496 10 2.08496C14.3713 2.08496 17.915 5.62867 17.915 10ZM13.165 8.5C13.165 10.248 11.748 11.665 10 11.665C8.25202 11.665 6.83496 10.248 6.83496 8.5C6.83496 6.75202 8.25202 5.33496 10 5.33496C11.748 5.33496 13.165 6.75202 13.165 8.5Z" fill="currentColor"/></svg>"##), "comments" => ("pull-request-comments", br##"<svg width="20" height="20" viewBox="0 0 20 20" fill="none" xmlns="http://www.w3.org/2000/svg"><path d="M11.335 12.083C11.3347 9.97242 9.44966 8.16504 7 8.16504C4.55034 8.16504 2.66527 9.97242 2.66504 12.083C2.66504 12.8512 2.90887 13.5704 3.33691 14.1797C3.4302 14.3125 3.47218 14.4745 3.4541 14.6357C3.40535 15.0678 3.31415 15.4843 3.19922 15.8877C3.66136 15.8098 4.10651 15.6986 4.54297 15.5508L4.66699 15.5215C4.79159 15.5045 4.91938 15.5238 5.03516 15.5771C5.62294 15.8481 6.2901 16.002 7 16.002C9.44981 16.002 11.335 14.1938 11.335 12.083ZM17.335 7.91309C17.3348 5.80247 15.4497 3.99512 13 3.99512C11.5595 3.99512 10.298 4.62925 9.51465 5.58496C9.28182 5.86891 8.86214 5.9105 8.57812 5.67773C8.29409 5.44493 8.25257 5.02526 8.48535 4.74121C9.52649 3.47094 11.1693 2.66504 13 2.66504C16.0729 2.66504 18.6648 4.96138 18.665 7.91309C18.665 8.8753 18.3824 9.77408 17.8984 10.5459C17.9866 11.1153 18.1604 11.6767 18.3848 12.2568C18.4665 12.4681 18.4355 12.7068 18.3018 12.8896C18.1681 13.0723 17.9505 13.1739 17.7246 13.1602C16.8659 13.1076 16.0585 12.9617 15.2734 12.7178C15.1054 12.7861 14.9347 12.8511 14.7588 12.9043C14.4073 13.0104 14.036 12.8113 13.9297 12.46C13.8235 12.1084 14.0226 11.7372 14.374 11.6309C14.5782 11.5692 14.7758 11.4944 14.9648 11.4072L15.084 11.3652C15.2063 11.3351 15.3361 11.3399 15.457 11.3809C15.8932 11.5286 16.338 11.6399 16.7998 11.7178C16.6849 11.3144 16.5946 10.8978 16.5459 10.4658C16.5278 10.3046 16.5698 10.1426 16.6631 10.0098C17.0911 9.40048 17.335 8.68131 17.335 7.91309ZM12.665 12.083C12.665 15.0349 10.073 17.332 7 17.332C6.19184 17.332 5.42143 17.1731 4.72266 16.8887C4.04698 17.0983 3.35521 17.2365 2.62793 17.3037L2.27539 17.3301C2.04946 17.3438 1.83192 17.2422 1.69824 17.0596C1.56452 16.8767 1.53354 16.638 1.61523 16.4268L1.79297 15.9375C1.93133 15.5279 2.03737 15.1238 2.10059 14.7158C1.61678 13.9441 1.33496 13.045 1.33496 12.083C1.33519 9.13134 3.92709 6.83496 7 6.83496C10.0729 6.83496 12.6648 9.13134 12.665 12.083Z" fill="currentColor"/></svg>"##), _ => unreachable!(), };
    gpui::canvas(
        |_, _, _| {},
        move |bounds, (), window, cx| {
            if !shapes::paint_icon(source, key, bounds, false, color, None, window, cx) {
                window
                    .paint_svg(
                        bounds,
                        key.into(),
                        Some(source),
                        gpui::TransformationMatrix::default(),
                        color,
                        cx,
                    )
                    .ok();
            }
        },
    )
    .size(px(side))
    .flex_none()
}

// Chromium a49e2e3d's IDR_BROKENIMAGE at 100%; attribution is in docs/inventory.md.
/*
Embedded image resource license:
Copyright (C) 2003, 2004, 2005, 2006 Apple Computer, Inc.  All rights reserved.
Redistribution and use in source and binary forms, with or without
modification, are permitted provided that the following conditions
are met:
1. Redistributions of source code must retain the above copyright
   notice, this list of conditions and the following disclaimer.
2. Redistributions in binary form must reproduce the above copyright
   notice, this list of conditions and the following disclaimer in the
   documentation and/or other materials provided with the distribution.
THIS SOFTWARE IS PROVIDED BY APPLE COMPUTER, INC. ``AS IS'' AND ANY
EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT LIMITED TO, THE
IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR
PURPOSE ARE DISCLAIMED.  IN NO EVENT SHALL APPLE COMPUTER, INC. OR
CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL, SPECIAL,
EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO,
PROCUREMENT OF SUBSTITUTE GOODS OR SERVICES; LOSS OF USE, DATA, OR
PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY
OF LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT
(INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE
OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.
*/
fn missing_avatar() -> gpui::AnyElement {
    static IMAGE: std::sync::LazyLock<Arc<gpui::Image>> = std::sync::LazyLock::new(|| {
        let bytes = base64::engine::general_purpose::STANDARD.decode(
            "iVBORw0KGgoAAAANSUhEUgAAAA4AAAAQCAYAAAAmlE46AAABh0lEQVQoU42QSS9DURiGv7VfhKWEnyA2WFizsLciJGrowDVU55VIjE3sxFhB1ZDiVlUapYYguLRor9f5DrdRuaJv8izOOc+zORQIBFAKRFRGP+f3B6BlP/CYMecpo0NRFLBXFPt8fpzf6ThO50xJ3rzLkMduIfZ6vTi51hE5fZMYM85q6rUQ8tiXsdvtQfxKx3biFb/Hd0citNlsRXBDLpdbhmbbimcRiWu4uMvhXstL+D+4IadzDPFLHRvHL6aETzLYT2ZxcPZF6lYHNzQ66kQsnUdIfS4Jdrmh4eERqOKwdqiVBLuiIVKUIagXeaxEtQLL0UfMRKII7sSK7hnhckM0OKjgSISLew8ST2getePlqJuskDTPNWI2rBrvJFxuiByOARye57Gw+wDbklfIlYXIoH6yChObERIOCVc2ZLc7ZOhaDZpG31DDVDXNhRMcghuyWu1YT6TROF3zZ2TQEmxCNPUObqi/34q2hdZ/I+Oud60HvdY+UGdnF9rbO8ygHxS9dVss+AQqZY47NSC2iwAAAABJRU5ErkJggg==").expect("embedded browser image");
        Arc::new(gpui::Image::from_bytes(gpui::ImageFormat::Png, bytes))
    });
    if !cfg!(target_os = "macos") {
        return crate::rounded_clip(
            px(9.),
            shapes::Corner::Round,
            div()
                .size(px(18.))
                .relative()
                .child(shapes::decoration(shapes::Decoration {
                    border: Some((px(1.), gpui::rgb(0xc0c0c0).into())),
                    ..Default::default()
                }))
                .child(
                    div()
                        .absolute()
                        .left(px(1.))
                        .right(px(1.))
                        .top(px(1.))
                        .bottom(px(1.))
                        .overflow_hidden()
                        .child(
                            gpui::img(IMAGE.clone())
                                .absolute()
                                .left(px(1.))
                                .top(px(1.))
                                .size(px(16.))
                                .object_fit(gpui::ObjectFit::Fill),
                        ),
                ),
        )
        .into_any_element();
    }
    static PIXELS: std::sync::LazyLock<image::RgbaImage> = std::sync::LazyLock::new(|| {
        let mut pixels = image::load_from_memory(&IMAGE.bytes)
            .expect("embedded browser PNG")
            .to_rgba8();
        shapes::premultiply(&mut pixels);
        pixels
    });
    static WHITE: std::sync::LazyLock<image::RgbaImage> = std::sync::LazyLock::new(|| {
        image::RgbaImage::from_pixel(1, 1, image::Rgba([255, 255, 255, 255]))
    });
    static SILVER: std::sync::LazyLock<image::RgbaImage> = std::sync::LazyLock::new(|| {
        image::RgbaImage::from_pixel(1, 1, image::Rgba([192, 192, 192, 255]))
    });
    type Cached = (
        gpui::Bounds<gpui::Pixels>,
        f32,
        gpui::Hsla,
        gpui::Bounds<gpui::Pixels>,
        Arc<gpui::RenderImage>,
    );
    thread_local! {
        static CACHE: RefCell<Option<Cached>> = const { RefCell::new(None) };
    }
    gpui::canvas(
        |_, _, _| {},
        move |bounds, (), window, cx| {
            let scale = window.scale_factor();
            let page = cx.theme().surface;
            let cached = CACHE.with_borrow(|cache| {
                cache
                    .as_ref()
                    .filter(|entry| entry.0 == bounds && entry.1 == scale && entry.2 == page)
                    .map(|entry| (entry.3, entry.4.clone()))
            });
            let (at, rendered) = if let Some(cached) = cached {
                cached
            } else {
                let rect = |x, y, w, h| {
                    gpui::Bounds::new(
                        bounds.origin + point(px(x), px(y)),
                        gpui::size(px(w), px(h)),
                    )
                };
                let snapped = |b: gpui::Bounds<gpui::Pixels>| {
                    gpui::Bounds::from_corners(
                        point(
                            px((f32::from(b.left()) * scale).round() / scale),
                            px((f32::from(b.top()) * scale).round() / scale),
                        ),
                        point(
                            px((f32::from(b.right()) * scale).round() / scale),
                            px((f32::from(b.bottom()) * scale).round() / scale),
                        ),
                    )
                };
                let inner_bounds = rect(1., 1., 16., 16.);
                let inner = shapes::raster_images(
                    &[(&PIXELS, rect(2., 2., 16., 16.))],
                    inner_bounds,
                    gpui::Corners::default(),
                    shapes::Corner::Round,
                    shapes::ImageComposition::Direct(gpui::transparent_black()),
                    window,
                )
                .expect("Mac image rasterizer");
                let size = inner.size(0);
                let mut bytes = inner.as_bytes(0).expect("raster frame").to_vec();
                for pixel in bytes.chunks_exact_mut(4) {
                    pixel.swap(0, 2);
                }
                let clipped =
                    image::RgbaImage::from_raw(size.width.into(), size.height.into(), bytes)
                        .expect("raster size");
                let images: [(&image::RgbaImage, gpui::Bounds<gpui::Pixels>); 6] = [
                    (&WHITE, bounds),
                    (&SILVER, rect(0., 0., 18., 1.)),
                    (&SILVER, rect(0., 17., 18., 1.)),
                    (&SILVER, rect(0., 1., 1., 16.)),
                    (&SILVER, rect(17., 1., 1., 16.)),
                    (&clipped, snapped(inner_bounds)),
                ];
                let rendered = Arc::new(
                    shapes::raster_images(
                        &images,
                        bounds,
                        gpui::Corners::all(px(9.)),
                        shapes::Corner::Round,
                        shapes::ImageComposition::Direct(page),
                        window,
                    )
                    .expect("Mac image rasterizer"),
                );
                let at = snapped(bounds);
                CACHE.set(Some((bounds, scale, page, at, rendered.clone())));
                (at, rendered)
            };
            window
                .paint_image(at, at, gpui::Corners::default(), rendered, 0, false)
                .ok();
        },
    )
    .size(px(18.))
    .into_any_element()
}

pub const INSTALL_PROMPT: &str = "Install GitHub CLI (`gh`) on this machine, verify it works, and help me authenticate if needed so I can view and manage pull requests in Codex";

const PREVIEW_MEMORY_LIMIT: usize = 512 * 1024 * 1024;
const PREVIEW_COUNT_LIMIT: usize = 8;
const PREVIEW_IMAGE_LIMIT: usize = PREVIEW_MEMORY_LIMIT / PREVIEW_COUNT_LIMIT;

#[derive(Clone, Debug)]
pub enum Availability {
    Checking,
    Missing,
    Unavailable(String),
    Ready(Account),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DetailTab {
    Summary,
    Code,
}

#[derive(Clone)]
pub enum Event {
    Query,
    Retry,
    More,
    Select(Option<Identity>),
    AskCodex(String),
    Code,
    Media { generation: u64, url: String },
}

pub struct Page {
    pub availability: Availability,
    pub streams: Vec<Stream>,
    pub view: View,
    pub lifecycle: String,
    pub repository: Option<Repository>,
    pub selected: Option<Identity>,
    pub detail: Option<Result<Value, String>>,
    pub detail_tab: DetailTab,
    pub code: Option<Result<Entity<DiffPanel>, String>>,
    code_loading: bool,
    detail_fullscreen: bool,
    media_images: HashMap<String, Option<Result<Arc<gpui::RenderImage>, String>>>,
    media_queue: VecDeque<String>,
    media_inflight: bool,
    pub generation: u64,
    pub detail_generation: u64,
    pub query: String,
    search: Entity<Composer>,
    filter_open: bool,
    filter_submenu: Option<&'static str>,
    collapsed_detail: [bool; 3],
    scroll: ScrollHandle,
    detail_scroll: ScrollHandle,
    last_filters: Option<Filters>,
    controls: RefCell<HashMap<gpui::ElementId, FocusHandle>>,
    clock: Clock,
}

impl EventEmitter<Event> for Page {}
impl Focusable for Page {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.search.focus_handle(cx)
    }
}

pub fn shell(page: Entity<Page>, cx: &mut App) -> Entity<Shell> {
    let sidebar = cx.new(|_| EmptySidebar).into();
    cx.new(|_| {
        let mut shell = Shell::new(sidebar, page.into());
        shell.sidebar_open = false;
        shell.selected_rail = None;
        shell.chat_chrome = false;
        shell.new_tab = false;
        shell.main_backgrounds = 1;
        shell
    })
}

impl Page {
    pub fn new(clock: Clock, cx: &mut Context<Self>) -> Self {
        let search =
            cx.new(|cx| Composer::embedded_field("Search pull requests", "", px(14.), px(20.), cx));
        cx.observe(&search, |this, editor, cx| {
            let query = model::limit_query(editor.read(cx).field_text()).to_owned();
            if editor.read(cx).field_text() != query {
                editor.update(cx, |editor, cx| editor.set_text(&query, cx));
            }
            if query != this.query {
                this.query = query;
                this.query_changed(cx);
            }
        })
        .detach();
        Self {
            availability: Availability::Checking,
            streams: Vec::new(),
            view: View::All,
            lifecycle: "open".into(),
            repository: None,
            selected: None,
            detail: None,
            detail_tab: DetailTab::Summary,
            code: None,
            code_loading: false,
            detail_fullscreen: false,
            media_images: HashMap::new(),
            media_queue: VecDeque::new(),
            media_inflight: false,
            generation: 0,
            detail_generation: 0,
            query: String::new(),
            search,
            filter_open: false,
            filter_submenu: None,
            collapsed_detail: [false; 3],
            scroll: ScrollHandle::new(),
            detail_scroll: ScrollHandle::new(),
            last_filters: None,
            controls: RefCell::new(HashMap::new()),
            clock,
        }
    }

    pub fn account(&self) -> Option<&Account> {
        if let Availability::Ready(account) = &self.availability {
            Some(account)
        } else {
            None
        }
    }

    pub fn set_availability(&mut self, availability: Availability, cx: &mut Context<Self>) {
        match &availability {
            Availability::Checking => {
                self.detail_generation += 1;
                self.detail = None;
                self.code = None;
                self.code_loading = false;
                self.media_images.clear();
                self.media_queue.clear();
                self.streams.clear();
                self.last_filters = None;
            }
            Availability::Missing if self.selected.is_some() => {
                self.detail = Some(Err("GitHub CLI (gh) is not installed".into()));
            }
            Availability::Unavailable(error) if self.selected.is_some() => {
                self.detail = Some(Err(error.clone()));
            }
            _ => {}
        }
        self.availability = availability;
        cx.notify();
    }

    pub fn begin_query(
        &mut self,
        more: bool,
        cx: &mut Context<Self>,
    ) -> Vec<(usize, Filters, Option<String>)> {
        let (filters, explicit_relation, _) = model::filters(
            &self.query,
            self.view,
            &self.lifecycle,
            self.repository.clone(),
        );
        if !more {
            self.controls.borrow_mut().clear();
            self.generation += 1;
            let sections = self.query_sections(explicit_relation);
            if self.last_filters.as_ref() != Some(&filters)
                || self
                    .streams
                    .iter()
                    .map(|s| (s.key, s.title))
                    .collect::<Vec<_>>()
                    != sections
            {
                self.streams = sections
                    .into_iter()
                    .map(|(key, title)| Stream::new(key, title))
                    .collect();
            }
        }
        self.last_filters = Some(filters.clone());
        let mut requests = Vec::new();
        for (index, stream) in self.streams.iter_mut().enumerate() {
            if more && (stream.loading || stream.cursor.is_none()) {
                continue;
            }
            stream.loading = true;
            let mut filters = filters.clone();
            filters.relationship = stream.key.into();
            requests.push((
                index,
                filters,
                if more { stream.cursor.clone() } else { None },
            ));
        }
        cx.notify();
        requests
    }

    fn query_sections(&self, explicit_relation: bool) -> Vec<(&'static str, &'static str)> {
        let sections: Vec<_> = if explicit_relation {
            vec![("all", "Results")]
        } else if self.view == View::Authored {
            vec![("authored", "Authored")]
        } else {
            model::SECTION_KEYS
                .iter()
                .zip(model::SECTION_TITLES)
                .take(if self.view == View::Reviewing { 3 } else { 4 })
                .map(|(key, title)| (*key, title))
                .collect()
        };
        sections
    }

    pub fn apply_search(
        &mut self,
        generation: u64,
        index: usize,
        more: bool,
        result: Result<SearchPage, String>,
        cx: &mut Context<Self>,
    ) {
        if generation != self.generation {
            return;
        }
        if let Some(stream) = self.streams.get_mut(index) {
            stream.apply(result, more);
        }
        cx.notify();
    }

    pub fn select(&mut self, identity: Option<Identity>, cx: &mut Context<Self>) -> bool {
        if self.selected == identity {
            return false;
        }
        self.selected = identity;
        self.detail = None;
        self.code = None;
        self.code_loading = false;
        if self.selected.is_none() {
            self.detail_tab = DetailTab::Summary;
            self.detail_fullscreen = false;
        }
        self.media_images.clear();
        self.media_queue.clear();
        self.detail_generation += 1;
        self.collapsed_detail = [false; 3];
        self.detail_scroll.set_offset(point(px(0.), px(0.)));
        cx.notify();
        true
    }

    pub fn apply_detail(
        &mut self,
        generation: u64,
        result: Result<Value, String>,
        cx: &mut Context<Self>,
    ) {
        if self.detail_generation != generation {
            return;
        }
        if let Ok(detail) = &result {
            for (_, url) in media_spans(detail["body"].as_str().unwrap_or("")) {
                if !self.media_images.contains_key(&url) {
                    self.media_images.insert(url.clone(), None);
                    self.media_queue.push_back(url);
                }
            }
        }
        self.detail = Some(result);
        self.start_next_media(cx);
        if self.detail_tab == DetailTab::Code && self.code.is_none() && !self.code_loading {
            self.request_code(cx);
        }
        cx.notify();
    }

    pub fn set_detail_tab(&mut self, tab: DetailTab, cx: &mut Context<Self>) {
        self.detail_tab = tab;
        if tab == DetailTab::Code && self.code.is_none() && !self.code_loading {
            self.request_code(cx);
        }
        cx.notify();
    }

    fn request_code(&mut self, cx: &mut Context<Self>) {
        self.code = None;
        self.code_loading = true;
        cx.emit(Event::Code);
        cx.notify();
    }

    pub fn apply_code(
        &mut self,
        generation: u64,
        result: Result<String, String>,
        cx: &mut Context<Self>,
    ) {
        if generation != self.detail_generation {
            return;
        }
        self.code_loading = false;
        self.code = Some(result.map(|patch| cx.new(|_| DiffPanel::from_patch("", &patch))));
        cx.notify();
    }

    fn start_next_media(&mut self, cx: &mut Context<Self>) {
        if !self.media_inflight
            && let Some(url) = self.media_queue.pop_front()
        {
            self.media_inflight = true;
            cx.emit(Event::Media {
                generation: self.detail_generation,
                url,
            });
        }
    }

    pub fn apply_media(
        &mut self,
        generation: u64,
        url: &str,
        result: Result<Arc<gpui::RenderImage>, String>,
        cx: &mut Context<Self>,
    ) {
        self.media_inflight = false;
        if self.detail_generation != generation {
            self.start_next_media(cx);
            return;
        }
        let retained_bytes: usize = self
            .media_images
            .iter()
            .filter(|(key, _)| key.as_str() != url)
            .filter_map(|(_, preview)| preview.as_ref()?.as_ref().ok())
            .map(|image| image.as_bytes(0).map_or(0, <[u8]>::len))
            .sum();
        let result = result.and_then(|image| {
            if image.as_bytes(0).map_or(0, <[u8]>::len)
                <= PREVIEW_MEMORY_LIMIT.saturating_sub(retained_bytes)
            {
                Ok(image)
            } else {
                Err("Preview exceeds the image memory limit".into())
            }
        });
        self.media_images.insert(url.to_owned(), Some(result));
        self.start_next_media(cx);
        cx.notify();
    }

    fn unavailable_media(&self, url: &str, cx: &Context<Self>) -> Div {
        let t = cx.theme();
        let url = url.to_owned();
        div()
            .my(px(12.))
            .min_h(px(96.))
            .min_w(px(160.))
            .px(px(16.))
            .py(px(12.))
            .rounded(px(12.5))
            .bg(t.ghost_hover)
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap(px(8.))
            .child(text(
                "Preview unavailable",
                13.,
                18.571_428,
                430.,
                t.text_secondary,
            ))
            .child(
                self.button(
                    SharedString::from(format!("open-media:{url}")),
                    "Open in GitHub",
                    cx,
                )
                .bg(gpui::transparent_black())
                .border_color(t.border)
                .on_click(move |_, _, cx| cx.open_url(&url)),
            )
    }

    fn description(&self, body: &str, window: &Window, cx: &Context<Self>) -> Div {
        if body.trim().is_empty() {
            return text(
                "No description provided",
                14.,
                21.,
                430.,
                cx.theme().text_tertiary,
            );
        }
        let mut content = div().flex().flex_col().gap(px(12.));
        let mut cursor = 0;
        for (index, (span, url)) in media_spans(body).into_iter().enumerate() {
            if span.start > cursor {
                content = content.child(crate::screens::thread::view::render_markdown(
                    &body[cursor..span.start],
                    window,
                    cx.theme(),
                ));
            }
            let preview = match self.media_images.get(&url).and_then(Option::as_ref) {
                None => div()
                    .id(("github-media", index))
                    .aria_label("Loading GitHub media…")
                    .min_h(px(96.))
                    .min_w(px(160.))
                    .px(px(16.))
                    .py(px(12.))
                    .rounded(px(10.))
                    .bg(cx.theme().ghost_hover)
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(icon("sidebar-cmd-spinner", 16., cx.theme().text_secondary))
                    .into_any_element(),
                Some(Err(_)) => self.unavailable_media(&url, cx).into_any_element(),
                Some(Ok(image)) => div()
                    .my(px(12.))
                    .child(
                        gpui::img(image.clone())
                            .max_w_full()
                            .max_h((window.viewport_size().height * 0.7).min(px(640.)))
                            .rounded(px(10.))
                            .object_fit(gpui::ObjectFit::Contain),
                    )
                    .into_any_element(),
            };
            content = content.child(preview);
            cursor = span.end;
        }
        if cursor < body.len() {
            content = content.child(crate::screens::thread::view::render_markdown(
                &body[cursor..],
                window,
                cx.theme(),
            ));
        }
        content
    }

    fn changed(&mut self, cx: &mut Context<Self>) {
        self.filter_open = false;
        self.filter_submenu = None;
        self.scroll.set_offset(point(px(0.), px(0.)));
        cx.emit(Event::Query);
        cx.notify();
    }

    fn query_changed(&mut self, cx: &mut Context<Self>) {
        self.generation += 1;
        let generation = self.generation;
        cx.notify();
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(200))
                .await;
            this.update(cx, |this, cx| {
                if this.generation == generation {
                    this.changed(cx);
                }
            })
            .ok();
        })
        .detach();
    }

    fn control(&self, id: impl Into<gpui::ElementId>, cx: &Context<Self>) -> FocusHandle {
        self.controls
            .borrow_mut()
            .entry(id.into())
            .or_insert_with(|| cx.focus_handle().tab_stop(true))
            .clone()
    }

    fn button(
        &self,
        id: impl Into<gpui::ElementId>,
        label: impl Into<SharedString>,
        cx: &Context<Self>,
    ) -> Stateful<Div> {
        let t = cx.theme();
        let id = id.into();
        let focus = self.control(id.clone(), cx);
        div()
            .id(id)
            .role(gpui::Role::Button)
            .track_focus(&focus)
            .cursor_pointer()
            .rounded(px(10.))
            .px(px(12.))
            .h(px(32.))
            .flex()
            .items_center()
            .justify_center()
            .bg(t.primary_soft_alpha)
            .border_1()
            .border_color(gpui::transparent_black())
            .focus(|s| s.border_color(t.text_accent))
            .hover(|s| s.bg(t.ghost_hover))
            .child(text(label, 14., 20., 500., t.text))
    }

    fn empty(
        &self,
        title: &str,
        description: Option<&str>,
        actions: Option<Div>,
        faded: bool,
        cx: &Context<Self>,
    ) -> Div {
        let t = cx.theme();
        let content = div()
            .w_full()
            .max_w(px(576.))
            .flex()
            .flex_col()
            .items_center()
            .gap(px(12.))
            .when(faded, |d| d.opacity(0.6))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap(px(8.))
                    .child(text(title.to_owned(), 16., 24., 500., t.text))
                    .when_some(description, |d, description| {
                        d.child(
                            text(description.to_owned(), 14., 21., 430., t.text_secondary)
                                .whitespace_normal(),
                        )
                    }),
            )
            .when_some(actions, |d, actions| d.child(actions));
        div()
            .w_full()
            .flex_1()
            .min_h(px(256.))
            .p(px(24.))
            .flex()
            .items_center()
            .justify_center()
            .child(content)
    }

    fn setup(&self, cx: &Context<Self>) -> Div {
        match &self.availability {
            Availability::Checking => loading_rows("Checking GitHub access", cx),
            Availability::Missing => {
                let command = div()
                    .w(px(320.))
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap(px(12.))
                    .child(
                        div()
                            .w_full()
                            .p(px(6.))
                            .pl(px(12.))
                            .rounded(px(8.))
                            .border_1()
                            .border_color(cx.theme().border)
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .child(
                                text("brew install gh", 13., 20., 430., cx.theme().text)
                                    .font_family(crate::MONO)
                                    .flex_1(),
                            )
                            .child(
                                self.button("copy-install", "Copy", cx)
                                    .aria_label("Copy install command")
                                    .on_click(|_, _, cx| {
                                        cx.write_to_clipboard(ClipboardItem::new_string(
                                            "brew install gh".into(),
                                        ))
                                    }),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .gap(px(8.))
                            .child(
                                self.button("retry-install", "Retry", cx)
                                    .on_click(cx.listener(|_, _, _, cx| cx.emit(Event::Retry))),
                            )
                            .child(self.button("ask-install", "Ask Codex", cx).on_click(
                                cx.listener(|_, _, _, cx| {
                                    cx.emit(Event::AskCodex(INSTALL_PROMPT.into()))
                                }),
                            )),
                    );
                self.empty(
                    "Install GitHub CLI",
                    Some("Install GitHub CLI to view and manage your pull requests"),
                    Some(command),
                    false,
                    cx,
                )
            }
            Availability::Unavailable(error) => self.empty(
                "GitHub CLI setup required",
                Some(error),
                Some(
                    div().child(
                        self.button("check-again", "Check again", cx)
                            .on_click(cx.listener(|_, _, _, cx| cx.emit(Event::Retry))),
                    ),
                ),
                false,
                cx,
            ),
            Availability::Ready(_) => div(),
        }
    }

    fn input(&self, cx: &Context<Self>) -> Stateful<Div> {
        let t = cx.theme();
        div()
            .id("pull-request-inbox-search")
            .relative()
            .flex_1()
            .min_w_0()
            .h(px(32.))
            .px(px(11.))
            .flex()
            .items_center()
            .gap(px(8.))
            .child(shapes::decoration(shapes::Decoration {
                radius: gpui::Corners::all(px(16.)),
                fill: Some(t.control_background),
                border: Some((px(1.), t.border_strong)),
                ..Default::default()
            }))
            .child(review_icon("search", 18., t.text_secondary))
            .child(div().flex_1().min_w_0().child(self.search.clone()))
            .when(!self.query.is_empty(), |d| {
                d.child(
                    div()
                        .id("clear-pull-request-search")
                        .role(gpui::Role::Button)
                        .aria_label("Clear search")
                        .track_focus(&self.control("clear-pull-request-search", cx))
                        .cursor_pointer()
                        .size(px(16.))
                        .flex_none()
                        .child(review_icon("clear", 16., t.text_secondary))
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.search.update(cx, |search, cx| search.set_text("", cx));
                            this.search.focus_handle(cx).focus(window, cx);
                        })),
                )
            })
    }

    fn filters(&self, cx: &Context<Self>) -> Div {
        let t = cx.theme();
        let status_disabled = self.view == View::Reviewing
            || model::filters(
                &self.query,
                self.view,
                &self.lifecycle,
                self.repository.clone(),
            )
            .2;
        let mut content = div()
            .absolute()
            .right(px(0.))
            .top(px(36.))
            .w(px(224.))
            .p(px(4.))
            .rounded(px(12.))
            .border_1()
            .border_color(t.border)
            .bg(t.menu_background)
            .flex()
            .flex_col();
        match self.filter_submenu {
            Some("status") => {
                for (value, label) in [
                    ("all", "All states"),
                    ("open", "Open"),
                    ("merged", "Merged"),
                    ("closed", "Closed"),
                ] {
                    content =
                        content.child(self.button(value, label, cx).justify_start().on_click(
                            cx.listener(move |this, _, _, cx| {
                                this.lifecycle = value.into();
                                this.changed(cx);
                            }),
                        ));
                }
            }
            Some("repository") => {
                content = content.child(
                    self.button("repo-all", "All repositories", cx)
                        .justify_start()
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.repository = None;
                            this.changed(cx);
                        })),
                );
                let mut repos = self
                    .streams
                    .iter()
                    .flat_map(|s| &s.items)
                    .map(|p| {
                        (
                            p.pull_request.owner.clone(),
                            p.pull_request.repository.clone(),
                        )
                    })
                    .collect::<Vec<_>>();
                if let Some(repo) = &self.repository {
                    repos.push((repo.owner.clone(), repo.repository.clone()));
                }
                repos.sort();
                repos.dedup();
                for (index, (owner, repository)) in repos.into_iter().enumerate() {
                    content = content.child(
                        self.button(("repo", index), format!("{owner}/{repository}"), cx)
                            .justify_start()
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.repository = Some(Repository {
                                    owner: owner.clone(),
                                    repository: repository.clone(),
                                });
                                this.changed(cx);
                            })),
                    );
                }
            }
            _ => {
                content = content.child(
                    self.button("status-filter", "Status", cx)
                        .justify_start()
                        .when(status_disabled, |d| d.opacity(0.5))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if !status_disabled {
                                this.filter_submenu = Some("status");
                                cx.notify();
                            }
                        })),
                );
                content = content.child(
                    self.button("repository-filter", "Repository", cx)
                        .justify_start()
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.filter_submenu = Some("repository");
                            cx.notify();
                        })),
                );
            }
        }
        content.on_mouse_down_out(cx.listener(|this, _, _, cx| {
            this.filter_open = false;
            this.filter_submenu = None;
            cx.notify();
        }))
    }

    fn inbox(&mut self, window: &Window, cx: &mut Context<Self>) -> Div {
        let t = cx.theme();
        let viewport = window.viewport_size();
        let width = (f32::from(viewport.width - crate::RAIL_WIDTH - crate::PAGE_INSET)
            - default_detail_width(viewport))
        .max(0.);
        let mut navigation = div()
            .h(px(46.))
            .flex_none()
            .px(px(8.))
            .flex()
            .items_center()
            .gap(px(2.));
        for (index, (view, label)) in [
            (View::All, "All"),
            (View::Reviewing, "Reviewing"),
            (View::Authored, "Authored"),
        ]
        .into_iter()
        .enumerate()
        {
            navigation = navigation.child(
                div()
                    .id(("view", index))
                    .role(gpui::Role::Button)
                    .aria_label(label)
                    .track_focus(&self.control(("view", index), cx))
                    .relative()
                    .h(px(28.))
                    .px(px(9.))
                    .flex()
                    .items_center()
                    .rounded(px(12.5))
                    .hover(|s| s.bg(t.ghost_hover))
                    .when(self.view == view, |d| {
                        d.child(shapes::decoration(shapes::Decoration {
                            radius: gpui::Corners::all(px(12.5)),
                            corner: shapes::Corner::Squircle,
                            fill: Some(t.text_alpha_5),
                            ..Default::default()
                        }))
                    })
                    .child(text(
                        label,
                        13.,
                        18.,
                        430.,
                        if self.view == view {
                            t.text
                        } else {
                            t.text_tertiary
                        },
                    ))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if this.view != view {
                            this.view = view;
                            this.changed(cx);
                        }
                    })),
            );
        }
        let search = self.input(cx);
        let mut toolbar = div()
            .relative()
            .px(px(30.))
            .pt(px(20.))
            .pb(px(8.))
            .flex_none()
            .flex()
            .gap(px(8.))
            .items_center()
            .child(search)
            .child(
                div()
                    .id("pull-request-filters")
                    .role(gpui::Role::Button)
                    .aria_label("Filter pull requests")
                    .track_focus(&self.control("pull-request-filters", cx))
                    .relative()
                    .size(px(28.))
                    .flex_none()
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(12.5))
                    .cursor_pointer()
                    .hover(|s| s.bg(t.ghost_hover))
                    .child(shapes::decoration(shapes::Decoration {
                        radius: gpui::Corners::all(px(12.5)),
                        corner: shapes::Corner::Squircle,
                        fill: Some(t.text_alpha_5),
                        ..Default::default()
                    }))
                    .child(review_icon(
                        if self.lifecycle != "open" || self.repository.is_some() {
                            "filter-active"
                        } else {
                            "filter"
                        },
                        18.,
                        t.text,
                    ))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.filter_open = !this.filter_open;
                        this.filter_submenu = None;
                        cx.notify();
                    })),
            );
        if self.filter_open {
            toolbar = toolbar.child(gpui::deferred(self.filters(cx)).with_priority(2));
        }
        let body = self.inbox_body(cx);
        div()
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .min_h_0()
            .child(navigation)
            .child(toolbar)
            .child(body.px(px(30.)).pt(px(20.)))
            .child(
                crate::screens::thread::view::skia_gradient(
                    &[(t.surface, 0.), (t.surface.opacity(0.), 1.)],
                    gpui::transparent_black(),
                    (width, 32.),
                    (0., 60.),
                    false,
                    window.scale_factor(),
                )
                .absolute()
                .top(px(106.))
                .left_0(),
            )
    }

    fn inbox_body(&self, cx: &Context<Self>) -> Div {
        let t = cx.theme();
        let rows = model::section_items(&self.streams);
        let has_rows = rows.iter().any(|rows| !rows.is_empty());
        let loading = self.streams.iter().any(|s| s.loading && !s.loaded);
        let error = self.streams.iter().find_map(|s| s.error.as_deref());
        let mut body = div().flex_1().min_h_0().flex().flex_col().gap(px(16.));
        if error.is_some() && self.streams.iter().any(|s| s.loaded) {
            body = body.child(
                div()
                    .p(px(12.))
                    .rounded(px(8.))
                    .border_1()
                    .border_color(t.border)
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(
                        text(
                            "Some pull requests couldn’t be loaded",
                            14.,
                            21.,
                            430.,
                            t.text,
                        )
                        .flex_1(),
                    )
                    .child(
                        self.button("partial-retry", "Retry", cx)
                            .on_click(cx.listener(|_, _, _, cx| cx.emit(Event::Query))),
                    ),
            );
        }
        if !has_rows && !loading {
            body = body.child(if let Some(error) = error {
                self.empty(
                    "Unable to load pull requests",
                    Some(error),
                    Some(
                        div().child(
                            self.button("retry-list", "Try again", cx)
                                .on_click(cx.listener(|_, _, _, cx| cx.emit(Event::Query))),
                        ),
                    ),
                    false,
                    cx,
                )
            } else {
                self.empty(
                    if !self.query.trim().is_empty() {
                        "No pull requests match this search"
                    } else if self.view == View::Reviewing {
                        "You’re all caught up"
                    } else {
                        "No pull requests found"
                    },
                    None,
                    None,
                    true,
                    cx,
                )
            });
        } else {
            body = body.child(self.request_list(&rows, cx));
        }
        body
    }

    fn request_list(&self, rows: &[Vec<&model::Item>], cx: &Context<Self>) -> Stateful<Div> {
        let t = cx.theme();
        let mut list = div()
            .id("pull-request-list")
            .overflow_y_scroll()
            .track_scroll(&self.scroll)
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .layer()
            .pb(px(20.))
            .gap(px(16.));
        for (index, stream) in self.streams.iter().enumerate() {
            if rows[index].is_empty() && !stream.loading {
                continue;
            }
            let mut section = div().flex().flex_col().gap(px(2.)).child(
                div()
                    .id(("section", index))
                    .role(gpui::Role::Button)
                    .aria_label(stream.title)
                    .aria_expanded(stream.expanded)
                    .track_focus(&self.control(("section", index), cx))
                    .px(px(12.))
                    .pb(px(4.))
                    .flex()
                    .gap(px(4.))
                    .cursor_pointer()
                    .items_center()
                    .child(text(stream.title, 12., 16., 500., t.text_tertiary))
                    .child(icon("thread-chevron-right", 14., t.text_tertiary).when(
                        stream.expanded,
                        |glyph| {
                            glyph.with_transformation(gpui::Transformation::rotate(gpui::radians(
                                std::f32::consts::FRAC_PI_2,
                            )))
                        },
                    ))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.streams[index].expanded = !this.streams[index].expanded;
                        cx.notify();
                    })),
            );
            if stream.expanded {
                if stream.loading && !stream.loaded {
                    section = section.child(
                        loading_rows("Loading pull requests", cx)
                            .max_h(px(112.))
                            .overflow_hidden(),
                    );
                } else {
                    for (position, item) in rows[index].iter().enumerate() {
                        let next = rows[index].get(position + 1);
                        let divider = [Some(item), next]
                            .into_iter()
                            .flatten()
                            .all(|item| self.selected.as_ref() != Some(&item.pull_request));
                        section = section.child(self.row(item, stream.key != "authored", cx).when(
                            divider,
                            |row| {
                                row.child(
                                    div()
                                        .absolute()
                                        .left(px(12.))
                                        .right(px(12.))
                                        .bottom(px(if next.is_some() { -1. } else { 0. }))
                                        .h(px(1.))
                                        .bg(t.border_subtle)
                                        .group_hover("pull-request-row", |s| s.opacity(0.)),
                                )
                            },
                        ));
                    }
                }
            }
            list = list.child(section);
        }
        if self.streams.iter().any(|s| s.cursor.is_some()) {
            list = list.child(
                self.button(
                    "load-more",
                    if self.streams.iter().any(|s| s.loading) {
                        "Loading…"
                    } else {
                        "Load more"
                    },
                    cx,
                )
                .on_click(cx.listener(|_, _, _, cx| cx.emit(Event::More))),
            );
        }
        list
    }

    fn row(&self, item: &model::Item, show_author: bool, cx: &Context<Self>) -> Stateful<Div> {
        static STATS_FEATURES: std::sync::LazyLock<gpui::FontFeatures> =
            std::sync::LazyLock::new(|| crate::diff::stats_features(12.));
        let t = cx.theme();
        let identity = item.pull_request.clone();
        let selected = self.selected.as_ref() == Some(&identity);
        let mut metadata = div().flex_1().flex().items_center().gap(px(8.)).min_w_0();
        if show_author && let Some(url) = &item.author_avatar_url {
            metadata = metadata.child(
                gpui::img(url.clone())
                    .size(px(16.))
                    .rounded_full()
                    .object_fit(gpui::ObjectFit::Cover),
            );
        }
        if self.repository.is_none() {
            metadata = metadata.child(
                text(
                    format!("{}/{}", identity.owner, identity.repository),
                    12.,
                    16.,
                    430.,
                    t.text_tertiary,
                )
                .flex_none()
                .max_w(gpui::relative(0.35))
                .truncate(),
            );
        }
        metadata = metadata.child(
            text(item.head_branch.clone(), 12., 16., 430., t.text_tertiary)
                .flex_1()
                .min_w_0()
                .truncate(),
        );
        div()
            .id(SharedString::from(item.node_id.clone()))
            .role(gpui::Role::Button)
            .aria_label(item.title.clone())
            .track_focus(&self.control(SharedString::from(item.node_id.clone()), cx))
            .relative()
            .group("pull-request-row")
            .rounded(px(15.))
            .px(px(12.))
            .py(px(10.))
            .cursor_pointer()
            .flex()
            .items_center()
            .gap(px(8.))
            .when(selected, |d| {
                d.child(shapes::decoration(shapes::Decoration {
                    radius: gpui::Corners::all(px(15.)),
                    corner: shapes::Corner::Squircle,
                    fill: Some(t.text.opacity(0.05)),
                    ..Default::default()
                }))
            })
            .hover(|s| s.bg(t.ghost_hover))
            .child(status_icon(item, t))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .child(
                        div()
                            .flex()
                            .gap(px(12.))
                            .items_start()
                            .child(
                                text(item.title.clone(), 14., 24., 430., t.text)
                                    .flex_1()
                                    .min_w_0()
                                    .truncate(),
                            )
                            .child(
                                text(
                                    relative_age(&item.updated_at, self.clock),
                                    12.,
                                    16.,
                                    430.,
                                    t.text_tertiary,
                                )
                                .mt(px(2.))
                                .flex_none(),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(12.))
                            .child(metadata)
                            .when(item.has_diff_stats, |d| {
                                d.child(
                                    div()
                                        .flex_none()
                                        .flex()
                                        .font_features(STATS_FEATURES.clone())
                                        .gap(px(4.))
                                        .child(text(
                                            format!("+{}", grouped_count(item.additions)),
                                            12.,
                                            12.,
                                            430.,
                                            t.text_tertiary,
                                        ))
                                        .child(text(
                                            format!("-{}", grouped_count(item.deletions)),
                                            12.,
                                            12.,
                                            430.,
                                            t.text_tertiary,
                                        )),
                                )
                            }),
                    ),
            )
            .on_click(cx.listener(move |this, _, _, cx| {
                cx.emit(Event::Select(
                    if this.selected.as_ref() == Some(&identity) {
                        None
                    } else {
                        Some(identity.clone())
                    },
                ))
            }))
    }

    fn detail_control(
        &self,
        id: &'static str,
        label: &'static str,
        active: bool,
        cx: &Context<Self>,
    ) -> Stateful<Div> {
        let t = cx.theme();
        div()
            .id(id)
            .role(gpui::Role::Button)
            .aria_label(label)
            .track_focus(&self.control(id, cx))
            .relative()
            .h(px(28.))
            .px(px(9.))
            .rounded(px(12.5))
            .flex()
            .items_center()
            .justify_center()
            .cursor_pointer()
            .focus_visible(|s| {
                s.shadow(vec![gpui::BoxShadow {
                    color: t.text_accent,
                    offset: point(px(0.), px(0.)),
                    blur_radius: px(0.),
                    spread_radius: px(2.),
                    inset: true,
                }])
            })
            .hover(|s| s.bg(t.ghost_hover))
            .when(active, |d| {
                d.child(shapes::decoration(shapes::Decoration {
                    radius: gpui::Corners::all(px(12.5)),
                    corner: shapes::Corner::Squircle,
                    fill: Some(t.text_alpha_5),
                    ..Default::default()
                }))
            })
    }

    fn discuss_selected(&self, cx: &mut Context<Self>) {
        if let (Some(identity), Some(Ok(detail))) = (&self.selected, &self.detail) {
            cx.emit(Event::AskCodex(format!(
                "Help me understand pull request #{}: {}\n{}",
                identity.number,
                detail["title"].as_str().unwrap_or(""),
                identity.url(),
            )));
        }
    }

    fn open_selected(&self, cx: &mut Context<Self>) {
        if let Some(identity) = &self.selected {
            cx.open_url(&identity.url());
        }
    }

    fn detail_toolbar(&self, detail: Option<&Value>, cx: &Context<Self>) -> Div {
        let t = cx.theme();
        let (glyph, color) = request_status(
            detail
                .and_then(|value| value["state"].as_str())
                .unwrap_or("OPEN"),
            detail.is_some_and(|value| value["isDraft"] == true),
            t,
        );
        let mut tabs = div()
            .id("detail-tabs")
            .role(gpui::Role::TabList)
            .flex()
            .gap(px(2.));
        for (tab, id, label) in [
            (DetailTab::Summary, "detail-summary", "Summary"),
            (DetailTab::Code, "detail-code", "Code"),
        ] {
            let active = self.detail_tab == tab;
            tabs = tabs.child(
                self.detail_control(id, label, active, cx)
                    .role(gpui::Role::Tab)
                    .aria_selected(active)
                    .child(text(
                        label,
                        13.,
                        18.,
                        430.,
                        if active { t.text } else { t.text_tertiary },
                    ))
                    .on_click(cx.listener(move |this, _, _, cx| this.set_detail_tab(tab, cx)))
                    .on_key_down(cx.listener(move |this, event: &gpui::KeyDownEvent, _, cx| {
                        if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                            this.set_detail_tab(tab, cx);
                            cx.stop_propagation();
                        }
                    })),
            );
        }
        div()
            .h(px(46.))
            .flex_none()
            .px(px(16.))
            .flex()
            .items_center()
            .gap(px(4.))
            .child(icon(glyph, 18., color))
            .child(tabs)
            .child(div().flex_1())
            .child(self.detail_actions(detail.is_some(), cx))
    }

    fn detail_actions(&self, available: bool, cx: &Context<Self>) -> Div {
        let t = cx.theme();
        div()
            .flex()
            .items_center()
            .gap(px(4.))
            .child(
                self.detail_control("open-github", "Open in browser", false, cx)
                    .w(px(28.))
                    .px_0()
                    .child(icon("diff-open-in", 16., t.text))
                    .on_click(cx.listener(|this, _, _, cx| this.open_selected(cx)))
                    .on_key_down(cx.listener(|this, event: &gpui::KeyDownEvent, _, cx| {
                        if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                            this.open_selected(cx);
                            cx.stop_propagation();
                        }
                    })),
            )
            .child(
                self.detail_control("ask-pull-request", "Chat", true, cx)
                    .aria_disabled(!available)
                    .child(text("Chat", 13., 18., 430., t.text))
                    .on_click(cx.listener(|this, _, _, cx| this.discuss_selected(cx)))
                    .on_key_down(cx.listener(|this, event: &gpui::KeyDownEvent, _, cx| {
                        if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                            this.discuss_selected(cx);
                            cx.stop_propagation();
                        }
                    })),
            )
            .child(
                self.detail_control(
                    "pull-request-fullscreen",
                    if self.detail_fullscreen {
                        "Exit full screen"
                    } else {
                        "Enter full screen"
                    },
                    false,
                    cx,
                )
                .w(px(28.))
                .px_0()
                .child(review_icon("fullscreen", 16., t.text_tertiary))
                .on_click(cx.listener(|this, _, _, cx| {
                    this.detail_fullscreen = !this.detail_fullscreen;
                    cx.notify();
                }))
                .on_key_down(cx.listener(
                    |this, event: &gpui::KeyDownEvent, _, cx| {
                        if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                            this.detail_fullscreen = !this.detail_fullscreen;
                            cx.notify();
                            cx.stop_propagation();
                        }
                    },
                )),
            )
    }

    fn detail_metadata(&self, detail: &Value, cx: &Context<Self>) -> Div {
        let t = cx.theme();
        let reviews = model::review_summary(detail);
        let reviewers = reviews.as_ref().map_or_else(
            || "Couldn’t load reviewers".to_owned(),
            model::ReviewSummary::names_label,
        );
        let comments = reviews.as_ref().map_or_else(
            || "Couldn’t load comments".to_owned(),
            model::ReviewSummary::comments_label,
        );
        let status = match detail["state"].as_str() {
            Some("MERGED") => "Merged",
            Some("CLOSED") => "Closed",
            Some("OPEN") if detail["isDraft"] == true => "Draft",
            Some("OPEN") => "Open",
            _ => "Unknown",
        };
        let (glyph, color) = request_status(
            detail["state"].as_str().unwrap_or(""),
            detail["isDraft"] == true,
            t,
        );
        let branch_names = div()
            .flex()
            .items_center()
            .gap(px(8.))
            .min_w_0()
            .child(
                text(
                    detail["headRefName"].as_str().unwrap_or("").to_owned(),
                    14.,
                    20.,
                    430.,
                    t.text,
                )
                .truncate(),
            )
            .child(icon("thread-chevron-right", 14., t.text_tertiary))
            .child(
                text(
                    detail["baseRefName"].as_str().unwrap_or("").to_owned(),
                    14.,
                    20.,
                    430.,
                    t.text,
                )
                .truncate(),
            );
        let branch = div()
            .flex()
            .items_center()
            .gap(px(8.))
            .child(branch_names)
            .child(
                self.detail_control(
                    "review-pull-request-changes",
                    "Review pull request changes",
                    false,
                    cx,
                )
                .child(crate::screens::diff::panel::stats(
                    detail["additions"].as_u64().unwrap_or(0) as usize,
                    detail["deletions"].as_u64().unwrap_or(0) as usize,
                    13.,
                    t,
                ))
                .on_click(cx.listener(|this, _, _, cx| this.set_detail_tab(DetailTab::Code, cx)))
                .on_key_down(cx.listener(
                    |this, event: &gpui::KeyDownEvent, _, cx| {
                        if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                            this.set_detail_tab(DetailTab::Code, cx);
                            cx.stop_propagation();
                        }
                    },
                )),
            );
        div()
            .px(px(8.))
            .pb(px(8.))
            .flex()
            .flex_col()
            .child(metadata_row(
                "Branch",
                review_icon("branch", 18., t.text_tertiary),
                branch,
                t,
            ))
            .child(metadata_row(
                "Reviewers",
                review_icon("reviewers", 18., t.text_tertiary),
                text(reviewers, 14., 20., 430., t.text_tertiary).whitespace_normal(),
                t,
            ))
            .child(metadata_row(
                "Comments",
                review_icon("comments", 18., t.text_tertiary),
                text(comments, 14., 20., 430., t.text),
                t,
            ))
            .child(metadata_row(
                "Checks",
                icon("sidebar-cmd-manage-scheduled-tasks", 18., t.text_tertiary),
                text(check_label(detail), 14., 20., 430., t.text),
                t,
            ))
            .child(metadata_row(
                "Status",
                icon(glyph, 18., color),
                text(status, 14., 20., 430., t.text),
                t,
            ))
    }

    fn detail_sections(&self, detail: &Value, window: &Window, cx: &Context<Self>) -> Div {
        let t = cx.theme();
        let mut sections = div().flex().flex_col().gap(px(24.));
        for (index, title) in ["Description", "Checks"].into_iter().enumerate() {
            let collapsed = self.collapsed_detail[index];
            let header = div()
                .id(("detail-section", index))
                .role(gpui::Role::Button)
                .aria_label(title)
                .aria_expanded(!collapsed)
                .track_focus(&self.control(("detail-section", index), cx))
                .pl(px(8.))
                .pr(px(2.))
                .pb(px(8.))
                .border_b_1()
                .border_color(t.border_subtle)
                .flex()
                .items_center()
                .gap(px(6.))
                .cursor_pointer()
                .child(
                    text(title, 16., 24., 500., t.text)
                        .h(px(28.))
                        .flex()
                        .items_center(),
                )
                .child(icon(
                    if collapsed {
                        "thread-chevron-right"
                    } else {
                        "chevron-down"
                    },
                    14.,
                    t.text,
                ))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.collapsed_detail[index] = !this.collapsed_detail[index];
                    cx.notify();
                }))
                .on_key_down(cx.listener(move |this, event: &gpui::KeyDownEvent, _, cx| {
                    if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                        this.collapsed_detail[index] = !this.collapsed_detail[index];
                        cx.notify();
                        cx.stop_propagation();
                    }
                }));
            let mut section = div().flex().flex_col().gap(px(16.)).child(header);
            if !collapsed {
                let body = if index == 0 {
                    self.description(detail["body"].as_str().unwrap_or(""), window, cx)
                } else {
                    check_details(detail, t)
                };
                section = section.child(body.px(px(8.)));
            }
            sections = sections.child(section);
        }
        sections
    }

    fn detail_summary(&self, detail: &Value, window: &Window, cx: &Context<Self>) -> Stateful<Div> {
        let t = cx.theme();
        let author = &detail["author"];
        let identity = div()
            .flex()
            .items_center()
            .gap(px(8.))
            .mt(px(1.))
            .when_some(author["avatarUrl"].as_str(), |d, url| {
                d.child(
                    gpui::img(url.to_owned())
                        .size(px(18.))
                        .rounded_full()
                        .bg(gpui::white())
                        .with_fallback(missing_avatar)
                        .object_fit(gpui::ObjectFit::Cover),
                )
            })
            .child(text(
                author["login"].as_str().unwrap_or("").to_owned(),
                14.,
                21.,
                430.,
                t.text_secondary,
            ))
            .child(text("·", 14., 21., 430., t.text_secondary))
            .child(text(
                relative_age(detail["createdAt"].as_str().unwrap_or(""), self.clock),
                14.,
                21.,
                430.,
                t.text_secondary,
            ));
        div()
            .id("pull-request-detail-scroll")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .track_scroll(&self.detail_scroll)
            .px(px(20.))
            .pb(px(20.))
            .child(
                div()
                    .w_full()
                    .max_w(px(768.))
                    .mx_auto()
                    .flex()
                    .flex_col()
                    .gap(px(24.))
                    .layer()
                    .child(
                        div()
                            .pt(px(16.))
                            .px(px(8.))
                            .flex()
                            .flex_col()
                            .gap(px(6.))
                            .child(
                                text(
                                    detail["title"].as_str().unwrap_or("").to_owned(),
                                    24.,
                                    28.8,
                                    500.,
                                    t.text,
                                )
                                .whitespace_normal(),
                            )
                            .child(identity),
                    )
                    .child(self.detail_metadata(detail, cx))
                    .child(self.detail_sections(detail, window, cx)),
            )
    }

    fn detail(&self, window: &Window, cx: &Context<Self>) -> Div {
        if self.selected.is_none() {
            return div()
                .size_full()
                .pt(px(46.))
                .flex()
                .flex_col()
                .child(self.empty("Select pull request to view", None, None, false, cx));
        }
        let value = self.detail.as_ref().and_then(|detail| detail.as_ref().ok());
        let pane = div()
            .size_full()
            .flex()
            .flex_col()
            .child(self.detail_toolbar(value, cx));
        if self.detail_tab == DetailTab::Code {
            return match &self.code {
                Some(Ok(panel)) => pane.child(div().flex_1().min_h_0().child(panel.clone())),
                Some(Err(error)) => pane.child(
                    self.empty(
                        "Unable to load pull request changes",
                        Some(error),
                        Some(
                            div().child(
                                self.button("retry-code", "Try again", cx)
                                    .on_click(cx.listener(|this, _, _, cx| this.request_code(cx))),
                            ),
                        ),
                        false,
                        cx,
                    ),
                ),
                None => {
                    pane.child(self.empty("Loading pull request changes", None, None, true, cx))
                }
            };
        }
        match &self.detail {
            Some(Ok(detail)) => pane.child(self.detail_summary(detail, window, cx)),
            Some(Err(error)) => pane.child(
                self.empty(
                    "Pull request details are unavailable",
                    Some(error),
                    Some(
                        div().child(
                            self.button("retry-detail", "Try again", cx)
                                .on_click(cx.listener(|_, _, _, cx| cx.emit(Event::Retry))),
                        ),
                    ),
                    false,
                    cx,
                ),
            ),
            None => pane.child(self.empty("Loading pull request details", None, None, true, cx)),
        }
    }
}

fn metadata_row(
    label: &'static str,
    glyph: impl IntoElement,
    value: impl IntoElement,
    t: &theme::Theme,
) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(12.))
        .min_h(px(30.))
        .py(px(5.))
        .child(
            div()
                .w(px(120.))
                .flex_none()
                .flex()
                .items_center()
                .gap(px(8.))
                .child(glyph)
                .child(text(label, 14., 20., 430., t.text_tertiary)),
        )
        .child(div().flex_1().min_w_0().child(value))
}

fn check_label(detail: &Value) -> &'static str {
    match detail["ciStatus"].as_str() {
        Some("none") => "No CI checks",
        Some("passing") => "Checks successful",
        Some("failing") => "Checks failing",
        Some("pending") => "Checks pending",
        _ => "Couldn’t load checks",
    }
}

fn check_details(detail: &Value, t: &theme::Theme) -> Div {
    let mut content = div().flex().flex_col().gap(px(8.));
    let Some(checks) = detail["statusCheckRollup"]["contexts"]["nodes"]
        .as_array()
        .filter(|nodes| !nodes.is_empty())
    else {
        return content.child(text(check_label(detail), 14., 21., 430., t.text_tertiary));
    };
    for check in checks {
        let name = check["name"]
            .as_str()
            .or_else(|| check["context"].as_str())
            .unwrap_or("");
        let status = check["conclusion"]
            .as_str()
            .or_else(|| check["state"].as_str())
            .or_else(|| check["status"].as_str())
            .unwrap_or("");
        content = content.child(text(
            format!("{name}    {status}"),
            14.,
            21.,
            430.,
            t.text_secondary,
        ));
    }
    content
}

fn request_status(state: &str, draft: bool, t: &theme::Theme) -> (&'static str, gpui::Hsla) {
    match state {
        "closed" | "CLOSED" => ("pull-request-closed", t.text_error),
        "merged" | "MERGED" => (
            "pull-request-merged",
            gpui::rgb(if t.appearance == theme::Appearance::Dark {
                0xad7bf9
            } else {
                0x924ff7
            })
            .into(),
        ),
        _ if draft => ("pull-request-draft", t.text_tertiary),
        _ => ("pull-request-status", t.text_tertiary),
    }
}

impl Render for Page {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let ready = self.account().is_some();
        let pane_width = if self.detail_fullscreen {
            window.viewport_size().width - crate::RAIL_WIDTH - crate::PAGE_INSET
        } else {
            px(default_detail_width(window.viewport_size()))
        };
        if let Some(Ok(panel)) = &self.code {
            panel.update(cx, |panel, cx| {
                panel.set_container_width(
                    pane_width - px(if self.detail_fullscreen { 0. } else { 1. }),
                    cx,
                );
            });
        }
        let main = if self.detail_fullscreen {
            div()
        } else if ready {
            self.inbox(window, cx)
        } else {
            div()
                .size_full()
                .flex()
                .flex_col()
                .child(
                    div()
                        .h(px(46.))
                        .flex_none()
                        .px(px(16.))
                        .flex()
                        .items_center()
                        .child(text("Pull requests", 14., 20., 500., cx.theme().text)),
                )
                .child(self.setup(cx))
        };
        div()
            .relative()
            .size_full()
            .flex()
            .bg(cx.theme().surface)
            .when(!self.detail_fullscreen, |d| {
                d.child(div().flex_1().min_w_0().child(main))
            })
            .when(ready || self.selected.is_some(), |d| {
                d.child(
                    div()
                        .w(pane_width)
                        .flex_none()
                        .h_full()
                        .when(!self.detail_fullscreen, |d| {
                            d.border_l_1().border_color(cx.theme().border)
                        })
                        .child(self.detail(window, cx)),
                )
            })
    }
}

fn grouped_count(value: u64) -> String {
    let mut text = value.to_string();
    let mut offset = text.len();
    while offset > 3 {
        offset -= 3;
        text.insert(offset, ',');
    }
    text
}

fn default_detail_width(viewport: gpui::Size<gpui::Pixels>) -> f32 {
    let width = f32::from(viewport.width - crate::RAIL_WIDTH - crate::PAGE_INSET);
    let height = f32::from(viewport.height - theme::Theme::TITLEBAR_HEIGHT - crate::PAGE_INSET);
    let maximum = (width - 352.).max(320.);
    (height * 1.6)
        .min(width - 500.)
        .max(640_f32.min(width - 352.))
        .clamp(320., maximum)
}

fn relative_age(date: &str, clock: Clock) -> String {
    let Ok(date) = chrono::DateTime::parse_from_rfc3339(date) else {
        return String::new();
    };
    let now = clock.now();
    let seconds = (now - date.timestamp()).max(0);
    if seconds < 60 {
        return "just now".into();
    }
    if seconds < 3600 {
        return format!("{}m", seconds / 60);
    }
    if seconds < 86400 {
        return format!("{}h", seconds / 3600);
    }
    let days = ((now + clock.utc_offset).div_euclid(86400)
        - (date.timestamp() + clock.utc_offset).div_euclid(86400))
    .max(1);
    if days < 7 {
        format!("{days}d")
    } else if days < 30 {
        format!("{}w", days / 7)
    } else if days < 365 {
        format!("{}mo", days / 30)
    } else {
        format!("{}y", days / 365)
    }
}

fn media_spans(body: &str) -> Vec<(std::ops::Range<usize>, String)> {
    let mut spans = Vec::new();
    let mut paragraph = None;
    for (event, span) in pulldown_cmark::Parser::new(body).into_offset_iter() {
        let url = match event {
            pulldown_cmark::Event::Start(pulldown_cmark::Tag::Paragraph) => {
                paragraph = Some(span.clone());
                let raw = body[span.clone()].trim();
                video_link(raw).then(|| raw.to_owned())
            }
            pulldown_cmark::Event::End(pulldown_cmark::TagEnd::Paragraph) => {
                paragraph = None;
                None
            }
            pulldown_cmark::Event::Start(pulldown_cmark::Tag::Image { dest_url, .. }) => {
                Some(dest_url.into_string())
            }
            pulldown_cmark::Event::Start(pulldown_cmark::Tag::Link { dest_url, .. })
                if paragraph.as_ref().is_some_and(|range| {
                    body[range.clone()].trim() == body[span.clone()].trim()
                }) =>
            {
                video_link(&dest_url).then(|| dest_url.into_string())
            }
            pulldown_cmark::Event::Html(html) | pulldown_cmark::Event::InlineHtml(html) => {
                html_media_source(&html).map(|url| url.replace("&amp;", "&"))
            }
            _ => None,
        };
        if let Some(url) = url.filter(|url| media_url(url))
            && spans
                .last()
                .is_none_or(|(previous, _): &(std::ops::Range<usize>, String)| {
                    previous.end <= span.start
                })
        {
            spans.push((span, url));
            if spans.len() == PREVIEW_COUNT_LIMIT {
                break;
            }
        }
    }
    spans
}

fn media_url(url: &str) -> bool {
    url.starts_with("https://github.com/user-attachments/assets/")
        || url.starts_with("https://user-images.githubusercontent.com/")
        || url.starts_with("https://private-user-images.githubusercontent.com/")
}

fn video_link(url: &str) -> bool {
    let path = url
        .split(['?', '#'])
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    media_url(url)
        && !url.chars().any(char::is_whitespace)
        && (url.starts_with("https://github.com/user-attachments/assets/")
            || [".mp4", ".mov", ".webm"]
                .iter()
                .any(|extension| path.ends_with(extension)))
}

pub fn decode_image_preview(value: Value) -> Result<Arc<gpui::RenderImage>, String> {
    let mime = value["mimeType"].as_str().ok_or("Preview unavailable")?;
    if !mime.starts_with("image/") {
        return Err("Preview unavailable".into());
    }
    let source = value["contentsBase64"]
        .as_str()
        .ok_or("Preview unavailable")?;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(source)
        .map_err(|e| e.to_string())?;
    let reader = image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|e| e.to_string())?;
    let decoder = reader.into_decoder().map_err(|e| e.to_string())?;
    let (width, height) = decoder.dimensions();
    let rgba_bytes = u64::from(width)
        .saturating_mul(u64::from(height))
        .saturating_mul(4);
    if decoder.total_bytes().max(rgba_bytes) > PREVIEW_IMAGE_LIMIT as u64 {
        return Err("Preview exceeds the image memory limit".into());
    }
    let pixels = image::DynamicImage::from_decoder(decoder)
        .map_err(|e| e.to_string())?
        .into_rgba8();
    Ok(Arc::new(gpui::RenderImage::new([image::Frame::new(
        pixels,
    )])))
}

fn html_media_source(html: &str) -> Option<&str> {
    let tag = html.trim_start().strip_prefix('<')?;
    let name_end = tag.find(|c: char| c.is_whitespace() || c == '>')?;
    if !["img", "video"]
        .iter()
        .any(|name| tag[..name_end].eq_ignore_ascii_case(name))
    {
        return None;
    }
    let mut attributes = &tag[name_end..];
    loop {
        attributes = attributes.trim_start();
        if attributes.is_empty() || attributes.starts_with(['>', '/']) {
            return None;
        }
        let end = attributes
            .find(|c: char| c.is_whitespace() || matches!(c, '=' | '>' | '/'))
            .unwrap_or(attributes.len());
        let name = &attributes[..end];
        attributes = attributes[end..].trim_start();
        let Some(value) = attributes.strip_prefix('=') else {
            continue;
        };
        attributes = value.trim_start();
        let quote = attributes.chars().next()?;
        let value = if matches!(quote, '\'' | '"') {
            attributes = &attributes[1..];
            let end = attributes.find(quote)?;
            let value = &attributes[..end];
            attributes = &attributes[end + 1..];
            value
        } else {
            let end = attributes
                .find(|c: char| c.is_whitespace() || c == '>')
                .unwrap_or(attributes.len());
            let value = &attributes[..end];
            attributes = &attributes[end..];
            value
        };
        if name.eq_ignore_ascii_case("src") {
            return Some(value);
        }
    }
}

fn status_icon(item: &model::Item, t: &theme::Theme) -> Div {
    let (glyph, color) = request_status(&item.state, item.is_draft, t);
    let mut icon_box = div()
        .relative()
        .size(px(20.))
        .flex_none()
        .child(icon(glyph, 20., color));
    if item.state == "open" && !item.is_draft {
        let color = status_color(item, t);
        icon_box = icon_box.child(
            div()
                .absolute()
                .left(px(12.))
                .top(px(12.))
                .size(px(6.282))
                .rounded_full()
                .bg(color),
        );
    }
    icon_box
}

fn status_color(item: &model::Item, t: &theme::Theme) -> gpui::Hsla {
    let dark = t.appearance == theme::Appearance::Dark;
    if item.merge_blocker.as_deref() == Some("conflicts") || item.ci_status == "failing" {
        t.text_error
    } else if item.can_merge || item.ci_status == "passing" {
        t.text_success
    } else {
        gpui::rgb(if dark { 0xffd240 } else { 0xffc300 }).into()
    }
}

fn loading_rows(label: &'static str, cx: &Context<Page>) -> Div {
    let mut rows = div().flex().flex_col().gap(px(2.));
    for (title, subtitle) in [
        (2. / 3., 0.5),
        (0.5, 0.4),
        (0.75, 0.6),
        (5. / 12., 1. / 3.),
        (7. / 12., 0.5),
    ] {
        rows = rows.child(
            div()
                .w_full()
                .px(px(12.))
                .py(px(10.))
                .flex()
                .flex_col()
                .child(
                    div().h(px(24.)).flex().items_center().child(
                        div()
                            .w(gpui::relative(title))
                            .h(px(16.))
                            .rounded(px(4.))
                            .bg(cx.theme().text_alpha_10),
                    ),
                )
                .child(
                    div().h(px(16.)).flex().items_center().child(
                        div()
                            .w(gpui::relative(subtitle))
                            .h(px(12.))
                            .rounded(px(4.))
                            .bg(cx.theme().text_alpha_10),
                    ),
                ),
        );
    }
    div().child(div().id(label).aria_label(label).child(rows))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stale_preview_completion_releases_the_next_request_without_retaining_old_data() {
        let mut app = gpui::HeadlessAppContext::with_platform(
            textsys::text_system(),
            Arc::new(theme::Assets),
            gpui_platform::current_headless_renderer,
        );
        app.update(|cx| {
            let page = cx.new(|cx| {
                Page::new(
                    Clock {
                        utc_offset: 0,
                        pinned_now: Some(0),
                        elapsed_since: None,
                    },
                    cx,
                )
            });
            page.update(cx, |page, cx| {
                let body = (0..9)
                    .map(|index| {
                        format!("![image](https://github.com/user-attachments/assets/{index})")
                    })
                    .collect::<Vec<_>>()
                    .join("\n\n");
                page.apply_detail(0, Ok(serde_json::json!({"body":body})), cx);
                assert!(page.media_inflight);
                assert_eq!(page.media_images.len(), 8);
                assert_eq!(page.media_queue.len(), 7);

                page.select(
                    Some(Identity {
                        hostname: "github.com".into(),
                        owner: "owner".into(),
                        repository: "repo".into(),
                        number: 2,
                    }),
                    cx,
                );
                let current = "https://github.com/user-attachments/assets/current";
                page.apply_detail(
                    1,
                    Ok(serde_json::json!({"body":format!("![image]({current})")})),
                    cx,
                );
                assert_eq!(page.media_queue.len(), 1);
                page.apply_media(
                    0,
                    "https://github.com/user-attachments/assets/0",
                    Err("old".into()),
                    cx,
                );
                assert!(page.media_inflight);
                assert!(page.media_queue.is_empty());
                assert_eq!(page.media_images.len(), 1);
                assert!(page.media_images[current].is_none());
                page.apply_media(1, current, Err("current".into()), cx);
                assert!(!page.media_inflight);
                assert_eq!(
                    page.media_images[current]
                        .as_ref()
                        .unwrap()
                        .as_ref()
                        .unwrap_err(),
                    "current"
                );
            });
        });
    }

    #[test]
    fn detail_pane_reserves_the_list_and_respects_short_windows() {
        for (width, height, expected) in [
            (1280., 832., 724.),
            (1024., 832., 616.),
            (1440., 300., 640.),
        ] {
            assert_eq!(
                default_detail_width(gpui::size(px(width), px(height))),
                expected,
            );
        }
    }

    #[test]
    fn diff_counts_keep_three_digit_groups() {
        assert_eq!(grouped_count(0), "0");
        assert_eq!(grouped_count(999), "999");
        assert_eq!(grouped_count(1_000), "1,000");
        assert_eq!(grouped_count(u64::MAX), "18,446,744,073,709,551,615");
    }

    #[test]
    fn compact_age_uses_the_original_units_and_capture_clock() {
        let date = "2026-10-20T00:00:00Z";
        let base = chrono::DateTime::parse_from_rfc3339(date)
            .unwrap()
            .timestamp();
        for (seconds, expected) in [
            (0, "just now"),
            (60, "1m"),
            (3600, "1h"),
            (86400, "1d"),
            (604800, "1w"),
            (2592000, "1mo"),
            (31536000, "1y"),
        ] {
            let clock = Clock {
                utc_offset: 0,
                pinned_now: Some(base + seconds),
                elapsed_since: None,
            };
            assert_eq!(relative_age(date, clock), expected);
        }
    }

    #[test]
    fn github_media_keeps_its_markdown_position_and_ignores_code_blocks() {
        let body = "Before ![image](https://github.com/user-attachments/assets/one) after.\n\n`![code](https://github.com/user-attachments/assets/two)`";
        let spans = media_spans(body);
        assert_eq!(spans.len(), 1);
        assert_eq!(
            &body[spans[0].0.clone()],
            "![image](https://github.com/user-attachments/assets/one)"
        );
        assert_eq!(spans[0].1, "https://github.com/user-attachments/assets/one");
    }

    #[test]
    fn html_image_attributes_keep_quoted_boundaries_and_validate_the_host() {
        let body = r#"<img alt="before > after" width=400 src='https://github.com/user-attachments/assets/one?a=1&amp;b=2' />"#;
        let spans = media_spans(body);
        assert_eq!(spans.len(), 1);
        assert_eq!(
            spans[0].1,
            "https://github.com/user-attachments/assets/one?a=1&b=2"
        );
        assert!(media_spans("<img src='https://example.com/image.png'>").is_empty());
    }

    #[test]
    fn video_attachments_use_the_original_unavailable_preview_path() {
        let url = "https://github.com/user-attachments/assets/video";
        for body in [
            url.to_owned(),
            format!("[Video]({url})"),
            format!("<video src='{url}' controls></video>"),
        ] {
            let spans = media_spans(&body);
            assert_eq!(spans.len(), 1, "{body}");
            assert_eq!(spans[0].1, url);
        }
        let video =
            serde_json::json!({"status":"success", "mimeType":"video/mp4", "contentsBase64":""});
        assert_eq!(
            decode_image_preview(video).err().as_deref(),
            Some("Preview unavailable")
        );
        assert!(media_spans(&format!("Text before [video]({url})")).is_empty());
    }

    #[test]
    fn oversized_preview_is_rejected_before_pixel_decoding() {
        let value = serde_json::json!({
            "mimeType": "image/png",
            "contentsBase64": "iVBORw0KGgoAAAANSUhEUgAATiAAAE4gCAYAAADjcEY5AAAACElEQVR4nAMAAAAAAUgGidIAAAAASUVORK5CYII="
        });
        assert_eq!(
            decode_image_preview(value).unwrap_err(),
            "Preview exceeds the image memory limit"
        );
    }
}
