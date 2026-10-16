use super::*;

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

#[derive(Debug, PartialEq)]
pub(super) enum Inline<'a> {
    Text(&'a str),
    Code(&'a str),
    Link { label: &'a str, target: &'a str },
}

/// Inline markdown the answers use: `code` spans and [label](target) links.
pub(super) fn inlines(mut rest: &str) -> Vec<Inline<'_>> {
    let mut out = Vec::new();
    while let Some(i) = rest.find(['`', '[']) {
        let (head, tail) = rest.split_at(i);
        let parsed = if let Some(code) = tail.strip_prefix('`') {
            code.find('`')
                .map(|j| (Inline::Code(&code[..j]), &code[j + 1..]))
        } else {
            tail.find("](").and_then(|j| {
                let after = &tail[j + 2..];
                after.find(')').map(|k| {
                    let link = Inline::Link {
                        label: &tail[1..j],
                        target: &after[..k],
                    };
                    (link, &after[k + 1..])
                })
            })
        };
        match parsed {
            Some((inline, next)) => {
                if !head.is_empty() {
                    out.push(Inline::Text(head));
                }
                out.push(inline);
                rest = next;
            }
            None => {
                out.push(Inline::Text(&rest[..=i]));
                rest = &rest[i + 1..];
            }
        }
    }
    if !rest.is_empty() {
        out.push(Inline::Text(rest));
    }
    out
}

/// Agent markdown: paragraphs of words, inline code chips and file links, wrapping at words.
pub(super) fn markdown(source: &str, window: &Window, t: &Theme) -> Div {
    let tones = tones(t);
    let paragraphs = source
        .split("\n\n")
        .filter(|p| !p.trim().is_empty())
        .map(|p| {
            let p = p.replace('\n', " ");
            let parts = inlines(&p);
            // Each flex line is as tall as Blink's line box: 22.75 px, or 23.75 px with inline
            // code (the 12.88 px mono font's descent). Items stretch to it.
            let mut row = wrap_row();
            for part in parts {
                row = match part {
                    Inline::Text(s) => row.children(words(s, 430., t.text, window)),
                    Inline::Code(code) => row.child(
                        div().pt(px(3.5)).pb(px(3.25)).child(
                            div()
                                .relative()
                                .h(px(17.))
                                .py(px(1.))
                                .px(px(6.))
                                .child(shapes::decoration(shapes::Decoration {
                                    radius: Corners::all(px(6.)),
                                    fill: Some(tones.inline_code),
                                    ..Default::default()
                                }))
                                .child(measured(
                                    window,
                                    code.to_owned(),
                                    12.88,
                                    15.,
                                    430.,
                                    t.text,
                                    Some(MONO),
                                )),
                        ),
                    ),
                    // A file link opens the Changes panel.
                    Inline::Link { label, target } => row.child(
                        div()
                            .id(SharedString::from(format!("file-link-{target}")))
                            .relative()
                            .flex()
                            .on_click(crate::dispatch::<crate::OpenChangesPanel>)
                            // ponytail: only the Python language icon is bundled; other files show
                            // the bare label until their icons are added.
                            .when(target.ends_with(".py"), |d| {
                                // The icon box is bottom-aligned in the line box.
                                d.child(div().w(px(19.))).child(
                                    div().absolute().left_0().bottom(px(3.37)).child(icon(
                                        "thread-lang-python",
                                        16.,
                                        tones.mention,
                                    )),
                                )
                            })
                            .child(measured(
                                window,
                                label.to_owned(),
                                14.,
                                22.75,
                                500.,
                                tones.mention,
                                None,
                            )),
                    ),
                };
            }
            row
        });
    div().flex().flex_col().gap(px(12.)).children(paragraphs)
}
