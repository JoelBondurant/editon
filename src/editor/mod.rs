/// DejaVu Sans Mono font bytes — pass to `.font()` on your iced app builder
/// so the editor's whitespace glyphs (▸ ␣ ¬) render correctly.
pub const DEJAVU_SANS_MONO: &[u8] = include_bytes!("../../fonts/DejaVuSansMono.ttf");

pub mod buffer;
pub mod folding;
pub mod highlight;
pub mod search;
pub mod theme;
pub mod widget;
pub mod wrap;

use iced::keyboard::{self, Key};
use iced::widget::{column, container, row, text, Space};
use iced::{event, Element, Length, Subscription, Task, Theme};

use self::buffer::{Buffer, CursorPos, Selection, UndoConfig};
use self::highlight::SyntaxLanguage;
use self::theme::EditorTheme;
use self::widget::{EditorAction, SqlEditor};

// ─── Vim mode ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
pub enum VimMode {
    Normal,
    Insert,
    Visual,
    VisualLine,
    Command,
}

// ─── Public message type ──────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub enum EditorMsg {
    Action(EditorAction),
    Key(Key, keyboard::Modifiers, Option<String>),
    Scroll(f32, f32),
    MouseMove(iced::Point),
    MouseUp,
}

// ─── CodeEditor ───────────────────────────────────────────────────────────────

/// Self-contained code editor state. Embed in your app's state, drive with
/// `update` / `view` / `subscription`, and map messages to your own type.
pub struct CodeEditor {
    pub buffer: Buffer,
    pub theme: EditorTheme,
    pub scroll_y: f32,
    pub scroll_x: f32,
    pub status: String,
    pub show_minimap: bool,
    pub show_whitespace: bool,
    pub vim_mode: VimMode,

    viewport_w: f32,
    viewport_h: f32,
    is_dragging: bool,
    click_count: u32,
    vim_command: String,
    pending_g: bool,
    vim_count: String,
    pending_op: Option<char>,
}

#[allow(dead_code)] // public API — used by the consuming application, not the demo
impl CodeEditor {
    /// Create a new editor with the given initial content and syntax language.
    pub fn new(content: &str, language: SyntaxLanguage) -> Self {
        let undo_cfg = UndoConfig { max_history: 1000, group_timeout_ms: 600 };
        let buffer = Buffer::with_undo_config(content, language, undo_cfg);
        let dc = buffer.diagnostics.len();
        let mut ed = Self {
            buffer,
            theme: EditorTheme::dark(),
            scroll_y: 0.0,
            scroll_x: 0.0,
            status: String::new(),
            viewport_w: 1200.0,
            viewport_h: 750.0,
            is_dragging: false,
            click_count: 0,
            show_minimap: true,
            show_whitespace: true,
            vim_mode: VimMode::Normal,
            vim_command: String::new(),
            pending_g: false,
            vim_count: String::new(),
            pending_op: None,
        };
        ed.status = format!("NOR | Ln 1, Col 1 | {} diag", dc);
        ed
    }

    /// The current text content of the buffer.
    pub fn content(&self) -> String {
        self.buffer.rope.to_string()
    }

    /// Replace the buffer content (resets scroll and undo history).
    pub fn set_content(&mut self, content: &str) {
        let lang = self.buffer.language();
        self.buffer = Buffer::with_undo_config(content, lang, UndoConfig {
            max_history: 1000,
            group_timeout_ms: 600,
        });
        self.scroll_y = 0.0;
        self.scroll_x = 0.0;
        self.update_status();
    }

    /// Replace content and switch language in one call.
    pub fn set_content_with_language(&mut self, content: &str, language: SyntaxLanguage) {
        self.buffer = Buffer::with_undo_config(content, language, UndoConfig {
            max_history: 1000,
            group_timeout_ms: 600,
        });
        self.scroll_y = 0.0;
        self.scroll_x = 0.0;
        self.update_status();
    }

    /// Switch the syntax highlighting language (preserves content).
    pub fn set_language(&mut self, lang: SyntaxLanguage) {
        let content = self.content();
        self.set_content_with_language(&content, lang);
    }

    /// Swap the active color theme.
    pub fn set_theme(&mut self, theme: EditorTheme) {
        self.theme = theme;
    }

    /// Notify the editor of its viewport size (pixels). Call whenever the
    /// containing pane is resized so cursor-scroll math stays accurate.
    pub fn set_viewport(&mut self, w: f32, h: f32) {
        self.viewport_w = w;
        self.viewport_h = h;
    }

    // ─── iced integration ─────────────────────────────────────────────────────

    pub fn subscription(&self) -> Subscription<EditorMsg> {
        event::listen_with(|event, _status, _id| match event {
            iced::Event::Keyboard(keyboard::Event::KeyPressed { key, modifiers, text, .. }) => {
                Some(EditorMsg::Key(key, modifiers, text.map(|t| t.to_string())))
            }
            iced::Event::Mouse(iced::mouse::Event::WheelScrolled { delta }) => {
                let (dx, dy) = match delta {
                    iced::mouse::ScrollDelta::Lines { x, y } => (-x * 40.0, -y * 40.0),
                    iced::mouse::ScrollDelta::Pixels { x, y } => (-x, -y),
                };
                Some(EditorMsg::Scroll(dx, dy))
            }
            iced::Event::Mouse(iced::mouse::Event::CursorMoved { position }) => {
                Some(EditorMsg::MouseMove(position))
            }
            iced::Event::Mouse(iced::mouse::Event::ButtonReleased(
                iced::mouse::Button::Left,
            )) => Some(EditorMsg::MouseUp),
            _ => None,
        })
    }

    pub fn update(&mut self, msg: EditorMsg) -> Task<EditorMsg> {
        match msg {
            EditorMsg::Action(EditorAction::MouseDown(pos)) => {
                let cursor_pos = self.pos_from_pixel(pos);
                self.buffer.selection.anchor = cursor_pos;
                self.buffer.selection.head = cursor_pos;
                self.is_dragging = true;
                self.click_count = 1;
                self.update_status();
            }
            EditorMsg::Action(_) => {}

            EditorMsg::MouseMove(pos) => {
                if self.is_dragging && self.click_count == 1 {
                    let target = self.pos_from_pixel(pos);
                    self.buffer.selection.head = target;
                    self.update_status();
                }
            }
            EditorMsg::MouseUp => {
                self.is_dragging = false;
            }

            EditorMsg::Key(key, mods, text) => {
                if self.vim_mode == VimMode::Command {
                    return self.handle_vim_command_key(key, text);
                }
                if self.vim_mode == VimMode::Normal {
                    return self.handle_vim_normal_key(key, mods, text);
                }
                if self.vim_mode == VimMode::Visual || self.vim_mode == VimMode::VisualLine {
                    return self.handle_vim_visual_key(key, mods, text);
                }

                // Insert mode: Escape → Normal
                if matches!(&key, Key::Named(keyboard::key::Named::Escape))
                    && !self.buffer.search.is_open
                {
                    self.vim_mode = VimMode::Normal;
                    if self.buffer.selection.head.col > 0 {
                        self.buffer.move_left(false);
                    }
                    self.update_status();
                    return Task::none();
                }

                let shift = mods.shift();
                let ctrl = mods.command();

                if self.buffer.search.is_open {
                    match key {
                        Key::Named(keyboard::key::Named::Escape) => {
                            self.buffer.search_close();
                            self.update_status();
                            return Task::none();
                        }
                        Key::Named(keyboard::key::Named::Enter) if ctrl && shift => {
                            self.buffer.search_replace_all();
                            self.update_status();
                            return Task::none();
                        }
                        Key::Named(keyboard::key::Named::Enter) if shift => {
                            self.buffer.search_prev();
                            self.ensure_cursor_visible();
                            self.update_status();
                            return Task::none();
                        }
                        Key::Named(keyboard::key::Named::Enter) => {
                            self.buffer.search_next();
                            self.ensure_cursor_visible();
                            self.update_status();
                            return Task::none();
                        }
                        _ => {}
                    }
                }

                match key {
                    Key::Character(ref ch) if ctrl && ch.as_str() == "f" => {
                        self.buffer.search_open();
                    }
                    Key::Character(ref ch) if ctrl && shift && ch.as_str() == "h" => {
                        self.buffer.search_replace_current();
                    }
                    Key::Character(ref ch) if ctrl && shift && ch.as_str() == "[" => {
                        let l = self.buffer.selection.head.line;
                        self.buffer.toggle_fold(l);
                    }
                    Key::Character(ref ch) if ctrl && shift && ch.as_str() == "]" => {
                        let l = self.buffer.selection.head.line;
                        self.buffer.toggle_fold(l);
                    }
                    Key::Character(ref ch) if ctrl && ch.as_str() == "w" => {
                        let enabled = !self.buffer.wrap_config.enabled;
                        self.buffer.set_wrap(enabled);
                    }
                    Key::Character(ref ch) if ctrl && ch.as_str() == "m" => {
                        self.show_minimap = !self.show_minimap;
                    }
                    Key::Character(ref ch) if ctrl && ch.as_str() == "l" => {
                        self.show_whitespace = !self.show_whitespace;
                    }
                    Key::Named(keyboard::key::Named::ArrowLeft) if ctrl => {
                        self.buffer.move_word_left(shift)
                    }
                    Key::Named(keyboard::key::Named::ArrowRight) if ctrl => {
                        self.buffer.move_word_right(shift)
                    }
                    Key::Named(keyboard::key::Named::ArrowLeft) => self.buffer.move_left(shift),
                    Key::Named(keyboard::key::Named::ArrowRight) => self.buffer.move_right(shift),
                    Key::Named(keyboard::key::Named::ArrowUp) => self.buffer.move_up(shift),
                    Key::Named(keyboard::key::Named::ArrowDown) => self.buffer.move_down(shift),
                    Key::Named(keyboard::key::Named::Home) if ctrl => {
                        self.buffer.move_to_start(shift)
                    }
                    Key::Named(keyboard::key::Named::End) if ctrl => {
                        self.buffer.move_to_end(shift)
                    }
                    Key::Named(keyboard::key::Named::Home) => self.buffer.move_home(shift),
                    Key::Named(keyboard::key::Named::End) => self.buffer.move_end(shift),
                    Key::Named(keyboard::key::Named::PageUp) => {
                        let v = widget::visible_line_count(self.viewport_h);
                        self.buffer.page_up(v, shift);
                    }
                    Key::Named(keyboard::key::Named::PageDown) => {
                        let v = widget::visible_line_count(self.viewport_h);
                        self.buffer.page_down(v, shift);
                    }
                    Key::Named(keyboard::key::Named::Backspace) if ctrl => {
                        self.buffer.delete_word_back()
                    }
                    Key::Named(keyboard::key::Named::Delete) if ctrl => {
                        self.buffer.delete_word_forward()
                    }
                    Key::Named(keyboard::key::Named::Backspace) => self.buffer.backspace(),
                    Key::Named(keyboard::key::Named::Delete) => self.buffer.delete(),
                    Key::Named(keyboard::key::Named::Enter) => self.buffer.insert_newline(),
                    Key::Named(keyboard::key::Named::Tab) if shift => {
                        self.buffer.dedent_lines()
                    }
                    Key::Named(keyboard::key::Named::Tab) => {
                        if self.buffer.selection.is_caret() {
                            self.buffer.insert_char('\t');
                        } else {
                            self.buffer.indent_lines();
                        }
                    }
                    Key::Character(ref ch) => {
                        let s = ch.as_str();
                        if ctrl {
                            match s {
                                "a" => self.buffer.select_all(),
                                "z" if shift => self.buffer.redo(),
                                "z" => self.buffer.undo(),
                                "y" => self.buffer.redo(),
                                "d" => self.buffer.duplicate_line(),
                                "c" => {
                                    let copied = self.buffer.copy();
                                    if !copied.is_empty() {
                                        return iced::clipboard::write(copied);
                                    }
                                }
                                "x" => {
                                    let _ = self.buffer.cut();
                                }
                                "v" => {
                                    let clip = self.buffer.clipboard.clone();
                                    if !clip.is_empty() {
                                        self.buffer.paste(&clip);
                                    }
                                }
                                _ => {}
                            }
                        } else {
                            let insert = text.as_deref().unwrap_or(s);
                            for c in insert.chars() {
                                self.buffer.insert_char_auto_pair(c);
                            }
                        }
                    }
                    Key::Named(keyboard::key::Named::Space) if !mods.command() => {
                        self.buffer.insert_char_auto_pair(' ');
                    }
                    _ => {}
                }
                self.update_status();
                self.ensure_cursor_visible();
            }

            EditorMsg::Scroll(dx, dy) => {
                let sp = if self.buffer.search.is_open {
                    widget::search_panel_height()
                } else {
                    0.0
                };
                let eh = self.viewport_h - sp;
                let max_y = (self.buffer.line_count() as f32 * widget::line_height()
                    + widget::top_pad() * 2.0
                    - eh)
                    .max(0.0);
                self.scroll_y = (self.scroll_y + dy).clamp(0.0, max_y);
                self.scroll_x = (self.scroll_x + dx).max(0.0);
            }
        }
        Task::none()
    }

    pub fn view(&self) -> Element<'_, EditorMsg> {
        let editor = SqlEditor::new(&self.buffer, &self.theme, EditorMsg::Action)
            .scroll_y(self.scroll_y)
            .scroll_x(self.scroll_x)
            .show_minimap(self.show_minimap)
            .show_whitespace(self.show_whitespace)
            .block_cursor(self.vim_mode == VimMode::Normal);

        let sc = iced::Color::from_rgb(0.55, 0.58, 0.62);
        let sep = iced::Color::from_rgb(0.35, 0.37, 0.40);
        let lang = self.buffer.language().display_name();
        let wrap_status = if self.buffer.wrap_config.enabled {
            "Wrap:On"
        } else {
            "Wrap:Off"
        };

        let status_bar = container(
            row![
                text(&self.status).size(13).color(sc),
                Space::new().width(Length::Fill),
                text(wrap_status).size(13).color(sc),
                text("  ·  ").size(13).color(sep),
                text("UTF-8").size(13).color(sc),
                text("  ·  ").size(13).color(sep),
                text(lang).size(13).color(sc),
                text("  ·  ").size(13).color(sep),
                text("C-l=ws  C-m=map  C-w=wrap").size(11).color(sep),
            ]
            .padding(6)
            .spacing(4),
        )
        .style(|_: &Theme| container::Style {
            background: Some(iced::Background::Color(iced::Color::from_rgb(
                0.09, 0.10, 0.12,
            ))),
            ..Default::default()
        })
        .width(Length::Fill);

        let cmd_bar_color = iced::Color::from_rgb(0.90, 0.92, 0.95);
        let cmd_bar = container(
            row![
                text(":").size(14).color(cmd_bar_color),
                text(&self.vim_command).size(14).color(cmd_bar_color),
                text("█")
                    .size(14)
                    .color(iced::Color::from_rgba(0.90, 0.92, 0.95, 0.7)),
            ]
            .padding(iced::Padding {
                top: 4.0,
                bottom: 4.0,
                left: 8.0,
                right: 8.0,
            })
            .spacing(0),
        )
        .style(|_: &Theme| container::Style {
            background: Some(iced::Background::Color(iced::Color::from_rgb(
                0.11, 0.12, 0.16,
            ))),
            ..Default::default()
        })
        .width(Length::Fill);

        if self.vim_mode == VimMode::Command {
            column![
                container(Element::from(editor))
                    .width(Length::Fill)
                    .height(Length::Fill),
                cmd_bar,
                status_bar,
            ]
            .into()
        } else {
            column![
                container(Element::from(editor))
                    .width(Length::Fill)
                    .height(Length::Fill),
                status_bar,
            ]
            .into()
        }
    }

    // ─── Internal helpers ──────────────────────────────────────────────────────

    fn pos_from_pixel(&self, pixel: iced::Point) -> CursorPos {
        let gw = widget::gutter_width(self.buffer.line_count());
        let bounds = iced::Rectangle {
            x: 0.0,
            y: 0.0,
            width: self.viewport_w,
            height: self.viewport_h,
        };
        widget::pixel_to_pos(
            &self.buffer,
            &bounds,
            gw,
            self.scroll_x,
            self.scroll_y,
            pixel.x,
            pixel.y,
        )
    }

    fn update_status(&mut self) {
        let mode = match self.vim_mode {
            VimMode::Normal => "NOR",
            VimMode::Insert => "INS",
            VimMode::Visual => "VIS",
            VimMode::VisualLine => "V-LINE",
            VimMode::Command => "CMD",
        };
        let p = self.buffer.selection.head;
        let dc = self.buffer.diagnostics.len();
        let sel = if !self.buffer.selection.is_caret() {
            let (s, e) = self.buffer.selection.ordered();
            let cs = self.buffer.rope.line_to_char(s.line) + s.col;
            let ce = self.buffer.rope.line_to_char(e.line) + e.col;
            format!(
                " | {} sel ({} ln)",
                ce.saturating_sub(cs),
                e.line - s.line + 1
            )
        } else {
            String::new()
        };
        let search = if self.buffer.search.is_open {
            format!(
                " | Search: {}/{}",
                self.buffer.search.current_match + 1,
                self.buffer.search.match_count()
            )
        } else {
            String::new()
        };
        self.status = format!(
            "{} | Ln {}, Col {}{}{} | {} diag",
            mode,
            p.line + 1,
            p.col + 1,
            sel,
            search,
            dc,
        );
    }

    fn ensure_cursor_visible(&mut self) {
        let sp = if self.buffer.search.is_open {
            widget::search_panel_height()
        } else {
            0.0
        };
        let vh = self.viewport_h - widget::top_pad() * 2.0 - sp;
        let cy = self.buffer.selection.head.line as f32 * widget::line_height();
        if cy < self.scroll_y {
            self.scroll_y = cy;
        } else if cy + widget::line_height() > self.scroll_y + vh {
            self.scroll_y = cy + widget::line_height() - vh;
        }
        let head = self.buffer.selection.head;
        let hlt = self.buffer.line_text(head.line);
        let vcol = buffer::visual_col_of(&hlt, head.col);
        let cx = vcol as f32 * widget::CHAR_W;
        let gw = widget::gutter_width(self.buffer.line_count());
        let mm = if self.show_minimap {
            widget::minimap_width()
        } else {
            0.0
        };
        let vw = self.viewport_w - gw - widget::scrollbar_width() - mm;
        if cx < self.scroll_x {
            self.scroll_x = cx;
        } else if cx + widget::CHAR_W > self.scroll_x + vw {
            self.scroll_x = cx + widget::CHAR_W - vw;
        }
    }

    // ─── Vim normal mode ───────────────────────────────────────────────────────

    fn handle_vim_normal_key(
        &mut self,
        key: Key,
        mods: keyboard::Modifiers,
        text: Option<String>,
    ) -> Task<EditorMsg> {
        use keyboard::key::Named;
        let shift = mods.shift();
        let ctrl = mods.command();
        let was_g = self.pending_g;
        self.pending_g = false;

        // `r` (replace char) consumes the very next key as the replacement
        if self.pending_op == Some('r') {
            self.pending_op = None;
            let ch = match &key {
                Key::Named(Named::Space) => Some(' '),
                Key::Named(Named::Tab) => Some('\t'),
                Key::Named(Named::Enter) => Some('\n'),
                Key::Named(Named::Escape) => None,
                Key::Character(_) => text.as_deref().and_then(|t| t.chars().next()),
                _ => None,
            };
            if let Some(c) = ch {
                let count = self.vim_count.parse::<usize>().unwrap_or(1).max(1);
                self.vim_count.clear();
                for _ in 0..count {
                    self.buffer.replace_char(c);
                }
            } else {
                self.vim_count.clear();
            }
            self.update_status();
            self.ensure_cursor_visible();
            return Task::none();
        }

        match key {
            Key::Named(Named::Escape) => {
                if self.buffer.search.is_open {
                    self.buffer.search_close();
                }
                self.buffer.selection.anchor = self.buffer.selection.head;
                self.vim_count.clear();
                self.pending_op = None;
            }
            Key::Named(Named::ArrowLeft) if ctrl => self.buffer.move_word_left(shift),
            Key::Named(Named::ArrowRight) if ctrl => self.buffer.move_word_right(shift),
            Key::Named(Named::ArrowLeft) => self.buffer.move_left(shift),
            Key::Named(Named::ArrowRight) => self.buffer.move_right(shift),
            Key::Named(Named::ArrowUp) => self.buffer.move_up(shift),
            Key::Named(Named::ArrowDown) => self.buffer.move_down(shift),
            Key::Named(Named::Home) if ctrl => self.buffer.move_to_start(shift),
            Key::Named(Named::End) if ctrl => self.buffer.move_to_end(shift),
            Key::Named(Named::Home) => self.buffer.move_home(shift),
            Key::Named(Named::End) => self.buffer.move_end(shift),
            Key::Named(Named::PageUp) => {
                let v = widget::visible_line_count(self.viewport_h);
                self.buffer.page_up(v, false);
            }
            Key::Named(Named::PageDown) => {
                let v = widget::visible_line_count(self.viewport_h);
                self.buffer.page_down(v, false);
            }

            Key::Character(_) => {
                let ch = text.as_deref().unwrap_or("");

                if ctrl {
                    match ch {
                        "f" | "F" => self.buffer.search_open(),
                        "w" | "W" => {
                            let e = !self.buffer.wrap_config.enabled;
                            self.buffer.set_wrap(e);
                        }
                        "m" | "M" => self.show_minimap = !self.show_minimap,
                        "l" | "L" => self.show_whitespace = !self.show_whitespace,
                        "r" | "R" => self.buffer.redo(),
                        _ => {}
                    }
                } else {
                    // Count prefix digits
                    let is_count_digit = ch.len() == 1
                        && ch.chars().next().map_or(false, |c| c.is_ascii_digit())
                        && (ch != "0" || !self.vim_count.is_empty());
                    if is_count_digit {
                        self.vim_count.push_str(ch);
                        self.update_status();
                        return Task::none();
                    }

                    let count = self.vim_count.parse::<usize>().unwrap_or(1).max(1);
                    self.vim_count.clear();

                    // Pending operator + motion/doubling
                    if let Some(op) = self.pending_op.take() {
                        if (op == '>' && ch == ">") || (op == '<' && ch == "<") {
                            let line = self.buffer.selection.head.line;
                            let last = (line + count - 1)
                                .min(self.buffer.line_count().saturating_sub(1));
                            self.buffer.selection = Selection {
                                anchor: CursorPos::new(line, 0),
                                head: CursorPos::new(last, self.buffer.line_len(last)),
                            };
                            if op == '>' {
                                self.buffer.indent_lines();
                            } else {
                                self.buffer.dedent_lines();
                            }
                            self.buffer.selection =
                                Selection::caret(CursorPos::new(line, 0));
                            self.update_status();
                            self.ensure_cursor_visible();
                            return Task::none();
                        }
                        // `g` inside dg/yg/cg — wait for second `g`
                        if ch == "g" && !was_g {
                            self.pending_op = Some(op);
                            self.pending_g = true;
                            return Task::none();
                        }
                        let task = match (op, ch) {
                            ('d', "d") => {
                                let line = self.buffer.selection.head.line;
                                let yanked = self.buffer.yank_lines(line, count);
                                self.buffer.delete_lines(line, count);
                                self.update_status();
                                self.ensure_cursor_visible();
                                iced::clipboard::write(yanked)
                            }
                            ('y', "y") => {
                                let line = self.buffer.selection.head.line;
                                let yanked = self.buffer.yank_lines(line, count);
                                self.update_status();
                                iced::clipboard::write(yanked)
                            }
                            ('c', "c") => {
                                let line = self.buffer.selection.head.line;
                                let len = self.buffer.line_len(line);
                                self.buffer.selection = Selection {
                                    anchor: CursorPos::new(line, 0),
                                    head: CursorPos::new(line, len),
                                };
                                let _ = self.buffer.cut();
                                self.buffer.selection =
                                    Selection::caret(CursorPos::new(line, 0));
                                self.vim_mode = VimMode::Insert;
                                self.update_status();
                                self.ensure_cursor_visible();
                                Task::none()
                            }
                            (op, motion) => {
                                let motion_str = if was_g { "gg" } else { motion };
                                self.exec_operator_motion(op, motion_str, count)
                            }
                        };
                        return task;
                    }

                    match ch {
                        "i" => self.vim_mode = VimMode::Insert,
                        "I" => {
                            self.buffer.move_home(false);
                            self.vim_mode = VimMode::Insert;
                        }
                        "a" => {
                            self.buffer.move_right(false);
                            self.vim_mode = VimMode::Insert;
                        }
                        "A" => {
                            self.buffer.move_end(false);
                            self.vim_mode = VimMode::Insert;
                        }
                        "o" => {
                            self.buffer.move_end(false);
                            self.buffer.insert_newline();
                            self.vim_mode = VimMode::Insert;
                        }
                        "O" => {
                            self.buffer.move_home(false);
                            self.buffer.insert_newline();
                            self.buffer.move_up(false);
                            self.vim_mode = VimMode::Insert;
                        }
                        "v" => {
                            self.vim_mode = VimMode::Visual;
                            self.buffer.selection.anchor = self.buffer.selection.head;
                        }
                        "V" => {
                            self.vim_mode = VimMode::VisualLine;
                            self.buffer.select_lines(count);
                        }
                        "d" => self.pending_op = Some('d'),
                        "y" => self.pending_op = Some('y'),
                        "c" => self.pending_op = Some('c'),
                        "r" => self.pending_op = Some('r'),
                        ">" => self.pending_op = Some('>'),
                        "<" => self.pending_op = Some('<'),
                        "C" => {
                            return self.exec_operator_motion('c', "$", 1);
                        }
                        "p" => {
                            if self.buffer.clipboard_is_line {
                                for _ in 0..count {
                                    self.buffer.paste_line_below();
                                }
                            } else {
                                let clip = self.buffer.clipboard.clone();
                                if !clip.is_empty() {
                                    self.buffer.move_right(false);
                                    for _ in 0..count {
                                        self.buffer.paste(&clip);
                                    }
                                }
                            }
                        }
                        "P" => {
                            if self.buffer.clipboard_is_line {
                                for _ in 0..count {
                                    self.buffer.paste_line_above();
                                }
                            } else {
                                let clip = self.buffer.clipboard.clone();
                                if !clip.is_empty() {
                                    for _ in 0..count {
                                        self.buffer.paste(&clip);
                                    }
                                }
                            }
                        }
                        "~" => {
                            for _ in 0..count {
                                let pos = self.buffer.selection.head;
                                let lt = self.buffer.line_text(pos.line);
                                if let Some(c) = lt.chars().nth(pos.col) {
                                    let toggled = if c.is_uppercase() {
                                        c.to_lowercase().next().unwrap_or(c)
                                    } else {
                                        c.to_uppercase().next().unwrap_or(c)
                                    };
                                    self.buffer.replace_char(toggled);
                                    self.buffer.move_right(false);
                                }
                            }
                        }
                        "*" => {
                            if let Some(word) = self.buffer.word_under_cursor() {
                                self.buffer.search_star(&word, true);
                                self.ensure_cursor_visible();
                            }
                        }
                        "#" => {
                            if let Some(word) = self.buffer.word_under_cursor() {
                                self.buffer.search_star(&word, false);
                                self.ensure_cursor_visible();
                            }
                        }
                        ":" => self.vim_mode = VimMode::Command,
                        "h" => {
                            for _ in 0..count {
                                self.buffer.move_left(false);
                            }
                        }
                        "j" => {
                            for _ in 0..count {
                                self.buffer.move_down(false);
                            }
                        }
                        "k" => {
                            for _ in 0..count {
                                self.buffer.move_up(false);
                            }
                        }
                        "l" => {
                            for _ in 0..count {
                                self.buffer.move_right(false);
                            }
                        }
                        "w" => {
                            for _ in 0..count {
                                self.buffer.move_word_right(false);
                            }
                        }
                        "b" => {
                            for _ in 0..count {
                                self.buffer.move_word_left(false);
                            }
                        }
                        "e" => {
                            for _ in 0..count {
                                self.buffer.move_word_right(false);
                            }
                        }
                        "0" => self.buffer.move_home(false),
                        "$" => self.buffer.move_end(false),
                        "^" => self.buffer.move_home(false),
                        "g" if was_g => self.buffer.move_to_start(false),
                        "g" => self.pending_g = true,
                        "G" => self.buffer.move_to_end(false),
                        "x" => {
                            for _ in 0..count {
                                self.buffer.delete();
                            }
                        }
                        "X" => {
                            for _ in 0..count {
                                self.buffer.backspace();
                            }
                        }
                        "u" => self.buffer.undo(),
                        "n" => self.buffer.search_next(),
                        "N" => self.buffer.search_prev(),
                        _ => {}
                    }
                }
            }
            _ => {}
        }

        self.update_status();
        self.ensure_cursor_visible();
        Task::none()
    }

    // ─── Operator + motion engine ──────────────────────────────────────────────

    fn exec_operator_motion(
        &mut self,
        op: char,
        motion: &str,
        count: usize,
    ) -> Task<EditorMsg> {
        let origin = self.buffer.selection.head;
        self.buffer.selection.anchor = origin;

        match motion {
            "h" => {
                for _ in 0..count {
                    self.buffer.move_left(true);
                }
            }
            "j" => {
                for _ in 0..count {
                    self.buffer.move_down(true);
                }
            }
            "k" => {
                for _ in 0..count {
                    self.buffer.move_up(true);
                }
            }
            "l" => {
                for _ in 0..count {
                    self.buffer.move_right(true);
                }
            }
            "w" | "e" => {
                for _ in 0..count {
                    self.buffer.move_word_right(true);
                }
            }
            "b" => {
                for _ in 0..count {
                    self.buffer.move_word_left(true);
                }
            }
            "0" | "^" => self.buffer.move_home(true),
            "$" => self.buffer.move_end(true),
            "G" => self.buffer.move_to_end(true),
            "gg" => self.buffer.move_to_start(true),
            _ => {
                self.buffer.selection = Selection::caret(origin);
                self.update_status();
                return Task::none();
            }
        }

        match op {
            'd' => {
                let yanked = self.buffer.cut();
                self.buffer.selection =
                    Selection::caret(self.buffer.selection.head);
                self.buffer.clipboard_is_line = false;
                self.update_status();
                self.ensure_cursor_visible();
                if !yanked.is_empty() {
                    return iced::clipboard::write(yanked);
                }
            }
            'y' => {
                let yanked = self.buffer.copy();
                let start = origin.min(self.buffer.selection.head);
                self.buffer.selection = Selection::caret(start);
                self.buffer.clipboard_is_line = false;
                self.update_status();
                self.ensure_cursor_visible();
                if !yanked.is_empty() {
                    return iced::clipboard::write(yanked);
                }
            }
            'c' => {
                let _ = self.buffer.cut();
                self.buffer.selection =
                    Selection::caret(self.buffer.selection.head);
                self.vim_mode = VimMode::Insert;
                self.update_status();
                self.ensure_cursor_visible();
            }
            _ => {}
        }
        Task::none()
    }

    // ─── Vim visual mode ───────────────────────────────────────────────────────

    fn handle_vim_visual_key(
        &mut self,
        key: Key,
        mods: keyboard::Modifiers,
        text: Option<String>,
    ) -> Task<EditorMsg> {
        use keyboard::key::Named;
        let ctrl = mods.command();
        let is_line = self.vim_mode == VimMode::VisualLine;

        if let Key::Character(_) = &key {
            let ch = text.as_deref().unwrap_or("");
            let is_count_digit = ch.len() == 1
                && ch.chars().next().map_or(false, |c| c.is_ascii_digit())
                && (ch != "0" || !self.vim_count.is_empty());
            if is_count_digit {
                self.vim_count.push_str(ch);
                return Task::none();
            }
        }
        let count = self.vim_count.parse::<usize>().unwrap_or(1).max(1);
        self.vim_count.clear();

        match key {
            Key::Named(Named::Escape) => {
                self.buffer.selection.anchor = self.buffer.selection.head;
                self.vim_mode = VimMode::Normal;
            }
            Key::Named(Named::ArrowLeft) => self.buffer.move_left(true),
            Key::Named(Named::ArrowRight) => self.buffer.move_right(true),
            Key::Named(Named::ArrowUp) => self.buffer.move_up(true),
            Key::Named(Named::ArrowDown) => self.buffer.move_down(true),
            Key::Character(_) => {
                let ch = text.as_deref().unwrap_or("");
                if ctrl {
                    match ch {
                        "f" | "F" => self.buffer.search_open(),
                        _ => {}
                    }
                } else {
                    match ch {
                        "h" => {
                            for _ in 0..count {
                                self.buffer.move_left(true);
                            }
                        }
                        "j" => {
                            for _ in 0..count {
                                self.buffer.move_down(true);
                            }
                        }
                        "k" => {
                            for _ in 0..count {
                                self.buffer.move_up(true);
                            }
                        }
                        "l" => {
                            for _ in 0..count {
                                self.buffer.move_right(true);
                            }
                        }
                        "w" => {
                            for _ in 0..count {
                                self.buffer.move_word_right(true);
                            }
                        }
                        "b" => {
                            for _ in 0..count {
                                self.buffer.move_word_left(true);
                            }
                        }
                        "0" | "^" => self.buffer.move_home(true),
                        "$" => self.buffer.move_end(true),
                        "G" => self.buffer.move_to_end(true),
                        "g" => self.buffer.move_to_start(true),
                        ">" => {
                            self.buffer.indent_lines();
                            self.vim_mode = VimMode::Normal;
                            self.buffer.selection.anchor = self.buffer.selection.head;
                        }
                        "<" => {
                            self.buffer.dedent_lines();
                            self.vim_mode = VimMode::Normal;
                            self.buffer.selection.anchor = self.buffer.selection.head;
                        }
                        "y" => {
                            let yanked = if is_line {
                                let (s, e) = self.buffer.selection.ordered();
                                self.buffer.yank_lines(s.line, e.line - s.line + 1)
                            } else {
                                self.buffer.copy()
                            };
                            self.buffer.selection.anchor = self.buffer.selection.head;
                            self.vim_mode = VimMode::Normal;
                            self.update_status();
                            self.ensure_cursor_visible();
                            if !yanked.is_empty() {
                                return iced::clipboard::write(yanked);
                            }
                            return Task::none();
                        }
                        "d" | "x" => {
                            let yanked = if is_line {
                                let (s, e) = self.buffer.selection.ordered();
                                let lcount = e.line - s.line + 1;
                                let y = self.buffer.yank_lines(s.line, lcount);
                                self.buffer.delete_lines(s.line, lcount);
                                y
                            } else {
                                self.buffer.cut()
                            };
                            self.vim_mode = VimMode::Normal;
                            self.update_status();
                            self.ensure_cursor_visible();
                            if !yanked.is_empty() {
                                return iced::clipboard::write(yanked);
                            }
                            return Task::none();
                        }
                        "c" => {
                            let _ = self.buffer.cut();
                            self.vim_mode = VimMode::Insert;
                            self.update_status();
                            self.ensure_cursor_visible();
                            return Task::none();
                        }
                        "v" => {
                            self.vim_mode =
                                if is_line { VimMode::Visual } else { VimMode::Normal };
                            if self.vim_mode == VimMode::Normal {
                                self.buffer.selection.anchor = self.buffer.selection.head;
                            }
                        }
                        "V" => {
                            if is_line {
                                self.vim_mode = VimMode::Normal;
                                self.buffer.selection.anchor = self.buffer.selection.head;
                            } else {
                                self.vim_mode = VimMode::VisualLine;
                                let (s, e) = self.buffer.selection.ordered();
                                self.buffer.select_lines(e.line - s.line + 1);
                            }
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }

        // V-LINE: snap selection to whole lines
        if self.vim_mode == VimMode::VisualLine {
            let (s, e) = self.buffer.selection.ordered();
            if self.buffer.selection.head >= self.buffer.selection.anchor {
                self.buffer.selection.anchor = CursorPos::new(s.line, 0);
                self.buffer.selection.head =
                    CursorPos::new(e.line, self.buffer.line_len(e.line));
            } else {
                self.buffer.selection.head = CursorPos::new(s.line, 0);
                self.buffer.selection.anchor =
                    CursorPos::new(e.line, self.buffer.line_len(e.line));
            }
        }

        self.update_status();
        self.ensure_cursor_visible();
        Task::none()
    }

    // ─── Vim command bar ───────────────────────────────────────────────────────

    fn handle_vim_command_key(
        &mut self,
        key: Key,
        text: Option<String>,
    ) -> Task<EditorMsg> {
        use keyboard::key::Named;
        match key {
            Key::Named(Named::Escape) => {
                self.vim_mode = VimMode::Normal;
                self.vim_command.clear();
            }
            Key::Named(Named::Enter) => {
                self.execute_vim_command();
                self.vim_mode = VimMode::Normal;
                self.vim_command.clear();
            }
            Key::Named(Named::Backspace) => {
                if self.vim_command.pop().is_none() {
                    self.vim_mode = VimMode::Normal;
                }
            }
            Key::Named(Named::Space) => {
                self.vim_command.push(' ');
            }
            Key::Character(_) => {
                if let Some(t) = text {
                    self.vim_command.push_str(&t);
                }
            }
            _ => {}
        }
        self.update_status();
        Task::none()
    }

    fn execute_vim_command(&mut self) {
        let cmd = self.vim_command.trim().to_string();

        if let Ok(n) = cmd.parse::<usize>() {
            let line =
                n.saturating_sub(1).min(self.buffer.line_count().saturating_sub(1));
            self.buffer.selection.anchor = CursorPos { line, col: 0 };
            self.buffer.selection.head = CursorPos { line, col: 0 };
            self.ensure_cursor_visible();
            return;
        }

        if let Some((first, last, pat, rep, global, icase)) = parse_substitute(
            &cmd,
            self.buffer.selection.head.line,
            self.buffer.line_count().saturating_sub(1),
        ) {
            let changed = self.buffer.substitute(first, last, &pat, &rep, global, icase);
            if changed > 0 {
                let line = first.min(self.buffer.line_count().saturating_sub(1));
                self.buffer.selection.anchor = CursorPos { line, col: 0 };
                self.buffer.selection.head = CursorPos { line, col: 0 };
                self.ensure_cursor_visible();
            }
            self.update_status();
            return;
        }

        match cmd.as_str() {
            "noh" | "nohl" | "nohlsearch" => self.buffer.search_close(),
            "q" | "q!" | "wq" | "w" => {}
            _ => {}
        }
    }
}

// ─── Vim :substitute parser ────────────────────────────────────────────────────

fn parse_substitute(
    cmd: &str,
    current_line: usize,
    last_line: usize,
) -> Option<(usize, usize, String, String, bool, bool)> {
    let mut i = 0;
    let bytes = cmd.as_bytes();
    while i < bytes.len()
        && matches!(bytes[i], b'0'..=b'9' | b'%' | b'.' | b'$' | b',')
    {
        i += 1;
    }
    let range_str = &cmd[..i];
    if bytes.get(i) != Some(&b's') {
        return None;
    }
    i += 1;
    let sep = *bytes.get(i)? as char;
    i += 1;
    let rest = &cmd[i..];
    let sep_str = sep.to_string();
    let mut parts = rest.splitn(3, sep_str.as_str());
    let pattern = parts.next().unwrap_or("");
    let replacement = parts.next().unwrap_or("");
    let flags = parts.next().unwrap_or("");
    if pattern.is_empty() {
        return None;
    }
    let (first, last) = parse_vim_range(range_str, current_line, last_line);
    let global = flags.contains('g');
    let icase = flags.contains('i');
    Some((first, last, pattern.to_string(), replacement.to_string(), global, icase))
}

fn parse_vim_range(range: &str, current: usize, last: usize) -> (usize, usize) {
    match range.trim() {
        "" | "." => (current, current),
        "%" => (0, last),
        "$" => (last, last),
        s => {
            if let Some((a, b)) = s.split_once(',') {
                (
                    parse_line_addr(a, current, last),
                    parse_line_addr(b, current, last),
                )
            } else {
                let n = parse_line_addr(s, current, last);
                (n, n)
            }
        }
    }
}

fn parse_line_addr(s: &str, current: usize, last: usize) -> usize {
    match s.trim() {
        "." => current,
        "$" => last,
        n => n
            .parse::<usize>()
            .map(|n| n.saturating_sub(1).min(last))
            .unwrap_or(current),
    }
}
