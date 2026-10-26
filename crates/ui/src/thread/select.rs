//! Text selection over the answer markdown (TH-127, TH-128, SH-132): the words register
//! themselves at prepaint, in reading order, in their transcript's per-frame registry of the
//! [`Selection`] global (a side chat shows a second transcript); the view keeps the anchor and
//! head carets. Everything is in the scroller's content px, so the selection stays put while
//! the thread scrolls.

use super::*;
use gpui::{Global, Point, canvas, fill, point};
use std::sync::atomic::{AtomicUsize, Ordering};

/// A caret: `y` is the middle of its line box, `x` the pointer's x; both in content px.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Caret {
    y: Pixels,
    x: Pixels,
}

impl Caret {
    /// Carets within 4 px vertically share a line (a code chip's text box sits lower than the
    /// words next to it); the leftmost comes first.
    fn before(self, other: Caret) -> bool {
        if (self.y - other.y).abs() < px(4.) {
            self.x < other.x
        } else {
            self.y < other.y
        }
    }
}

struct Word {
    bounds: Bounds<Pixels>,
    /// Where the glyphs start inside `bounds` (a chip's horizontal padding).
    pad: Pixels,
    text: SharedString,
    size: f32,
    weight: f32,
    family: Option<&'static str>,
}

enum Entry {
    Word(Word),
    /// End of a block: the newlines Chrome's `Selection.toString` emits after it.
    Break(usize),
}

/// This frame's registry of every transcript on screen, in prepaint order.
#[derive(Default)]
pub struct Selection {
    transcripts: Vec<Transcript>,
}

impl Global for Selection {}

impl Selection {
    fn get(cx: &mut App) -> &mut Self {
        cx.default_global::<Self>()
    }

    /// The registry of the transcript with carets `id`, as its last frame left it.
    fn of(&self, id: usize) -> Option<&Transcript> {
        self.transcripts.iter().find(|t| t.carets == id)
    }
}

/// One transcript's words (written at prepaint) and the range they highlight (written at
/// render).
struct Transcript {
    /// Its view's [`Carets::id`].
    carets: usize,
    entries: Vec<Entry>,
    /// Ordered `(start, end)`, when the view has a non-empty selection.
    range: Option<(Caret, Caret)>,
    /// Window y of content y 0 this frame.
    origin: Pixels,
}

impl Transcript {
    /// Chars `[start, end)` of `word` inside the range: byte offsets of its text. `wraps`: the
    /// word ends its line, so its trailing space hangs past the wrap: a caret on that line
    /// stops before the space, as Chrome's does, and a caret on a later line takes it.
    fn span(
        &self,
        word: &Word,
        wraps: bool,
        window: &Window,
    ) -> Option<(usize, usize, Pixels, Pixels)> {
        let (start, end) = self.range?;
        let b = word.bounds;
        let line = Caret {
            y: (b.top() + b.bottom()) / 2. - self.origin,
            x: b.left() + word.pad,
        };
        let layout = window.text_system().layout_line(
            &word.text,
            px(word.size),
            &[run(&word.text, word.weight, word.family)],
            None,
        );
        let len = word.text.len();
        let on_line = if wraps && word.text.ends_with(' ') {
            len - 1
        } else {
            len
        };
        // The nearest glyph boundary, the line's end included (`closest_index_for_x` keeps a
        // pointer inside the last glyph past it).
        let boundary = |x: Pixels| {
            // A caret off the line's end (`f32::MAX`) is as far from every boundary.
            let x = x.clamp(px(0.), layout.width);
            layout
                .runs
                .iter()
                .flat_map(|r| r.glyphs.iter().map(|g| (g.position.x, g.index)))
                .chain(std::iter::once((layout.width, len)))
                .filter(|&(_, i)| i <= on_line)
                .min_by(|a, b| (a.0 - x).abs().partial_cmp(&(b.0 - x).abs()).unwrap())
                .map_or(0, |(_, i)| i)
        };
        let index = |caret: Caret| {
            if caret.before(line) {
                0
            } else if (caret.y - line.y).abs() < px(4.) {
                boundary(caret.x - line.x)
            } else {
                len
            }
        };
        let (s, e) = (index(start), index(end));
        (s < e).then(|| (s, e, layout.x_for_index(s), layout.x_for_index(e)))
    }

    /// `entries[i]` ends its line: the next word sits lower.
    fn wraps(entries: &[Entry], i: usize) -> bool {
        match (entries.get(i), entries.get(i + 1)) {
            (Some(Entry::Word(word)), Some(Entry::Word(next))) => {
                next.bounds.top() > word.bounds.top()
            }
            _ => false,
        }
    }

    /// The selected text, joined the way Chrome's `Selection.toString` does.
    pub(super) fn text(&self, window: &Window) -> String {
        let mut out = String::new();
        let mut pending = 0;
        for (i, entry) in self.entries.iter().enumerate() {
            match entry {
                Entry::Break(n) => pending = pending.max(*n),
                Entry::Word(word) => {
                    if let Some((s, e, _, _)) =
                        self.span(word, Self::wraps(&self.entries, i), window)
                    {
                        if !out.is_empty() {
                            out.extend(std::iter::repeat_n('\n', pending));
                        }
                        pending = 0;
                        out.push_str(&word.text[s..e]);
                    }
                }
            }
        }
        out
    }

    /// The caret under content point `p`: on the line box holding `p.y`, else at the end of
    /// the nearest line above it (or the start of the nearest one below).
    pub(super) fn caret(&self, p: Point<Pixels>) -> Option<Caret> {
        let words = self.entries.iter().filter_map(|e| match e {
            Entry::Word(w) => Some(w.bounds),
            Entry::Break(_) => None,
        });
        let mut best: Option<(Pixels, Caret)> = None;
        for b in words {
            let (top, bottom) = (b.top() - self.origin, b.bottom() - self.origin);
            let y = (top + bottom) / 2.;
            let (distance, x) = if p.y < top {
                (top - p.y, px(f32::MIN))
            } else if p.y >= bottom {
                (p.y - bottom, px(f32::MAX))
            } else {
                (px(0.), p.x)
            };
            if best.is_none_or(|(d, _)| distance < d) {
                best = Some((distance, Caret { y, x }));
            }
        }
        best.map(|(_, c)| c)
    }
}

fn run(s: &str, weight: f32, family: Option<&'static str>) -> TextRun {
    TextRun {
        len: s.len(),
        font: gpui::Font {
            weight: FontWeight(weight),
            ..font(family.unwrap_or(SANS))
        },
        color: white(),
        background_color: None,
        underline: None,
        strikethrough: None,
    }
}

/// The selection highlight under a word's glyphs: registers the word for [`Carets::text`]
/// at prepaint (the whole tree prepaints before it paints, so the word after it is known at
/// paint) and paints the selected span over its line box (Chrome paints the line box, not the
/// glyph box). `pad`: where the glyphs start inside the painted bounds.
pub(super) fn highlight(
    text: SharedString,
    size: f32,
    weight: f32,
    family: Option<&'static str>,
    pad: f32,
) -> gpui::Canvas<Option<(usize, usize)>> {
    canvas(
        move |bounds, _, cx| {
            let transcripts = &mut Selection::get(cx).transcripts;
            let at = transcripts.len().checked_sub(1)?;
            let entries = &mut transcripts[at].entries;
            entries.push(Entry::Word(Word {
                bounds,
                pad: px(pad),
                text,
                size,
                weight,
                family,
            }));
            Some((at, entries.len() - 1))
        },
        move |bounds, at, window, cx| {
            let Some((at, i)) = at else { return };
            let Some(t) = Selection::get(cx).transcripts.get(at) else {
                return;
            };
            let Some(Entry::Word(word)) = t.entries.get(i) else {
                return;
            };
            let span = t.span(word, Transcript::wraps(&t.entries, i), window);
            if let Some((_, _, x0, x1)) = span {
                let left = bounds.left() + px(pad);
                let r = Bounds::from_corners(
                    point(left + x0, bounds.top()),
                    point(left + x1, bounds.bottom()),
                );
                window.paint_quad(fill(r, cx.theme().text_selection));
            }
        },
    )
}

/// Marks the end of a block in reading order: `newlines` is what Chrome emits after it.
pub(super) fn block_end(newlines: usize) -> gpui::Canvas<()> {
    canvas(
        move |_, _, cx| {
            if let Some(t) = Selection::get(cx).transcripts.last_mut() {
                t.entries.push(Entry::Break(newlines));
            }
        },
        |_, (), _, _| (),
    )
}

/// Starts the transcript's registry for this frame: first inside the scroller, before any
/// word, with the range `carets` select. A transcript that registered already means a new
/// frame: every registry of the last one goes.
pub(super) fn frame_start(carets: &Carets, scroll: ScrollHandle) -> gpui::Canvas<()> {
    let (id, range) = (carets.id, carets.range());
    canvas(
        move |_, _, cx| {
            let sel = Selection::get(cx);
            if sel.of(id).is_some() {
                sel.transcripts.clear();
            }
            sel.transcripts.push(Transcript {
                carets: id,
                entries: Vec::new(),
                range,
                origin: scroll.bounds().top() + scroll.offset().y,
            });
        },
        |_, (), _, _| (),
    )
}

/// The view's anchor and head carets while it has or drags a selection.
pub(super) struct Carets {
    /// Names the view's [`Transcript`].
    id: usize,
    pub anchor: Option<Caret>,
    pub head: Option<Caret>,
    pub dragging: bool,
}

impl Default for Carets {
    fn default() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        Self {
            id: NEXT.fetch_add(1, Ordering::Relaxed),
            anchor: None,
            head: None,
            dragging: false,
        }
    }
}

impl Carets {
    pub fn range(&self) -> Option<(Caret, Caret)> {
        let (a, h) = (self.anchor?, self.head?);
        if a == h {
            None
        } else if a.before(h) {
            Some((a, h))
        } else {
            Some((h, a))
        }
    }

    /// The caret under window point `p`, in this view's transcript.
    fn caret(&self, p: Point<Pixels>, cx: &mut App) -> Option<Caret> {
        let t = Selection::get(cx).of(self.id)?;
        t.caret(point(p.x, p.y - t.origin))
    }

    pub fn press(&mut self, p: Point<Pixels>, cx: &mut App) {
        let caret = self.caret(p, cx);
        self.anchor = caret;
        self.head = caret;
        self.dragging = caret.is_some();
    }

    pub fn drag(&mut self, p: Point<Pixels>, cx: &mut App) {
        if self.dragging {
            self.head = self.caret(p, cx);
        }
    }

    /// The selected text of this view's transcript.
    pub fn text(&self, window: &Window, cx: &App) -> String {
        cx.try_global::<Selection>()
            .and_then(|sel| sel.of(self.id))
            .map(|t| t.text(window))
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn carets_order_by_line_then_x() {
        let a = Caret {
            y: px(10.),
            x: px(50.),
        };
        let b = Caret {
            y: px(12.),
            x: px(20.),
        };
        let c = Caret {
            y: px(40.),
            x: px(0.),
        };
        assert!(b.before(a));
        assert!(a.before(c));
        assert!(!c.before(a));
    }

    #[test]
    fn caret_snaps_to_the_nearest_line() {
        let word = |top: f32| Word {
            bounds: Bounds::new(point(px(10.), px(top)), gpui::size(px(50.), px(20.))),
            pad: px(0.),
            text: "x".into(),
            size: 14.,
            weight: 430.,
            family: None,
        };
        let sel = Transcript {
            carets: 0,
            entries: vec![
                Entry::Word(word(0.)),
                Entry::Break(2),
                Entry::Word(word(100.)),
            ],
            range: None,
            origin: px(0.),
        };
        assert_eq!(
            sel.caret(point(px(30.), px(5.))),
            Some(Caret {
                y: px(10.),
                x: px(30.)
            })
        );
        assert_eq!(
            sel.caret(point(px(30.), px(30.))),
            Some(Caret {
                y: px(10.),
                x: px(f32::MAX)
            })
        );
        assert_eq!(
            sel.caret(point(px(30.), px(95.))),
            Some(Caret {
                y: px(110.),
                x: px(f32::MIN)
            })
        );
    }
}
