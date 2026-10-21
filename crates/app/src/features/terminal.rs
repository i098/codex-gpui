use std::{
    io::{Read, Write},
    ops::Range,
    sync::mpsc,
};

use crate::{Root, Route, SidePanel};
use anyhow::Result;
use gpui::{
    AppContext as _, Bounds, Context, ElementInputHandler, EntityInputHandler, EventEmitter,
    FocusHandle, Focusable, InteractiveElement, IntoElement, KeyDownEvent, MouseButton,
    ParentElement, Pixels, Point, Render, SharedString, StatefulInteractiveElement, Styled,
    UTF16Selection, Window, canvas, div, point, prelude::FluentBuilder as _, px, size,
};
use portable_pty::{CommandBuilder, MasterPty, PtySize};
use theme::ActiveTheme as _;

pub(super) struct Closed;

enum Output {
    Bytes(Vec<u8>),
    Error(String),
    Finished,
}

struct Session {
    master: Box<dyn MasterPty + Send>,
    child: Option<Box<dyn portable_pty::Child + Send + Sync>>,
    input: mpsc::Sender<Vec<u8>>,
}

impl Session {
    fn spawn(command: CommandBuilder) -> Result<(Self, async_channel::Receiver<Output>)> {
        let pair = portable_pty::native_pty_system().openpty(PtySize {
            rows: 24,
            cols: 80,
            pixel_width: 0,
            pixel_height: 0,
        })?;
        let mut reader = pair.master.try_clone_reader()?;
        let mut writer = pair.master.take_writer()?;
        let child = pair.slave.spawn_command(command)?;
        drop(pair.slave);
        let (output, receive) = async_channel::bounded(32);
        let errors = output.clone();
        let (input, pending) = mpsc::channel::<Vec<u8>>();
        std::thread::spawn(move || {
            for bytes in pending {
                if let Err(error) = writer.write_all(&bytes).and_then(|()| writer.flush()) {
                    let _ = errors.send_blocking(Output::Error(error.to_string()));
                    break;
                }
            }
        });
        std::thread::spawn(move || {
            let mut bytes = [0u8; 8192];
            loop {
                match reader.read(&mut bytes) {
                    Ok(0) => break,
                    Ok(count) => {
                        if output
                            .send_blocking(Output::Bytes(bytes[..count].to_vec()))
                            .is_err()
                        {
                            return;
                        }
                    }
                    Err(error) => {
                        if error.raw_os_error() != Some(5) {
                            let _ = output.send_blocking(Output::Error(error.to_string()));
                        }
                        break;
                    }
                }
            }
            let _ = output.send_blocking(Output::Finished);
        });
        Ok((
            Self {
                master: pair.master,
                child: Some(child),
                input,
            },
            receive,
        ))
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            std::thread::spawn(move || {
                if child.try_wait().ok().flatten().is_none() {
                    let _ = child.kill();
                }
                let _ = child.wait();
            });
        }
    }
}

#[derive(Clone)]
struct Run {
    text: SharedString,
    columns: u16,
    foreground: vt100::Color,
    background: vt100::Color,
    bold: bool,
    underline: bool,
    inverse: bool,
}

pub(crate) struct Terminal {
    focus: FocusHandle,
    session: Option<Session>,
    parser: vt100::Parser,
    lines: Vec<Vec<Run>>,
    error: Option<String>,
    finished: bool,
    bounds: Bounds<Pixels>,
    cell_width: Pixels,
    selection: Option<((u16, u16), (u16, u16))>,
    composing: String,
    pending_input: Vec<Vec<u8>>,
    docked: bool,
}

impl EventEmitter<Closed> for Terminal {}
impl Focusable for Terminal {
    fn focus_handle(&self, _: &gpui::App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Terminal {
    pub(super) fn new(cwd: String, cx: &mut Context<Self>) -> Self {
        let mut command = CommandBuilder::new(std::env::var("SHELL").unwrap_or_else(|_| {
            if cfg!(windows) {
                "cmd.exe".into()
            } else {
                "/bin/sh".into()
            }
        }));
        command.cwd(cwd);
        command.env("TERM", "xterm-256color");
        command.env("COLORTERM", "truecolor");
        Self::with_command(command, cx)
    }

    pub(super) fn with_command(command: CommandBuilder, cx: &mut Context<Self>) -> Self {
        cx.spawn(async move |this, cx| {
            let started = cx
                .background_executor()
                .spawn(async move { Session::spawn(command) })
                .await;
            let receive = match started {
                Ok((session, receive)) => {
                    if this
                        .update(cx, |this, cx| {
                            let (rows, cols) = this.parser.screen().size();
                            if let Err(error) = session.master.resize(PtySize {
                                rows,
                                cols,
                                pixel_width: 0,
                                pixel_height: 0,
                            }) {
                                this.error = Some(error.to_string());
                            }
                            for bytes in this.pending_input.drain(..) {
                                if let Err(error) = session.input.send(bytes) {
                                    this.error = Some(error.to_string());
                                    break;
                                }
                            }
                            this.session = Some(session);
                            cx.notify();
                        })
                        .is_err()
                    {
                        return;
                    }
                    receive
                }
                Err(error) => {
                    let _ = this.update(cx, |this, cx| {
                        this.error = Some(error.to_string());
                        cx.notify();
                    });
                    return;
                }
            };
            while let Ok(output) = receive.recv().await {
                if this
                    .update(cx, |this, cx| {
                        match output {
                            Output::Bytes(bytes) => this.parser.process(&bytes),
                            Output::Error(error) => this.error = Some(error),
                            Output::Finished => this.finished = true,
                        }
                        this.update_lines();
                        cx.notify();
                    })
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
        Self {
            focus: cx.focus_handle(),
            session: None,
            parser: vt100::Parser::new(24, 80, 1000),
            lines: Vec::new(),
            error: None,
            finished: false,
            bounds: Bounds::default(),
            cell_width: px(7.2),
            selection: None,
            composing: String::new(),
            pending_input: Vec::new(),
            docked: true,
        }
    }

    fn update_lines(&mut self) {
        let screen = self.parser.screen();
        let (rows, columns) = screen.size();
        self.lines.clear();
        for row in 0..rows {
            let mut runs: Vec<Run> = Vec::new();
            let mut contents = String::new();
            for column in 0..columns {
                let Some(cell) = screen.cell(row, column) else {
                    continue;
                };
                if cell.is_wide_continuation() {
                    continue;
                }
                let mut text = cell.contents();
                if text.is_empty() {
                    text.push(' ');
                }
                let cells = if cell.is_wide() { 2 } else { 1 };
                if let Some(run) = runs.last_mut().filter(|run| {
                    run.foreground == cell.fgcolor()
                        && run.background == cell.bgcolor()
                        && run.bold == cell.bold()
                        && run.underline == cell.underline()
                        && run.inverse == cell.inverse()
                }) {
                    contents.push_str(&text);
                    run.columns += cells;
                } else {
                    if let Some(previous) = runs.last_mut() {
                        previous.text = std::mem::take(&mut contents).into();
                    }
                    contents.push_str(&text);
                    runs.push(Run {
                        text: SharedString::default(),
                        columns: cells,
                        foreground: cell.fgcolor(),
                        background: cell.bgcolor(),
                        bold: cell.bold(),
                        underline: cell.underline(),
                        inverse: cell.inverse(),
                    });
                }
            }
            if let Some(last) = runs.last_mut() {
                last.text = contents.into();
            }
            self.lines.push(runs);
        }
    }

    fn send(&mut self, bytes: Vec<u8>, cx: &mut Context<Self>) {
        if self.finished || self.error.is_some() {
            return;
        }
        self.parser.set_scrollback(0);
        self.selection = None;
        if let Some(session) = &self.session {
            if let Err(error) = session.input.send(bytes) {
                self.error = Some(error.to_string());
            }
        } else {
            self.pending_input.push(bytes);
        }
        cx.notify();
    }

    fn key(&mut self, event: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        let stroke = &event.keystroke;
        if stroke.modifiers.platform {
            return;
        }
        let app = self.parser.screen().application_cursor();
        let bytes = match stroke.key.as_str() {
            "enter" => b"\r".to_vec(),
            "tab" if stroke.modifiers.shift => b"\x1b[Z".to_vec(),
            "tab" => b"\t".to_vec(),
            "escape" => b"\x1b".to_vec(),
            "backspace" => vec![127],
            "delete" => b"\x1b[3~".to_vec(),
            "up" => if app { b"\x1bOA" } else { b"\x1b[A" }.to_vec(),
            "down" => if app { b"\x1bOB" } else { b"\x1b[B" }.to_vec(),
            "right" => if app { b"\x1bOC" } else { b"\x1b[C" }.to_vec(),
            "left" => if app { b"\x1bOD" } else { b"\x1b[D" }.to_vec(),
            "home" => b"\x1b[H".to_vec(),
            "end" => b"\x1b[F".to_vec(),
            "pageup" => b"\x1b[5~".to_vec(),
            "pagedown" => b"\x1b[6~".to_vec(),
            key if stroke.modifiers.control
                && !stroke.modifiers.shift
                && !stroke.modifiers.alt
                && key.len() == 1 =>
            {
                vec![key.as_bytes()[0].to_ascii_uppercase() & 0x1f]
            }
            _ => return,
        };
        self.send(bytes, cx);
        cx.stop_propagation();
    }

    fn cell_at(&self, position: Point<Pixels>) -> (u16, u16) {
        let (rows, columns) = self.parser.screen().size();
        let point = position - self.bounds.origin;
        (
            ((f32::from(point.y) / 16.5).floor().max(0.) as u16).min(rows - 1),
            ((point.x / self.cell_width).floor().max(0.) as u16).min(columns - 1),
        )
    }

    fn selected_text(&self) -> Option<String> {
        let (mut start, mut end) = self.selection?;
        if start > end {
            std::mem::swap(&mut start, &mut end);
        }
        Some(self.parser.screen().contents_between(
            start.0,
            start.1,
            end.0,
            end.1.saturating_add(1),
        ))
    }

    fn resize(&mut self, bounds: Bounds<Pixels>, cell: Pixels, cx: &mut Context<Self>) {
        self.bounds = bounds;
        self.cell_width = cell;
        let rows = (f32::from(bounds.size.height) / 16.5).floor().max(1.) as u16;
        let columns = (bounds.size.width / cell).floor().max(1.) as u16;
        if self.parser.screen().size() == (rows, columns) {
            return;
        }
        self.parser.set_size(rows, columns);
        if let Some(session) = &self.session {
            if let Err(error) = session.master.resize(PtySize {
                rows,
                cols: columns,
                pixel_width: u16::from(columns).saturating_mul(f32::from(cell).ceil() as u16),
                pixel_height: rows.saturating_mul(17),
            }) {
                self.error = Some(error.to_string());
            }
        }
        self.update_lines();
        cx.notify();
    }
}

fn color(color: vt100::Color, default: gpui::Hsla) -> gpui::Hsla {
    let rgb = match color {
        vt100::Color::Default => return default,
        vt100::Color::Rgb(r, g, b) => (u32::from(r) << 16) | (u32::from(g) << 8) | u32::from(b),
        vt100::Color::Idx(index) => {
            const ANSI: [u32; 16] = [
                0x000000, 0xcd0000, 0x00cd00, 0xcdcd00, 0x0000ee, 0xcd00cd, 0x00cdcd, 0xe5e5e5,
                0x7f7f7f, 0xff0000, 0x00ff00, 0xffff00, 0x5c5cff, 0xff00ff, 0x00ffff, 0xffffff,
            ];
            if index < 16 {
                ANSI[index as usize]
            } else if index >= 232 {
                let value = 8 + u32::from(index - 232) * 10;
                value * 0x010101
            } else {
                let index = u32::from(index - 16);
                let component = |v| if v == 0 { 0 } else { 55 + v * 40 };
                (component(index / 36) << 16)
                    | (component(index / 6 % 6) << 8)
                    | component(index % 6)
            }
        }
    };
    gpui::rgb(rgb).into()
}

impl Render for Terminal {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let foreground = gpui::rgb(0xdfdfdf).into();
        let entity = cx.entity();
        let focus = self.focus.clone();
        let mut font = gpui::font(ui::MONO);
        font.weight = gpui::FontWeight(400.);
        let cell = window
            .text_system()
            .shape_line(
                "M".into(),
                px(12.),
                &[gpui::TextRun {
                    len: 1,
                    font,
                    color: foreground,
                    background_color: None,
                    underline: None,
                    strikethrough: None,
                }],
                None,
            )
            .width;
        let rows = self
            .lines
            .iter()
            .map(|line| {
                div().h(px(16.5)).flex().children(line.iter().map(|run| {
                    let mut fg = color(run.foreground, foreground);
                    let mut bg = color(run.background, theme.surface);
                    if run.inverse {
                        std::mem::swap(&mut fg, &mut bg);
                    }
                    ui::text(
                        run.text.clone(),
                        12.,
                        16.5,
                        if run.bold { 700. } else { 400. },
                        fg,
                    )
                    .font_family(ui::MONO)
                    .w(cell * f32::from(run.columns))
                    .flex_none()
                    .when(
                        run.background != vt100::Color::Default || run.inverse,
                        |d| d.bg(bg),
                    )
                    .when(run.underline, |d| d.underline())
                }))
            })
            .collect::<Vec<_>>();
        let cursor = self.parser.screen().cursor_position();
        let cursor_visible = self.session.is_some()
            && self.focus.is_focused(window)
            && !self.parser.screen().hide_cursor()
            && self.parser.screen().scrollback() == 0;
        let columns = self.parser.screen().size().1;
        let selection_color = theme.text_selection;
        let highlights: Vec<_> = self
            .selection
            .into_iter()
            .flat_map(|(mut start, mut end)| {
                if start > end {
                    std::mem::swap(&mut start, &mut end);
                }
                (start.0..=end.0)
                    .map(move |row| {
                        let left = if row == start.0 { start.1 } else { 0 };
                        let right = if row == end.0 {
                            end.1.saturating_add(1)
                        } else {
                            columns
                        };
                        div()
                            .absolute()
                            .left(cell * f32::from(left))
                            .top(px(16.5 * f32::from(row)))
                            .w(cell * f32::from(right.saturating_sub(left)))
                            .h(px(16.5))
                            .bg(selection_color)
                    })
                    .collect::<Vec<_>>()
            })
            .collect();
        div()
            .id("interactive-terminal")
            .size_full()
            .flex()
            .flex_col()
            .bg(theme.surface)
            .key_context("Terminal Editable")
            .track_focus(&self.focus)
            .capture_key_down(cx.listener(Self::key))
            .on_action(cx.listener(|this, _: &crate::Copy, _, cx| {
                if let Some(text) = this.selected_text() {
                    cx.write_to_clipboard(gpui::ClipboardItem::new_string(text));
                }
            }))
            .on_action(cx.listener(|this, _: &crate::SelectAll, _, cx| {
                let (rows, cols) = this.parser.screen().size();
                this.selection = Some(((0, 0), (rows - 1, cols - 1)));
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &crate::Paste, _, cx| {
                if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
                    let text = text.replace("\r\n", "\n").replace('\n', "\r");
                    let bytes = if this.parser.screen().bracketed_paste() {
                        format!("\x1b[200~{text}\x1b[201~").into_bytes()
                    } else {
                        text.into_bytes()
                    };
                    this.send(bytes, cx);
                }
            }))
            .on_action(cx.listener(|_, _: &crate::CloseWindow, _, cx| cx.emit(Closed)))
            .when(self.docked, |d| {
                d.child(
                    div()
                        .h(px(36.))
                        .px(px(12.))
                        .flex()
                        .items_center()
                        .justify_between()
                        .child(ui::text(
                            if self.finished {
                                "Terminal (exited)"
                            } else {
                                "Terminal"
                            },
                            13.,
                            18.,
                            430.,
                            theme.text_secondary,
                        ))
                        .child(
                            div()
                                .id("close-interactive-terminal")
                                .role(gpui::Role::Button)
                                .aria_label("Close terminal")
                                .child(ui::icon::icon("close", 14., theme.text_tertiary))
                                .on_click(cx.listener(|_, _, _, cx| cx.emit(Closed))),
                        ),
                )
            })
            .children(
                self.error
                    .clone()
                    .map(|error| ui::text(error, 12., 16., 430., theme.text_error)),
            )
            .when(self.session.is_none() && self.error.is_none(), |d| {
                d.child(ui::text(
                    "Starting terminal…",
                    12.,
                    16.5,
                    400.,
                    theme.text_tertiary,
                ))
            })
            .child(
                div()
                    .id("terminal-grid")
                    .flex_1()
                    .min_h_0()
                    .relative()
                    .ml(px(16.))
                    .mt(px(8.))
                    .mb(px(12.))
                    .overflow_hidden()
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, event: &gpui::MouseDownEvent, window, cx| {
                            this.focus.focus(window, cx);
                            let cell = this.cell_at(event.position);
                            this.selection = Some((cell, cell));
                            cx.notify();
                        }),
                    )
                    .on_mouse_move(cx.listener(|this, event: &gpui::MouseMoveEvent, _, cx| {
                        if event.pressed_button == Some(MouseButton::Left) {
                            if let Some((start, _)) = this.selection {
                                this.selection = Some((start, this.cell_at(event.position)));
                                cx.notify();
                            }
                        }
                    }))
                    .on_scroll_wheel(cx.listener(|this, event: &gpui::ScrollWheelEvent, _, cx| {
                        let delta = match event.delta {
                            gpui::ScrollDelta::Pixels(point) => f32::from(point.y) / 16.5,
                            gpui::ScrollDelta::Lines(point) => point.y,
                        };
                        let old = this.parser.screen().scrollback() as isize;
                        this.parser
                            .set_scrollback((old + delta.round() as isize).max(0) as usize);
                        this.update_lines();
                        cx.notify();
                        cx.stop_propagation();
                    }))
                    .children(highlights)
                    .child(div().flex().flex_col().children(rows))
                    .when(cursor_visible, |d| {
                        d.child(
                            div()
                                .absolute()
                                .left(cell * f32::from(cursor.1))
                                .top(px(16.5 * f32::from(cursor.0)))
                                .w(px(1.))
                                .h(px(16.5))
                                .bg(foreground),
                        )
                    })
                    .when(!self.composing.is_empty(), |d| {
                        d.child(
                            ui::text(self.composing.clone(), 12., 16.5, 400., foreground)
                                .font_family(ui::MONO)
                                .absolute()
                                .left(cell * f32::from(cursor.1))
                                .top(px(16.5 * f32::from(cursor.0)))
                                .underline(),
                        )
                    })
                    .child(
                        canvas(
                            move |bounds, _, cx| {
                                let _ = entity.update(cx, |this, cx| this.resize(bounds, cell, cx));
                            },
                            {
                                let entity = cx.entity();
                                move |bounds, (), window, cx| {
                                    window.handle_input(
                                        &focus,
                                        ElementInputHandler::new(bounds, entity.clone()),
                                        cx,
                                    )
                                }
                            },
                        )
                        .absolute()
                        .size_full(),
                    ),
            )
    }
}

impl EntityInputHandler for Terminal {
    fn text_for_range(
        &mut self,
        range: Range<usize>,
        actual: &mut Option<Range<usize>>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<String> {
        let units: Vec<_> = self.composing.encode_utf16().collect();
        let start = range.start.min(units.len());
        let end = range.end.min(units.len()).max(start);
        *actual = Some(start..end);
        Some(String::from_utf16_lossy(&units[start..end]))
    }
    fn selected_text_range(
        &mut self,
        _: bool,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        let end = self.composing.encode_utf16().count();
        Some(UTF16Selection {
            range: end..end,
            reversed: false,
        })
    }
    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        (!self.composing.is_empty()).then(|| 0..self.composing.encode_utf16().count())
    }
    fn unmark_text(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        self.composing.clear();
        cx.notify();
    }
    fn replace_text_in_range(
        &mut self,
        _: Option<Range<usize>>,
        text: &str,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.composing.clear();
        self.send(text.as_bytes().to_vec(), cx);
    }
    fn replace_and_mark_text_in_range(
        &mut self,
        _: Option<Range<usize>>,
        text: &str,
        _: Option<Range<usize>>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.composing = text.to_owned();
        cx.notify();
    }
    fn bounds_for_range(
        &mut self,
        _: Range<usize>,
        bounds: Bounds<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let (row, column) = self.parser.screen().cursor_position();
        Some(Bounds::new(
            bounds.origin
                + point(
                    self.cell_width * f32::from(column),
                    px(16.5 * f32::from(row)),
                ),
            size(self.cell_width, px(16.5)),
        ))
    }
    fn character_index_for_point(
        &mut self,
        _: Point<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<usize> {
        Some(self.composing.encode_utf16().count())
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn a_real_pty_accepts_input_and_updates_the_ansi_screen() -> Result<()> {
        #[cfg(target_os = "macos")]
        std::panic::set_hook(Box::new(|info| {
            eprintln!("{info}");
            std::process::exit(101);
        }));
        let mut command = CommandBuilder::new("/bin/sh");
        command.args(["-c", "printf '\\033[31mready\\033[0m\\n'; IFS= read -r value; printf 'received:%s\\n' \"$value\""]);
        let (session, receive) = Session::spawn(command)?;
        session.input.send(b"hello\r".to_vec())?;
        let mut parser = vt100::Parser::new(24, 80, 100);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while std::time::Instant::now() < deadline {
            match receive.try_recv() {
                Ok(Output::Bytes(bytes)) => parser.process(&bytes),
                Ok(Output::Error(error)) => anyhow::bail!("{error}"),
                Ok(Output::Finished) => {
                    let contents = parser.screen().contents();
                    anyhow::ensure!(
                        contents.contains("ready") && contents.contains("received:hello"),
                        "{contents}"
                    );
                    return Ok(());
                }
                Err(async_channel::TryRecvError::Empty) => {
                    std::thread::sleep(std::time::Duration::from_millis(5))
                }
                Err(async_channel::TryRecvError::Closed) => break,
            }
        }
        anyhow::bail!("The terminal did not finish its input/output exchange")
    }
}

impl Root {
    pub(crate) fn show_terminal(
        &mut self,
        force_bottom: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.shortcut_state.terminal.is_none() {
            let cwd = self
                .thread
                .as_ref()
                .map(|view| view.read(cx).cwd().to_owned())
                .or_else(|| self.state.project_root.clone())
                .unwrap_or_else(|| self.cwd.clone());
            let terminal = cx.new(|cx| Terminal::new(cwd, cx));
            cx.subscribe_in(&terminal, window, |this, _, _: &Closed, window, cx| {
                this.shortcut_state.terminal = None;
                this.shortcut_state.bottom_open = false;
                if this.side == Some(SidePanel::Console) {
                    this.close_side(SidePanel::Console, window, cx);
                } else {
                    this.sync_terminal_panel(cx);
                    this.composer(cx).focus_handle(cx).focus(window, cx);
                    cx.notify();
                }
            })
            .detach();
            self.shortcut_state.terminal = Some(terminal);
        }
        let bottom = force_bottom
            || !matches!(self.state.route, Route::Thread(_))
            || self.config["desktop"]["defaultTerminalLocation"]
                .as_str()
                .unwrap_or("bottom")
                == "bottom";
        self.shortcut_state.bottom_open = bottom;
        if let Some(terminal) = &self.shortcut_state.terminal {
            terminal.update(cx, |terminal, cx| {
                terminal.docked = bottom;
                cx.notify();
            });
        }
        if bottom {
            if self.side == Some(SidePanel::Console) {
                self.close_side(SidePanel::Console, window, cx);
            }
            self.sync_terminal_panel(cx);
        } else {
            self.set_side(SidePanel::Console, window, cx);
        }
        if let Some(terminal) = &self.shortcut_state.terminal {
            terminal.focus_handle(cx).focus(window, cx);
        }
        cx.notify();
    }

    pub(crate) fn toggle_bottom_panel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.shortcut_state.bottom_open {
            self.shortcut_state.bottom_open = false;
            self.sync_terminal_panel(cx);
            self.composer(cx).focus_handle(cx).focus(window, cx);
            cx.notify();
        } else {
            self.show_terminal(true, window, cx);
        }
    }

    pub(crate) fn sync_terminal_panel(&mut self, cx: &mut Context<Self>) {
        let terminal = self
            .shortcut_state
            .bottom_open
            .then(|| self.shortcut_state.terminal.clone())
            .flatten()
            .map(gpui::AnyView::from);
        let eligible = matches!(self.state.route, Route::Home | Route::Thread(_));
        self.shell.update(cx, |shell, cx| {
            shell.set_bottom(if eligible { terminal } else { None }, cx)
        });
    }

    pub fn terminal_visible(&self) -> bool {
        self.shortcut_state.terminal.is_some()
            && (self.shortcut_state.bottom_open || self.side == Some(SidePanel::Console))
    }

    pub fn terminal_contents(&self, cx: &gpui::App) -> Option<String> {
        self.shortcut_state
            .terminal
            .as_ref()
            .map(|terminal| terminal.read(cx).parser.screen().contents())
    }
}
