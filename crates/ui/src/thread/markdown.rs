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

use pulldown_cmark::{Event, HeadingLevel, Options, Parser, Tag, TagEnd};
use std::iter::Peekable;

const OPTIONS: Options = Options::ENABLE_TASKLISTS.union(Options::ENABLE_STRIKETHROUGH);

/// The text of `event` in [`plain`]: block ends and rules become line breaks so that a find
/// query never matches across blocks.
fn plain_text<'a>(event: &'a Event<'_>) -> Option<&'a str> {
    match event {
        Event::Text(s) | Event::Html(s) | Event::InlineHtml(s) | Event::Code(s) => Some(s),
        Event::SoftBreak => Some(" "),
        Event::HardBreak | Event::Rule => Some("\n"),
        Event::End(end) => (!matches!(
            end,
            TagEnd::Emphasis
                | TagEnd::Strong
                | TagEnd::Strikethrough
                | TagEnd::Link
                | TagEnd::Image
                | TagEnd::Subscript
                | TagEnd::Superscript
        ))
        .then_some("\n"),
        _ => None,
    }
}

/// The searchable text of an answer: the byte offsets the find matches use.
pub(super) fn plain(source: &str) -> String {
    Parser::new_ext(source, OPTIONS)
        .filter_map(|event| plain_text(&event).map(str::to_owned))
        .collect()
}

/// The parser events; `start` is the [`plain`] offset of the last consumed one.
struct Events<'a> {
    inner: Peekable<Parser<'a>>,
    offset: usize,
    start: usize,
}

impl<'a> Events<'a> {
    fn peek(&mut self) -> Option<&Event<'a>> {
        self.inner.peek()
    }

    fn next(&mut self) -> Option<Event<'a>> {
        let event = self.inner.next()?;
        self.start = self.offset;
        self.offset += plain_text(&event).map_or(0, str::len);
        Some(event)
    }
}

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
    with_find(source, window, t, None)
}

/// [`markdown`] with the find matches of this answer highlighted.
pub(super) fn with_find(source: &str, window: &Window, t: &Theme, find: Option<&FindPaint>) -> Div {
    Markdown {
        events: Events {
            inner: Parser::new_ext(source, OPTIONS).peekable(),
            offset: 0,
            start: 0,
        },
        window,
        theme: t,
        find,
    }
    .blocks(Typography::default(), 0)
    .0
}

struct Markdown<'a> {
    events: Events<'a>,
    window: &'a Window,
    theme: &'a Theme,
    find: Option<&'a FindPaint>,
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
                    (wrap_row().children(row), 7., 3.5)
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
                    (wrap_row().children(row), top, bottom)
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
                    (wrap_row().children(row), 0., 0.)
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
                div()
                    .flex()
                    .justify_end()
                    .child(self.word(&format!("{n}. "), None, style))
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
                Event::Text(s) | Event::Html(s) | Event::InlineHtml(s) => {
                    if self.find.is_none()
                        && style.size == 14.
                        && style.line_height == 22.75
                        && !style.italic
                        && !style.strike
                    {
                        row.extend(
                            super::items::words(
                                &s,
                                style.weight,
                                style.color.unwrap_or(self.theme.text),
                                self.window,
                            )
                            .into_iter()
                            .map(IntoElement::into_any_element),
                        );
                    } else {
                        let mut at = self.events.start;
                        row.extend(s.split_inclusive(' ').map(|s| {
                            let here = at;
                            at += s.len();
                            self.word(s, Some(here), style).into_any_element()
                        }));
                    }
                }
                Event::SoftBreak => {
                    let at = self.events.start;
                    row.push(self.word(" ", Some(at), style).into_any_element());
                }
                Event::HardBreak => row.push(div().w_full().h_0().into_any_element()),
                Event::Code(code) => {
                    let at = self.events.start;
                    row.push(self.code(&code, at).into_any_element());
                }
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

    fn word(&self, s: &str, at: Option<usize>, style: Typography) -> Div {
        let color = style.color.unwrap_or(self.theme.text);
        let find = self.find.zip(at);
        if !style.italic && !style.strike && find.is_none() {
            return measured(
                self.window,
                s.to_owned(),
                style.size,
                style.line_height,
                style.weight,
                color,
                None,
            );
        }
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
        let label = run_style(style.size, style.line_height, style.weight, color)
            .when(style.italic, |d| d.italic())
            .when(style.strike, |d| d.line_through());
        div().flex_none().w(width).child(match find {
            Some((find, at)) => label.child(find.text(s, at)),
            None => label.child(s.to_owned()),
        })
    }

    fn code(&self, code: &str, at: usize) -> Div {
        let label = match self.find {
            Some(find) => div()
                .flex_none()
                .w(width(self.window, code, 12.88, 430., Some(MONO)))
                .child(
                    run_style(12.88, 15., 430., self.theme.text)
                        .font_family(MONO)
                        .child(find.text(code, at)),
                ),
            None => measured(
                self.window,
                code.to_owned(),
                12.88,
                15.,
                430.,
                self.theme.text,
                Some(MONO),
            ),
        };
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
                .child(label),
        )
    }
}

/// The text style of [`text`] without its content: the find highlights go in as `StyledText`.
fn run_style(size: f32, line_height: f32, weight: f32, color: Hsla) -> Div {
    div()
        .text_size(px(size))
        .line_height(px(line_height))
        .font_weight(FontWeight(weight))
        .text_color(color)
        .whitespace_nowrap()
}
