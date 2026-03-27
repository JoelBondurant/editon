use iced::Task;
use iced::keyboard::{self, Key};

use super::{VimMode, parse_substitute};
use super::super::coords::CursorPos;
use super::super::core::{CodeEditor, EditorMsg};

impl CodeEditor {
	// ─── Command bar ──────────────────────────────────────────────────────────

	pub(in crate::editor) fn handle_vim_command_key(
		&mut self,
		key: Key,
		text: Option<String>,
	) -> Task<EditorMsg> {
		use keyboard::key::Named;
		match key {
			Key::Named(Named::Escape) => {
				self.vim.mode = VimMode::Normal;
				self.vim.command.clear();
			}
			Key::Named(Named::Enter) => {
				self.execute_vim_command();
				self.vim.mode = VimMode::Normal;
				self.vim.command.clear();
			}
			Key::Named(Named::Backspace) => {
				if self.vim.command.pop().is_none() {
					self.vim.mode = VimMode::Normal;
				}
			}
			Key::Named(Named::Space) => {
				self.vim.command.push(' ');
			}
			Key::Character(_) => {
				if let Some(t) = text {
					self.vim.command.push_str(&t);
				}
			}
			_ => {}
		}
		self.update_status();
		Task::none()
	}

	fn execute_vim_command(&mut self) {
		let cmd = self.vim.command.trim().to_string();

		if let Ok(n) = cmd.parse::<usize>() {
			let line = n
				.saturating_sub(1)
				.min(self.buffer.line_count().saturating_sub(1));
			self.buffer.session.selection.anchor = CursorPos { line, col: 0 };
			self.buffer.session.selection.head = CursorPos { line, col: 0 };
			self.ensure_cursor_visible();
			return;
		}

		if let Some((first, last, pat, rep, global, icase)) = parse_substitute(
			&cmd,
			self.buffer.session.selection.head.line,
			self.buffer.line_count().saturating_sub(1),
		) {
			let changed = self
				.buffer
				.substitute(first, last, &pat, &rep, global, icase);
			if changed > 0 {
				let line = first.min(self.buffer.line_count().saturating_sub(1));
				self.buffer.session.selection.anchor = CursorPos { line, col: 0 };
				self.buffer.session.selection.head = CursorPos { line, col: 0 };
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
