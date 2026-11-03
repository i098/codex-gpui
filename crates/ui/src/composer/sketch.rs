//! `Sketch` (`/sketch` and the `@` menu's `Draw a sketch`): a drawing canvas over the composer.
//! `Attach` rasterizes the strokes to `Codex Sketch.png` and attaches it like a pasted image.
use super::*;
use gpui::{MouseButton, MouseDownEvent, MouseMoveEvent, PathBuilder, canvas, point};
use std::{cell::Cell, rc::Rc};

const HEIGHT: f32 = 280.;
const PEN: f32 = 3.;

#[derive(Default)]
pub(super) struct Sketch {
    /// Each stroke's points, relative to the canvas's top-left corner.
    strokes: Vec<Vec<Point<Pixels>>>,
    drawing: bool,
    /// The canvas's window bounds as last painted.
    bounds: Rc<Cell<Bounds<Pixels>>>,
}

impl Sketch {
    /// The strokes as a white PNG with black lines, at the canvas's size.
    fn png(&self) -> anyhow::Result<Vec<u8>> {
        let size = self.bounds.get().size;
        let (width, height) = (
            f32::from(size.width).max(1.),
            f32::from(size.height).max(1.),
        );
        let mut image = image::RgbaImage::from_pixel(
            width as u32,
            height as u32,
            image::Rgba([255, 255, 255, 255]),
        );
        let radius = PEN / 2.;
        let mut dot = |x: f32, y: f32| {
            let (left, top) = ((x - radius).floor() as i64, (y - radius).floor() as i64);
            for py in top..=(y + radius).ceil() as i64 {
                for px in left..=(x + radius).ceil() as i64 {
                    let inside = (px as f32 + 0.5 - x).hypot(py as f32 + 0.5 - y) <= radius;
                    if inside && px >= 0 && py >= 0 && px < width as i64 && py < height as i64 {
                        image.put_pixel(px as u32, py as u32, image::Rgba([0, 0, 0, 255]));
                    }
                }
            }
        };
        for stroke in &self.strokes {
            let mut points = stroke.iter().map(|p| (f32::from(p.x), f32::from(p.y)));
            let Some(mut last) = points.next() else {
                continue;
            };
            dot(last.0, last.1);
            for next in points {
                let steps = (next.0 - last.0).hypot(next.1 - last.1).ceil().max(1.);
                for step in 1..=steps as usize {
                    let k = step as f32 / steps;
                    dot(
                        last.0 + (next.0 - last.0) * k,
                        last.1 + (next.1 - last.1) * k,
                    );
                }
                last = next;
            }
        }
        let mut bytes = Vec::new();
        image.write_to(
            &mut std::io::Cursor::new(&mut bytes),
            image::ImageFormat::Png,
        )?;
        Ok(bytes)
    }
}

impl Composer {
    /// Opens the canvas; the `/sketch` or `@` token is removed by the caller.
    pub(super) fn open_sketch(&mut self, cx: &mut Context<Self>) {
        self.sketch = Some(Sketch::default());
        cx.notify();
    }

    pub fn sketch_open(&self) -> bool {
        self.sketch.is_some()
    }

    /// Adds a stroke through `points` (canvas coordinates), as a drag would.
    pub fn sketch_stroke(&mut self, points: &[(f32, f32)], cx: &mut Context<Self>) {
        if let Some(sketch) = &mut self.sketch {
            sketch
                .strokes
                .push(points.iter().map(|&(x, y)| point(px(x), px(y))).collect());
            cx.notify();
        }
    }

    /// `Attach`: the PNG joins the attachments and the canvas closes; an empty canvas only closes.
    pub fn attach_sketch(&mut self, cx: &mut Context<Self>) {
        let Some(sketch) = self.sketch.take() else {
            return;
        };
        if !sketch.strokes.is_empty() {
            match sketch.png() {
                Ok(bytes) => self.attach_png(bytes, cx),
                Err(error) => eprintln!("codex-gpui: cannot save the sketch: {error:#}"),
            }
        }
        cx.notify();
    }

    pub(super) fn sketch_panel(&self, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let sketch = self.sketch.as_ref().expect("open sketch");
        let strokes = sketch.strokes.clone();
        let bounds = sketch.bounds.clone();
        let button = |id: &'static str, label: &'static str| {
            div()
                .id(id)
                .px(px(10.))
                .h(px(28.))
                .flex()
                .items_center()
                .rounded(Theme::RADIUS_MD)
                .cursor_pointer()
                .hover(|s| s.bg(t.text.opacity(0.06)))
                .aria_label(label)
                .role(gpui::Role::Button)
                .child(text(label, 13., 18., 430., t.text))
        };
        let surface = div()
            .id("sketch-canvas")
            .h(px(HEIGHT))
            .w_full()
            .rounded(Theme::RADIUS_MD)
            .bg(gpui::white())
            .cursor_crosshair()
            .child(
                canvas(
                    move |b, _, _| bounds.set(b),
                    move |b, _, window, _| {
                        for stroke in &strokes {
                            let mut path = PathBuilder::stroke(px(PEN));
                            let mut points = stroke.iter().map(|&p| b.origin + p);
                            let Some(first) = points.next() else { continue };
                            path.move_to(first);
                            // A single click paints a dot: a zero-length segment.
                            path.line_to(first + point(px(0.01), px(0.)));
                            for p in points {
                                path.line_to(p);
                            }
                            if let Ok(path) = path.build() {
                                window.paint_path(path, gpui::black());
                            }
                        }
                    },
                )
                .size_full(),
            )
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, e: &MouseDownEvent, _, cx| {
                    if let Some(sketch) = &mut this.sketch {
                        let at = e.position - sketch.bounds.get().origin;
                        sketch.strokes.push(vec![at]);
                        sketch.drawing = true;
                        cx.stop_propagation();
                        cx.notify();
                    }
                }),
            )
            .on_mouse_move(cx.listener(|this, e: &MouseMoveEvent, _, cx| {
                if let Some(sketch) = &mut this.sketch
                    && sketch.drawing
                {
                    let at = e.position - sketch.bounds.get().origin;
                    if let Some(stroke) = sketch.strokes.last_mut() {
                        stroke.push(at);
                    }
                    cx.notify();
                }
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, _, _| {
                    if let Some(sketch) = &mut this.sketch {
                        sketch.drawing = false;
                    }
                }),
            );
        let toolbar = div()
            .flex()
            .items_center()
            .gap(px(4.))
            .child(div().flex_1().child(text("Sketch", 13., 18., 530., t.text)))
            .child(
                button("sketch-clear", "Clear").on_click(cx.listener(|this, _, _, cx| {
                    if let Some(sketch) = &mut this.sketch {
                        sketch.strokes.clear();
                        cx.notify();
                    }
                })),
            )
            .child(
                button("sketch-cancel", "Cancel").on_click(cx.listener(|this, _, _, cx| {
                    this.sketch = None;
                    cx.notify();
                })),
            )
            .child(
                button("sketch-attach", "Attach")
                    .on_click(cx.listener(|this, _, _, cx| this.attach_sketch(cx))),
            );
        deferred(
            div()
                .absolute()
                .bottom(relative(1.))
                .mb(px(8.))
                .w_full()
                .child(
                    div()
                        .id("sketch")
                        .occlude()
                        .p(px(8.))
                        .flex()
                        .flex_col()
                        .gap(px(8.))
                        .border_1()
                        .border_color(t.text.opacity(0.082))
                        .rounded(px(20.))
                        .bg(t.surface_elevated_secondary)
                        .child(toolbar)
                        .child(surface),
                ),
        )
        .with_priority(2)
        .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn png_draws_the_strokes_on_white() -> anyhow::Result<()> {
        let sketch = Sketch {
            strokes: vec![vec![point(px(4.), px(5.)), point(px(16.), px(5.))]],
            ..Default::default()
        };
        let size = gpui::size(px(20.), px(10.));
        sketch.bounds.set(Bounds::new(point(px(0.), px(0.)), size));
        let image = image::load_from_memory(&sketch.png()?)?.to_rgba8();
        assert_eq!(image.dimensions(), (20, 10));
        assert_eq!(image.get_pixel(10, 5).0, [0, 0, 0, 255]);
        assert_eq!(image.get_pixel(10, 0).0, [255, 255, 255, 255]);
        assert_eq!(image.get_pixel(0, 5).0, [255, 255, 255, 255]);
        Ok(())
    }
}
