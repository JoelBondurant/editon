use iced::keyboard::{self, Key};
use iced::Task;

use super::buffer::{CursorPos, Selection};
use super::widget;
use super::code_editor::{CodeEditor, EditorMsg};

// ─── Vim mode ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
pub enum VimMode {
    Off,
    Normal,
    Insert,
    Visual,
    VisualLine,
    VisualBlock,
    Command,
}

// ─── Repeatable edit record (for dot-repeat) ──────────────────────────────────

#[derive(Clone, Debug)]
pub(in crate::editor) enum NormalEdit {
    /// d/y + motion (e.g. dw, d$)
    OperatorMotion { op: char, motion: String, count: usize },
    /// c + motion: cut + re-insert last_insert_text on replay
    ChangeMotion { motion: String, count: usize },
    /// dd / cc / yy
    LineOp { op: char, count: usize },
    /// x — delete char forward
    DeleteChar { count: usize },
    /// X — delete char backward
    BackspaceChar { count: usize },
    /// ~ — toggle case
    ToggleCase { count: usize },
    /// r<c> — replace char
    ReplaceChar { ch: char, count: usize },
}

// ─── Vim key handlers (impl CodeEditor) ───────────────────────────────────────

impl CodeEditor {
    // ─── Normal mode ──────────────────────────────────────────────────────────

    pub(in crate::editor) fn handle_vim_normal_key(
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
        let was_z = self.pending_z;
        self.pending_z = false;

        // f/F/t/T pending: next key is the target char
        if let Some(find_kind) = self.pending_find.take() {
            let target = match &key {
                Key::Named(Named::Space) => Some(' '),
                Key::Character(kc) => {
                    let s = if ctrl { kc.as_str() } else { text.as_deref().unwrap_or(kc.as_str()) };
                    s.chars().next()
                }
                _ => None,
            };
            if let Some(tc) = target {
                let count = self.take_count();
                self.last_find = Some((find_kind, tc));
                self.do_find(find_kind, tc, count, false);
                self.update_status();
                self.ensure_cursor_visible();
            }
            return Task::none();
        }

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
                let count = self.take_count();
                for _ in 0..count {
                    self.buffer.replace_char(c);
                }
                self.last_edit = Some(NormalEdit::ReplaceChar { ch: c, count });
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

            Key::Character(ref kc) => {
                let key_ch = kc.as_str();
                let ch = if ctrl { key_ch } else { text.as_deref().unwrap_or(key_ch) };

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
                        "v" | "V" => {
                            self.vim_mode = VimMode::VisualBlock;
                            self.buffer.selection.anchor = self.buffer.selection.head;
                        }
                        _ => {}
                    }
                } else {
                    // z-prefix commands: zz / zt / zb
                    if was_z {
                        match ch {
                            "z" => self.scroll_cursor_z('z'),
                            "t" => self.scroll_cursor_z('t'),
                            "b" => self.scroll_cursor_z('b'),
                            _ => {}
                        }
                        return Task::none();
                    }

                    // Count prefix digits
                    let is_count_digit = ch.len() == 1
                        && ch.chars().next().map_or(false, |c| c.is_ascii_digit())
                        && (ch != "0" || !self.vim_count.is_empty());
                    if is_count_digit {
                        self.vim_count.push_str(ch);
                        self.update_status();
                        return Task::none();
                    }

                    let count = self.take_count();

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
                        // Text object prefix: 'i'/'a' followed by object key (w, s, …)
                        if let Some(obj) = self.pending_obj_prefix.take() {
                            let motion = format!("{}{}", obj, ch);
                            return self.exec_operator_motion(op, &motion, count);
                        }
                        // Wait for text-object key
                        if (ch == "i" || ch == "a") && !was_g {
                            self.pending_op = Some(op);
                            self.pending_obj_prefix = ch.chars().next();
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
                                self.last_edit = Some(NormalEdit::LineOp { op: 'd', count });
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
                                self.last_edit = Some(NormalEdit::LineOp { op: 'c', count });
                                self.enter_insert_mode();
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
                        "i" => self.enter_insert_mode(),
                        "I" => {
                            self.buffer.move_home(false);
                            self.enter_insert_mode();
                        }
                        "a" => {
                            self.buffer.move_right(false);
                            self.enter_insert_mode();
                        }
                        "A" => {
                            self.buffer.move_end(false);
                            self.enter_insert_mode();
                        }
                        "o" => {
                            self.buffer.move_end(false);
                            self.buffer.insert_newline();
                            self.enter_insert_mode();
                        }
                        "O" => {
                            self.buffer.move_home(false);
                            self.buffer.insert_newline();
                            self.buffer.move_up(false);
                            self.enter_insert_mode();
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
                            return iced::clipboard::read()
                                .map(|t| EditorMsg::PasteAfter(t.unwrap_or_default()));
                        }
                        "P" => {
                            return iced::clipboard::read()
                                .map(|t| EditorMsg::Paste(t.unwrap_or_default()));
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
                            self.last_edit = Some(NormalEdit::ToggleCase { count });
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
                            for _ in 0..count { self.buffer.delete(); }
                            self.last_edit = Some(NormalEdit::DeleteChar { count });
                        }
                        "X" => {
                            for _ in 0..count { self.buffer.backspace(); }
                            self.last_edit = Some(NormalEdit::BackspaceChar { count });
                        }
                        "u" => self.buffer.undo(),
                        "n" => self.buffer.search_next(),
                        "N" => self.buffer.search_prev(),
                        // ── find-char motions ───────────────────────────────
                        "f" => { self.pending_find = Some('f'); return Task::none(); }
                        "F" => { self.pending_find = Some('F'); return Task::none(); }
                        "t" => { self.pending_find = Some('t'); return Task::none(); }
                        "T" => { self.pending_find = Some('T'); return Task::none(); }
                        ";" => {
                            if let Some((kind, target)) = self.last_find {
                                self.do_find(kind, target, count, false);
                            }
                        }
                        "," => {
                            if let Some((kind, target)) = self.last_find {
                                let rev = match kind { 'f'=>'F', 'F'=>'f', 't'=>'T', 'T'=>'t', c=>c };
                                self.do_find(rev, target, count, false);
                            }
                        }
                        // ── scroll centering ────────────────────────────────
                        "z" => { self.pending_z = true; return Task::none(); }
                        // ── dot repeat ──────────────────────────────────────
                        "." => {
                            if let Some(edit) = self.last_edit.clone() {
                                let task = self.replay_edit(edit);
                                self.update_status();
                                self.ensure_cursor_visible();
                                return task;
                            }
                        }
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
            // ── text objects ────────────────────────────────────────────
            "iw" => {
                let lt = self.buffer.line_text(origin.line);
                let chars: Vec<char> = lt.chars().collect();
                if chars.is_empty() { return Task::none(); }
                let is_w = |c: char| c.is_alphanumeric() || c == '_';
                let col = origin.col.min(chars.len().saturating_sub(1));
                let mut s = col;
                while s > 0 && is_w(chars[s - 1]) { s -= 1; }
                let mut e = col;
                while e < chars.len() && is_w(chars[e]) { e += 1; }
                self.buffer.selection = Selection {
                    anchor: CursorPos::new(origin.line, s),
                    head: CursorPos::new(origin.line, e),
                };
            }
            "aw" => {
                let lt = self.buffer.line_text(origin.line);
                let chars: Vec<char> = lt.chars().collect();
                if chars.is_empty() { return Task::none(); }
                let is_w = |c: char| c.is_alphanumeric() || c == '_';
                let col = origin.col.min(chars.len().saturating_sub(1));
                let mut s = col;
                while s > 0 && is_w(chars[s - 1]) { s -= 1; }
                let mut e = col;
                while e < chars.len() && is_w(chars[e]) { e += 1; }
                let pre_ws = e;
                while e < chars.len() && chars[e].is_whitespace() { e += 1; }
                if e == pre_ws && s > 0 {
                    while s > 0 && chars[s - 1].is_whitespace() { s -= 1; }
                }
                self.buffer.selection = Selection {
                    anchor: CursorPos::new(origin.line, s),
                    head: CursorPos::new(origin.line, e),
                };
            }
            _ => {
                self.buffer.selection = Selection::caret(origin);
                self.update_status();
                return Task::none();
            }
        }

        match op {
            'd' => {
                let yanked = self.buffer.cut();
                self.buffer.selection = Selection::caret(self.buffer.selection.head);
                self.buffer.clipboard_is_line = false;
                self.last_edit = Some(NormalEdit::OperatorMotion { op: 'd', motion: motion.to_string(), count });
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
                self.buffer.selection = Selection::caret(self.buffer.selection.head);
                self.last_edit = Some(NormalEdit::ChangeMotion { motion: motion.to_string(), count });
                self.enter_insert_mode();
                self.update_status();
                self.ensure_cursor_visible();
            }
            _ => {}
        }
        Task::none()
    }

    // ─── Visual mode ──────────────────────────────────────────────────────────

    pub(in crate::editor) fn handle_vim_visual_key(
        &mut self,
        key: Key,
        mods: keyboard::Modifiers,
        text: Option<String>,
    ) -> Task<EditorMsg> {
        use keyboard::key::Named;
        let ctrl = mods.command();
        let is_line = self.vim_mode == VimMode::VisualLine;

        // Text object completion: pending i/a + this char (e.g. viw, vaw)
        if let Some(obj) = self.pending_obj_prefix.take() {
            if let Key::Character(ref kc) = key {
                let ch = if ctrl { kc.as_str() } else { text.as_deref().unwrap_or(kc.as_str()) };
                if ch == "w" {
                    let pos = self.buffer.selection.head;
                    self.buffer.select_word_at(pos);
                    if obj == 'a' {
                        let lt = self.buffer.line_text(pos.line);
                        let chars: Vec<char> = lt.chars().collect();
                        let (_, se) = self.buffer.selection.ordered();
                        let mut e = se.col;
                        while e < chars.len() && chars[e].is_whitespace() { e += 1; }
                        self.buffer.selection.head = CursorPos::new(se.line, e);
                    }
                }
            }
            self.update_status();
            self.ensure_cursor_visible();
            return Task::none();
        }

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
        let count = self.take_count();

        match key {
            Key::Named(Named::Escape) => {
                self.buffer.selection.anchor = self.buffer.selection.head;
                self.vim_mode = VimMode::Normal;
            }
            Key::Named(Named::ArrowLeft) => self.buffer.move_left(true),
            Key::Named(Named::ArrowRight) => self.buffer.move_right(true),
            Key::Named(Named::ArrowUp) => self.buffer.move_up(true),
            Key::Named(Named::ArrowDown) => self.buffer.move_down(true),
            Key::Character(ref kc) => {
                let key_ch = kc.as_str();
                let ch = if ctrl { key_ch } else { text.as_deref().unwrap_or(key_ch) };
                if ctrl {
                    match ch {
                        "f" | "F" => self.buffer.search_open(),
                        "v" | "V" => {
                            self.vim_mode = VimMode::VisualBlock;
                        }
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
                            self.enter_insert_mode();
                            self.update_status();
                            self.ensure_cursor_visible();
                            return Task::none();
                        }
                        // ── visual paste: replace selection with system clipboard ──
                        "p" => {
                            return iced::clipboard::read()
                                .map(|t| EditorMsg::VisualPaste(t.unwrap_or_default()));
                        }
                        // ── text objects in visual mode ──────────────────
                        "i" | "a" => {
                            self.pending_obj_prefix = ch.chars().next();
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
                        "u" => {
                            self.buffer.transform_case(false);
                            self.vim_mode = VimMode::Normal;
                            self.update_status();
                            self.ensure_cursor_visible();
                            return Task::none();
                        }
                        "U" => {
                            self.buffer.transform_case(true);
                            self.vim_mode = VimMode::Normal;
                            self.update_status();
                            self.ensure_cursor_visible();
                            return Task::none();
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

    // ─── Visual block mode ────────────────────────────────────────────────────

    pub(in crate::editor) fn handle_vim_visual_block_key(
        &mut self,
        key: Key,
        mods: keyboard::Modifiers,
        text: Option<String>,
    ) -> Task<EditorMsg> {
        use keyboard::key::Named;
        let ctrl = mods.command();

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
        let count = self.take_count();

        match key {
            Key::Named(Named::Escape) => {
                self.buffer.selection.anchor = self.buffer.selection.head;
                self.vim_mode = VimMode::Normal;
            }
            Key::Named(Named::ArrowLeft) => self.buffer.move_left(true),
            Key::Named(Named::ArrowRight) => self.buffer.move_right(true),
            Key::Named(Named::ArrowUp) => self.buffer.move_up(true),
            Key::Named(Named::ArrowDown) => self.buffer.move_down(true),
            Key::Character(ref kc) => {
                let key_ch = kc.as_str();
                let ch = if ctrl { key_ch } else { text.as_deref().unwrap_or(key_ch) };
                if ctrl {
                    match ch {
                        "v" | "V" => {
                            // Ctrl+V again collapses back to Normal
                            self.buffer.selection.anchor = self.buffer.selection.head;
                            self.vim_mode = VimMode::Normal;
                        }
                        _ => {}
                    }
                } else {
                    match ch {
                        "h" => { for _ in 0..count { self.buffer.move_left(true); } }
                        "j" => { for _ in 0..count { self.buffer.move_down(true); } }
                        "k" => { for _ in 0..count { self.buffer.move_up(true); } }
                        "l" => { for _ in 0..count { self.buffer.move_right(true); } }
                        "w" => { for _ in 0..count { self.buffer.move_word_right(true); } }
                        "b" => { for _ in 0..count { self.buffer.move_word_left(true); } }
                        "0" | "^" => self.buffer.move_home(true),
                        "$" => self.buffer.move_end(true),
                        "G" => self.buffer.move_to_end(true),
                        "g" => self.buffer.move_to_start(true),
                        "v" => self.vim_mode = VimMode::Visual,
                        "V" => {
                            self.vim_mode = VimMode::VisualLine;
                            let (s, e) = self.buffer.selection.ordered();
                            self.buffer.select_lines(e.line - s.line + 1);
                        }
                        "I" => {
                            let (s, e) = self.buffer.selection.ordered();
                            let left_col = self.buffer.selection.anchor.col
                                .min(self.buffer.selection.head.col);
                            self.block_insert = Some((left_col, s.line, e.line));
                            self.buffer.selection =
                                Selection::caret(CursorPos::new(s.line, left_col));
                            self.enter_insert_mode();
                            self.update_status();
                            self.ensure_cursor_visible();
                            return Task::none();
                        }
                        "A" => {
                            let (s, e) = self.buffer.selection.ordered();
                            let right_col = self.buffer.selection.anchor.col
                                .max(self.buffer.selection.head.col) + 1;
                            self.block_insert = Some((right_col, s.line, e.line));
                            self.buffer.selection =
                                Selection::caret(CursorPos::new(s.line, right_col));
                            self.enter_insert_mode();
                            self.update_status();
                            self.ensure_cursor_visible();
                            return Task::none();
                        }
                        "d" | "x" => {
                            let (s, e) = self.buffer.selection.ordered();
                            let left_col = self.buffer.selection.anchor.col
                                .min(self.buffer.selection.head.col);
                            let right_col = self.buffer.selection.anchor.col
                                .max(self.buffer.selection.head.col) + 1;
                            self.buffer.block_delete(s.line, e.line, left_col, right_col);
                            self.vim_mode = VimMode::Normal;
                            self.update_status();
                            self.ensure_cursor_visible();
                            return Task::none();
                        }
                        "u" => {
                            self.buffer.transform_case(false);
                            self.vim_mode = VimMode::Normal;
                            self.update_status();
                            self.ensure_cursor_visible();
                            return Task::none();
                        }
                        "U" => {
                            self.buffer.transform_case(true);
                            self.vim_mode = VimMode::Normal;
                            self.update_status();
                            self.ensure_cursor_visible();
                            return Task::none();
                        }
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

    // ─── Command bar ──────────────────────────────────────────────────────────

    pub(in crate::editor) fn handle_vim_command_key(
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

    // ─── Dot-repeat ───────────────────────────────────────────────────────────

    pub(in crate::editor) fn replay_edit(&mut self, edit: NormalEdit) -> Task<EditorMsg> {
        match edit {
            NormalEdit::OperatorMotion { op, motion, count } => {
                self.exec_operator_motion(op, &motion, count)
            }
            NormalEdit::ChangeMotion { motion, count } => {
                let _ = self.exec_operator_motion('c', &motion, count);
                // exec_operator_motion for 'c' leaves us in Insert mode;
                // directly insert the saved text and return to Normal.
                let text = self.last_insert_text.clone();
                for c in text.chars() {
                    self.buffer.insert_char(c);
                }
                self.vim_mode = VimMode::Normal;
                Task::none()
            }
            NormalEdit::LineOp { op: 'd', count } => {
                let line = self.buffer.selection.head.line;
                let yanked = self.buffer.yank_lines(line, count);
                self.buffer.delete_lines(line, count);
                iced::clipboard::write(yanked)
            }
            NormalEdit::LineOp { op: 'c', count: _ } => {
                let line = self.buffer.selection.head.line;
                let len = self.buffer.line_len(line);
                self.buffer.selection = Selection {
                    anchor: CursorPos::new(line, 0),
                    head: CursorPos::new(line, len),
                };
                let _ = self.buffer.cut();
                self.buffer.selection = Selection::caret(CursorPos::new(line, 0));
                let text = self.last_insert_text.clone();
                for c in text.chars() {
                    self.buffer.insert_char(c);
                }
                Task::none()
            }
            NormalEdit::LineOp { .. } => Task::none(),
            NormalEdit::DeleteChar { count } => {
                for _ in 0..count { self.buffer.delete(); }
                Task::none()
            }
            NormalEdit::BackspaceChar { count } => {
                for _ in 0..count { self.buffer.backspace(); }
                Task::none()
            }
            NormalEdit::ToggleCase { count } => {
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
                Task::none()
            }
            NormalEdit::ReplaceChar { ch, count } => {
                for _ in 0..count { self.buffer.replace_char(ch); }
                Task::none()
            }
        }
    }
}

// ─── :substitute parser ────────────────────────────────────────────────────────

pub(crate) fn parse_substitute(
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
