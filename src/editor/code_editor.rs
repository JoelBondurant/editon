use iced::keyboard::{self, Key};
use iced::widget::{column, container, row, text, Space};
use iced::{event, Element, Length, Subscription, Task, Theme};

use super::{buffer, widget};
use super::buffer::{Buffer, CursorPos, Selection, UndoConfig};
use super::highlight::SyntaxLanguage;
use super::theme::EditorTheme;
use super::widget::{EditorAction, SqlEditor};
use super::vim::VimMode;

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

    pub(in crate::editor) viewport_w: f32,
    pub(in crate::editor) viewport_h: f32,
    pub(in crate::editor) vim_command: String,
    pub(in crate::editor) pending_g: bool,
    pub(in crate::editor) vim_count: String,
    pub(in crate::editor) pending_op: Option<char>,
    /// Pending block insert: (insert_col, top_line, bottom_line)
    pub(in crate::editor) block_insert: Option<(usize, usize, usize)>,

    is_dragging: bool,
    click_count: u32,
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
            block_insert: None,
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
            EditorMsg::Action(EditorAction::DoubleClick(pos)) => {
                let cursor_pos = self.pos_from_pixel(pos);
                self.buffer.select_word_at(cursor_pos);
                self.is_dragging = true;
                self.click_count = 2;
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
                if self.vim_mode == VimMode::VisualBlock {
                    return self.handle_vim_visual_block_key(key, mods, text);
                }

                // Insert mode: Escape → Normal
                if matches!(&key, Key::Named(keyboard::key::Named::Escape))
                    && !self.buffer.search.is_open
                {
                    let col_before = self.buffer.selection.head.col;
                    let line_before = self.buffer.selection.head.line;
                    self.vim_mode = VimMode::Normal;
                    if col_before > 0 {
                        self.buffer.move_left(false);
                    }
                    if let Some((insert_col, top_line, bottom_line)) = self.block_insert.take() {
                        if col_before > insert_col && line_before == top_line {
                            let inserted: String = self.buffer.line_text(top_line)
                                .chars()
                                .skip(insert_col)
                                .take(col_before - insert_col)
                                .collect();
                            if !inserted.is_empty() {
                                self.buffer.block_insert_text(
                                    top_line,
                                    bottom_line,
                                    insert_col,
                                    &inserted,
                                );
                            }
                        }
                        self.buffer.selection =
                            Selection::caret(CursorPos::new(top_line, insert_col));
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
        let visual_block = if self.vim_mode == VimMode::VisualBlock
            && !self.buffer.selection.is_caret()
        {
            let (s, e) = self.buffer.selection.ordered();
            let left_col = self.buffer.selection.anchor.col
                .min(self.buffer.selection.head.col);
            let right_col = self.buffer.selection.anchor.col
                .max(self.buffer.selection.head.col);
            Some((s.line, e.line, left_col, right_col))
        } else {
            None
        };
        let editor = SqlEditor::new(&self.buffer, &self.theme, EditorMsg::Action)
            .scroll_y(self.scroll_y)
            .scroll_x(self.scroll_x)
            .show_minimap(self.show_minimap)
            .show_whitespace(self.show_whitespace)
            .block_cursor(self.vim_mode == VimMode::Normal)
            .visual_block(visual_block);

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

    pub(in crate::editor) fn take_count(&mut self) -> usize {
        let n = self.vim_count.parse::<usize>().unwrap_or(1).max(1);
        self.vim_count.clear();
        n
    }

    pub(in crate::editor) fn update_status(&mut self) {
        let mode = match self.vim_mode {
            VimMode::Normal => "NOR",
            VimMode::Insert => "INS",
            VimMode::Visual => "VIS",
            VimMode::VisualLine => "V-LINE",
            VimMode::VisualBlock => "V-BLOCK",
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

    pub(in crate::editor) fn ensure_cursor_visible(&mut self) {
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
}
