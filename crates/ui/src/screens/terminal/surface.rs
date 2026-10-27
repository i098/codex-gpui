use super::buffer::{Buffer, Row};
use crate::{Copy, Paste, SelectAll, text};
use gpui::{
    App, Bounds, ClipboardItem, Context, ElementInputHandler, EntityInputHandler, EventEmitter,
    FocusHandle, Focusable, Global, Hsla, InteractiveElement, IntoElement, KeyDownEvent,
    MouseButton, ParentElement, Pixels, Point, Render, SharedString, StatefulInteractiveElement,
    Styled, StyledText, TextRun, UTF16Selection, UnderlineStyle, UniformListScrollHandle, Window,
    canvas, div, font, point, prelude::FluentBuilder as _, px, rgb, uniform_list,
};
use std::ops::Range;
use theme::{ActiveTheme as _, Appearance};

#[derive(Clone)]
pub struct TerminalStyle {
    pub font: SharedString,
    pub size: f32,
    pub letter_spacing: f32,
}

impl Default for TerminalStyle {
    fn default() -> Self {
        Self {
            font: crate::MONO.into(),
            size: 12.,
            letter_spacing: 0.,
        }
    }
}

impl Global for TerminalStyle {}

#[derive(Clone, Debug)]
pub enum TerminalEvent {
    Input(Vec<u8>),
    Resize { rows: u16, cols: u16 },
    New,
    Directory(usize),
}

#[derive(Clone, Default)]
pub(super) struct RowDecorations {
    pub selection: Option<((usize, usize), (usize, usize))>,
    /// Row, column and whether the terminal has focus: blurred, xterm shows an outline cursor.
    pub cursor: Option<(usize, usize, bool)>,
    pub link: Option<(usize, Range<usize>)>,
    /// A selection that ends above the viewport: xterm draws it over the first visible row,
    /// from column 0 to its end column, and the text under it is hidden.
    pub above: Option<(usize, usize)>,
}

pub struct TerminalSurface {
    pub buffer: Buffer,
    pub writable: bool,
    pub interacted: bool,
    /// The terminal had focus once; xterm then keeps drawing its cursor after a blur.
    had_focus: bool,
    focus: FocusHandle,
    scroll: UniformListScrollHandle,
    bounds: Bounds<Pixels>,
    cell_width: Pixels,
    row_height: Pixels,
    selection: Option<((usize, usize), (usize, usize))>,
    selecting: bool,
    hovered_link: Option<(usize, Range<usize>, String)>,
    marked: String,
}

impl EventEmitter<TerminalEvent> for TerminalSurface {}
impl Focusable for TerminalSurface {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl TerminalSurface {
    pub fn new(writable: bool, cx: &mut Context<Self>) -> Self {
        Self {
            buffer: Buffer::default(),
            writable,
            interacted: false,
            had_focus: false,
            focus: cx.focus_handle(),
            scroll: UniformListScrollHandle::new(),
            bounds: Bounds::default(),
            cell_width: px(7.23),
            row_height: px(16.5),
            selection: None,
            selecting: false,
            hovered_link: None,
            marked: String::new(),
        }
    }

    pub fn append(&mut self, bytes: &[u8], cx: &mut Context<Self>) {
        if bytes.is_empty() {
            return;
        }
        let follow = self.at_bottom();
        self.buffer.append(bytes);
        self.hovered_link = None;
        if follow {
            self.scroll.scroll_to_bottom();
        }
        cx.notify();
    }

    pub fn replace(&mut self, bytes: &[u8], cx: &mut Context<Self>) {
        self.buffer.replace(bytes);
        self.selection = None;
        self.hovered_link = None;
        self.scroll.scroll_to_bottom();
        cx.notify();
    }

    pub fn input(&mut self, bytes: Vec<u8>, cx: &mut Context<Self>) {
        if self.writable {
            self.interacted = true;
            self.selection = None;
            self.scroll.scroll_to_bottom();
            cx.emit(TerminalEvent::Input(bytes));
            cx.notify();
        }
    }

    fn at_bottom(&self) -> bool {
        let scroll = &self.scroll.0.borrow().base_handle;
        scroll.offset().y <= -scroll.max_offset().y + px(1.)
    }

    fn caret(&self, position: Point<Pixels>) -> (usize, usize) {
        let local = position - self.bounds.origin - self.scroll.0.borrow().base_handle.offset();
        let row = (local.y / self.row_height).floor().max(0.) as usize;
        let col = (local.x / self.cell_width).round().max(0.) as usize;
        (
            row.min(self.buffer.rows.len().saturating_sub(1)),
            col.min(usize::from(self.buffer.size.1)),
        )
    }

    fn copy(&self, cx: &mut App) {
        if let Some((anchor, head)) = self.selection {
            let text = self.buffer.selected(anchor, head);
            if !text.is_empty() {
                cx.write_to_clipboard(ClipboardItem::new_string(text));
            }
        }
    }

    fn paste(&mut self, cx: &mut Context<Self>) {
        if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
            let text = text.replace("\r\n", "\n").replace('\n', "\r");
            let bytes = if self.buffer.bracketed_paste() {
                format!("\x1b[200~{text}\x1b[201~").into_bytes()
            } else {
                text.into_bytes()
            };
            self.input(bytes, cx);
        }
    }

    fn key_down(&mut self, event: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        let key = &event.keystroke;
        let m = &key.modifiers;
        if (m.control && m.shift && key.key == "c")
            || (m.control && key.key == "c" && self.selection.is_some_and(|(a, b)| a != b))
        {
            self.copy(cx);
            cx.stop_propagation();
            return;
        }
        if m.control && m.shift && key.key == "v" {
            self.paste(cx);
            cx.stop_propagation();
            return;
        }
        if !self.writable {
            return;
        }
        if m.alt
            && !m.control
            && !m.shift
            && !m.platform
            && !self.interacted
            && let Ok(number) = key.key.parse::<usize>()
            && number > 0
        {
            cx.emit(TerminalEvent::Directory(number - 1));
            cx.stop_propagation();
            return;
        }
        if m.platform && key.key == "t" {
            cx.emit(TerminalEvent::New);
            cx.stop_propagation();
            return;
        }
        if m.platform {
            return;
        }
        let prefix = if self.buffer.application_cursor() {
            "\x1bO"
        } else {
            "\x1b["
        };
        let bytes = match key.key.as_str() {
            "enter" => Some(b"\r".to_vec()),
            "backspace" => Some(vec![127]),
            "tab" => Some(if m.shift { b"\x1b[Z".to_vec() } else { vec![9] }),
            "escape" => Some(vec![27]),
            "delete" => Some(b"\x1b[3~".to_vec()),
            "up" => Some(format!("{prefix}A").into_bytes()),
            "down" => Some(format!("{prefix}B").into_bytes()),
            "right" => Some(format!("{prefix}C").into_bytes()),
            "left" => Some(format!("{prefix}D").into_bytes()),
            "home" => Some(b"\x1b[H".to_vec()),
            "end" => Some(b"\x1b[F".to_vec()),
            "pageup" => Some(b"\x1b[5~".to_vec()),
            "pagedown" => Some(b"\x1b[6~".to_vec()),
            _ if m.control && key.key.len() == 1 => {
                Some(vec![key.key.as_bytes()[0].to_ascii_uppercase() & 31])
            }
            _ if m.alt => key
                .key_char
                .as_ref()
                .map(|s| [b"\x1b", s.as_bytes()].concat()),
            _ => None,
        };
        if let Some(bytes) = bytes {
            self.input(bytes, cx);
            cx.stop_propagation();
        }
    }
}

impl Render for TerminalSurface {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mut style = cx
            .try_global::<TerminalStyle>()
            .cloned()
            .unwrap_or_default();
        let nominal_height = style.size * 1.375;
        let rows = (f32::from(self.bounds.size.height) / nominal_height)
            .floor()
            .max(1.);
        let screen_height = px((rows * nominal_height).ceil());
        self.row_height = screen_height / rows;
        let measured_width = window
            .text_system()
            .shape_line(
                "M".into(),
                px(style.size),
                &[TextRun {
                    len: 1,
                    font: font(style.font.clone()),
                    color: cx.theme().text,
                    background_color: None,
                    underline: None,
                    strikethrough: None,
                }],
                None,
            )
            .width
            .max(px(1.));
        // Xterm rounds the canvas width and measures character widths across 32 repeats.
        let columns = if self.bounds.size.width > px(17.) {
            ((f32::from(self.bounds.size.width) - 17.) / f32::from(measured_width))
                .floor()
                .max(1.)
        } else {
            f32::from(self.buffer.size.1)
        };
        self.cell_width = px((f32::from(measured_width) * columns).round() / columns);
        style.letter_spacing =
            f32::from(self.cell_width) - (f32::from(measured_width) * 32.).round() / 32.;
        let entity = cx.entity();
        let layout = entity.clone();
        let measure = canvas(
            move |bounds, _, cx| {
                layout.update(cx, |surface, cx| {
                    surface.bounds = bounds;
                    let cols = ((bounds.size.width - px(17.)) / surface.cell_width)
                        .floor()
                        .clamp(1., 4096.) as u16;
                    let rows = (bounds.size.height / surface.row_height)
                        .floor()
                        .clamp(1., 4096.) as u16;
                    let follow = surface.at_bottom();
                    if surface.buffer.resize(rows, cols) {
                        surface.selection = None;
                        if follow {
                            surface.scroll.scroll_to_bottom();
                        }
                        cx.emit(TerminalEvent::Resize { rows, cols });
                        cx.notify();
                    }
                });
            },
            move |bounds, (), window, cx| {
                let (writable, focus) = {
                    let surface = entity.read(cx);
                    (surface.writable, surface.focus.clone())
                };
                if writable {
                    window.handle_input(
                        &focus,
                        ElementInputHandler::new(bounds, entity.clone()),
                        cx,
                    );
                }
            },
        )
        .absolute()
        .top_0()
        .left_0()
        .size_full();
        let focused = self.focus.is_focused(window);
        self.had_focus |= focused;
        let rows_entity = cx.entity();
        let decorations = RowDecorations {
            above: None,
            selection: self.selection,
            cursor: (!self.buffer.cursor_hidden && (focused || self.had_focus)).then_some((
                self.buffer.cursor.0,
                self.buffer.cursor.1,
                focused,
            )),
            link: self
                .hovered_link
                .as_ref()
                .map(|(row, columns, _)| (*row, columns.clone())),
        };
        let cell_width = self.cell_width;
        let row_height = self.row_height;
        let foreground = if cx.theme().appearance == Appearance::Dark {
            rgb(0xdfdfdf).into()
        } else {
            cx.theme().text
        };
        let list = uniform_list(
            "terminal-rows",
            self.buffer.rows.len(),
            move |range, _, cx| {
                let surface = rows_entity.read(cx);
                // The list's current scroll position (not the previous frame's) decides the
                // overlay of a selection that ends above the viewport.
                let first_visible = range.start;
                let decorations = RowDecorations {
                    above: surface.selection.and_then(|(a, b)| {
                        let end = a.max(b);
                        (end.0 < first_visible).then_some((first_visible, end.1))
                    }),
                    ..decorations.clone()
                };
                range
                    .map(|index| {
                        row_element(
                            &surface.buffer.rows[index],
                            index,
                            &style,
                            &decorations,
                            gpui::size(cell_width, row_height),
                            foreground,
                            cx,
                        )
                    })
                    .collect()
            },
        )
        .w_full()
        .h(screen_height)
        .track_scroll(&self.scroll);
        let empty = self.buffer.is_empty() && !self.writable;
        div()
            .id("terminal-output")
            .size_full()
            .min_w_0()
            .relative()
            .bg(cx.theme().surface)
            .track_focus(&self.focus)
            .key_context("Terminal")
            .on_key_down(cx.listener(Self::key_down))
            .on_action(cx.listener(|this, _: &Copy, _, cx| {
                this.copy(cx);
                cx.stop_propagation();
            }))
            .on_action(cx.listener(|this, _: &Paste, _, cx| {
                this.paste(cx);
                cx.stop_propagation();
            }))
            .on_action(cx.listener(|this, _: &SelectAll, _, cx| {
                this.selection = Some((
                    (0, 0),
                    (
                        this.buffer.rows.len().saturating_sub(1),
                        usize::from(this.buffer.size.1),
                    ),
                ));
                cx.stop_propagation();
                cx.notify();
            }))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, event: &gpui::MouseDownEvent, window, cx| {
                    this.focus.focus(window, cx);
                    let at = this.caret(event.position);
                    if event.modifiers.platform
                        && let Some((_, url)) =
                            this.buffer.rows.get(at.0).and_then(|row| row.link_at(at.1))
                    {
                        cx.open_url(&url);
                        cx.stop_propagation();
                        return;
                    }
                    let anchor = if event.modifiers.shift {
                        this.selection.map_or(at, |s| s.0)
                    } else {
                        at
                    };
                    this.selection = Some((anchor, at));
                    this.selecting = true;
                    cx.stop_propagation();
                    cx.notify();
                }),
            )
            .on_mouse_move(cx.listener(|this, event: &gpui::MouseMoveEvent, _, cx| {
                if this.selecting {
                    let at = this.caret(event.position);
                    if let Some(selection) = &mut this.selection {
                        selection.1 = at;
                    }
                    cx.notify();
                } else {
                    let (row, column) = this.caret(event.position);
                    if this
                        .hovered_link
                        .as_ref()
                        .is_some_and(|(index, columns, _)| {
                            *index == row && columns.contains(&column)
                        })
                    {
                        return;
                    }
                    let link = this
                        .buffer
                        .rows
                        .get(row)
                        .and_then(|line| line.link_at(column))
                        .map(|(columns, url)| (row, columns, url));
                    if link != this.hovered_link {
                        this.hovered_link = link;
                        cx.notify();
                    }
                }
            }))
            .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                if !hovered && this.hovered_link.take().is_some() {
                    cx.notify();
                }
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    this.selecting = false;
                    cx.notify();
                }),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    this.selecting = false;
                    cx.notify();
                }),
            )
            .when(empty, |d| {
                d.child(text(
                    "No output yet",
                    12.,
                    16.5,
                    400.,
                    cx.theme().text_tertiary,
                ))
            })
            .when(!empty, |d| {
                d.child(div().w_full().h(screen_height).layer().child(list))
            })
            .child(measure)
    }
}

pub(super) fn row_element(
    row: &Row,
    index: usize,
    style: &TerminalStyle,
    decorations: &RowDecorations,
    cell_size: gpui::Size<Pixels>,
    foreground: Hsla,
    cx: &App,
) -> gpui::Div {
    let gpui::Size {
        width: cell_width,
        height: row_height,
    } = cell_size;
    let t = cx.theme();
    let dark = t.appearance == Appearance::Dark;
    let mut content = String::new();
    let mut runs = Vec::new();
    let features = textsys::letter_spacing(style.letter_spacing);
    for (column, cell) in row.cells.iter().enumerate() {
        if cell.is_wide_continuation() {
            continue;
        }
        let chars = if cell.has_contents() {
            cell.contents()
        } else {
            " "
        };
        content.push_str(chars);
        let (fg, bg) = if cell.inverse() {
            (cell.bgcolor(), cell.fgcolor())
        } else {
            (cell.fgcolor(), cell.bgcolor())
        };
        let mut color = terminal_color(
            fg,
            dark,
            if cell.inverse() {
                t.surface
            } else {
                foreground
            },
            t.surface,
        );
        if cell.dim() {
            color.a *= 0.5;
        }
        let mut face = font(style.font.clone());
        face.features = features.clone();
        face.weight = gpui::FontWeight(if cell.bold() { 700. } else { 400. });
        if cell.italic() {
            face.style = gpui::FontStyle::Italic;
        }
        runs.push(TextRun {
            len: chars.len(),
            font: face,
            color,
            background_color: (bg != vt100::Color::Default || cell.inverse())
                .then(|| terminal_color(bg, dark, foreground, t.surface)),
            underline: (cell.underline()
                || decorations
                    .link
                    .as_ref()
                    .is_some_and(|(row, columns)| *row == index && columns.contains(&column)))
            .then_some(UnderlineStyle {
                thickness: px(1.),
                color: Some(color),
                wavy: false,
            }),
            strikethrough: None,
        });
    }
    let mut line = div()
        .relative()
        .h(row_height)
        .w_full()
        .text_size(px(style.size))
        .line_height(row_height)
        .font_family(style.font.clone())
        .whitespace_nowrap();
    if let Some((a, b)) = decorations.selection {
        let (start, end) = if a <= b { (a, b) } else { (b, a) };
        if start.0 <= index && index <= end.0 {
            let left = if index == start.0 { start.1 } else { 0 };
            let right = if index == end.0 {
                end.1
            } else {
                row.cells.len()
            };
            line = line.child(
                div()
                    .absolute()
                    .left(cell_width * left as f32)
                    .top_0()
                    .w(cell_width * right.saturating_sub(left) as f32)
                    .h(row_height)
                    .bg(if dark {
                        gpui::rgba(0xffffff1b)
                    } else {
                        gpui::rgba(0x00000019)
                    }),
            );
        }
    }
    line = line.child(StyledText::new(content).with_runs(runs));
    if let Some((row, columns)) = decorations.above
        && row == index
    {
        let fill = if dark {
            gpui::rgba(0xffffff1b)
        } else {
            gpui::rgba(0x00000019)
        };
        let under = gpui::Rgba::from(t.surface);
        let over = |s: f32, u: f32| s * fill.a + u * (1. - fill.a);
        let opaque: Hsla = gpui::Rgba {
            r: over(fill.r, under.r),
            g: over(fill.g, under.g),
            b: over(fill.b, under.b),
            a: 1.,
        }
        .into();
        line = line.child(
            div()
                .absolute()
                .left_0()
                .top_0()
                .w(cell_width * columns as f32)
                .h(row_height)
                .bg(opaque),
        );
    }
    if let Some((cursor_row, col, focused)) = decorations.cursor
        && cursor_row == index
    {
        let cursor = div()
            .absolute()
            .left(cell_width * col as f32)
            .top_0()
            .h(row_height);
        line = line.child(if focused {
            cursor.w(px(1.)).bg(foreground)
        } else {
            cursor.w(cell_width).border_1().border_color(foreground)
        });
    }
    line
}

fn terminal_color(color: vt100::Color, dark: bool, default: Hsla, background: Hsla) -> Hsla {
    match color {
        vt100::Color::Default => default,
        vt100::Color::Rgb(r, g, b) => {
            rgb((u32::from(r) << 16) | (u32::from(g) << 8) | u32::from(b)).into()
        }
        vt100::Color::Idx(index) if index < 16 => {
            let colors = if dark {
                [
                    0xffffff, 0xff6764, 0x40c977, 0xffd240, 0x3a83f7, 0xad7bf9, 0x3a83f7, 0xffffff,
                ]
            } else {
                [
                    0x1a1c1f, 0xe02e2a, 0x00a240, 0xffc300, 0x3a83f7, 0x924ff7, 0x3a83f7, 0x1a1c1f,
                ]
            };
            let mut color: Hsla = rgb(colors[usize::from(index % 8)]).into();
            if index == 0 {
                color.a = if dark { 0.498 } else { 0.495 };
            }
            if index == 8 {
                color.a = if dark { 0.71 } else { 0.695 };
            }
            if color.a < 1. {
                crate::over(color, background)
            } else {
                color
            }
        }
        vt100::Color::Idx(index) if index >= 232 => {
            let c = u32::from(8 + (index - 232) * 10);
            rgb((c << 16) | (c << 8) | c).into()
        }
        vt100::Color::Idx(index) => {
            let values = [0, 95, 135, 175, 215, 255];
            let n = usize::from(index - 16);
            rgb((values[n / 36] << 16) | (values[n / 6 % 6] << 8) | values[n % 6]).into()
        }
    }
}

impl EntityInputHandler for TerminalSurface {
    fn text_for_range(
        &mut self,
        _: Range<usize>,
        actual: &mut Option<Range<usize>>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<String> {
        *actual = Some(0..self.marked.encode_utf16().count());
        Some(self.marked.clone())
    }
    fn selected_text_range(
        &mut self,
        _: bool,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        let end = self.marked.encode_utf16().count();
        Some(UTF16Selection {
            range: end..end,
            reversed: false,
        })
    }
    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        (!self.marked.is_empty()).then(|| 0..self.marked.encode_utf16().count())
    }
    fn unmark_text(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        self.marked.clear();
        cx.notify();
    }
    fn replace_text_in_range(
        &mut self,
        _: Option<Range<usize>>,
        text: &str,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.marked.clear();
        self.input(text.as_bytes().to_vec(), cx);
    }
    fn replace_and_mark_text_in_range(
        &mut self,
        _: Option<Range<usize>>,
        text: &str,
        _: Option<Range<usize>>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.marked = text.to_owned();
        cx.notify();
    }
    fn bounds_for_range(
        &mut self,
        _: Range<usize>,
        _: Bounds<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let (row, col) = self.buffer.cursor;
        Some(Bounds::new(
            self.bounds.origin
                + point(self.cell_width * col as f32, self.row_height * row as f32)
                + self.scroll.0.borrow().base_handle.offset(),
            gpui::size(self.cell_width, self.row_height),
        ))
    }
    fn character_index_for_point(
        &mut self,
        _: Point<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<usize> {
        Some(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ansi_black_stays_visible_on_white_backgrounds() {
        let background = rgb(0x181818).into();
        let black: gpui::Rgba =
            terminal_color(vt100::Color::Idx(0), true, background, background).into();
        let white: gpui::Rgba =
            terminal_color(vt100::Color::Idx(7), true, background, background).into();
        assert_eq!(black.a, 1.);
        assert!(black.r < white.r && black.g < white.g && black.b < white.b);
    }
}
