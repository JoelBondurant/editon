use iced::Task;
use iced::keyboard::{self, Key};
use regex::RegexBuilder;

use super::{ConfirmSubstituteMatch, ConfirmSubstituteState, PromptKind, VimHandler, parse_substitute};
use super::super::buffer::apply_vim_replacement;
use super::super::coords::{CharIdx, CursorPos, LineIdx};
use super::super::core::{CodeEditor, EditorMsg};

pub(in crate::editor) fn handle_command_key(
	vim: &mut VimHandler,
	ed: &mut CodeEditor,
	key: Key,
	text: Option<String>,
) -> Task<EditorMsg> {
	match vim.prompt_kind {
		PromptKind::Command => handle_ex_prompt_key(vim, ed, key, text),
		PromptKind::SearchForward => handle_search_prompt_key(vim, ed, key, text),
		PromptKind::SubstituteConfirm => handle_substitute_confirm_key(vim, ed, key, text),
	}
}

fn handle_ex_prompt_key(
	vim: &mut VimHandler,
	ed: &mut CodeEditor,
	key: Key,
	text: Option<String>,
) -> Task<EditorMsg> {
	use keyboard::key::Named;
	match key {
		Key::Named(Named::Escape) => vim.close_prompt(),
		Key::Named(Named::Enter) => {
			if execute_vim_command(vim, ed) {
				vim.close_prompt();
			}
		}
		Key::Named(Named::Backspace) => {
			if vim.command.pop().is_none() {
				vim.close_prompt();
			}
		}
		Key::Named(Named::Space) => vim.command.push(' '),
		Key::Character(_) => {
			if let Some(t) = text {
				vim.command.push_str(&t);
			}
		}
		_ => {}
	}
	ed.update_status();
	Task::none()
}

fn handle_search_prompt_key(
	vim: &mut VimHandler,
	ed: &mut CodeEditor,
	key: Key,
	text: Option<String>,
) -> Task<EditorMsg> {
	use keyboard::key::Named;
	match key {
		Key::Named(Named::Escape) => {
			if let Some(saved) = vim.saved_search.take() {
				ed.buffer.session.search = saved;
			}
			vim.close_prompt();
		}
		Key::Named(Named::Enter) => {
			vim.saved_search = None;
			vim.close_prompt();
		}
		Key::Named(Named::Backspace) => {
			vim.command.pop();
			refresh_search_prompt(vim, ed);
		}
		Key::Named(Named::Space) => {
			vim.command.push(' ');
			refresh_search_prompt(vim, ed);
		}
		Key::Character(_) => {
			if let Some(t) = text {
				vim.command.push_str(&t);
				refresh_search_prompt(vim, ed);
			}
		}
		_ => {}
	}
	ed.update_status();
	ed.ensure_cursor_visible();
	Task::none()
}

fn handle_substitute_confirm_key(
	vim: &mut VimHandler,
	ed: &mut CodeEditor,
	key: Key,
	text: Option<String>,
) -> Task<EditorMsg> {
	use keyboard::key::Named;
	match key {
		Key::Named(Named::Escape) => vim.close_prompt(),
		Key::Character(_) => {
			let ch = text.as_deref().unwrap_or("").chars().next();
			match ch {
				Some('y') | Some('Y') => {
					accept_current_substitute(vim, ed);
					if !advance_confirm_substitute(vim, ed) {
						vim.close_prompt();
					}
				}
				Some('n') | Some('N') => {
					skip_current_substitute(vim);
					if !advance_confirm_substitute(vim, ed) {
						vim.close_prompt();
					}
				}
				Some('a') | Some('A') => {
					while vim.confirm_substitute.as_ref().and_then(|s| s.current.as_ref()).is_some() {
						accept_current_substitute(vim, ed);
						if !advance_confirm_substitute(vim, ed) {
							break;
						}
					}
					vim.close_prompt();
				}
				Some('q') | Some('Q') => vim.close_prompt(),
				_ => {}
			}
		}
		_ => {}
	}
	ed.update_status();
	ed.ensure_cursor_visible();
	Task::none()
}

fn refresh_search_prompt(vim: &mut VimHandler, ed: &mut CodeEditor) {
	if vim.command.is_empty() {
		ed.buffer.session.search.is_open = true;
		ed.buffer.session.search.query.clear();
		ed.buffer.session.search.matches.clear();
		ed.buffer.session.search.current_match = 0;
		return;
	}
	ed.buffer.search_activate(&vim.command, true);
}

fn execute_vim_command(vim: &mut VimHandler, ed: &mut CodeEditor) -> bool {
	let cmd = vim.command.trim().to_string();

	if let Ok(n) = cmd.parse::<usize>() {
		let line_count = *ed.buffer.line_count();
		let line = n
			.saturating_sub(1usize)
			.min(line_count.saturating_sub(1usize));
		let target = LineIdx(line);
		ed.buffer.session.selection.anchor = CursorPos::new(target, CharIdx(0));
		ed.buffer.session.selection.head = CursorPos::new(target, CharIdx(0));
		ed.ensure_cursor_visible();
		return true;
	}

	if let Some((first, last, pat, rep, global, icase, confirm)) = parse_substitute(
		&cmd,
		*ed.buffer.session.selection.head.line,
		(*ed.buffer.line_count()).saturating_sub(1usize),
	) {
		if confirm {
			start_confirm_substitute(
				vim,
				ed,
				LineIdx(first),
				LineIdx(last),
				pat,
				rep,
				global,
				icase,
			);
			return false;
		}

		let changed = ed
			.buffer
			.substitute(LineIdx(first), LineIdx(last), &pat, &rep, global, icase);
		if changed > 0 {
			let line_count = *ed.buffer.line_count();
			let line = first.min(line_count.saturating_sub(1usize));
			let target = LineIdx(line);
			ed.buffer.session.selection.anchor = CursorPos::new(target, CharIdx(0));
			ed.buffer.session.selection.head = CursorPos::new(target, CharIdx(0));
			ed.ensure_cursor_visible();
		}
		ed.update_status();
		return true;
	}

	match cmd.as_str() {
		"noh" | "nohl" | "nohlsearch" => ed.buffer.search_close(),
		"q" | "q!" | "wq" | "w" => {}
		_ => {}
	}
	true
}

fn start_confirm_substitute(
	vim: &mut VimHandler,
	ed: &mut CodeEditor,
	first: LineIdx,
	last: LineIdx,
	pattern: String,
	replacement: String,
	global: bool,
	case_insensitive: bool,
) {
	vim.prompt_kind = PromptKind::SubstituteConfirm;
	vim.confirm_substitute = Some(ConfirmSubstituteState {
		last,
		pattern,
		replacement,
		global,
		case_insensitive,
		next_line: first,
		next_col: CharIdx(0),
		current: None,
	});
	if !advance_confirm_substitute(vim, ed) {
		vim.close_prompt();
	}
}

fn advance_confirm_substitute(vim: &mut VimHandler, ed: &mut CodeEditor) -> bool {
	let Some(state) = vim.confirm_substitute.as_mut() else {
		return false;
	};
	let Some(next_match) = find_next_confirm_match(ed, state) else {
		state.current = None;
		return false;
	};

	ed.buffer.session.selection.anchor = CursorPos::new(next_match.line, next_match.col_start);
	ed.buffer.session.selection.head = CursorPos::new(next_match.line, next_match.col_end);
	state.current = Some(next_match.clone());
	vim.command = format!("replace with {:?}? [y/n/a/q]", next_match.replacement);
	true
}

fn accept_current_substitute(vim: &mut VimHandler, ed: &mut CodeEditor) {
	let Some(state) = vim.confirm_substitute.as_mut() else {
		return;
	};
	let Some(current) = state.current.clone() else {
		return;
	};

	ed.buffer
		.replace_char_range(current.char_start, current.char_end, &current.replacement);

	if state.global {
		state.next_line = current.line;
		state.next_col = current.col_start + current.replacement.chars().count();
	} else {
		state.next_line = current.line + 1;
		state.next_col = CharIdx(0);
	}
}

fn skip_current_substitute(vim: &mut VimHandler) {
	let Some(state) = vim.confirm_substitute.as_mut() else {
		return;
	};
	let Some(current) = state.current.clone() else {
		return;
	};

	if state.global {
		state.next_line = current.line;
		state.next_col = current.col_end;
	} else {
		state.next_line = current.line + 1;
		state.next_col = CharIdx(0);
	}
}

fn find_next_confirm_match(
	ed: &CodeEditor,
	state: &ConfirmSubstituteState,
) -> Option<ConfirmSubstituteMatch> {
	let re = RegexBuilder::new(&state.pattern)
		.case_insensitive(state.case_insensitive)
		.build()
		.ok()?;

	let line_count = *ed.buffer.line_count();
	let last = (*state.last).min(line_count.saturating_sub(1usize));
	for line_raw in *state.next_line..=last {
		let line = LineIdx(line_raw);
		let text = ed.buffer.line_text(line);
		let start_col = if line == state.next_line {
			*state.next_col
		} else {
			0
		};

		if !state.global && line == state.next_line && start_col > 0 {
			continue;
		}

		let start_byte = char_to_byte_idx(&text, start_col);
		let haystack = &text[start_byte..];
		let Some(caps) = re.captures(haystack) else {
			continue;
		};
		let m = caps.get(0)?;
		let prefix = &haystack[..m.start()];
		let local_start = start_col + prefix.chars().count();
		let matched_len = m.as_str().chars().count();
		let local_end = local_start + matched_len;
		let line_char_start = ed.buffer.document.rope.line_to_char(line_raw);
		return Some(ConfirmSubstituteMatch {
			line,
			col_start: CharIdx(local_start),
			col_end: CharIdx(local_end),
			char_start: line_char_start + local_start,
			char_end: line_char_start + local_end,
			replacement: apply_vim_replacement(&state.replacement, &caps),
		});
	}
	None
}

fn char_to_byte_idx(text: &str, char_idx: usize) -> usize {
	text.char_indices()
		.map(|(i, _)| i)
		.nth(char_idx)
		.unwrap_or(text.len())
}
