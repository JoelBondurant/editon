use iced::Task;
use iced::keyboard::{self, Key};

use super::VimMode;
use super::super::coords::{CharIdx, CursorPos, Selection};
use super::super::core::{CodeEditor, EditorMsg};

impl CodeEditor {
	// ─── Visual mode ──────────────────────────────────────────────────────────

	pub(in crate::editor) fn handle_vim_visual_key(
		&mut self,
		key: Key,
		mods: keyboard::Modifiers,
		text: Option<String>,
	) -> Task<EditorMsg> {
		use keyboard::key::Named;
		let ctrl = mods.command();
		let is_line = self.vim.mode == VimMode::VisualLine;

		// Text object completion: pending i/a + this char (e.g. viw, vaw)
		if let Some(obj) = self.vim.pending_obj_prefix.take() {
			if let Key::Character(ref kc) = key {
				let ch = if ctrl {
					kc.as_str()
				} else {
					text.as_deref().unwrap_or(kc.as_str())
				};
				if ch == "w" {
					let pos = self.buffer.session.selection.head;
					self.buffer.select_word_at(pos);
					if obj == 'a' {
						let lt = self.buffer.line_text(pos.line);
						let chars: Vec<char> = lt.chars().collect();
						let (_, se) = self.buffer.session.selection.ordered();
						let mut e = se.col;
						while *e < chars.len() && chars[*e].is_whitespace() {
							e += 1;
						}
						self.buffer.session.selection.head = CursorPos::new(se.line, e);
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
				&& (ch != "0" || !self.vim.count.is_empty());
			if is_count_digit {
				self.vim.count.push_str(ch);
				return Task::none();
			}
		}
		let count = self.take_count();

		match key {
			Key::Named(Named::Escape) => {
				self.buffer.session.selection.anchor = self.buffer.session.selection.head;
				self.vim.mode = VimMode::Normal;
			}
			Key::Named(Named::ArrowLeft) => self.buffer.move_left(true),
			Key::Named(Named::ArrowRight) => self.buffer.move_right(true),
			Key::Named(Named::ArrowUp) => self.buffer.move_up(true),
			Key::Named(Named::ArrowDown) => self.buffer.move_down(true),
			Key::Character(ref kc) => {
				let key_ch = kc.as_str();
				let ch = if ctrl {
					key_ch
				} else {
					text.as_deref().unwrap_or(key_ch)
				};
				if ctrl {
					match ch {
						"f" | "F" => self.buffer.search_open(),
						"v" | "V" => {
							self.vim.mode = VimMode::VisualBlock;
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
							self.vim.mode = VimMode::Normal;
							self.buffer.session.selection.anchor =
								self.buffer.session.selection.head;
						}
						"<" => {
							self.buffer.dedent_lines();
							self.vim.mode = VimMode::Normal;
							self.buffer.session.selection.anchor =
								self.buffer.session.selection.head;
						}
						"y" => {
							let yanked = if is_line {
								let (s, e) = self.buffer.session.selection.ordered();
								self.buffer.yank_lines(s.line, *e.line - *s.line + 1)
							} else {
								self.buffer.copy()
							};
							self.buffer.session.selection.anchor =
								self.buffer.session.selection.head;
							self.vim.mode = VimMode::Normal;
							self.update_status();
							self.ensure_cursor_visible();
							if !yanked.is_empty() {
								return iced::clipboard::write::<EditorMsg>(yanked).map(|_| EditorMsg::Noop);
							}
							return Task::none();
						}
						"d" | "x" => {
							let yanked = if is_line {
								let (s, e) = self.buffer.session.selection.ordered();
								let lcount = *e.line - *s.line + 1;
								let y = self.buffer.yank_lines(s.line, lcount);
								self.buffer.delete_lines(s.line, lcount);
								y
							} else {
								self.buffer.cut()
							};
							self.vim.mode = VimMode::Normal;
							self.update_status();
							self.ensure_cursor_visible();
							if !yanked.is_empty() {
								return iced::clipboard::write::<EditorMsg>(yanked).map(|_| EditorMsg::Noop);
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
							self.vim.pending_obj_prefix = ch.chars().next();
							return Task::none();
						}
						"v" => {
							self.vim.mode = if is_line {
								VimMode::Visual
							} else {
								VimMode::Normal
							};
							if self.vim.mode == VimMode::Normal {
								self.buffer.session.selection.anchor =
									self.buffer.session.selection.head;
							}
						}
						"V" => {
							if is_line {
								self.vim.mode = VimMode::Normal;
								self.buffer.session.selection.anchor =
									self.buffer.session.selection.head;
							} else {
								self.vim.mode = VimMode::VisualLine;
								let (s, e) = self.buffer.session.selection.ordered();
								self.buffer.select_lines(*e.line - *s.line + 1);
							}
						}
						"u" => {
							self.buffer.transform_case(false);
							self.vim.mode = VimMode::Normal;
							self.update_status();
							self.ensure_cursor_visible();
							return Task::none();
						}
						"U" => {
							self.buffer.transform_case(true);
							self.vim.mode = VimMode::Normal;
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
		if self.vim.mode == VimMode::VisualLine {
			let (s, e) = self.buffer.session.selection.ordered();
			if self.buffer.session.selection.head >= self.buffer.session.selection.anchor {
				self.buffer.session.selection.anchor = CursorPos::new(s.line, CharIdx(0));
				self.buffer.session.selection.head =
					CursorPos::new(e.line, self.buffer.line_len(e.line));
			} else {
				self.buffer.session.selection.head = CursorPos::new(s.line, CharIdx(0));
				self.buffer.session.selection.anchor =
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
				&& (ch != "0" || !self.vim.count.is_empty());
			if is_count_digit {
				self.vim.count.push_str(ch);
				return Task::none();
			}
		}
		let count = self.take_count();

		match key {
			Key::Named(Named::Escape) => {
				self.buffer.session.selection.anchor = self.buffer.session.selection.head;
				self.vim.mode = VimMode::Normal;
			}
			Key::Named(Named::ArrowLeft) => self.buffer.move_left(true),
			Key::Named(Named::ArrowRight) => self.buffer.move_right(true),
			Key::Named(Named::ArrowUp) => self.buffer.move_up(true),
			Key::Named(Named::ArrowDown) => self.buffer.move_down(true),
			Key::Character(ref kc) => {
				let key_ch = kc.as_str();
				let ch = if ctrl {
					key_ch
				} else {
					text.as_deref().unwrap_or(key_ch)
				};
				if ctrl {
					match ch {
						"v" | "V" => {
							// Ctrl+V again collapses back to Normal
							self.buffer.session.selection.anchor =
								self.buffer.session.selection.head;
							self.vim.mode = VimMode::Normal;
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
						"v" => self.vim.mode = VimMode::Visual,
						"V" => {
							self.vim.mode = VimMode::VisualLine;
							let (s, e) = self.buffer.session.selection.ordered();
							self.buffer.select_lines(*e.line - *s.line + 1);
						}
						"I" => {
							let (s, e) = self.buffer.session.selection.ordered();
							let left_col = self
								.buffer
								.session
								.selection
								.anchor
								.col
								.min(self.buffer.session.selection.head.col);
							self.vim.block_insert = Some((left_col, s.line, e.line));
							self.buffer.session.selection =
								Selection::caret(CursorPos::new(s.line, left_col));
							self.enter_insert_mode();
							self.update_status();
							self.ensure_cursor_visible();
							return Task::none();
						}
						"A" => {
							let (s, e) = self.buffer.session.selection.ordered();
							let right_col = self
								.buffer
								.session
								.selection
								.anchor
								.col
								.max(self.buffer.session.selection.head.col)
								+ 1;
							self.vim.block_insert = Some((right_col, s.line, e.line));
							self.buffer.session.selection =
								Selection::caret(CursorPos::new(s.line, right_col));
							self.enter_insert_mode();
							self.update_status();
							self.ensure_cursor_visible();
							return Task::none();
						}
						"d" | "x" => {
							let (s, e) = self.buffer.session.selection.ordered();
							let left_col = self
								.buffer
								.session
								.selection
								.anchor
								.col
								.min(self.buffer.session.selection.head.col);
							let right_col = self
								.buffer
								.session
								.selection
								.anchor
								.col
								.max(self.buffer.session.selection.head.col)
								+ 1;
							self.buffer
								.block_delete(s.line, e.line, left_col, right_col);
							self.vim.mode = VimMode::Normal;
							self.update_status();
							self.ensure_cursor_visible();
							return Task::none();
						}
						"u" => {
							self.buffer.transform_case(false);
							self.vim.mode = VimMode::Normal;
							self.update_status();
							self.ensure_cursor_visible();
							return Task::none();
						}
						"U" => {
							self.buffer.transform_case(true);
							self.vim.mode = VimMode::Normal;
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
}
