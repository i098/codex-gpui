//! A Markdown file as the original's file editor shows it (`refs/files-markdown`): CodeMirror's
//! live preview, one `cm-line` per source line (`padding: 2px 0`, line height 1.4 em). The
//! syntax marks of headings, emphasis and inline code are hidden, a list item keeps its `-` in
//! the tertiary color behind a 12 px indent, and a fenced block is a 12 px code line run with
//! its language at the right of the first fence.
use super::*;
use pulldown_cmark::{Event, Parser, Tag, TagEnd};

/// `--file-editor-font-size`, `--codex-chat-code-font-size`.
const BODY: f32 = 14.;
const CODE: f32 = 12.;
/// `--radius-lg` on the code block's first and last lines.
const CODE_RADIUS: f32 = 12.5;

/// The heading size of a `#` .. `######` line (`--text-heading-lg`, `-md`, `-sm`, then the
/// body size).
fn heading_size(level: usize) -> f32 {
    match level {
        1 => 24.,
        2 => 20.,
        3 => 18.,
        _ => BODY,
    }
}

/// `text` with its emphasis, strong and inline code marks dropped, as styled runs.
fn inline(text: &str, weight: f32, t: &Theme) -> StyledText {
    marked(None, text, weight, t)
}

/// [`inline`] after a `marker` run in the tertiary color (a list item's `- `).
fn marked(marker: Option<&str>, text: &str, weight: f32, t: &Theme) -> StyledText {
    let (mut out, mut runs) = (String::new(), Vec::new());
    if let Some(marker) = marker {
        out.push_str(marker);
        runs.push(TextRun {
            len: marker.len(),
            font: gpui::Font {
                weight: FontWeight(430.),
                ..font(SANS)
            },
            color: t.text_tertiary,
            background_color: None,
            underline: None,
            strikethrough: None,
        });
    }
    let (mut strong, mut emphasis) = (0, 0);
    let mut push = |s: &str, code: bool, strong: bool, emphasis: bool| {
        out.push_str(s);
        runs.push(TextRun {
            len: s.len(),
            font: gpui::Font {
                weight: FontWeight(if strong { 600. } else { weight }),
                style: if emphasis {
                    gpui::FontStyle::Italic
                } else {
                    gpui::FontStyle::Normal
                },
                ..font(if code { MONO } else { SANS })
            },
            color: t.text,
            background_color: None,
            underline: None,
            strikethrough: None,
        });
    };
    for event in Parser::new(text) {
        match event {
            Event::Start(Tag::Strong) => strong += 1,
            Event::End(TagEnd::Strong) => strong -= 1,
            Event::Start(Tag::Emphasis) => emphasis += 1,
            Event::End(TagEnd::Emphasis) => emphasis -= 1,
            Event::Text(s) => push(&s, false, strong > 0, emphasis > 0),
            Event::Code(s) => push(&s, true, strong > 0, emphasis > 0),
            _ => {}
        }
    }
    StyledText::new(out).with_runs(runs)
}

/// One `cm-line`: 2 px above and below its text.
fn line(size: f32) -> Div {
    div()
        .py(px(2.))
        .min_h(px(size * 1.4 + 4.))
        .text_size(px(size))
        .line_height(px(size * 1.4))
}

pub(super) fn view(source: &str, t: &Theme) -> Div {
    let mut rows = Vec::new();
    let mut fence: Option<usize> = None;
    let lines: Vec<&str> = source.split('\n').collect();
    for (i, raw) in lines.iter().enumerate() {
        let trimmed = raw.trim_start();
        if let Some(open) = fence {
            // The closing fence hides its marks like the opening one.
            let last = trimmed.starts_with("```");
            rows.push(code_line(
                if last { "" } else { raw },
                open == i,
                last,
                None,
                t,
            ));
            if last {
                fence = None;
            }
            continue;
        }
        if let Some(language) = trimmed.strip_prefix("```") {
            // A fence without its closing line runs to the end of the file.
            let closed = lines[i + 1..]
                .iter()
                .any(|l| l.trim_start().starts_with("```"));
            fence = closed.then_some(i);
            rows.push(code_line("", true, !closed, Some(language.trim()), t));
            continue;
        }
        let level = trimmed.chars().take_while(|c| *c == '#').count();
        if (1..=6).contains(&level) && trimmed[level..].starts_with(' ') {
            let size = heading_size(level);
            rows.push(
                line(size)
                    .child(inline(trimmed[level..].trim_start(), 500., t))
                    .into_any_element(),
            );
            continue;
        }
        if let Some(item) = ["- ", "* ", "+ "]
            .iter()
            .find_map(|marker| trimmed.strip_prefix(marker))
        {
            rows.push(
                line(BODY)
                    .pl(px(12.))
                    .child(marked(Some("- "), item, 430., t))
                    .into_any_element(),
            );
            continue;
        }
        rows.push(line(BODY).child(inline(raw, 430., t)).into_any_element());
    }
    div()
        .flex()
        .flex_col()
        .font_family(SANS)
        .font_weight(FontWeight(430.))
        .text_color(t.text)
        .children(rows)
}

/// A line of a fenced block: `color-mix(--color-background-secondary-soft-alpha 10%)`, rounded
/// at the block's ends; the opening fence shows its language 8 px down, 12 px from the right.
fn code_line(
    raw: &str,
    first: bool,
    last: bool,
    language: Option<&str>,
    t: &Theme,
) -> gpui::AnyElement {
    let radius = px(CODE_RADIUS);
    line(CODE)
        .relative()
        .font_family(MONO)
        .child(shapes::decoration(Decoration {
            radius: gpui::Corners {
                top_left: if first { radius } else { px(0.) },
                top_right: if first { radius } else { px(0.) },
                bottom_left: if last { radius } else { px(0.) },
                bottom_right: if last { radius } else { px(0.) },
            },
            fill: Some(t.text.opacity(2. / 255.)),
            ..Default::default()
        }))
        .child(text(raw.to_owned(), CODE, CODE * 1.4, 430., t.text).font_family(MONO))
        .children(language.filter(|l| !l.is_empty()).map(|language| {
            div().absolute().top(px(8.)).right(px(12.)).child(
                text(
                    language.to_owned(),
                    CODE,
                    CODE * 1.4,
                    430.,
                    t.text_secondary,
                )
                .font_family(MONO),
            )
        }))
        .into_any_element()
}
