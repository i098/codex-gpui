//! Fenced blocks keep whitespace and syntax colors, with horizontal overflow and a copy button.
//! Copy excludes the fence and its final newline; the checkmark lasts two seconds.
//! HTTP(S) links open the system browser on left or middle click. File links keep their existing action.

use super::*;
use crate::code_font;
use crate::tooltip::{Side, tooltip};
use gpui::{
    AnyElement, FocusHandle, MouseButton, PathBuilder, Stateful, StyledText, canvas, point,
};
use syntect::easy::HighlightLines;
use syntect::highlighting::{Color, FontStyle, StyleModifier, ThemeItem, ThemeSettings};
use syntect::parsing::SyntaxSet;

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

use pulldown_cmark::{
    CodeBlockKind, Event, HeadingLevel, OffsetIter, Options, Parser, Tag, TagEnd,
};
use std::{iter::Peekable, ops::Range};

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

/// The parser events; `start` is the [`plain`] offset of the last consumed one and `range`
/// its source bytes.
struct Events<'a> {
    inner: Peekable<OffsetIter<'a>>,
    offset: usize,
    start: usize,
    range: Range<usize>,
}

impl<'a> Events<'a> {
    fn peek(&mut self) -> Option<&Event<'a>> {
        self.inner.peek().map(|(event, _)| event)
    }

    fn next(&mut self) -> Option<Event<'a>> {
        let (event, range) = self.inner.next()?;
        self.start = self.offset;
        self.offset += plain_text(&event).map_or(0, str::len);
        self.range = range;
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
/// `segments` are the still-fading streamed byte ranges of `source`.
pub(super) fn markdown(
    source: &str,
    window: &Window,
    t: &Theme,
    segments: &[StreamSegment],
) -> Div {
    with_find(source, window, t, None, segments)
}

/// [`markdown`] with the find matches of this answer highlighted.
pub(super) fn with_find(
    source: &str,
    window: &Window,
    t: &Theme,
    find: Option<&FindPaint>,
    segments: &[StreamSegment],
) -> Div {
    Markdown {
        events: Events {
            inner: Parser::new_ext(source, OPTIONS)
                .into_offset_iter()
                .peekable(),
            offset: 0,
            start: 0,
            range: 0..0,
        },
        window,
        theme: t,
        find,
        source,
        segments,
        block_index: 0,
        link_width: std::cell::Cell::new(0.),
        indent: std::cell::Cell::new(0.),
    }
    .blocks(Typography::default(), 0)
    .0
}

struct Markdown<'a> {
    events: Events<'a>,
    window: &'a Window,
    theme: &'a Theme,
    find: Option<&'a FindPaint>,
    source: &'a str,
    segments: &'a [StreamSegment],
    block_index: usize,
    /// Width of the words of the link being built (see `Tag::Link`).
    link_width: std::cell::Cell<f32>,
    /// Indent of the first word of a link: its leading icon.
    indent: std::cell::Cell<f32>,
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
                Event::Start(Tag::CodeBlock(kind)) => {
                    let language = match kind {
                        CodeBlockKind::Fenced(info) => {
                            info.split_whitespace().next().unwrap_or("").to_owned()
                        }
                        CodeBlockKind::Indented => String::new(),
                    };
                    self.events.next();
                    let mut code = String::new();
                    while let Some(event) = self.events.next() {
                        match event {
                            Event::End(TagEnd::CodeBlock) => break,
                            Event::Text(text) => code.push_str(&text),
                            _ => {}
                        }
                    }
                    if code.ends_with('\n') {
                        code.pop();
                    }
                    self.block_index += 1;
                    // The state key is the block's position and content, which survive the
                    // item replacement at `item/completed` and the thread's re-renders.
                    // ponytail: identical blocks at one index share a key across messages;
                    // key on the item id through `markdown()` when that shows.
                    let mut hash = DefaultHasher::new();
                    code.hash(&mut hash);
                    language.hash(&mut hash);
                    let id = format!("code-{}-{:x}", self.block_index, hash.finish()).into();
                    (
                        div().pt(px(1.)).child(CodeBlock {
                            id,
                            code: code.into(),
                            language: language.into(),
                        }),
                        7.,
                        17.5,
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
                div()
                    .flex()
                    .justify_end()
                    .child(self.word(&format!("{n}. "), None, style, None))
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
        let mut glue = false;
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
            let glued = std::mem::take(&mut glue);
            match self.events.next().expect("peeked event") {
                Event::Text(s) | Event::Html(s) | Event::InlineHtml(s) => {
                    let mut glue = glued;
                    let mut at = self.events.start;
                    let line = LINE_WIDTH.with(|w| w.get());
                    let chunks: Vec<&str> = s
                        .split_inclusive(' ')
                        .flat_map(|word| {
                            let limit = line - self.indent.take();
                            break_long(self.window, word, style.size, style.weight, None, limit)
                        })
                        .collect();
                    let mut item = ItemPen::new(self.window);
                    let count = chunks.len();
                    for (index, chunk) in chunks.into_iter().enumerate() {
                        let here = at;
                        at += chunk.len();
                        let last = index + 1 == count;
                        let word = self.word(chunk, Some(here), style, Some((&mut item, last)));
                        // No break opportunity between a chip and the text that touches it.
                        let chip = if glue && !chunk.starts_with(char::is_whitespace) {
                            row.pop()
                        } else {
                            None
                        };
                        glue = false;
                        if let Some(chip) = chip {
                            row.push(
                                div()
                                    .flex()
                                    .flex_none()
                                    .child(chip)
                                    .child(word)
                                    .into_any_element(),
                            );
                            continue;
                        }
                        row.push(word.into_any_element());
                    }
                }
                Event::SoftBreak => {
                    let at = self.events.start;
                    row.push(self.word(" ", Some(at), style, None).into_any_element());
                }
                Event::HardBreak => row.push(block_end(1).w_full().h_0().into_any_element()),
                Event::Code(code) => {
                    let at = self.events.start;
                    row.extend(
                        self.code(&code, at)
                            .into_iter()
                            .map(|part| part.into_any_element()),
                    );
                    glue = true;
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
                        let external = dest_url.split_once("://").is_some_and(|(scheme, _)| {
                            scheme.eq_ignore_ascii_case("http")
                                || scheme.eq_ignore_ascii_case("https")
                        });
                        let python = !external && dest_url.ends_with(".py");
                        let icon_width = if python { 19. } else { 0. };
                        self.link_width.set(0.);
                        self.indent.set(icon_width);
                        let label = self
                            .inline(Typography {
                                weight: 500.,
                                color: Some(tones(self.theme).mention),
                                ..style
                            })
                            .0;
                        self.indent.set(0.);
                        let line = LINE_WIDTH.with(|w| w.get());
                        // A link wider than a line flows word by word like inline text; each
                        // word keeps the link's behavior.
                        let flow = line > 0. && self.link_width.get() + icon_width > line;
                        let dest = dest_url.to_string();
                        let link_box = |id: String| {
                            let dest = dest.clone();
                            div()
                                .id(SharedString::from(id))
                                .relative()
                                .flex()
                                .when(external, |d| {
                                    let url = dest.clone();
                                    let middle_url = url.clone();
                                    let menu_url = url.clone();
                                    d.tab_index(0)
                                        .role(gpui::Role::Link)
                                        .hover(|s| s.underline())
                                        .focus_visible(|s| s.border_2().border_color(rgb(0x3a83f7)))
                                        .on_click(move |_, _, cx| cx.open_url(&url))
                                        .on_aux_click(move |event, _, cx| {
                                            if event.is_middle_click() {
                                                cx.open_url(&middle_url);
                                                cx.stop_propagation();
                                            }
                                        })
                                        .on_mouse_down(MouseButton::Right, move |_, _, _| {
                                            super::menu::note_right_clicked_link(menu_url.clone())
                                        })
                                })
                                .when(!external, |d| {
                                    // `path:12` cites a line; the thread's cwd resolves a
                                    // relative path.
                                    let (path, line) = dest
                                        .rsplit_once(':')
                                        .and_then(|(p, n)| Some((p.to_owned(), n.parse().ok()?)))
                                        .map_or((dest.clone(), None), |(p, n)| (p, Some(n)));
                                    d.on_click(move |_, window, cx| {
                                        window.dispatch_action(
                                            Box::new(crate::screens::diff::panel::OpenFile {
                                                path: path.clone(),
                                                line,
                                            }),
                                            cx,
                                        )
                                    })
                                })
                        };
                        let python_icon = |d: Stateful<Div>| {
                            d.child(div().w(px(19.))).child(
                                div().absolute().left_0().bottom(px(3.37)).child(icon(
                                    "thread-lang-python",
                                    16.,
                                    tones(self.theme).mention,
                                )),
                            )
                        };
                        if flow {
                            for (index, word) in label.into_iter().enumerate() {
                                let mut piece =
                                    link_box(format!("file-link-{dest}-{index}")).flex_none();
                                if index == 0 && python {
                                    piece = python_icon(piece);
                                }
                                row.push(piece.child(word).into_any_element());
                            }
                        } else {
                            let mut piece = link_box(format!("file-link-{dest}"));
                            if python {
                                piece = python_icon(piece);
                            }
                            row.push(piece.children(label).into_any_element());
                        }
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

    /// A word box with its selection highlight under its glyphs: one box, so a long answer
    /// lays out a node per word. In a text `item`, the box takes its share of the item's
    /// advance (`last` ends the item).
    fn word(
        &self,
        s: &str,
        at: Option<usize>,
        style: Typography,
        item: Option<(&mut ItemPen, bool)>,
    ) -> Div {
        let (width, label) = self.label(s, at, style);
        self.link_width
            .set(self.link_width.get() + f32::from(width));
        let width = match item {
            Some((item, last)) => item.share(width, last),
            None => width,
        };
        div()
            .relative()
            .flex_none()
            .w(width)
            .child(
                highlight(s.to_owned().into(), style.size, style.weight, None, 0.)
                    .absolute()
                    .inset_0(),
            )
            .child(label)
    }

    /// The word's run width and its text.
    fn label(&self, s: &str, at: Option<usize>, style: Typography) -> (Pixels, AnyElement) {
        let color = style.color.unwrap_or(self.theme.text);
        let find = self.find.zip(at);
        let fades = self.fades(s);
        if !style.italic && !style.strike && find.is_none() && fades.is_empty() {
            return (
                width(self.window, s, style.size, style.weight, None),
                text(
                    s.to_owned(),
                    style.size,
                    style.line_height,
                    style.weight,
                    color,
                )
                .into_any_element(),
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
        // ponytail: a find highlight replaces the fade of its word; both at once is rare.
        let label = match find {
            Some((find, at)) => label.child(find.text(s, at)),
            None => label.child(StyledText::new(s.to_owned()).with_highlights(fades)),
        };
        (width, label.into_any_element())
    }

    /// The fade of each still-streaming part of `text`, by its position in [`Self::source`].
    fn fades(&self, text: &str) -> Vec<(Range<usize>, gpui::HighlightStyle)> {
        if self.segments.is_empty() {
            return Vec::new();
        }
        let fade = |alpha| gpui::HighlightStyle {
            fade_out: Some(1. - alpha),
            ..Default::default()
        };
        // Borrowed parser text retains its source offsets, including code and Unicode words.
        if let Some(start) = (text.as_ptr() as usize).checked_sub(self.source.as_ptr() as usize)
            && start <= self.source.len()
            && text.len() <= self.source.len() - start
        {
            let end = start + text.len();
            let first = self
                .segments
                .partition_point(|segment| segment.range.end <= start);
            return self.segments[first..]
                .iter()
                .take_while(|segment| segment.range.start < end)
                .map(|segment| {
                    (
                        segment.range.start.max(start) - start..segment.range.end.min(end) - start,
                        fade(segment.alpha),
                    )
                })
                .collect();
        }
        // Decoded entities and normalized code appear when their final source byte arrives.
        let range = &self.events.range;
        self.segments
            .iter()
            .rev()
            .find(|segment| !range.is_empty() && segment.range.contains(&(range.end - 1)))
            .map(|segment| vec![(0..text.len(), fade(segment.alpha))])
            .unwrap_or_default()
    }

    /// How the code font's inline box differs from the default code font's (CSS aligns it by
    /// baseline): `(line growth, text baseline shift, pill height change)` in px. A font whose
    /// descent plus half-leading is larger grows the line (Courier New by about 1 px); a
    /// different ascent moves the glyphs inside the 15 px text box; the pill is the font's
    /// content area plus 1 px each side.
    fn code_metrics(&self) -> (f32, f32, f32) {
        const LINE: f32 = 22.75;
        let system = self.window.text_system();
        let metrics = |family: &'static str, size: f32| {
            let id = system.resolve_font(&font(family));
            let size = px(size);
            let ascent = f32::from(system.ascent(id, size));
            let descent = f32::from(system.descent(id, size)).abs();
            (ascent, descent)
        };
        let (sa, sd) = metrics(SANS, 14.);
        let strut = sd + (LINE - (sa + sd)) / 2.;
        let below = |(a, d): (f32, f32)| (d + (LINE - (a + d)) / 2. - strut).max(0.);
        let baseline = |(a, d): (f32, f32)| (15. - (a + d)) / 2. + a;
        let (code, mono) = (metrics(code_font(), 12.88), metrics(MONO, 12.88));
        (
            below(code) - below(mono),
            baseline(mono) - baseline(code),
            (code.0 + code.1) - (mono.0 + mono.1),
        )
    }

    fn code(&self, code: &str, at: usize) -> Vec<Div> {
        let (growth, shift, pill) = self.code_metrics();
        // The 6 px padding of the first piece comes off the line.
        let limit = LINE_WIDTH.with(|w| w.get()) - 6.;
        let chunks: Vec<&str> = code
            .split_inclusive(' ')
            .flat_map(|word| break_long(self.window, word, 12.88, 430., Some(code_font()), limit))
            .collect();
        let count = chunks.len();
        let positions = Rc::new(RefCell::new(vec![
            (Bounds::<Pixels>::default(), px(0.));
            count
        ]));
        let painted = positions.clone();
        let decoration = shapes::Decoration {
            radius: Corners::all(px(6.)),
            fill: Some(tones(self.theme).inline_code),
            ..Default::default()
        };
        let mut background = Some(
            canvas(
                |_, _, _| {},
                move |_, (), window, cx| {
                    let mut line: Option<Bounds<Pixels>> = None;
                    for (bounds, trailing_space) in painted.borrow().iter() {
                        if bounds.size.height == px(0.) {
                            continue;
                        }
                        let top = bounds.top() - px(1. + pill);
                        let right = bounds.right() - *trailing_space + px(6.);
                        if let Some(rect) = line.as_mut().filter(|rect| rect.top() == top) {
                            rect.size.width = right - rect.left();
                        } else {
                            let next = Bounds::new(
                                point(bounds.left() - px(6.), top),
                                gpui::size(right - bounds.left() + px(6.), px(17. + pill)),
                            );
                            if let Some(rect) = line.replace(next) {
                                shapes::paint(&decoration, None, rect, window, cx);
                            }
                        }
                    }
                    if let Some(rect) = line {
                        shapes::paint(&decoration, None, rect, window, cx);
                    }
                },
            )
            .absolute()
            .size_full(),
        );
        let space = width(self.window, " ", 12.88, 430., Some(code_font()));
        let mut offset = at;
        let mut item = ItemPen::new(self.window);
        let mut parts = Vec::with_capacity(count);
        for (index, word) in chunks.into_iter().enumerate() {
            let fades = self.fades(word);
            let word_width = width(self.window, word, 12.88, 430., Some(code_font()));
            let word_width = item.share(word_width, index + 1 == count);
            let label = match self.find {
                Some(find) => div().flex_none().w(word_width).child(
                    run_style(12.88, 15., 430., self.theme.text)
                        .font_family(code_font())
                        .child(find.text(word, offset)),
                ),
                None if !fades.is_empty() => div().flex_none().w(word_width).child(
                    run_style(12.88, 15., 430., self.theme.text)
                        .font_family(code_font())
                        .child(StyledText::new(word.to_owned()).with_highlights(fades)),
                ),
                None => div().flex_none().w(word_width).child(
                    text(word.to_owned(), 12.88, 15., 430., self.theme.text)
                        .font_family(code_font()),
                ),
            };
            let measured = positions.clone();
            let trailing_space = space * (word.len() - word.trim_end_matches(' ').len()) as f32;
            parts.push(
                div()
                    .pt(px(3.5 + shift))
                    .pb(px(3.25 + growth - shift - pill))
                    // ponytail: shared padding can overhang a line by 12 px; use inline layout for exact edge fits.
                    .mr(px(if index + 1 == count { 0. } else { -12. }))
                    .child(
                        div()
                            .relative()
                            .h(px(17. + pill))
                            .py(px(1.))
                            .px(px(6.))
                            .children(background.take())
                            .child(
                                highlight(
                                    word.to_owned().into(),
                                    12.88,
                                    430.,
                                    Some(code_font()),
                                    6.,
                                )
                                .absolute()
                                .left_0()
                                .right_0()
                                .top(px(-3.5))
                                .bottom(px(-3.25)),
                            )
                            .child(label)
                            .on_children_prepainted(move |bounds, _, _| {
                                if let Some(bounds) = bounds.last() {
                                    measured.borrow_mut()[index] = (*bounds, trailing_space);
                                }
                            }),
                    ),
            );
            offset += word.len();
        }
        parts
    }
}

/// Blink lays a text node (or an inline code's text) out as one item: its glyphs at their
/// float advances, its width `ceil`ed to a LayoutUnit (1/64 device px,
/// `ShapeResult::SnappedWidth`). Its word boxes take LayoutUnit-rounded shares of the advance
/// so far, the last one up to that width.
/// ponytail: a line that breaks inside the item restarts with the shares' rounding (< 1/64
/// device px), not with the item's own advance; draw words at a sub-unit offset if it shows.
pub(super) struct ItemPen {
    unit: f32,
    advance: f32,
}

impl ItemPen {
    pub(super) fn new(window: &Window) -> Self {
        Self {
            unit: window.scale_factor() * 64.,
            advance: 0.,
        }
    }

    /// The box width of the item's next word of `width`; `last` ends the item.
    pub(super) fn share(&mut self, width: Pixels, last: bool) -> Pixels {
        let start = (self.advance * self.unit).round();
        self.advance += f32::from(width);
        let end = self.advance * self.unit;
        let end = if last { end.ceil() } else { end.round() };
        px((end - start) / self.unit)
    }
}

thread_local! {
    /// Width of the turn column the markdown flows in; `0` until the first layout.
    static LINE_WIDTH: std::cell::Cell<f32> = const { std::cell::Cell::new(0.) };
}

/// Tells the markdown below how wide its lines are (set by the thread before it builds turns).
pub(super) fn set_line_width(width: Pixels) {
    LINE_WIDTH.with(|w| w.set(f32::from(width)));
}

/// The turn column's width as last laid out; `0` before the first layout.
pub(super) fn line_width() -> f32 {
    LINE_WIDTH.with(|w| w.get())
}

/// `overflow-wrap: break-word`: a word wider than a whole line breaks into chunks that each fill
/// a line; the last chunk flows on with the following words.
fn break_long<'s>(
    window: &Window,
    word: &'s str,
    size: f32,
    weight: f32,
    family: Option<&'static str>,
    limit: f32,
) -> Vec<&'s str> {
    let trimmed = word.trim_end_matches(' ');
    if limit <= 0.
        || trimmed.chars().count() < 2
        || f32::from(width(window, trimmed, size, weight, family)) <= limit
    {
        return vec![word];
    }
    let mut chunks = Vec::new();
    let mut rest = word;
    while f32::from(width(
        window,
        rest.trim_end_matches(' '),
        size,
        weight,
        family,
    )) > limit
    {
        let ends: Vec<usize> = rest.char_indices().map(|(i, c)| i + c.len_utf8()).collect();
        let fit = ends
            .iter()
            .take_while(|&&end| {
                f32::from(width(window, &rest[..end], size, weight, family)) <= limit
            })
            .count()
            .max(1);
        let end = ends[fit - 1];
        if end >= rest.len() {
            break;
        }
        chunks.push(&rest[..end]);
        rest = &rest[end..];
    }
    chunks.push(rest);
    chunks
}

/// A wrapped inline row that ends a block: `newlines` is what Chrome's `Selection.toString`
/// emits after it (two for paragraphs and headings).
fn block(mut row: Vec<AnyElement>, newlines: usize) -> Div {
    // `wrap_row` gives every line 4 px for a hanging space; the block's last word has none, so
    // it carries those 4 px as padding to wrap as it would without the allowance.
    if let Some(last) = row.pop() {
        row.push(
            div()
                .flex()
                .flex_none()
                .pr(px(4.))
                .child(last)
                .into_any_element(),
        );
    }
    wrap_row().children(row).child(block_end(newlines))
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

#[derive(IntoElement)]
struct CodeBlock {
    id: SharedString,
    code: SharedString,
    language: SharedString,
}

struct CodeState {
    lines: Vec<(SharedString, Vec<TextRun>)>,
    width: Pixels,
    copied: bool,
    wrap: bool,
    copy_focus: FocusHandle,
    wrap_focus: FocusHandle,
    reset: Option<Task<()>>,
}

impl CodeState {
    fn copy(&mut self, code: &SharedString, cx: &mut Context<Self>) {
        cx.write_to_clipboard(ClipboardItem::new_string(code.to_string()));
        self.copied = true;
        self.reset = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_secs(2)).await;
            let _ = this.update(cx, |this, cx| {
                this.copied = false;
                this.reset = None;
                cx.notify();
            });
        }));
        cx.notify();
    }
}

static SYNTAXES: LazyLock<SyntaxSet> = LazyLock::new(SyntaxSet::load_defaults_newlines);
static CODE_THEMES: LazyLock<[syntect::highlighting::Theme; 2]> =
    LazyLock::new(|| [code_theme(Appearance::Dark), code_theme(Appearance::Light)]);

fn code_theme(appearance: Appearance) -> syntect::highlighting::Theme {
    let color = |dark: u32, light: u32| {
        let value = if appearance == Appearance::Dark {
            dark
        } else {
            light
        };
        Color {
            r: (value >> 16) as u8,
            g: (value >> 8) as u8,
            b: value as u8,
            a: 255,
        }
    };
    syntect::highlighting::Theme {
        settings: ThemeSettings {
            foreground: Some(color(0xffffff, 0x1a1c1f)),
            ..Default::default()
        },
        scopes: [
            (
                "keyword, storage",
                color(0xf8a6c8, 0xab4f7a),
                FontStyle::empty(),
            ),
            (
                "keyword.operator",
                color(0xffffff, 0x1a1c1f),
                FontStyle::empty(),
            ),
            (
                "entity.name.function, entity.name.class",
                color(0xb897f4, 0x643cae),
                FontStyle::empty(),
            ),
            ("string", color(0x83d197, 0x3a843f), FontStyle::empty()),
            (
                "constant, support.function, support.type",
                color(0xf1a275, 0xac4f23),
                FontStyle::empty(),
            ),
            ("comment", color(0xb9b9b9, 0x4f4f4f), FontStyle::ITALIC),
        ]
        .into_iter()
        .map(|(scope, foreground, font_style)| ThemeItem {
            scope: scope.parse().expect("fixed syntax scopes"),
            style: StyleModifier {
                foreground: Some(foreground),
                font_style: Some(font_style),
                ..Default::default()
            },
        })
        .collect(),
        ..Default::default()
    }
}

impl RenderOnce for CodeBlock {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let t = cx.theme();
        let light = usize::from(t.appearance == Appearance::Light);
        let foreground = t.text;
        let token = match self.language.as_ref() {
            "py" => "python",
            "js" | "jsx" => "javascript",
            "sh" | "bash" => "shell",
            other => other,
        };
        let syntax = SYNTAXES
            .find_syntax_by_token(token)
            .unwrap_or_else(|| SYNTAXES.find_syntax_plain_text());
        let label: SharedString = if self.language.is_empty() {
            "Code".into()
        } else if syntax.name == "Plain Text" {
            self.language.clone()
        } else {
            syntax.name.clone().into()
        };
        let key = SharedString::from(format!("{}-{light}", self.id));
        let state = window.use_keyed_state(key, cx, |window, cx| {
            let mut highlighter = HighlightLines::new(syntax, &CODE_THEMES[light]);
            let mut width = px(0.);
            let lines = self
                .code
                .split('\n')
                .map(|line| {
                    let parsed = format!("{line}\n");
                    let runs = match highlighter.highlight_line(&parsed, &SYNTAXES) {
                        Ok(spans) => {
                            let mut remaining = line.len();
                            spans
                                .into_iter()
                                .filter_map(|(style, text)| {
                                    let len = remaining.min(text.len());
                                    remaining -= len;
                                    (len > 0).then(|| TextRun {
                                        len,
                                        font: gpui::Font {
                                            weight: FontWeight(430.),
                                            style: if style.font_style.contains(FontStyle::ITALIC) {
                                                gpui::FontStyle::Italic
                                            } else {
                                                gpui::FontStyle::Normal
                                            },
                                            ..font(code_font())
                                        },
                                        color: rgb((u32::from(style.foreground.r) << 16)
                                            | (u32::from(style.foreground.g) << 8)
                                            | u32::from(style.foreground.b))
                                        .into(),
                                        background_color: None,
                                        underline: None,
                                        strikethrough: None,
                                    })
                                })
                                .collect::<Vec<_>>()
                        }
                        Err(error) => {
                            eprintln!("Code highlighting failed: {error}");
                            vec![TextRun {
                                len: line.len(),
                                font: gpui::Font {
                                    weight: FontWeight(430.),
                                    ..font(code_font())
                                },
                                color: foreground,
                                background_color: None,
                                underline: None,
                                strikethrough: None,
                            }]
                        }
                    };
                    width = width.max(
                        window
                            .text_system()
                            .layout_line(line, px(12.), &runs, None)
                            .width,
                    );
                    (SharedString::from(line.to_owned()), runs)
                })
                .collect();
            CodeState {
                lines,
                width,
                copied: false,
                wrap: false,
                copy_focus: cx.focus_handle(),
                wrap_focus: cx.focus_handle(),
                reset: None,
            }
        });
        let t = cx.theme();
        let s = state.read(cx);
        let (copied, wrap, width) = (s.copied, s.wrap, s.width);
        let button = |id: &'static str| {
            div()
                .id(id)
                .role(gpui::Role::Button)
                .size(px(36.))
                .flex()
                .items_center()
                .justify_center()
                .rounded_full()
                .hover(|s| s.bg(t.ghost_hover))
                .focus_visible(|s| s.border_2().border_color(rgb(0x3a83f7)))
        };
        let copy_state = state.clone();
        let code = self.code;
        let copy_focus = s.copy_focus.clone();
        let copy = button("copy-code")
            .aria_label(if copied { "Copied" } else { "Copy" })
            .track_focus(&copy_focus)
            .tab_index(0)
            .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                window.focus(&copy_focus, cx);
                cx.stop_propagation();
            })
            .on_click(move |_, _, cx| copy_state.update(cx, |s, cx| s.copy(&code, cx)))
            .child(if copied {
                code_symbol(t.text, true).into_any_element()
            } else {
                icon("thread-copy", 16., t.text).into_any_element()
            });
        let wrap_state = state.clone();
        let wrap_label = if wrap {
            "Disable word wrap"
        } else {
            "Enable word wrap"
        };
        let wrap_focus = s.wrap_focus.clone();
        let wrap_button = button("wrap-code")
            .aria_label(wrap_label)
            .track_focus(&wrap_focus)
            .tab_index(0)
            .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                window.focus(&wrap_focus, cx)
            })
            .on_click(move |_, _, cx| {
                wrap_state.update(cx, |s, cx| {
                    s.wrap = !s.wrap;
                    cx.notify();
                })
            })
            .child(icon("diff-word-wrap", 16., t.text));
        let body = div()
            .flex()
            .flex_col()
            .min_w_full()
            .when(!wrap, |d| d.w(width).flex_none())
            .when(wrap, |d| d.w_full())
            .children(s.lines.iter().map(|(line, runs)| {
                div()
                    .min_h(px(20.))
                    .text_size(px(12.))
                    .line_height(px(20.))
                    .font_family(code_font())
                    .font_weight(FontWeight(430.))
                    .when(!wrap, |d| d.whitespace_nowrap())
                    .child(StyledText::new(line.clone()).with_runs(runs.clone()))
            }));
        div()
            .id(self.id)
            .w_full()
            .min_w_0()
            .rounded(px(20.))
            .border_1()
            .border_color(if light == 0 {
                rgb(0x333333)
            } else {
                rgb(0xe8e8e8)
            })
            .bg(if light == 0 {
                rgb(0x2a2a2a)
            } else {
                rgb(0xf2f2f2)
            })
            .overflow_hidden()
            .child(
                div()
                    .h(px(48.))
                    .pl(px(20.))
                    .pr(px(6.))
                    .flex()
                    .when(light == 1, |d| d.bg(rgb(0xf3f3f3)))
                    .items_center()
                    .gap(px(8.))
                    .child(code_symbol(t.text, false))
                    .child(text(label, 13., 18.5714, 500., t.text).flex_1())
                    .child(
                        div()
                            .flex()
                            .gap(px(1.))
                            .child(tooltip(wrap_button, wrap_label, None, Side::Top))
                            .child(tooltip(
                                copy,
                                if copied { "Copied" } else { "Copy code" },
                                None,
                                Side::Top,
                            )),
                    ),
            )
            .child(
                div()
                    .id("code-scroll")
                    .px(px(20.))
                    .pb(px(12.))
                    .overflow_x_scroll()
                    .child(body),
            )
    }
}

fn code_symbol(color: Hsla, checked: bool) -> impl IntoElement {
    canvas(
        |_, _, _| (),
        move |bounds, _, window, _| {
            let mut path = PathBuilder::stroke(px(1.3));
            let p = |x, y| bounds.origin + point(px(x), px(y));
            if checked {
                path.move_to(p(3., 8.));
                path.line_to(p(6.5, 11.5));
                path.line_to(p(13., 4.5));
            } else {
                path.move_to(p(5., 6.7));
                path.line_to(p(2.7, 10.));
                path.line_to(p(5., 13.3));
                path.move_to(p(15., 6.7));
                path.line_to(p(17.3, 10.));
                path.line_to(p(15., 13.3));
                path.move_to(p(11.7, 6.7));
                path.line_to(p(8.3, 13.3));
            }
            if let Ok(path) = path.build() {
                window.paint_path(path, color);
            }
        },
    )
    .size(px(if checked { 16. } else { 20. }))
}

#[cfg(test)]
mod item_tests {
    use super::*;

    #[test]
    fn word_shares_end_at_the_item_layout_unit_width() {
        // ` next to ` at 14 px on a 2x display: its words' advances (css px). Blink's item is
        // their sum ceiled to 1/128 px, 52.296875; rounding each word gave 52.2890625.
        let mut item = ItemPen {
            unit: 128.,
            advance: 0.,
        };
        let shares = [
            (3.752_929_7, false),
            (31.715_332, false),
            (16.821_533, true),
        ]
        .map(|(width, last)| f32::from(item.share(px(width), last)));
        assert_eq!(shares, [3.75, 31.71875, 16.828125]);
    }
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;
    use gpui::{AppContext, HeadlessAppContext, Render, size};
    use std::sync::Arc;

    struct InlineCode;

    impl Render for InlineCode {
        fn render(&mut self, window: &mut Window, _: &mut gpui::Context<Self>) -> impl IntoElement {
            // This original box origin exposes the extra pixel from rebuilding its right edge after rounding its left edge.
            div().size_full().relative().bg(rgb(0x181818)).child(
                markdown("`2`.", window, &Theme::dark(), &[])
                    .absolute()
                    .left(px(1_081.765_6))
                    .top(px(282.796_88)),
            )
        }
    }

    #[test]
    fn inline_code_background_ends_at_the_snapped_label_edge() -> anyhow::Result<()> {
        std::panic::set_hook(Box::new(|info| {
            eprintln!("{info}");
            std::process::exit(101);
        }));
        let mut cx = HeadlessAppContext::with_platform(
            textsys::text_system(),
            Arc::new(theme::Assets),
            gpui_platform::current_headless_renderer,
        );
        cx.update(|cx| {
            cx.set_global(Theme::dark());
            theme::load_fonts(cx);
        });
        let window = cx.open_window(size(px(1280.), px(832.)), |_, cx| cx.new(|_| InlineCode))?;
        cx.run_until_parked();
        let image = cx.capture_screenshot(window.into())?;
        assert_eq!(image.get_pixel(2203, 590).0, [24, 24, 24, 255]);
        assert_eq!(image.get_pixel(2202, 590).0, [48, 48, 48, 255]);
        Ok(())
    }
}
