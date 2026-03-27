use iced::keyboard::{self, Key};
use iced::widget::{column, container, row, text, Space};
use iced::{event, Element, Length, Subscription, Task, Theme};

use super::buffer::{Buffer, CursorPos, Selection, UndoConfig};
use super::highlight::SyntaxLanguage;
use super::theme::EditorTheme;
use super::vim::{NormalEdit, VimMode};
use super::widget::{EditorAction, EditorWidget};
use super::{buffer, widget};

// ─── Public message type ──────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub enum EditorMsg {
	Action(EditorAction),
	Key(Key, keyboard::Modifiers, Option<String>),
	Scroll(f32, f32),
	MouseMove(iced::Point),
	MouseUp,
	/// Paste system-clipboard text at the current cursor position.
	Paste(String),
	/// Paste system-clipboard text after the current cursor (vim `p` semantics).
	PasteAfter(String),
	/// Replace the current visual selection with system-clipboard text (vim visual `p`).
	VisualPaste(String),
}

// ─── CodeEditor ───────────────────────────────────────────────────────────────

/// Self-contained code editor state. Embed in your app's state, drive with
/// `update` / `view` / `subscription`, and map messages to your own type.
pub struct CodeEditor {
	pub buffer: Buffer,
	pub theme: EditorTheme,
	pub view: EditorViewState,
	pub chrome: EditorChromeState,
	pub vim: VimState,
	pointer: PointerState,
}

pub struct EditorViewState {
	pub scroll_y: f32,
	pub scroll_x: f32,
	pub show_minimap: bool,
	pub show_whitespace: bool,
	pub viewport_w: f32,
	pub viewport_h: f32,
}

pub struct EditorChromeState {
	pub status: String,
}

pub struct VimState {
	pub mode: VimMode,
	pub(in crate::editor) command: String,
	pub(in crate::editor) count: String,
	pub(in crate::editor) pending_g: bool,
	pub(in crate::editor) pending_op: Option<char>,
	/// Pending block insert: (insert_col, top_line, bottom_line)
	pub(in crate::editor) block_insert: Option<(usize, usize, usize)>,
	/// Pending f/F/t/T: stores which variant is waiting for the target char
	pub(in crate::editor) pending_find: Option<char>,
	/// Last f/F/t/T find, for ; and , repeat
	pub(in crate::editor) last_find: Option<(char, char)>,
	/// Pending z-prefix (zz/zt/zb)
	pub(in crate::editor) pending_z: bool,
	/// Pending i/a text-object prefix inside an operator motion
	pub(in crate::editor) pending_obj_prefix: Option<char>,
	/// Last repeatable normal-mode edit (for `.`)
	pub(in crate::editor) last_edit: Option<NormalEdit>,
	/// Text inserted during the last Insert session (for dot-repeat of change ops)
	pub(in crate::editor) last_insert_text: String,
	/// Cursor col when Insert mode was entered (for last_insert_text capture)
	pub(in crate::editor) insert_enter_col: usize,
	/// Cursor line when Insert mode was entered
	pub(in crate::editor) insert_enter_line: usize,
}

struct PointerState {
	is_dragging: bool,
	click_count: u32,
}

fn default_undo_config() -> UndoConfig {
	UndoConfig {
		max_history: 1000,
		group_timeout_ms: 600,
	}
}

#[allow(dead_code)] // public API — used by the consuming application, not the demo
impl CodeEditor {
	/// Create a new editor with the given initial content and syntax language.
	pub fn new(content: &str, language: SyntaxLanguage) -> Self {
		let undo_cfg = default_undo_config();
		let buffer = Buffer::with_undo_config(content, language, undo_cfg);
		let mut ed = Self {
			buffer,
			theme: EditorTheme::dark(),
			view: EditorViewState {
				scroll_y: 0.0,
				scroll_x: 0.0,
				show_minimap: true,
				show_whitespace: true,
				viewport_w: 0.0,
				viewport_h: 0.0,
			},
			chrome: EditorChromeState {
				status: String::new(),
			},
			vim: VimState {
				mode: VimMode::Normal,
				command: String::new(),
				count: String::new(),
				pending_g: false,
				pending_op: None,
				block_insert: None,
				pending_find: None,
				last_find: None,
				pending_z: false,
				pending_obj_prefix: None,
				last_edit: None,
				last_insert_text: String::new(),
				insert_enter_col: 0,
				insert_enter_line: 0,
			},
			pointer: PointerState {
				is_dragging: false,
				click_count: 0,
			},
		};
		ed.update_status();
		ed
	}

	/// The current text content of the buffer.
	pub fn content(&self) -> String {
		self.buffer.rope.to_string()
	}

	/// Replace the buffer content (resets scroll and undo history).
	pub fn set_content(&mut self, content: &str) {
		let lang = self.buffer.language();
		self.buffer = Buffer::with_undo_config(content, lang, default_undo_config());
		self.view.scroll_y = 0.0;
		self.view.scroll_x = 0.0;
		self.update_status();
	}

	/// Replace content and switch language in one call.
	pub fn set_content_with_language(&mut self, content: &str, language: SyntaxLanguage) {
		self.buffer = Buffer::with_undo_config(content, language, default_undo_config());
		self.view.scroll_y = 0.0;
		self.view.scroll_x = 0.0;
		self.update_status();
	}

	/// Switch the syntax highlighting language (preserves content).
	pub fn set_language(&mut self, lang: SyntaxLanguage) {
		let content = self.content();
		self.set_content_with_language(&content, lang);
	}

	/// Enable or disable vim modal editing. When disabled the editor behaves
	/// like a conventional text editor (always in "insert" mode).
	pub fn set_vim_enabled(&mut self, enabled: bool) {
		self.vim.mode = if enabled {
			VimMode::Normal
		} else {
			VimMode::Off
		};
		self.update_status();
	}

	/// Returns `true` when vim modal editing is active.
	pub fn vim_enabled(&self) -> bool {
		self.vim.mode != VimMode::Off
	}

	/// Swap the active color theme.
	pub fn set_theme(&mut self, theme: EditorTheme) {
		self.theme = theme;
	}

	/// Notify the editor of its viewport size (pixels). Call whenever the
	/// containing pane is resized so cursor-scroll math stays accurate.
	pub fn set_viewport(&mut self, w: f32, h: f32) {
		self.view.viewport_w = w;
		self.view.viewport_h = h;
		if self.buffer.wrap_config.enabled {
			self.update_wrap_col();
		}
	}

	/// Enable or disable word wrap, computing the column from the current viewport.
	pub fn set_wrap_enabled(&mut self, enabled: bool) {
		self.buffer.set_wrap(enabled);
		if enabled {
			self.update_wrap_col();
		}
		if !enabled {
			// Horizontal scroll is meaningful again when wrap is off.
		}
	}

	/// Recompute the wrap column from the current viewport width and apply it.
	fn update_wrap_col(&mut self) {
		if self.view.viewport_w < 1.0 {
			return;
		}
		let gw = widget::gutter_width(self.buffer.line_count());
		let mm = if self.view.show_minimap {
			widget::minimap_width()
		} else {
			0.0
		};
		let usable =
			self.view.viewport_w - gw - widget::scrollbar_width() - mm - widget::left_pad();
		let col = ((usable / widget::CHAR_W) as usize).max(20);
		self.buffer.set_wrap_col(col);
		self.view.scroll_x = 0.0;
	}

	// ─── iced integration ─────────────────────────────────────────────────────

	pub fn subscription(&self) -> Subscription<EditorMsg> {
		event::listen_with(|event, _status, _id| match event {
			iced::Event::Keyboard(keyboard::Event::KeyPressed {
				key,
				modifiers,
				text,
				..
			}) => Some(EditorMsg::Key(key, modifiers, text.map(|t| t.to_string()))),
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
			iced::Event::Mouse(iced::mouse::Event::ButtonReleased(iced::mouse::Button::Left)) => {
				Some(EditorMsg::MouseUp)
			}
			_ => None,
		})
	}

	pub fn update(&mut self, msg: EditorMsg) -> Task<EditorMsg> {
		match msg {
			EditorMsg::Action(EditorAction::Resize(w, h)) => {
				self.set_viewport(w, h);
				return Task::none();
			}
			EditorMsg::Action(EditorAction::ToggleFold(line)) => {
				self.buffer.toggle_fold(line);
				if self.buffer.wrap_config.enabled {
					self.update_wrap_col();
				}
				self.update_status();
				self.ensure_cursor_visible();
				return Task::none();
			}
			EditorMsg::Action(EditorAction::MouseDown(pos)) => {
				let cursor_pos = self.pos_from_pixel(pos);
				self.buffer.selection.anchor = cursor_pos;
				self.buffer.selection.head = cursor_pos;
				self.pointer.is_dragging = true;
				self.pointer.click_count = 1;
				self.update_status();
			}
			EditorMsg::Action(EditorAction::DoubleClick(pos)) => {
				let cursor_pos = self.pos_from_pixel(pos);
				self.buffer.select_word_at(cursor_pos);
				self.pointer.is_dragging = true;
				self.pointer.click_count = 2;
				self.update_status();
			}
			EditorMsg::Action(_) => {}

			EditorMsg::MouseMove(pos) => {
				if self.pointer.is_dragging && self.pointer.click_count == 1 {
					let target = self.pos_from_pixel(pos);
					self.buffer.selection.head = target;
					self.update_status();
				}
			}
			EditorMsg::MouseUp => {
				self.pointer.is_dragging = false;
			}

			EditorMsg::Paste(text) => {
				if !text.is_empty() {
					self.buffer.clipboard = text.clone();
					self.buffer.clipboard_is_line = false;
					self.buffer.paste(&text);
					self.update_status();
					self.ensure_cursor_visible();
				}
			}

			EditorMsg::PasteAfter(text) => {
				if !text.is_empty() {
					self.buffer.clipboard = text.clone();
					self.buffer.clipboard_is_line = false;
					self.buffer.move_right(false);
					self.buffer.paste(&text);
					self.update_status();
					self.ensure_cursor_visible();
				}
			}

			EditorMsg::VisualPaste(yank) => {
				if !self.buffer.selection.is_caret() {
					let (s, e) = self.buffer.selection.ordered();
					let is_line = self.vim.mode == VimMode::VisualLine;
					let lcount = e.line - s.line + 1;
					let replaced = if is_line {
						let t = self.buffer.yank_lines(s.line, lcount);
						self.buffer.delete_lines(s.line, lcount);
						t
					} else {
						self.buffer.cut()
					};
					if !yank.is_empty() {
						self.buffer.paste(&yank);
					}
					self.buffer.clipboard = yank;
					self.buffer.clipboard_is_line = false;
					self.vim.mode = VimMode::Normal;
					self.update_status();
					self.ensure_cursor_visible();
					if !replaced.is_empty() {
						return iced::clipboard::write(replaced);
					}
				}
			}

			EditorMsg::Key(key, mods, text) => {
				// Ctrl+\ toggles vim on/off from any mode
				if mods.command() {
					if let Key::Character(ref ch) = key {
						if ch.as_str() == "\\" {
							self.set_vim_enabled(self.vim.mode == VimMode::Off);
							return Task::none();
						}
					}
				}

				match self.vim.mode {
					VimMode::Command => return self.handle_vim_command_key(key, text),
					VimMode::Normal => return self.handle_vim_normal_key(key, mods, text),
					VimMode::Visual | VimMode::VisualLine => {
						return self.handle_vim_visual_key(key, mods, text)
					}
					VimMode::VisualBlock => {
						return self.handle_vim_visual_block_key(key, mods, text)
					}
					VimMode::Insert | VimMode::Off => {}
				}

				// Insert mode (vim enabled): Escape → Normal
				if matches!(&key, Key::Named(keyboard::key::Named::Escape))
						&& self.vim.mode == VimMode::Insert
					&& !self.buffer.search.is_open
				{
					let col_before = self.buffer.selection.head.col;
					let line_before = self.buffer.selection.head.line;
					// Capture inserted text for dot-repeat
					if line_before == self.vim.insert_enter_line
						&& col_before > self.vim.insert_enter_col
					{
						self.vim.last_insert_text = self
							.buffer
							.line_text(line_before)
							.chars()
							.skip(self.vim.insert_enter_col)
							.take(col_before - self.vim.insert_enter_col)
							.collect();
					}
					self.vim.mode = VimMode::Normal;
					if col_before > 0 {
						self.buffer.move_left(false);
					}
					if let Some((insert_col, top_line, bottom_line)) = self.vim.block_insert.take() {
						if col_before > insert_col && line_before == top_line {
							let inserted: String = self
								.buffer
								.line_text(top_line)
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
						self.set_wrap_enabled(enabled);
					}
					Key::Character(ref ch) if ctrl && ch.as_str() == "m" => {
						self.view.show_minimap = !self.view.show_minimap;
					}
					Key::Character(ref ch) if ctrl && ch.as_str() == "l" => {
						self.view.show_whitespace = !self.view.show_whitespace;
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
					Key::Named(keyboard::key::Named::End) if ctrl => self.buffer.move_to_end(shift),
					Key::Named(keyboard::key::Named::Home) => self.buffer.move_home(shift),
					Key::Named(keyboard::key::Named::End) => self.buffer.move_end(shift),
					Key::Named(keyboard::key::Named::PageUp) => {
						let v = widget::visible_line_count(self.view.viewport_h);
						self.buffer.page_up(v, shift);
					}
					Key::Named(keyboard::key::Named::PageDown) => {
						let v = widget::visible_line_count(self.view.viewport_h);
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
					Key::Named(keyboard::key::Named::Tab) if shift => self.buffer.dedent_lines(),
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
									let cut = self.buffer.cut();
									if !cut.is_empty() {
										return iced::clipboard::write(cut);
									}
								}
								"v" => {
									return iced::clipboard::read()
										.map(|t| EditorMsg::Paste(t.unwrap_or_default()));
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
				if self.buffer.wrap_config.enabled {
					self.update_wrap_col();
				}
				self.ensure_cursor_visible();
			}

			EditorMsg::Scroll(dx, dy) => {
				let sp = if self.buffer.search.is_open {
					widget::search_panel_height()
				} else {
					0.0
				};
				let eh = self.view.viewport_h - sp;
				let vl_count = self.buffer.visual_lines.len();
				let max_y = (vl_count as f32 * widget::line_height() + widget::top_pad() * 2.0
					- eh)
					.max(0.0);
				self.view.scroll_y = (self.view.scroll_y + dy).clamp(0.0, max_y);
				if !self.buffer.wrap_config.enabled {
					self.view.scroll_x = (self.view.scroll_x + dx).max(0.0);
				}
			}
		}
		Task::none()
	}

	pub fn view(&self) -> Element<'_, EditorMsg> {
		let visual_block =
			if self.vim.mode == VimMode::VisualBlock && !self.buffer.selection.is_caret() {
				let (s, e) = self.buffer.selection.ordered();
				let left_col = self
					.buffer
					.selection
					.anchor
					.col
					.min(self.buffer.selection.head.col);
				let right_col = self
					.buffer
					.selection
					.anchor
					.col
					.max(self.buffer.selection.head.col);
				Some((s.line, e.line, left_col, right_col))
			} else {
				None
			};
		let editor = EditorWidget::new(&self.buffer, &self.theme, EditorMsg::Action)
			.scroll_y(self.view.scroll_y)
			.scroll_x(self.view.scroll_x)
			.show_minimap(self.view.show_minimap)
			.show_whitespace(self.view.show_whitespace)
			.block_cursor(self.vim.mode == VimMode::Normal)
			.visual_block(visual_block);

		let sc = self.theme.statusbar_text;
		let sep = self.theme.statusbar_sep;
		let lang = self.buffer.language().display_name();
		let wrap_status = if self.buffer.wrap_config.enabled {
			"Wrap:On"
		} else {
			"Wrap:Off"
		};

		let status_bar = container(
			row![
				text(&self.chrome.status).size(13).color(sc),
				Space::new().width(Length::Fill),
				text(wrap_status).size(13).color(sc),
				text("  ·  ").size(13).color(sep),
				text("UTF-8").size(13).color(sc),
				text("  ·  ").size(13).color(sep),
				text(lang).size(13).color(sc),
				text("  ·  ").size(13).color(sep),
				text("C-l=ws  C-m=map  C-w=wrap  C-\\=vim")
					.size(11)
					.color(sep),
			]
			.padding(6)
			.spacing(4),
		)
		.style({
			let bg = self.theme.statusbar_bg;
			move |_: &Theme| container::Style {
				background: Some(iced::Background::Color(bg)),
				..Default::default()
			}
		})
		.width(Length::Fill)
		.height(Length::Fixed(29.0))
		.clip(true);

		let cmd_bar_color = self.theme.cmdbar_text;
		let cmd_bar = container(
			row![
				text(":").size(14).color(cmd_bar_color),
				text(&self.vim.command).size(14).color(cmd_bar_color),
				text("█")
					.size(14)
					.color(iced::Color { a: 0.7, ..cmd_bar_color }),
			]
			.padding(iced::Padding {
				top: 4.0,
				bottom: 4.0,
				left: 8.0,
				right: 8.0,
			})
			.spacing(0),
		)
		.style({
			let bg = self.theme.cmdbar_bg;
			move |_: &Theme| container::Style {
				background: Some(iced::Background::Color(bg)),
				..Default::default()
			}
		})
		.width(Length::Fill);

		if self.vim.mode == VimMode::Command {
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
			width: self.view.viewport_w,
			height: self.view.viewport_h,
		};
		widget::pixel_to_pos(
			&self.buffer,
			&bounds,
			gw,
			self.view.scroll_x,
			self.view.scroll_y,
			pixel.x,
			pixel.y,
		)
	}

	pub(in crate::editor) fn take_count(&mut self) -> usize {
		let n = self.vim.count.parse::<usize>().unwrap_or(1).max(1);
		self.vim.count.clear();
		n
	}

	pub(in crate::editor) fn update_status(&mut self) {
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
		let mode = match self.vim.mode {
			VimMode::Off => Some("OFF"),
			VimMode::Normal => Some("NOR"),
			VimMode::Insert => Some("INS"),
			VimMode::Visual => Some("VIS"),
			VimMode::VisualLine => Some("V-LINE"),
			VimMode::VisualBlock => Some("V-BLOCK"),
			VimMode::Command => Some("CMD"),
		};
		self.chrome.status = if let Some(m) = mode {
			format!(
				"{} | Ln {}, Col {}{}{} | {} diag",
				m,
				p.line + 1,
				p.col + 1,
				sel,
				search,
				dc
			)
		} else {
			format!(
				"Ln {}, Col {}{}{} | {} diag",
				p.line + 1,
				p.col + 1,
				sel,
				search,
				dc
			)
		};
	}

	/// Record cursor position and enter Insert mode (for dot-repeat tracking).
	pub(in crate::editor) fn enter_insert_mode(&mut self) {
		self.vim.insert_enter_col = self.buffer.selection.head.col;
		self.vim.insert_enter_line = self.buffer.selection.head.line;
		self.vim.mode = VimMode::Insert;
	}

	/// Find char `target` in the f/F/t/T `kind` direction on current line.
	/// `extend` keeps selection anchor (visual / operator motion).
	pub(in crate::editor) fn do_find(
		&mut self,
		kind: char,
		target: char,
		count: usize,
		extend: bool,
	) {
		let forward = matches!(kind, 'f' | 't');
		let before_target = matches!(kind, 't' | 'T');

		let line = self.buffer.selection.head.line;
		let col = self.buffer.selection.head.col;
		let lt = self.buffer.line_text(line);
		let chars: Vec<char> = lt.chars().collect();

		let dest = if forward {
			let mut found = 0usize;
			let mut result = None;
			for i in (col + 1)..chars.len() {
				if chars[i] == target {
					found += 1;
					if found >= count {
						result = Some(if before_target {
							i.saturating_sub(1)
						} else {
							i
						});
						break;
					}
				}
			}
			result
		} else {
			let mut found = 0usize;
			let mut result = None;
			for i in (0..col).rev() {
				if chars[i] == target {
					found += 1;
					if found >= count {
						result = Some(if before_target { i + 1 } else { i });
						break;
					}
				}
			}
			result
		};

		if let Some(d) = dest {
			let pos = CursorPos::new(line, d);
			if extend {
				self.buffer.selection.head = pos;
			} else {
				self.buffer.selection = Selection::caret(pos);
			}
		}
	}

	/// Scroll so the cursor is centered (`z`), at top (`t`), or bottom (`b`).
	pub(in crate::editor) fn scroll_cursor_z(&mut self, mode: char) {
		let vl_idx = self.cursor_visual_line_idx();
		let cy = vl_idx as f32 * widget::line_height();
		let lh = widget::line_height();
		self.view.scroll_y = match mode {
			'z' => (cy - self.view.viewport_h / 2.0 + lh / 2.0).max(0.0),
			't' => cy,
			'b' => (cy - self.view.viewport_h + lh).max(0.0),
			_ => self.view.scroll_y,
		};
	}

	fn cursor_visual_line_idx(&self) -> usize {
		let head = self.buffer.selection.head;
		self.buffer
			.visual_lines
			.iter()
			.position(|vl| {
				vl.doc_line == head.line && vl.col_start <= head.col && head.col <= vl.col_end
			})
			.or_else(|| {
				self.buffer
					.visual_lines
					.iter()
					.position(|vl| vl.doc_line == head.line)
			})
			.unwrap_or(head.line)
	}

	pub(in crate::editor) fn ensure_cursor_visible(&mut self) {
		let sp = if self.buffer.search.is_open {
			widget::search_panel_height()
		} else {
			0.0
		};
		let vh = self.view.viewport_h - widget::top_pad() * 2.0 - sp;
		let vl_idx = self.cursor_visual_line_idx();
		let cy = vl_idx as f32 * widget::line_height();
		if cy < self.view.scroll_y {
			self.view.scroll_y = cy;
		} else if cy + widget::line_height() > self.view.scroll_y + vh {
			self.view.scroll_y = cy + widget::line_height() - vh;
		}
		if self.buffer.wrap_config.enabled {
			self.view.scroll_x = 0.0;
			return;
		}
		let head = self.buffer.selection.head;
		let hlt = self.buffer.line_text(head.line);
		let vcol = buffer::visual_col_of(&hlt, head.col);
		let cx = vcol as f32 * widget::CHAR_W;
		let gw = widget::gutter_width(self.buffer.line_count());
		let mm = if self.view.show_minimap {
			widget::minimap_width()
		} else {
			0.0
		};
		let vw = self.view.viewport_w - gw - widget::scrollbar_width() - mm;
		if cx < self.view.scroll_x {
			self.view.scroll_x = cx;
		} else if cx + widget::CHAR_W > self.view.scroll_x + vw {
			self.view.scroll_x = cx + widget::CHAR_W - vw;
		}
	}
}
