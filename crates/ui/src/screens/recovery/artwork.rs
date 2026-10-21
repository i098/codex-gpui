#[cfg(not(target_os = "macos"))]
use gpui::Styled as _;
use gpui::{AnyElement, Hsla, IntoElement};
use std::sync::{Arc, LazyLock};

#[cfg(target_os = "macos")]
use gpui::{
    App, Bounds, Corners, Element, ElementId, GlobalElementId, InspectorElementId, LayoutId,
    Pixels, RenderImage, Window, point, px, size,
};
#[cfg(target_os = "macos")]
use std::{cell::RefCell, rc::Rc};

#[cfg(target_os = "macos")]
type Cached = (Bounds<Pixels>, f32, Hsla, Bounds<Pixels>, Arc<RenderImage>);

#[derive(Default)]
pub(super) struct Artwork {
    #[cfg(target_os = "macos")]
    cache: Rc<RefCell<Option<Cached>>>,
}

impl Artwork {
    pub fn view(&self, page: Hsla) -> AnyElement {
        #[cfg(target_os = "macos")]
        {
            Raster {
                page,
                cache: self.cache.clone(),
            }
            .into_any_element()
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = page;
            static IMAGE: LazyLock<Arc<gpui::Image>> = LazyLock::new(|| {
                Arc::new(gpui::Image::from_bytes(
                    gpui::ImageFormat::Png,
                    include_bytes!("../../../assets/recovery-pet.png").to_vec(),
                ))
            });
            gpui::img(IMAGE.clone()).size_full().into_any_element()
        }
    }
}

#[cfg(target_os = "macos")]
struct Raster {
    page: Hsla,
    cache: Rc<RefCell<Option<Cached>>>,
}

#[cfg(target_os = "macos")]
impl IntoElement for Raster {
    type Element = Self;
    fn into_element(self) -> Self {
        self
    }
}

#[cfg(target_os = "macos")]
impl Element for Raster {
    type RequestLayoutState = LayoutId;
    type PrepaintState = Bounds<Pixels>;

    fn id(&self) -> Option<ElementId> {
        None
    }
    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, LayoutId) {
        let style = gpui::Style {
            size: size(px(160.).into(), px(160.).into()),
            ..Default::default()
        };
        let id = window.request_layout(style, [], cx);
        (id, id)
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        mut bounds: Bounds<Pixels>,
        id: &mut LayoutId,
        window: &mut Window,
        _: &mut App,
    ) -> Bounds<Pixels> {
        bounds.origin += point(
            window.layout_origin_residual_x(*id),
            window.layout_origin_residual_y(*id),
        );
        bounds
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut LayoutId,
        bounds: &mut Bounds<Pixels>,
        window: &mut Window,
        _: &mut App,
    ) {
        static PIXELS: LazyLock<image::RgbaImage> = LazyLock::new(|| {
            let mut image =
                image::load_from_memory(include_bytes!("../../../assets/recovery-pet.png"))
                    .expect("embedded recovery artwork")
                    .into_rgba8();
            shapes::premultiply(&mut image);
            image
        });
        let bounds = *bounds;
        let scale = window.scale_factor();
        let changed =
            self.cache
                .borrow()
                .as_ref()
                .is_none_or(|(old, old_scale, old_page, _, _)| {
                    *old != bounds || *old_scale != scale || *old_page != self.page
                });
        if changed {
            let fit = (f32::from(bounds.size.width) / PIXELS.width() as f32)
                .min(f32::from(bounds.size.height) / PIXELS.height() as f32);
            let image_size = size(
                px(PIXELS.width() as f32 * fit),
                px(PIXELS.height() as f32 * fit),
            );
            let image_bounds = Bounds::new(
                bounds.center() - point(image_size.width / 2., image_size.height / 2.),
                image_size,
            );
            let Some(image) = shapes::raster_images(
                &[(&PIXELS, image_bounds)],
                bounds,
                Corners::default(),
                shapes::Corner::Round,
                shapes::ImageComposition::Direct(self.page),
                window,
            ) else {
                return;
            };
            let snap = |value: Pixels| (f32::from(value) * scale).round() / scale;
            let origin = point(px(snap(bounds.left())), px(snap(bounds.top())));
            let at = Bounds::new(
                origin,
                size(
                    px(snap(bounds.right()) - f32::from(origin.x)),
                    px(snap(bounds.bottom()) - f32::from(origin.y)),
                ),
            );
            *self.cache.borrow_mut() = Some((bounds, scale, self.page, at, Arc::new(image)));
        }
        if let Some((_, _, _, at, image)) = self.cache.borrow().as_ref() {
            window
                .paint_image(*at, *at, Corners::default(), image.clone(), 0, false)
                .ok();
        }
    }
}
