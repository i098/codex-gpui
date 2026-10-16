use super::*;

impl Composer {
    pub(super) fn cursor(&self) -> usize {
        if self.selection_reversed {
            self.selected_range.start
        } else {
            self.selected_range.end
        }
    }

    pub(super) fn move_to(&mut self, offset: usize, cx: &mut Context<Self>) {
        self.selected_range = offset..offset;
        self.selection_reversed = false;
        cx.notify();
    }

    pub(super) fn select_to(&mut self, offset: usize, cx: &mut Context<Self>) {
        if self.selection_reversed {
            self.selected_range.start = offset;
        } else {
            self.selected_range.end = offset;
        }
        if self.selected_range.end < self.selected_range.start {
            self.selection_reversed = !self.selection_reversed;
            self.selected_range = self.selected_range.end..self.selected_range.start;
        }
        cx.notify();
    }

    // ponytail: char boundaries, not grapheme clusters; add unicode-segmentation if
    // combining sequences need one-step deletion.
    pub(super) fn previous_boundary(&self, offset: usize) -> usize {
        self.content[..offset]
            .char_indices()
            .next_back()
            .map_or(0, |(i, _)| i)
    }

    pub(super) fn next_boundary(&self, offset: usize) -> usize {
        self.content[offset..]
            .chars()
            .next()
            .map_or(offset, |c| offset + c.len_utf8())
    }

    pub(super) fn insert(&mut self, s: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.replace_text_in_range(None, s, window, cx);
    }

    pub(super) fn newline(&mut self, _: &Newline, window: &mut Window, cx: &mut Context<Self>) {
        self.insert("\n", window, cx);
    }

    pub(super) fn backspace(&mut self, _: &Backspace, window: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            self.select_to(self.previous_boundary(self.cursor()), cx);
        }
        self.insert("", window, cx);
    }

    pub(super) fn delete(&mut self, _: &Delete, window: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            self.select_to(self.next_boundary(self.cursor()), cx);
        }
        self.insert("", window, cx);
    }

    pub(super) fn left(&mut self, _: &Left, _: &mut Window, cx: &mut Context<Self>) {
        let to = if self.selected_range.is_empty() {
            self.previous_boundary(self.cursor())
        } else {
            self.selected_range.start
        };
        self.move_to(to, cx);
    }

    pub(super) fn right(&mut self, _: &Right, _: &mut Window, cx: &mut Context<Self>) {
        let to = if self.selected_range.is_empty() {
            self.next_boundary(self.cursor())
        } else {
            self.selected_range.end
        };
        self.move_to(to, cx);
    }

    pub(super) fn up(&mut self, _: &Up, _: &mut Window, cx: &mut Context<Self>) {
        let to = self.vertical(-1.).unwrap_or(0);
        self.move_to(to, cx);
    }

    pub(super) fn down(&mut self, _: &Down, _: &mut Window, cx: &mut Context<Self>) {
        let to = self.vertical(1.).unwrap_or(self.content.len());
        self.move_to(to, cx);
    }

    pub(super) fn select_left(&mut self, _: &SelectLeft, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.previous_boundary(self.cursor()), cx);
    }

    pub(super) fn select_right(&mut self, _: &SelectRight, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.next_boundary(self.cursor()), cx);
    }

    pub(super) fn select_up(&mut self, _: &SelectUp, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.vertical(-1.).unwrap_or(0), cx);
    }

    pub(super) fn select_down(&mut self, _: &SelectDown, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.vertical(1.).unwrap_or(self.content.len()), cx);
    }

    pub(super) fn line_start(&mut self, _: &LineStart, _: &mut Window, cx: &mut Context<Self>) {
        let to = self.content[..self.cursor()]
            .rfind('\n')
            .map_or(0, |i| i + 1);
        self.move_to(to, cx);
    }

    pub(super) fn line_end(&mut self, _: &LineEnd, _: &mut Window, cx: &mut Context<Self>) {
        let at = self.cursor();
        let to = self.content[at..]
            .find('\n')
            .map_or(self.content.len(), |i| at + i);
        self.move_to(to, cx);
    }

    pub(super) fn select_all(&mut self, _: &crate::SelectAll, _: &mut Window, cx: &mut Context<Self>) {
        self.selected_range = 0..self.content.len();
        self.selection_reversed = false;
        cx.notify();
    }

    pub(super) fn copy(&mut self, _: &crate::Copy, _: &mut Window, cx: &mut Context<Self>) {
        if !self.selected_range.is_empty() {
            let s = self.content[self.selected_range.clone()].to_string();
            cx.write_to_clipboard(ClipboardItem::new_string(s));
        }
    }

    pub(super) fn cut(&mut self, _: &crate::Cut, window: &mut Window, cx: &mut Context<Self>) {
        self.copy(&crate::Copy, window, cx);
        self.insert("", window, cx);
    }

    pub(super) fn paste(&mut self, _: &crate::Paste, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(s) = cx.read_from_clipboard().and_then(|item| item.text()) {
            self.insert(&s, window, cx);
        }
    }

    pub(super) fn on_mouse_down(&mut self, e: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus_handle, cx);
        self.is_selecting = true;
        self.menu = None;
        let at = self.index_for_point(e.position);
        if e.modifiers.shift {
            self.select_to(at, cx);
        } else {
            self.move_to(at, cx);
        }
    }

    pub(super) fn on_mouse_up(&mut self, _: &MouseUpEvent, _: &mut Window, _: &mut Context<Self>) {
        self.is_selecting = false;
    }

    pub(super) fn on_mouse_move(&mut self, e: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
        if self.is_selecting {
            self.select_to(self.index_for_point(e.position), cx);
        }
    }

    /// Byte offset where each shaped line starts (lines are split at `\n`).
    pub(super) fn line_starts(&self) -> impl Iterator<Item = (usize, Pixels, &WrappedLine)> {
        let mut start = 0;
        let mut y = px(0.);
        self.lines.iter().map(move |line| {
            let item = (start, y, line);
            start += line.len() + 1;
            y += line.size(LINE_HEIGHT).height;
            item
        })
    }

    /// Editor-local position of the caret before byte `offset`.
    pub(super) fn point_for_index(&self, offset: usize) -> Option<Point<Pixels>> {
        self.line_starts()
            .find(|(start, _, line)| offset <= start + line.len())
            .and_then(|(start, y, line)| {
                let p = line.position_for_index(offset - start, LINE_HEIGHT)?;
                Some(point(p.x, p.y + y))
            })
    }

    pub(super) fn index_for_local(&self, local: Point<Pixels>) -> usize {
        let mut last = 0;
        for (start, y, line) in self.line_starts() {
            let h = line.size(LINE_HEIGHT).height;
            if local.y < y + h {
                let p = point(local.x, (local.y - y).max(px(0.)));
                let i = line
                    .closest_index_for_position(p, LINE_HEIGHT)
                    .unwrap_or_else(|i| i);
                return start + i;
            }
            last = start + line.len();
        }
        last
    }

    pub(super) fn index_for_point(&self, position: Point<Pixels>) -> usize {
        match (self.bounds, self.content.is_empty()) {
            (Some(b), false) => self.index_for_local(position - b.origin),
            _ => 0,
        }
    }

    /// Offset one row above (`-1`) or below (`1`) the caret; `None` past the first/last row.
    pub(super) fn vertical(&self, rows: f32) -> Option<usize> {
        let p = self.point_for_index(self.cursor())?;
        let y = p.y + LINE_HEIGHT * rows + LINE_HEIGHT / 2.;
        let total: Pixels = self.lines.iter().map(|l| l.size(LINE_HEIGHT).height).sum();
        (y >= px(0.) && y < total).then(|| self.index_for_local(point(p.x, y)))
    }

    pub(super) fn offset_from_utf16(&self, offset: usize) -> usize {
        let mut utf8 = 0;
        let mut utf16 = 0;
        for ch in self.content.chars() {
            if utf16 >= offset {
                break;
            }
            utf16 += ch.len_utf16();
            utf8 += ch.len_utf8();
        }
        utf8
    }

    pub(super) fn offset_to_utf16(&self, offset: usize) -> usize {
        self.content[..offset].encode_utf16().count()
    }

    pub(super) fn range_to_utf16(&self, r: &Range<usize>) -> Range<usize> {
        self.offset_to_utf16(r.start)..self.offset_to_utf16(r.end)
    }

    pub(super) fn range_from_utf16(&self, r: &Range<usize>) -> Range<usize> {
        self.offset_from_utf16(r.start)..self.offset_from_utf16(r.end)
    }

    pub(super) fn editor(&self, cx: &mut Context<Self>) -> Div {
        div()
            .px(px(12.))
            .pb(px(4.))
            .min_h(px(48.))
            .cursor(CursorStyle::IBeam)
            .text_size(FONT_SIZE)
            .line_height(LINE_HEIGHT)
            .font_weight(gpui::FontWeight(430.))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_mouse_down))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_mouse_up_out(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_mouse_move(cx.listener(Self::on_mouse_move))
            // `--opacity-composer-placeholder: .5` on the placeholder. The text blends like
            // Chromium's: blitted into a transparent layer (`_ComposerLayoutInput`'s
            // `overflow-y: auto` contents), then composited onto the card.
            .child(
                div()
                    .when(self.content.is_empty(), |d| d.opacity(0.5))
                    .layer()
                    .child(Editor(cx.entity())),
            )
    }
}

impl Focusable for Composer {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl EntityInputHandler for Composer {
    fn text_for_range(
        &mut self,
        range_utf16: Range<usize>,
        actual_range: &mut Option<Range<usize>>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<String> {
        let range = self.range_from_utf16(&range_utf16);
        actual_range.replace(self.range_to_utf16(&range));
        Some(self.content[range].to_string())
    }

    fn selected_text_range(
        &mut self,
        _: bool,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        Some(UTF16Selection {
            range: self.range_to_utf16(&self.selected_range),
            reversed: self.selection_reversed,
        })
    }

    fn set_selected_text_range(
        &mut self,
        range_utf16: Range<usize>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.selected_range = self.range_from_utf16(&range_utf16);
        self.selection_reversed = false;
        cx.notify();
    }

    fn text_length_utf16(&mut self, _: &mut Window, _: &mut Context<Self>) -> Option<usize> {
        Some(self.content.encode_utf16().count())
    }

    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        self.marked_range.as_ref().map(|r| self.range_to_utf16(r))
    }

    fn unmark_text(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        self.marked_range = None;
        cx.notify();
    }

    fn replace_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.replace_and_mark_text_in_range(range_utf16, new_text, None, window, cx);
        self.marked_range = None;
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        new_selected_range_utf16: Option<Range<usize>>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = range_utf16
            .as_ref()
            .map(|r| self.range_from_utf16(r))
            .or(self.marked_range.clone())
            .unwrap_or(self.selected_range.clone());
        self.content.replace_range(range.clone(), new_text);
        let end = range.start + new_text.len();
        self.marked_range = (!new_text.is_empty()).then_some(range.start..end);
        self.selected_range = match new_selected_range_utf16 {
            Some(sel) => {
                let base = self.offset_to_utf16(range.start);
                self.range_from_utf16(&(base + sel.start..base + sel.end))
            }
            None => end..end,
        };
        self.selection_reversed = false;
        cx.notify();
    }

    fn bounds_for_range(
        &mut self,
        range_utf16: Range<usize>,
        bounds: Bounds<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let range = self.range_from_utf16(&range_utf16);
        let start = self.point_for_index(range.start)?;
        let end = self.point_for_index(range.end)?;
        Some(Bounds::from_corners(
            bounds.origin + start,
            bounds.origin + point(end.x, end.y + LINE_HEIGHT),
        ))
    }

    fn character_index_for_point(
        &mut self,
        p: Point<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<usize> {
        let local = p - self.bounds?.origin;
        Some(self.offset_to_utf16(self.index_for_local(local)))
    }
}

/// The text area: wraps to its width, grows one row per line.
pub(super) struct Editor(pub(super) Entity<Composer>);

impl IntoElement for Editor {
    type Element = Self;

    fn into_element(self) -> Self {
        self
    }
}

/// Shapes `content` (or the placeholder) for one paint.
pub(super) fn shape(
    composer: &Composer,
    color: Hsla,
    placeholder: Hsla,
    width: Option<Pixels>,
    window: &Window,
) -> Vec<WrappedLine> {
    let style = window.text_style();
    let (s, color) = if composer.content.is_empty() {
        (composer.placeholder.to_string(), placeholder)
    } else {
        (composer.content.clone(), color)
    };
    let run = TextRun {
        len: s.len(),
        font: style.font(),
        color,
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    let runs = match &composer.marked_range {
        Some(m) if !composer.content.is_empty() => vec![
            TextRun {
                len: m.start,
                ..run.clone()
            },
            TextRun {
                len: m.end - m.start,
                underline: Some(UnderlineStyle {
                    color: Some(color),
                    thickness: px(1.),
                    wavy: false,
                }),
                ..run.clone()
            },
            TextRun {
                len: s.len() - m.end,
                ..run
            },
        ],
        _ => vec![run],
    };
    window
        .text_system()
        .shape_text(s.into(), FONT_SIZE, &runs, width, None)
        .map(|lines| lines.into_iter().collect())
        .unwrap_or_default()
}

impl gpui::Element for Editor {
    type RequestLayoutState = LayoutId;
    type PrepaintState = ();

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
        _: &mut App,
    ) -> (LayoutId, LayoutId) {
        let mut style = Style::default();
        style.size.width = relative(1.).into();
        let input = self.0.clone();
        let id = window.request_measured_layout(style, move |known, available, window, cx| {
            let width = known.width.or(match available.width {
                AvailableSpace::Definite(w) => Some(w),
                _ => None,
            });
            let lines = shape(input.read(cx), gpui::black(), gpui::black(), width, window);
            let height = lines.iter().map(|l| l.size(LINE_HEIGHT).height).sum();
            Size {
                width: width.unwrap_or_default(),
                height,
            }
        });
        (id, id)
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut LayoutId,
        window: &mut Window,
        cx: &mut App,
    ) {
        let t = cx.theme().clone();
        let lines = shape(
            self.0.read(cx),
            t.text,
            t.text_tertiary,
            Some(bounds.size.width),
            window,
        );
        // The editor spans the card minus its 12 px side padding.
        let width = f32::from(bounds.size.width) + 24.;
        let collapse = |w: f32| (w <= SM_WIDTH, w <= XS_WIDTH);
        let changed = self.0.update(cx, |c, _| {
            c.lines = lines;
            c.bounds = Some(bounds);
            collapse(std::mem::replace(&mut c.footer_width, width)) != collapse(width)
        });
        if changed {
            let entity = self.0.clone();
            window.defer(cx, move |_, cx| entity.update(cx, |_, cx| cx.notify()));
        }
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        id: &mut LayoutId,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        let focus = self.0.read(cx).focus_handle.clone();
        window.handle_input(&focus, ElementInputHandler::new(bounds, self.0.clone()), cx);
        let selection_color = cx.theme().text_selection;
        let caret_color = cx.theme().text;
        let c = self.0.read(cx);
        if !c.selected_range.is_empty()
            && let (Some(a), Some(b)) = (
                c.point_for_index(c.selected_range.start),
                c.point_for_index(c.selected_range.end),
            )
        {
            let w = bounds.size.width;
            let mut row = a.y;
            while row <= b.y {
                let x0 = if row == a.y { a.x } else { px(0.) };
                let x1 = if row == b.y { b.x } else { w };
                let r = Bounds::from_corners(
                    bounds.origin + point(x0, row),
                    bounds.origin + point(x1, row + LINE_HEIGHT),
                );
                window.paint_quad(fill(r, selection_color));
                row += LINE_HEIGHT;
            }
        }
        let caret = (c.selected_range.is_empty() && focus.is_focused(window))
            .then(|| c.point_for_index(c.cursor()))
            .flatten();
        let lines = self.0.update(cx, |c, _| std::mem::take(&mut c.lines));
        // Glyphs land on the unsnapped LayoutUnit x, as Blink paints text.
        let x = window.layout_origin_residual_x(*id);
        let mut y = px(0.);
        for line in &lines {
            let origin = bounds.origin + point(x, y);
            line.paint(origin, LINE_HEIGHT, TextAlign::Left, None, window, cx)
                .ok();
            y += line.size(LINE_HEIGHT).height;
        }
        self.0.update(cx, |c, _| c.lines = lines);
        if let Some(p) = caret {
            let r = Bounds::new(
                bounds.origin + point(p.x, p.y + px(1.5)),
                size(px(1.), LINE_HEIGHT - px(3.)),
            );
            window.paint_quad(fill(r, caret_color));
        }
    }
}
