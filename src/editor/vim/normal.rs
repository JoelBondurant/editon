use iced::Task;
use iced::keyboard::{self, Key};

use super::{VimMode, NormalEdit};
use super::super::coords::{CharIdx, CursorPos, Selection};
use super::super::core::{CodeEditor, EditorMsg};
use super::super::widget;

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
		let was_g = self.vim.pending_g;
		self.vim.pending_g = false;
		let was_z = self.vim.pending_z;
		self.vim.pending_z = false;

		// f/F/t/T pending: next key is the target char
		if let Some(find_kind) = self.vim.pending_find.take() {
			let target = match &key {
				Key::Named(Named::Space) => Some(' '),
				Key::Character(kc) => {
					let s = if ctrl {
						kc.as_str()
					} else {
						text.as_deref().unwrap_or(kc.as_str())
					};
					s.chars().next()
				}
				_ => None,
			};
			if let Some(tc) = target {
				let count = self.take_count();
				self.vim.last_find = Some((find_kind, tc));
				self.do_find(find_kind, tc, count, false);
				self.update_status();
				self.ensure_cursor_visible();
			}
			return Task::none();
		}

		// `r` (replace char) consumes the very next key as the replacement
		if self.vim.pending_op == Some('r') {
			self.vim.pending_op = None;
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
				self.vim.last_edit = Some(NormalEdit::ReplaceChar { ch: c, count });
			} else {
				self.vim.count.clear();
			}
			self.update_status();
			self.ensure_cursor_visible();
			return Task::none();
		}

		match key {
			Key::Named(Named::Escape) => {
				if self.buffer.session.search.is_open {
					self.buffer.search_close();
				}
				self.buffer.session.selection.anchor = self.buffer.session.selection.head;
				self.vim.count.clear();
				self.vim.pending_op = None;
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
				let v = widget::visible_line_count(self.view.viewport_h);
				self.buffer.page_up(v, false);
			}
			Key::Named(Named::PageDown) => {
				let v = widget::visible_line_count(self.view.viewport_h);
				self.buffer.page_down(v, false);
			}

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
						"w" | "W" => {
							let e = !self.buffer.document.wrap_config.enabled;
							self.set_wrap_enabled(e);
						}
						"m" | "M" => self.view.show_minimap = !self.view.show_minimap,
						"l" | "L" => self.view.show_whitespace = !self.view.show_whitespace,
						"r" | "R" => self.buffer.redo(),
						"v" | "V" => {
							self.vim.mode = VimMode::VisualBlock;
							self.buffer.session.selection.anchor =
								self.buffer.session.selection.head;
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
						&& (ch != "0" || !self.vim.count.is_empty());
					if is_count_digit {
						self.vim.count.push_str(ch);
						self.update_status();
						return Task::none();
					}

					let count = self.take_count();

					// Pending operator + motion/doubling
					if let Some(op) = self.vim.pending_op.take() {
						if (op == '>' && ch == ">") || (op == '<' && ch == "<") {
							let line = self.buffer.session.selection.head.line;
							let last =
								(line + count - 1).min(self.buffer.line_count().saturating_sub(1));
							self.buffer.session.selection = Selection {
								anchor: CursorPos::new(line, CharIdx(0)),
								head: CursorPos::new(last, self.buffer.line_len(last)),
							};
							if op == '>' {
								self.buffer.indent_lines();
							} else {
								self.buffer.dedent_lines();
							}
							self.buffer.session.selection =
								Selection::caret(CursorPos::new(line, CharIdx(0)));
							self.update_status();
							self.ensure_cursor_visible();
							return Task::none();
						}
						// Text object prefix: 'i'/'a' followed by object key (w, s, …)
						if let Some(obj) = self.vim.pending_obj_prefix.take() {
							let motion = format!("{}{}", obj, ch);
							return self.exec_operator_motion(op, &motion, count);
						}
						// Wait for text-object key
						if (ch == "i" || ch == "a") && !was_g {
							self.vim.pending_op = Some(op);
							self.vim.pending_obj_prefix = ch.chars().next();
							return Task::none();
						}
						// `g` inside dg/yg/cg — wait for second `g`
						if ch == "g" && !was_g {
							self.vim.pending_op = Some(op);
							self.vim.pending_g = true;
							return Task::none();
						}
						let task = match (op, ch) {
							('d', "d") => {
								let line = self.buffer.session.selection.head.line;
								let yanked = self.buffer.yank_lines(line, count);
								self.buffer.delete_lines(line, count);
								self.vim.last_edit = Some(NormalEdit::LineOp { op: 'd', count });
								self.update_status();
								self.ensure_cursor_visible();
								iced::clipboard::write::<EditorMsg>(yanked).map(|_| EditorMsg::Noop)
							}
							('y', "y") => {
								let line = self.buffer.session.selection.head.line;
								let yanked = self.buffer.yank_lines(line, count);
								self.update_status();
								iced::clipboard::write::<EditorMsg>(yanked).map(|_| EditorMsg::Noop)
							}
							('c', "c") => {
								let line = self.buffer.session.selection.head.line;
								let len = self.buffer.line_len(line);
								self.buffer.session.selection = Selection {
									anchor: CursorPos::new(line, CharIdx(0)),
									head: CursorPos::new(line, len),
								};
								let _ = self.buffer.cut();
								self.buffer.session.selection =
									Selection::caret(CursorPos::new(line, CharIdx(0)));
								self.vim.last_edit = Some(NormalEdit::LineOp { op: 'c', count });
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
							self.vim.mode = VimMode::Visual;
							self.buffer.session.selection.anchor =
								self.buffer.session.selection.head;
						}
						"V" => {
							self.vim.mode = VimMode::VisualLine;
							self.buffer.select_lines(count);
						}
						"d" => self.vim.pending_op = Some('d'),
						"y" => self.vim.pending_op = Some('y'),
						"c" => self.vim.pending_op = Some('c'),
						"r" => self.vim.pending_op = Some('r'),
						">" => self.vim.pending_op = Some('>'),
						"<" => self.vim.pending_op = Some('<'),
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
								let pos = self.buffer.session.selection.head;
								let lt = self.buffer.line_text(pos.line);
								if let Some(c) = lt.chars().nth(*pos.col) {
									let toggled = if c.is_uppercase() {
										c.to_lowercase().next().unwrap_or(c)
									} else {
										c.to_uppercase().next().unwrap_or(c)
									};
									self.buffer.replace_char(toggled);
									self.buffer.move_right(false);
								}
							}
							self.vim.last_edit = Some(NormalEdit::ToggleCase { count });
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
						":" => self.vim.mode = VimMode::Command,
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
						"g" => self.vim.pending_g = true,
						"G" => self.buffer.move_to_end(false),
						"x" => {
							for _ in 0..count {
								self.buffer.delete();
							}
							self.vim.last_edit = Some(NormalEdit::DeleteChar { count });
						}
						"X" => {
							for _ in 0..count {
								self.buffer.backspace();
							}
							self.vim.last_edit = Some(NormalEdit::BackspaceChar { count });
						}
						"u" => self.buffer.undo(),
						"n" => self.buffer.search_next(),
						"N" => self.buffer.search_prev(),
						// ── find-char motions ───────────────────────────────
						"f" => {
							self.vim.pending_find = Some('f');
							return Task::none();
						}
						"F" => {
							self.vim.pending_find = Some('F');
							return Task::none();
						}
						"t" => {
							self.vim.pending_find = Some('t');
							return Task::none();
						}
						"T" => {
							self.vim.pending_find = Some('T');
							return Task::none();
						}
						";" => {
							if let Some((kind, target)) = self.vim.last_find {
								self.do_find(kind, target, count, false);
							}
						}
						"," => {
							if let Some((kind, target)) = self.vim.last_find {
								let rev = match kind {
									'f' => 'F',
									'F' => 'f',
									't' => 'T',
									'T' => 't',
									c => c,
								};
								self.do_find(rev, target, count, false);
							}
						}
						// ── scroll centering ────────────────────────────────
						"z" => {
							self.vim.pending_z = true;
							return Task::none();
						}
						// ── dot repeat ──────────────────────────────────────
						"." => {
							if let Some(edit) = self.vim.last_edit.clone() {
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
}
