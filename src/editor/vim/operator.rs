use iced::Task;

use super::NormalEdit;
use super::super::coords::{CursorPos, Selection};
use super::super::core::{CodeEditor, EditorMsg};

impl CodeEditor {
	// ─── Operator + motion engine ──────────────────────────────────────────────

	pub(in crate::editor) fn exec_operator_motion(&mut self, op: char, motion: &str, count: usize) -> Task<EditorMsg> {
		let origin = self.buffer.session.selection.head;
		self.buffer.session.selection.anchor = origin;

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
				if chars.is_empty() {
					return Task::none();
				}
				let is_w = |c: char| c.is_alphanumeric() || c == '_';
				let col = origin.col.min(chars.len().saturating_sub(1));
				let mut s = col;
				while s > 0 && is_w(chars[s - 1]) {
					s -= 1;
				}
				let mut e = col;
				while e < chars.len() && is_w(chars[e]) {
					e += 1;
				}
				self.buffer.session.selection = Selection {
					anchor: CursorPos::new(origin.line, s),
					head: CursorPos::new(origin.line, e),
				};
			}
			"aw" => {
				let lt = self.buffer.line_text(origin.line);
				let chars: Vec<char> = lt.chars().collect();
				if chars.is_empty() {
					return Task::none();
				}
				let is_w = |c: char| c.is_alphanumeric() || c == '_';
				let col = origin.col.min(chars.len().saturating_sub(1));
				let mut s = col;
				while s > 0 && is_w(chars[s - 1]) {
					s -= 1;
				}
				let mut e = col;
				while e < chars.len() && is_w(chars[e]) {
					e += 1;
				}
				let pre_ws = e;
				while e < chars.len() && chars[e].is_whitespace() {
					e += 1;
				}
				if e == pre_ws && s > 0 {
					while s > 0 && chars[s - 1].is_whitespace() {
						s -= 1;
					}
				}
				self.buffer.session.selection = Selection {
					anchor: CursorPos::new(origin.line, s),
					head: CursorPos::new(origin.line, e),
				};
			}
			_ => {
				self.buffer.session.selection = Selection::caret(origin);
				self.update_status();
				return Task::none();
			}
		}

		match op {
			'd' => {
				let yanked = self.buffer.cut();
				self.buffer.session.selection =
					Selection::caret(self.buffer.session.selection.head);
				self.buffer.session.clipboard_is_line = false;
				self.vim.last_edit = Some(NormalEdit::OperatorMotion {
					op: 'd',
					motion: motion.to_string(),
					count,
				});
				self.update_status();
				self.ensure_cursor_visible();
				if !yanked.is_empty() {
					return iced::clipboard::write(yanked);
				}
			}
			'y' => {
				let yanked = self.buffer.copy();
				let start = origin.min(self.buffer.session.selection.head);
				self.buffer.session.selection = Selection::caret(start);
				self.buffer.session.clipboard_is_line = false;
				self.update_status();
				self.ensure_cursor_visible();
				if !yanked.is_empty() {
					return iced::clipboard::write(yanked);
				}
			}
			'c' => {
				let _ = self.buffer.cut();
				self.buffer.session.selection =
					Selection::caret(self.buffer.session.selection.head);
				self.vim.last_edit = Some(NormalEdit::ChangeMotion {
					motion: motion.to_string(),
					count,
				});
				self.enter_insert_mode();
				self.update_status();
				self.ensure_cursor_visible();
			}
			_ => {}
		}
		Task::none()
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
				let text = self.vim.last_insert_text.clone();
				for c in text.chars() {
					self.buffer.insert_char(c);
				}
				self.vim.mode = super::VimMode::Normal;
				Task::none()
			}
			NormalEdit::LineOp { op: 'd', count } => {
				let line = self.buffer.session.selection.head.line;
				let yanked = self.buffer.yank_lines(line, count);
				self.buffer.delete_lines(line, count);
				iced::clipboard::write(yanked)
			}
			NormalEdit::LineOp { op: 'c', count: _ } => {
				let line = self.buffer.session.selection.head.line;
				let len = self.buffer.line_len(line);
				self.buffer.session.selection = Selection {
					anchor: CursorPos::new(line, 0),
					head: CursorPos::new(line, len),
				};
				let _ = self.buffer.cut();
				self.buffer.session.selection = Selection::caret(CursorPos::new(line, 0));
				let text = self.vim.last_insert_text.clone();
				for c in text.chars() {
					self.buffer.insert_char(c);
				}
				Task::none()
			}
			NormalEdit::LineOp { .. } => Task::none(),
			NormalEdit::DeleteChar { count } => {
				for _ in 0..count {
					self.buffer.delete();
				}
				Task::none()
			}
			NormalEdit::BackspaceChar { count } => {
				for _ in 0..count {
					self.buffer.backspace();
				}
				Task::none()
			}
			NormalEdit::ToggleCase { count } => {
				for _ in 0..count {
					let pos = self.buffer.session.selection.head;
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
				for _ in 0..count {
					self.buffer.replace_char(ch);
				}
				Task::none()
			}
		}
	}
}
