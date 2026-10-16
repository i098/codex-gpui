use super::*;
use gpui::AnyElement;

/// Colors the original mixes in CSS that have no theme token.
pub(super) struct Tones {
    /// `bg-token-…/50` resource card, translucent in the conversation's layer.
    pub(super) card: Hsla,
    pub(super) inline_code: Hsla,
    pub(super) mention: Hsla,
    pub(super) secondary_button: Hsla,
}

pub(super) fn tones(t: &Theme) -> Tones {
    match t.appearance {
        // `--color-token-text-code-block-background`-style mixes, from the specs' computed styles.
        Appearance::Dark => Tones {
            card: Hsla {
                a: 0.5,
                ..t.control_background
            },
            inline_code: Hsla {
                a: 0.107_059,
                ..t.text
            },
            mention: rgb(0x5685d1).into(),
            secondary_button: rgba(0xffffff08).into(),
        },
        Appearance::Light => Tones {
            // oklab(white / 0.5): sRGB red just above 1 (Skia's float blit).
            card: gpui::Rgba {
                r: 1.00003,
                g: 0.99999,
                b: 1.,
                a: 0.5,
            }
            .into(),
            inline_code: Hsla {
                a: 0.092_941,
                ..t.text
            },
            mention: rgb(0x2858a4).into(),
            secondary_button: rgba(0xfffffff5).into(),
        },
    }
}

use pulldown_cmark::{Event, HeadingLevel, Options, Parser, Tag};
use std::iter::Peekable;

#[derive(Clone, Copy)]
struct Typography {
    size: f32,
    line_height: f32,
    weight: f32,
    italic: bool,
    strike: bool,
    color: Option<Hsla>,
}

impl Default for Typography {
    fn default() -> Self {
        Self {
            size: 14.,
            line_height: 22.75,
            weight: 430.,
            italic: false,
            strike: false,
            color: None,
        }
    }
}

/// GFM blocks share the existing code chips and file links.
/// Task markers are read-only, as in the original assistant messages.
pub(super) fn markdown(source: &str, window: &Window, t: &Theme) -> Div {
    Markdown {
        events: Parser::new_ext(
            source,
            Options::ENABLE_TASKLISTS | Options::ENABLE_STRIKETHROUGH,
        )
        .peekable(),
        window,
        theme: t,
    }
    .blocks(Typography::default(), 0)
    .0
}

struct Markdown<'a> {
    events: Peekable<Parser<'a>>,
    window: &'a Window,
    theme: &'a Theme,
}

impl Markdown<'_> {
    fn blocks(&mut self, style: Typography, depth: usize) -> (Div, bool) {
        let mut column = div().flex().flex_col();
        let mut first = true;
        let mut bottom: f32 = 0.;
        let mut previous_paragraph = false;
        let mut task = false;
        while let Some(event) = self.events.peek() {
            if matches!(event, Event::End(_)) {
                break;
            }
            let paragraph = matches!(event, Event::Start(Tag::Paragraph));
            let (view, top, next_bottom) = match event {
                Event::Start(Tag::Paragraph) => {
                    self.events.next();
                    let (row, checked) = self.inline(style);
                    task |= checked;
                    self.events.next();
                    (block(row, 2), 7., 3.5)
                }
                Event::Start(Tag::Heading { level, .. }) => {
                    let (size, line_height, top, bottom) = match level {
                        HeadingLevel::H1 => (21., 28., 21., 7.),
                        HeadingLevel::H2 => (17.5, 24.5, 14., 3.5),
                        HeadingLevel::H3 => (15.75, 24.5, 14., 3.5),
                        HeadingLevel::H4 => (14., 21., 14., 0.),
                        HeadingLevel::H5 | HeadingLevel::H6 => (14., 22.75, 0., 0.),
                    };
                    self.events.next();
                    let row = self
                        .inline(Typography {
                            size,
                            line_height,
                            weight: 600.,
                            ..style
                        })
                        .0;
                    self.events.next();
                    (block(row, 2), top, bottom)
                }
                Event::Start(Tag::List(start)) => {
                    let start = *start;
                    self.events.next();
                    (self.list(start, style, depth), 0., 0.)
                }
                Event::Start(Tag::BlockQuote(_)) => {
                    self.events.next();
                    let quote = self
                        .blocks(
                            Typography {
                                weight: 400.,
                                ..style
                            },
                            depth,
                        )
                        .0;
                    self.events.next();
                    let rule = div()
                        .absolute()
                        .left_0()
                        .top(px(7.))
                        .bottom(px(7.))
                        .w(px(3.5))
                        .rounded(px(1.75))
                        .bg(self.theme.border_strong);
                    (
                        div()
                            .relative()
                            .py(px(7.))
                            .pl(px(21.))
                            .child(rule)
                            .child(quote),
                        0.,
                        7.,
                    )
                }
                Event::Rule => {
                    self.events.next();
                    (
                        div().h(px(1.)).w_full().bg(self.theme.border_strong),
                        24.5,
                        24.5,
                    )
                }
                Event::Start(_) => {
                    self.events.next();
                    let child = self.blocks(style, depth).0;
                    self.events.next();
                    (child, 0., 0.)
                }
                _ => {
                    let (row, checked) = self.inline(style);
                    task |= checked;
                    (block(row, 1), 0., 0.)
                }
            };
            let gap = if first || style.weight == 400. {
                0.
            } else if paragraph && previous_paragraph {
                // Keep the established spacing of plain answer paragraphs.
                12.
            } else {
                bottom.max(top)
            };
            column = column.child(div().mt(px(gap)).child(view));
            first = false;
            bottom = next_bottom;
            previous_paragraph = paragraph;
        }
        (column, task)
    }

    fn list(&mut self, mut number: Option<u64>, style: Typography, depth: usize) -> Div {
        let mut list = div().flex().flex_col();
        while matches!(self.events.peek(), Some(Event::Start(Tag::Item))) {
            self.events.next();
            let (body, task) = self.blocks(style, depth + 1);
            self.events.next();
            let marker = if let Some(n) = number {
                number = Some(n + 1);
                div().flex().justify_end().child(measured(
                    self.window,
                    format!("{n}. "),
                    style.size,
                    style.line_height,
                    style.weight,
                    style.color.unwrap_or(self.theme.text),
                    None,
                ))
            } else {
                let size = if depth == 1 { 6. } else { 5. };
                div().child(
                    div()
                        .ml(px(6.25))
                        .mt(px(8.75))
                        .size(px(size))
                        .when(depth < 2, |d| d.rounded_full())
                        .when(depth == 1, |d| d.border_1().border_color(self.theme.text))
                        .when(depth != 1, |d| d.bg(self.theme.text)),
                )
            };
            list = list.child(if task {
                body
            } else {
                div()
                    .flex()
                    .child(marker.w(px(22.75)).flex_none())
                    .child(div().pl(px(5.25)).flex_1().min_w_0().child(body))
            });
        }
        self.events.next();
        list
    }

    fn inline(&mut self, style: Typography) -> (Vec<AnyElement>, bool) {
        let mut row = Vec::new();
        let mut task = false;
        while let Some(event) = self.events.peek() {
            match event {
                Event::End(_)
                | Event::Start(
                    Tag::List(_)
                    | Tag::BlockQuote(_)
                    | Tag::Paragraph
                    | Tag::Heading { .. }
                    | Tag::CodeBlock(_),
                )
                | Event::Rule => break,
                _ => {}
            }
            match self.events.next().expect("peeked event") {
                Event::Text(s) | Event::Html(s) | Event::InlineHtml(s) => row.extend(
                    s.split_inclusive(' ')
                        .map(|s| self.word(s, style).into_any_element()),
                ),
                Event::SoftBreak => row.push(self.word(" ", style).into_any_element()),
                Event::HardBreak => row.push(block_end(1).w_full().h_0().into_any_element()),
                Event::Code(code) => row.push(self.code(&code).into_any_element()),
                Event::Start(tag) => {
                    let next = match tag {
                        Tag::Strong => Typography {
                            weight: 600.,
                            ..style
                        },
                        Tag::Emphasis => Typography {
                            italic: true,
                            ..style
                        },
                        Tag::Strikethrough => Typography {
                            strike: true,
                            ..style
                        },
                        _ => style,
                    };
                    if let Tag::Link { dest_url, .. } = tag {
                        let label = self
                            .inline(Typography {
                                weight: 500.,
                                color: Some(tones(self.theme).mention),
                                ..style
                            })
                            .0;
                        row.push(
                            div()
                                .id(SharedString::from(format!("file-link-{dest_url}")))
                                .relative()
                                .flex()
                                .on_click(crate::dispatch::<crate::OpenChangesPanel>)
                                .when(dest_url.ends_with(".py"), |d| {
                                    d.child(div().w(px(19.))).child(
                                        div().absolute().left_0().bottom(px(3.37)).child(icon(
                                            "thread-lang-python",
                                            16.,
                                            tones(self.theme).mention,
                                        )),
                                    )
                                })
                                .children(label)
                                .into_any_element(),
                        );
                    } else {
                        let (children, checked) = self.inline(next);
                        row.extend(children);
                        task |= checked;
                    }
                    self.events.next();
                }
                Event::TaskListMarker(checked) => {
                    task = true;
                    row.push(
                        div()
                            .w(px(20.))
                            .h(px(style.line_height))
                            .pt(px(if checked { 5. } else { 4. }))
                            .flex_none()
                            .child(
                                div()
                                    .size(px(14.))
                                    .rounded(px(5.))
                                    .border_1()
                                    .border_color(self.theme.border_strong)
                                    .when(checked, |d| {
                                        d.bg(Hsla {
                                            a: 0.96,
                                            ..self.theme.control_background
                                        })
                                        .child(text(
                                            "✓",
                                            12.,
                                            12.,
                                            430.,
                                            self.theme.text,
                                        ))
                                    }),
                            )
                            .into_any_element(),
                    );
                }
                _ => {}
            }
        }
        (row, task)
    }

    /// A word box with its selection highlight.
    fn word(&self, s: &str, style: Typography) -> Div {
        let color = style.color.unwrap_or(self.theme.text);
        let inner = if !style.italic && !style.strike {
            measured(
                self.window,
                s.to_owned(),
                style.size,
                style.line_height,
                style.weight,
                color,
                None,
            )
        } else {
            let font = gpui::Font {
                weight: FontWeight(style.weight),
                style: if style.italic {
                    gpui::FontStyle::Italic
                } else {
                    gpui::FontStyle::Normal
                },
                ..font(SANS)
            };
            let run = TextRun {
                len: s.len(),
                font,
                color,
                background_color: None,
                underline: None,
                strikethrough: None,
            };
            let width = self
                .window
                .text_system()
                .layout_line(s, px(style.size), &[run], None)
                .width;
            div().flex_none().w(width).child(
                text(
                    s.to_owned(),
                    style.size,
                    style.line_height,
                    style.weight,
                    color,
                )
                .when(style.italic, |d| d.italic())
                .when(style.strike, |d| d.line_through()),
            )
        };
        div()
            .relative()
            .flex_none()
            .child(
                highlight(s.to_owned().into(), style.size, style.weight, None, 0.)
                    .absolute()
                    .inset_0(),
            )
            .child(inner)
    }

    fn code(&self, code: &str) -> Div {
        div().pt(px(3.5)).pb(px(3.25)).child(
            div()
                .relative()
                .h(px(17.))
                .py(px(1.))
                .px(px(6.))
                .child(shapes::decoration(shapes::Decoration {
                    radius: Corners::all(px(6.)),
                    fill: Some(tones(self.theme).inline_code),
                    ..Default::default()
                }))
                // Over the chip's fill, under its glyphs, the height of the line box.
                .child(
                    highlight(code.to_owned().into(), 12.88, 430., Some(MONO), 6.)
                        .absolute()
                        .left_0()
                        .right_0()
                        .top(px(-3.5))
                        .bottom(px(-3.25)),
                )
                .child(measured(
                    self.window,
                    code.to_owned(),
                    12.88,
                    15.,
                    430.,
                    self.theme.text,
                    Some(MONO),
                )),
        )
    }
}

/// A wrapped inline row that ends a block: `newlines` is what Chrome's `Selection.toString`
/// emits after it (two for paragraphs and headings).
fn block(row: Vec<AnyElement>, newlines: usize) -> Div {
    wrap_row().children(row).child(block_end(newlines))
}
