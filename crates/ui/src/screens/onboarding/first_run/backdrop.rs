use super::noise::Noise3d;
use crate::screens::thread::view::Clock;
use gpui::{
    App, Div, ParentElement, SharedString, Styled, TextAlign, TextRun, Window, canvas, div, fill,
    font, point, px,
};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use theme::ActiveTheme as _;

const COLUMNS: usize = 130;
const ROWS: usize = 100;
const CHARACTERS: &[u8] = b"@%#*+=-:. ";

pub(in crate::screens::onboarding) struct Backdrop {
    noise: Noise3d,
    pixels: Vec<f64>,
    frame: SharedString,
    phase: f64,
    animate: bool,
    updated: Instant,
}

impl Backdrop {
    pub(in crate::screens::onboarding) fn new(clock: Option<Clock>) -> Self {
        let seed = clock.and_then(|clock| clock.pinned_now).map_or_else(
            || {
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map_or(0, |time| time.as_millis() as u32)
            },
            |seconds| seconds.wrapping_mul(1000) as u32,
        );
        Self {
            noise: Noise3d::new(seed),
            pixels: vec![0.; COLUMNS * ROWS],
            frame: SharedString::default(),
            phase: 0.,
            // A pinned fixture clock holds the frame before the first tick, as the original captures do.
            animate: clock.is_none_or(|clock| clock.pinned_now.is_none()),
            updated: Instant::now(),
        }
    }

    fn update_frame(&mut self) {
        for y in 0..ROWS {
            for x in 0..COLUMNS {
                self.pixels[y * COLUMNS + x] =
                    ((self
                        .noise
                        .sample(x as f64 * 0.15, y as f64 * 0.15, self.phase)
                        + 1.)
                        / 2.
                        * 255.)
                        .round();
            }
        }
        for y in 0..ROWS {
            for x in 0..COLUMNS {
                let index = y * COLUMNS + x;
                let old = self.pixels[index];
                let quantized = (old / 255. * 9.).round() * (255. / 9.);
                self.pixels[index] = quantized;
                let error = old - quantized;
                if x + 1 < COLUMNS {
                    self.pixels[index + 1] += error * 7. / 16.;
                }
                if y + 1 < ROWS {
                    if x > 0 {
                        self.pixels[index + COLUMNS - 1] += error * 3. / 16.;
                    }
                    self.pixels[index + COLUMNS] += error * 5. / 16.;
                    if x + 1 < COLUMNS {
                        self.pixels[index + COLUMNS + 1] += error / 16.;
                    }
                }
            }
        }
        let mut frame = String::with_capacity((COLUMNS + 1) * ROWS);
        for (row_index, row) in self.pixels.as_chunks::<COLUMNS>().0.iter().enumerate() {
            if row_index > 0 {
                frame.push('\n');
            }
            for value in row {
                let index = ((1. - value.clamp(0., 255.) / 255.) * 9.).round() as usize;
                frame.push(char::from(CHARACTERS[index]));
            }
        }
        self.frame = frame.into();
    }

    pub(super) fn render(&mut self, window: &mut Window, cx: &App) -> Div {
        if self.animate {
            if self.updated.elapsed() >= Duration::from_millis(50) {
                self.update_frame();
                self.phase += 0.03;
                self.updated = Instant::now();
            }
            window.request_animation_frame();
        }
        let text = self.frame.clone();
        let background = cx.theme().surface_secondary;
        let foreground = cx.theme().border;
        div()
            .absolute()
            .top_0()
            .bottom_0()
            .left(px(-24.))
            .right_0()
            .layer()
            .opacity(0.15)
            .overflow_hidden()
            .child(
                canvas(
                    move |bounds, window, _| {
                        let scale = (f32::from(bounds.size.width) / (COLUMNS as f32 * 7.))
                            .max(f32::from(bounds.size.height) / (ROWS as f32 * 13.))
                            * 1.02;
                        let origin = bounds.center()
                            - point(
                                px(COLUMNS as f32 * 7. * scale / 2.),
                                px(ROWS as f32 * 13. * scale / 2.),
                            );
                        let lines = window.text_system().shape_text(
                            text.clone(),
                            px(12. * scale),
                            &[TextRun {
                                len: text.len(),
                                font: font("Menlo"),
                                color: foreground,
                                background_color: None,
                                underline: None,
                                strikethrough: None,
                            }],
                            None,
                            None,
                        );
                        (origin, px(13. * scale), lines)
                    },
                    move |bounds, (origin, line_height, lines), window, cx| {
                        window.paint_quad(fill(bounds, background));
                        match lines {
                            Ok(lines) => {
                                for (index, line) in lines.iter().enumerate() {
                                    let at = origin + point(px(0.), line_height * index as f32);
                                    if at.y + line_height >= bounds.top()
                                        && at.y <= bounds.bottom()
                                        && let Err(error) = line.paint(
                                            at,
                                            line_height,
                                            TextAlign::Left,
                                            None,
                                            window,
                                            cx,
                                        )
                                    {
                                        eprintln!(
                                            "Could not paint the first-run backdrop: {error}"
                                        );
                                    }
                                }
                            }
                            Err(error) => {
                                eprintln!("Could not shape the first-run backdrop: {error}")
                            }
                        }
                    },
                )
                .size_full(),
            )
            .child(shapes::radial_fade([(0.35, 1.), (0.6, 0.6), (0.78, 0.)]))
    }
}
