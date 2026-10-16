//! Fenced blocks keep whitespace and syntax colors, with horizontal overflow and a copy button.
//! Copy excludes the fence and its final newline; the checkmark lasts two seconds.
//! HTTP(S) links open the system browser on left or middle click. File links keep their existing action.

use super::*;
use crate::tooltip::{Side, tooltip};
use gpui::{AnyElement, FocusHandle, MouseButton, PathBuilder, StyledText, canvas, point};
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

use pulldown_cmark::{CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};
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
        block_index: 0,
    }
    .blocks(Typography::default(), 0)
    .0
}

struct Markdown<'a> {
    events: Events<'a>,
    window: &'a Window,
    theme: &'a Theme,
    find: Option<&'a FindPaint>,
    block_index: usize,
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
                    let mut at = self.events.start;
                    row.extend(s.split_inclusive(' ').map(|s| {
                        let here = at;
                        at += s.len();
                        self.word(s, Some(here), style).into_any_element()
                    }));
                }
                Event::SoftBreak => {
                    let at = self.events.start;
                    row.push(self.word(" ", Some(at), style).into_any_element());
                }
                Event::HardBreak => row.push(block_end(1).w_full().h_0().into_any_element()),
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
                        let external = dest_url.split_once("://").is_some_and(|(scheme, _)| {
                            scheme.eq_ignore_ascii_case("http")
                                || scheme.eq_ignore_ascii_case("https")
                        });
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
                                .when(external, |d| {
                                    let url = dest_url.to_string();
                                    let middle_url = url.clone();
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
                                })
                                .when(!external, |d| {
                                    d.on_click(crate::dispatch::<crate::OpenChangesPanel>)
                                })
                                .when(!external && dest_url.ends_with(".py"), |d| {
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
    fn word(&self, s: &str, at: Option<usize>, style: Typography) -> Div {
        div()
            .relative()
            .flex_none()
            .child(
                highlight(s.to_owned().into(), style.size, style.weight, None, 0.)
                    .absolute()
                    .inset_0(),
            )
            .child(self.label(s, at, style))
    }

    fn label(&self, s: &str, at: Option<usize>, style: Typography) -> Div {
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
                // Over the chip's fill, under its glyphs, the height of the line box.
                .child(
                    highlight(code.to_owned().into(), 12.88, 430., Some(MONO), 6.)
                        .absolute()
                        .left_0()
                        .right_0()
                        .top(px(-3.5))
                        .bottom(px(-3.25)),
                )
                .child(label),
        )
    }
}

/// A wrapped inline row that ends a block: `newlines` is what Chrome's `Selection.toString`
/// emits after it (two for paragraphs and headings).
fn block(row: Vec<AnyElement>, newlines: usize) -> Div {
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
                                            ..font(MONO)
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
                                    ..font(MONO)
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
                    .font_family(MONO)
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
