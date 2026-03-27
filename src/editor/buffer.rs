use super::coords::{document, line, CursorPos, Selection, TAB_WIDTH};
use super::folding::FoldState;
use super::highlight::{Highlighter, SyntaxLanguage, SyntaxToken, TokenKind};
use super::search::SearchState;
use super::undo::{EditKind, UndoConfig, UndoStack};
use super::wrap::{self, VisualLine, WrapConfig};
use regex::{Captures, RegexBuilder};
use ropey::Rope;

// ─── Diagnostic ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct Diagnostic {
	pub line: usize,
	pub col_start: usize,
	pub col_end: usize,
	pub message: String,
}

#[derive(Debug, Clone, Copy)]
pub struct TokenSpan {
	pub col_start: usize,
	pub col_end: usize,
	pub kind: TokenKind,
}

// ─── Bracket matching ─────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BracketPair {
	pub open_line: usize,
	pub open_col: usize,
	pub close_line: usize,
	pub close_col: usize,
}

#[derive(Clone, Copy)]
struct RefreshFlags {
	parse: bool,
	diagnostics: bool,
	brackets: bool,
	folds: bool,
	visual_lines: bool,
	search: bool,
}

impl RefreshFlags {
	const TEXT_EDIT: Self = Self {
		parse: true,
		diagnostics: true,
		brackets: true,
		folds: true,
		visual_lines: true,
		search: true,
	};

	const VISUAL_LINES_ONLY: Self = Self {
		parse: false,
		diagnostics: false,
		brackets: false,
		folds: false,
		visual_lines: true,
		search: false,
	};

	const BRACKETS_ONLY: Self = Self {
		parse: false,
		diagnostics: false,
		brackets: true,
		folds: false,
		visual_lines: false,
		search: false,
	};
}

// ─── Auto-pairs ───────────────────────────────────────────────────────────────

fn matching_close(c: char) -> Option<char> {
	match c {
		'(' => Some(')'),
		'[' => Some(']'),
		'{' => Some('}'),
		'\'' => Some('\''),
		'"' => Some('"'),
		_ => None,
	}
}
fn is_open_bracket(c: char) -> bool {
	matches!(c, '(' | '[' | '{')
}
fn is_close_bracket(c: char) -> bool {
	matches!(c, ')' | ']' | '}')
}
fn matching_open(c: char) -> Option<char> {
	match c {
		')' => Some('('),
		']' => Some('['),
		'}' => Some('{'),
		_ => None,
	}
}

// ─── Buffer ───────────────────────────────────────────────────────────────────

pub struct Buffer {
	pub rope: Rope,
	pub highlighter: Highlighter,
	pub diagnostics: Vec<Diagnostic>,
	pub selection: Selection,
	pub matched_bracket: Option<BracketPair>,
	pub search: SearchState,
	pub folds: FoldState,
	pub wrap_config: WrapConfig,
	pub visual_lines: Vec<VisualLine>,
	undo_stack: UndoStack,
	desired_col: Option<usize>,
	/// Clipboard contents (internal; real clipboard via iced Clipboard trait).
	pub clipboard: String,
	/// True when `clipboard` holds whole lines (dd/yy), so `p`/`P` paste linewise.
	pub clipboard_is_line: bool,
}

impl Buffer {
	pub fn new(text: &str, language: SyntaxLanguage) -> Self {
		Self::with_undo_config(text, language, UndoConfig::default())
	}

	pub fn with_undo_config(text: &str, language: SyntaxLanguage, undo_config: UndoConfig) -> Self {
		let rope = Rope::from_str(text);
		let mut hl = Highlighter::new(language);
		hl.parse(text);

		let sel = Selection::caret(CursorPos::zero());
		let undo = UndoStack::new(undo_config);

		let mut buf = Self {
			rope,
			highlighter: hl,
			diagnostics: Vec::new(),
			selection: sel,
			matched_bracket: None,
			search: SearchState::new(),
			folds: FoldState::new(),
			wrap_config: WrapConfig::default(),
			visual_lines: Vec::new(),
			undo_stack: undo,
			desired_col: None,
			clipboard: String::new(),
			clipboard_is_line: false,
		};
		buf.post_edit();
		buf
	}

	// ── Accessors ─────────────────────────────────────────────────────────

	pub fn tokens(&self) -> &[SyntaxToken] {
		&self.highlighter.tokens
	}
	pub fn language(&self) -> SyntaxLanguage {
		self.highlighter.language
	}

	pub fn set_language(&mut self, lang: SyntaxLanguage) {
		self.highlighter = Highlighter::new(lang);
		let text = self.rope.to_string();
		self.highlighter.parse(&text);
		self.post_edit();
	}

	pub fn line_count(&self) -> usize {
		self.rope.len_lines().max(1)
	}

	pub fn line_len(&self, line: usize) -> usize {
		if line >= self.rope.len_lines() {
			return 0;
		}
		let s: String = self.rope.line(line).chars().collect();
		s.trim_end_matches('\n')
			.trim_end_matches('\r')
			.chars()
			.count()
	}

	pub fn clamp_pos(&self, p: CursorPos) -> CursorPos {
		document::clamp_pos(&self.rope, p)
	}

	fn pos_to_char(&self, p: CursorPos) -> usize {
		document::pos_to_char(&self.rope, p)
	}

	pub fn line_text(&self, line: usize) -> String {
		if line >= self.rope.len_lines() {
			return String::new();
		}
		let s: String = self.rope.line(line).chars().collect();
		s.trim_end_matches('\n').trim_end_matches('\r').to_string()
	}

	pub fn line_slice(&self, line: usize, start_col: usize, end_col: usize) -> String {
		let text = self.line_text(line);
		line::slice_chars(&text, start_col, end_col)
	}

	pub fn token_spans_for_line(
		&self,
		line: usize,
		start_col: usize,
		end_col: usize,
	) -> Vec<TokenSpan> {
		let text = self.line_text(line);
		let line_len = text.chars().count();
		let clipped_start = start_col.min(line_len);
		let clipped_end = end_col.min(line_len);
		if clipped_start >= clipped_end {
			return Vec::new();
		}

		let line_byte_start = self.rope.line_to_byte(line);
		let line_byte_end = line_byte_start + text.len();
		let clip_byte_start = line_byte_start + line::char_to_byte_idx(&text, clipped_start);
		let clip_byte_end = line_byte_start + line::char_to_byte_idx(&text, clipped_end);

		let mut spans = Vec::new();
		for tok in self.tokens() {
			if tok.byte_range.end <= line_byte_start || tok.byte_range.start >= line_byte_end {
				continue;
			}
			let byte_start = tok.byte_range.start.max(clip_byte_start);
			let byte_end = tok.byte_range.end.min(clip_byte_end);
			if byte_start >= byte_end {
				continue;
			}
			let col_start = line::byte_to_char_idx(&text, byte_start - line_byte_start);
			let col_end = line::byte_to_char_idx(&text, byte_end - line_byte_start);
			if col_start < col_end {
				spans.push(TokenSpan {
					col_start,
					col_end,
					kind: tok.kind,
				});
			}
		}
		spans.sort_by_key(|span| span.col_start);
		spans
	}

	pub fn full_text(&self) -> String {
		self.rope.to_string()
	}

	pub fn selected_text(&self) -> String {
		if self.selection.is_caret() {
			return String::new();
		}
		let (s, e) = self.selection.ordered();
		self.rope
			.slice(self.pos_to_char(s)..self.pos_to_char(e))
			.to_string()
	}

	fn line_indent(&self, line: usize) -> String {
		self.line_text(line)
			.chars()
			.take_while(|c| c.is_whitespace())
			.collect()
	}

	fn char_at(&self, p: CursorPos) -> Option<char> {
		self.line_text(self.clamp_pos(p).line)
			.chars()
			.nth(self.clamp_pos(p).col)
	}

	fn char_before(&self, p: CursorPos) -> Option<char> {
		if p.col == 0 {
			None
		} else {
			self.line_text(p.line).chars().nth(p.col - 1)
		}
	}

	// ── Post-edit refresh ─────────────────────────────────────────────────

	fn post_edit(&mut self) {
		self.finish_undo();
		self.refresh(RefreshFlags::TEXT_EDIT);
	}

	fn refresh(&mut self, flags: RefreshFlags) {
		let text = flags.parse.then(|| self.rope.to_string());

		if flags.parse {
			if let Some(ref text) = text {
				self.highlighter.parse(text);
			}
		}
		if flags.diagnostics {
			self.collect_diagnostics();
		}
		if flags.brackets {
			self.update_bracket_match();
		}
		if flags.folds {
			let line_count = self.line_count();
			let lang = self.language();
			let tree = self.highlighter.tree();
			let rope = &self.rope;
			let mut line_text = |l: usize| {
				if l >= rope.len_lines() {
					return String::new();
				}
				let s: String = rope.line(l).chars().collect();
				s.trim_end_matches('\n').trim_end_matches('\r').to_string()
			};
			self.folds.detect_regions(tree, lang, line_count, &mut line_text);
		}
		if flags.visual_lines {
			self.recompute_visual_lines();
		}
		if flags.search && self.search.is_open {
			self.search.find_all(&self.rope);
		}
	}

	fn refresh_visual_lines(&mut self) {
		self.refresh(RefreshFlags::VISUAL_LINES_ONLY);
	}

	fn refresh_bracket_match(&mut self) {
		self.refresh(RefreshFlags::BRACKETS_ONLY);
	}

	fn recompute_visual_lines(&mut self) {
		self.visual_lines = wrap::compute_visual_lines(
			self.line_count(),
			&|l| self.line_text(l),
			&|l| self.folds.is_hidden(l),
			&self.wrap_config,
		);
	}

	// ── Undo / Redo ───────────────────────────────────────────────────────

	fn save_undo(&mut self, kind: EditKind) {
		self.undo_stack.begin_edit(self.selection, kind);
	}

	fn save_undo_boundary(&mut self) {
		self.undo_stack.force_boundary(self.selection);
	}

	fn finish_undo(&mut self) {
		self.undo_stack.end_edit(self.selection);
	}

	fn replace_range(&mut self, start: usize, end: usize, insert: &str) {
		let deleted = self.rope.slice(start..end).to_string();
		self.undo_stack
			.record_change(start, deleted, insert.to_string());
		self.rope.remove(start..end);
		if !insert.is_empty() {
			self.rope.insert(start, insert);
		}
	}

	fn insert_at(&mut self, start: usize, insert: &str) {
		self.replace_range(start, start, insert);
	}

	fn insert_char_at(&mut self, start: usize, ch: char) {
		self.replace_range(start, start, &ch.to_string());
	}

	fn remove_range(&mut self, start: usize, end: usize) {
		self.replace_range(start, end, "");
	}

	pub fn undo(&mut self) {
		if self.undo_stack.undo(&mut self.rope, &mut self.selection) {
			self.post_edit();
		}
	}

	pub fn redo(&mut self) {
		if self.undo_stack.redo(&mut self.rope, &mut self.selection) {
			self.post_edit();
		}
	}

	// ── Clipboard ─────────────────────────────────────────────────────────

	pub fn copy(&mut self) -> String {
		let text = self.selected_text();
		if !text.is_empty() {
			self.clipboard = text.clone();
			self.clipboard_is_line = false;
		}
		text
	}

	/// Delete a rectangular block from `top..=bottom` lines, columns `left_col..right_col_excl`.
	pub fn block_delete(
		&mut self,
		top: usize,
		bottom: usize,
		left_col: usize,
		right_col_excl: usize,
	) {
		if left_col >= right_col_excl {
			return;
		}
		self.save_undo_boundary();
		let bottom = bottom.min(self.line_count().saturating_sub(1));
		for li in (top..=bottom).rev() {
			let line_len = self.line_len(li);
			if left_col >= line_len {
				continue;
			}
			let ci_start = self.rope.line_to_char(li) + left_col;
			let ci_end = self.rope.line_to_char(li) + right_col_excl.min(line_len);
			if ci_start < ci_end {
				self.remove_range(ci_start, ci_end);
			}
		}
		self.selection = Selection::caret(CursorPos::new(top, left_col));
		self.post_edit();
	}

	/// Insert `text` at `col` on every line from `top+1..=bottom`, replicating a block insert.
	/// The top line already has the text from normal insert-mode editing.
	pub fn block_insert_text(&mut self, top: usize, bottom: usize, col: usize, text: &str) {
		if text.is_empty() {
			return;
		}
		let bottom = bottom.min(self.line_count().saturating_sub(1));
		if bottom <= top {
			return;
		}
		for li in (top + 1..=bottom).rev() {
			let line_len = self.line_len(li);
			if col <= line_len {
				let ci = self.rope.line_to_char(li) + col;
				self.insert_at(ci, text);
			} else {
				// Pad with spaces to reach col, then insert
				let pad: String = " ".repeat(col - line_len);
				let ci = self.rope.line_to_char(li) + line_len;
				self.insert_at(ci, &format!("{}{}", pad, text));
			}
		}
		self.post_edit();
	}

	pub fn transform_case(&mut self, uppercase: bool) {
		if self.selection.is_caret() {
			return;
		}
		let text = self.selected_text();
		let transformed: String = if uppercase {
			text.chars().flat_map(|c| c.to_uppercase()).collect()
		} else {
			text.chars().flat_map(|c| c.to_lowercase()).collect()
		};
		let (s, e) = self.selection.ordered();
		self.save_undo_boundary();
		let ci_start = self.pos_to_char(s);
		let ci_end = self.pos_to_char(e);
		self.replace_range(ci_start, ci_end, &transformed);
		self.selection = Selection::caret(s);
		self.post_edit();
	}

	pub fn cut(&mut self) -> String {
		let text = self.copy();
		if !text.is_empty() {
			self.save_undo_boundary();
			self.delete_selection_inner();
			self.post_edit();
		}
		text
	}

	pub fn paste(&mut self, text: &str) {
		if text.is_empty() {
			return;
		}
		self.save_undo(EditKind::Paste);
		self.delete_selection_inner();
		self.desired_col = None;
		let pos = self.selection.head;
		let ci = self.pos_to_char(pos);
		self.insert_at(ci, text);

		let newlines = text.chars().filter(|c| *c == '\n').count();
		let new_pos = if newlines > 0 {
			let after = &text[text.rfind('\n').unwrap() + 1..];
			CursorPos::new(pos.line + newlines, after.len())
		} else {
			CursorPos::new(pos.line, pos.col + text.chars().count())
		};
		self.selection = Selection::caret(new_pos);
		self.post_edit();
	}

	/// Yank `count` whole lines starting at `line` into the internal clipboard.
	/// Returns the yanked text so callers can also write to the system clipboard.
	pub fn yank_lines(&mut self, line: usize, count: usize) -> String {
		let last = (line + count - 1).min(self.line_count().saturating_sub(1));
		let start_ci = self.rope.line_to_char(line);
		let end_ci = if last + 1 < self.rope.len_lines() {
			self.rope.line_to_char(last + 1)
		} else {
			self.rope.len_chars()
		};
		let mut text: String = self.rope.slice(start_ci..end_ci).to_string();
		// Ensure the yanked text always ends with a newline so paste works correctly.
		if !text.ends_with('\n') {
			text.push('\n');
		}
		self.clipboard = text.clone();
		self.clipboard_is_line = true;
		text
	}

	/// Delete `count` whole lines starting at `line`.
	pub fn delete_lines(&mut self, line: usize, count: usize) {
		let last = (line + count - 1).min(self.line_count().saturating_sub(1));
		self.save_undo(EditKind::Delete);
		let start_ci = self.rope.line_to_char(line);
		let end_ci = if last + 1 < self.rope.len_lines() {
			self.rope.line_to_char(last + 1)
		} else if line > 0 {
			// Last line with no trailing newline: delete preceding newline too
			let prev_end = self.rope.line_to_char(line);
			let prev_line_start = self.rope.line_to_char(line - 1);
			let prev_text: String = self.rope.slice(prev_line_start..prev_end).to_string();
			let trim = prev_text
				.trim_end_matches('\n')
				.trim_end_matches('\r')
				.len();
			prev_line_start + trim
		} else {
			self.rope.len_chars()
		};
		let real_start = start_ci.min(end_ci);
		let real_end = start_ci.max(end_ci);
		self.remove_range(real_start, real_end);
		let new_line = line.min(self.line_count().saturating_sub(1));
		self.selection = Selection::caret(CursorPos::new(new_line, 0));
		self.post_edit();
	}

	/// Paste linewise clipboard content as new line(s) below the current line.
	pub fn paste_line_below(&mut self) {
		if self.clipboard.is_empty() {
			return;
		}
		self.save_undo(EditKind::Paste);
		let line = self.selection.head.line;
		// Insert after the newline at end of current line
		let insert_ci = if line + 1 < self.rope.len_lines() {
			self.rope.line_to_char(line + 1)
		} else {
			// No trailing newline on last line — add one first
			let end = self.rope.len_chars();
				self.insert_char_at(end, '\n');
				end + 1
			};
		let text = self.clipboard.clone();
		self.insert_at(insert_ci, &text);
		self.selection = Selection::caret(CursorPos::new(line + 1, 0));
		self.post_edit();
	}

	/// Paste linewise clipboard content as new line(s) above the current line.
	pub fn paste_line_above(&mut self) {
		if self.clipboard.is_empty() {
			return;
		}
		self.save_undo(EditKind::Paste);
		let line = self.selection.head.line;
		let insert_ci = self.rope.line_to_char(line);
		let text = self.clipboard.clone();
		self.insert_at(insert_ci, &text);
		self.selection = Selection::caret(CursorPos::new(line, 0));
		self.post_edit();
	}

	/// Select the full extent of `count` lines starting at the cursor's line.
	/// Sets anchor to line start and head to end of last line (exclusive of newline).
	pub fn select_lines(&mut self, count: usize) {
		let line = self.selection.head.line;
		let last = (line + count - 1).min(self.line_count().saturating_sub(1));
		self.selection.anchor = CursorPos::new(line, 0);
		self.selection.head = CursorPos::new(last, self.line_len(last));
	}

	// ── Indent / Dedent ───────────────────────────────────────────────────

	/// Indent selected lines (or current line) by one tab character.
	pub fn indent_lines(&mut self) {
		let (first, last) = if self.selection.is_caret() {
			let l = self.selection.head.line;
			(l, l)
		} else {
			let (s, e) = self.selection.ordered();
			(s.line, e.line)
		};
		self.save_undo(EditKind::Insert);
		for line in (first..=last).rev() {
			let ci = self.rope.line_to_char(line);
			self.insert_at(ci, "\t");
		}
		let shift = |p: CursorPos| CursorPos::new(p.line, p.col + 1);
		self.selection.anchor = shift(self.selection.anchor);
		self.selection.head = shift(self.selection.head);
		self.post_edit();
	}

	/// Dedent selected lines (or current line) by one tab stop.
	/// Removes a leading tab first; if none, removes up to 4 leading spaces.
	pub fn dedent_lines(&mut self) {
		let (first, last) = if self.selection.is_caret() {
			let l = self.selection.head.line;
			(l, l)
		} else {
			let (s, e) = self.selection.ordered();
			(s.line, e.line)
		};
		self.save_undo(EditKind::Delete);
		let mut removed = vec![0usize; last - first + 1];
		for (i, line) in (first..=last).rev().enumerate() {
			let text = self.line_text(line);
			let ci = self.rope.line_to_char(line);
			let n = if text.starts_with('\t') {
				self.remove_range(ci, ci + 1);
				1
			} else {
				let spaces = text
					.chars()
					.take_while(|c| *c == ' ')
					.count()
					.min(TAB_WIDTH);
				if spaces > 0 {
					self.remove_range(ci, ci + spaces);
				}
				spaces
			};
			removed[last - first - i] = n;
		}
		let clamp = |p: CursorPos| {
			let rm = removed
				.get(p.line.saturating_sub(first))
				.copied()
				.unwrap_or(0);
			CursorPos::new(p.line, p.col.saturating_sub(rm))
		};
		self.selection.anchor = clamp(self.selection.anchor);
		self.selection.head = clamp(self.selection.head);
		self.post_edit();
	}

	// ── Editing ───────────────────────────────────────────────────────────

	pub fn insert_char(&mut self, ch: char) {
		self.save_undo(EditKind::Insert);
		self.delete_selection_inner();
		self.desired_col = None;
		let pos = self.selection.head;
		let ci = self.pos_to_char(pos);
		self.insert_char_at(ci, ch);
		let new = if ch == '\n' {
			CursorPos::new(pos.line + 1, 0)
		} else {
			CursorPos::new(pos.line, pos.col + 1)
		};
		self.selection = Selection::caret(new);
		self.post_edit();
	}

	pub fn insert_str(&mut self, text: &str) {
		if text.is_empty() {
			return;
		}
		self.save_undo(EditKind::Insert);
		self.delete_selection_inner();
		self.desired_col = None;
		let pos = self.selection.head;
		let ci = self.pos_to_char(pos);
		self.insert_at(ci, text);
		let newlines = text.chars().filter(|c| *c == '\n').count();
		let new = if newlines > 0 {
			let after = &text[text.rfind('\n').unwrap() + 1..];
			CursorPos::new(pos.line + newlines, after.len())
		} else {
			CursorPos::new(pos.line, pos.col + text.chars().count())
		};
		self.selection = Selection::caret(new);
		self.post_edit();
	}

	/// Syntax-aware Enter: auto-indent + extra indent after openers.
	pub fn insert_newline(&mut self) {
		self.save_undo(EditKind::Newline);
		self.delete_selection_inner();
		self.desired_col = None;
		let pos = self.selection.head;
		let indent = self.line_indent(pos.line);
		let before = self.line_text(pos.line);
		let before_cursor = &before[..pos.col.min(before.len())];
		let trimmed = before_cursor.trim_end();

		let extra = match self.highlighter.language {
			SyntaxLanguage::Sql => {
				if trimmed.ends_with('(')
					|| trimmed.ends_with('{')
					|| trimmed.ends_with('[')
					|| trimmed.to_uppercase().ends_with(" AS")
					|| trimmed.to_uppercase().ends_with(" BEGIN")
					|| trimmed.to_uppercase().ends_with(" THEN")
				{
					"    "
				} else {
					""
				}
			}
			SyntaxLanguage::Rust => {
				if trimmed.ends_with('{')
					|| trimmed.ends_with('(')
					|| trimmed.ends_with('[')
					|| trimmed.ends_with("=>")
				{
					"    "
				} else {
					""
				}
			}
		};

		let ins = format!("\n{}{}", indent, extra);
		let ci = self.pos_to_char(pos);
		self.insert_at(ci, &ins);
		self.selection = Selection::caret(CursorPos::new(pos.line + 1, indent.len() + extra.len()));
		self.post_edit();
	}

	pub fn insert_char_auto_pair(&mut self, ch: char) {
		// Skip over matching close
		if is_close_bracket(ch) || ch == '\'' || ch == '"' {
			if self.char_at(self.selection.head) == Some(ch) {
				let p = self.selection.head;
				self.selection = Selection::caret(CursorPos::new(p.line, p.col + 1));
				self.desired_col = None;
				self.refresh_bracket_match();
				return;
			}
		}
		if let Some(close) = matching_close(ch) {
			if ch == '\'' || ch == '"' {
				if let Some(prev) = self.char_before(self.selection.head) {
					if prev.is_alphanumeric() || prev == '_' {
						self.insert_char(ch);
						return;
					}
				}
			}
			self.save_undo(EditKind::Insert);
			self.delete_selection_inner();
			self.desired_col = None;
			let p = self.selection.head;
			let ci = self.pos_to_char(p);
			self.insert_at(ci, &format!("{}{}", ch, close));
			self.selection = Selection::caret(CursorPos::new(p.line, p.col + 1));
			self.post_edit();
		} else {
			self.insert_char(ch);
		}
	}

	pub fn backspace(&mut self) {
		self.desired_col = None;
		if !self.selection.is_caret() {
			self.save_undo(EditKind::Delete);
			self.delete_selection_inner();
			self.post_edit();
			return;
		}
		let p = self.selection.head;
		if p.line == 0 && p.col == 0 {
			return;
		}
		self.save_undo(EditKind::Delete);

		// Auto-pair removal
		if p.col > 0 {
			if let Some(prev) = self.char_before(p) {
				if let Some(exp) = matching_close(prev) {
					if self.char_at(p) == Some(exp) {
						let cs = self.pos_to_char(CursorPos::new(p.line, p.col - 1));
						self.remove_range(cs, cs + 2);
						self.selection = Selection::caret(CursorPos::new(p.line, p.col - 1));
						self.post_edit();
						return;
					}
				}
			}
		}

		let (new_pos, ds, de);
		if p.col == 0 {
			let pl = p.line - 1;
			new_pos = CursorPos::new(pl, self.line_len(pl));
			ds = self.pos_to_char(new_pos);
			de = self.pos_to_char(p);
		} else {
			new_pos = CursorPos::new(p.line, p.col - 1);
			ds = self.pos_to_char(new_pos);
			de = self.pos_to_char(p);
		}
		self.remove_range(ds, de);
		self.selection = Selection::caret(new_pos);
		self.post_edit();
	}

	pub fn delete(&mut self) {
		self.desired_col = None;
		if !self.selection.is_caret() {
			self.save_undo(EditKind::Delete);
			self.delete_selection_inner();
			self.post_edit();
			return;
		}
		let ci = self.pos_to_char(self.selection.head);
		if ci >= self.rope.len_chars() {
			return;
		}
		self.save_undo(EditKind::Delete);
		self.remove_range(ci, ci + 1);
		self.post_edit();
	}

	pub fn delete_word_back(&mut self) {
		self.desired_col = None;
		if !self.selection.is_caret() {
			self.save_undo(EditKind::Delete);
			self.delete_selection_inner();
			self.post_edit();
			return;
		}
		let p = self.selection.head;
		if p.line == 0 && p.col == 0 {
			return;
		}
		self.save_undo(EditKind::Delete);
		let t = self.word_boundary_left(p);
		self.remove_range(self.pos_to_char(t), self.pos_to_char(p));
		self.selection = Selection::caret(t);
		self.post_edit();
	}

	pub fn delete_word_forward(&mut self) {
		self.desired_col = None;
		if !self.selection.is_caret() {
			self.save_undo(EditKind::Delete);
			self.delete_selection_inner();
			self.post_edit();
			return;
		}
		let p = self.selection.head;
		if self.pos_to_char(p) >= self.rope.len_chars() {
			return;
		}
		self.save_undo(EditKind::Delete);
		let t = self.word_boundary_right(p);
		self.remove_range(self.pos_to_char(p), self.pos_to_char(t));
		self.post_edit();
	}

	pub fn duplicate_line(&mut self) {
		self.save_undo_boundary();
		let l = self.selection.head.line;
		let t = self.line_text(l);
		let ls = self.rope.line_to_char(l);
		let lc = self.rope.line(l).len_chars();
		let at = ls + lc;
		let ins = if at >= self.rope.len_chars() {
			format!("\n{}", t)
		} else {
			format!("{}\n", t)
		};
		self.insert_at(at, &ins);
		self.selection = Selection::caret(CursorPos::new(l + 1, self.selection.head.col));
		self.post_edit();
	}

	fn delete_selection_inner(&mut self) {
		if self.selection.is_caret() {
			return;
		}
		let (s, e) = self.selection.ordered();
		self.remove_range(self.pos_to_char(s), self.pos_to_char(e));
		self.selection = Selection::caret(s);
	}

	// ── Search ────────────────────────────────────────────────────────────

	pub fn search_open(&mut self) {
		self.search.is_open = true;
		// Pre-fill with selected text
		let sel = self.selected_text();
		if !sel.is_empty() && !sel.contains('\n') {
			self.search.query = sel;
		}
		self.search.find_all(&self.rope);
	}

	pub fn search_close(&mut self) {
		self.search.is_open = false;
		self.search.matches.clear();
	}

	pub fn search_update_query(&mut self, query: &str) {
		self.search.query = query.to_string();
		self.search.find_all(&self.rope);
	}

	pub fn search_next(&mut self) {
		self.search.next_match();
		self.jump_to_current_match();
	}

	pub fn search_prev(&mut self) {
		self.search.prev_match();
		self.jump_to_current_match();
	}

	pub fn search_replace_current(&mut self) {
		self.save_undo_boundary();
		if let Some(m) = self.search.current().cloned() {
			let replacement = self.search.replacement.clone();
			self.replace_range(m.char_start, m.char_end, &replacement);
			self.post_edit();
		}
	}

	pub fn search_replace_all(&mut self) {
		self.save_undo_boundary();
		let replacement = self.search.replacement.clone();
		let matches = self.search.matches.clone();
		let mut changed = 0usize;
		for m in matches.iter().rev() {
			self.replace_range(m.char_start, m.char_end, &replacement);
			changed += 1;
		}
		if changed > 0 {
			self.post_edit();
		}
	}

	fn jump_to_current_match(&mut self) {
		if let Some(m) = self.search.current() {
			self.selection = Selection {
				anchor: CursorPos::new(m.line, m.col_start),
				head: CursorPos::new(m.line, m.col_end),
			};
		}
	}

	/// Search for `word` without opening the panel (used by `*` / `#`).
	/// Jumps to the nearest match at or after the cursor.
	pub fn search_star(&mut self, word: &str, forward: bool) {
		self.search.query = word.to_string();
		self.search.find_all(&self.rope);
		let ci = self.pos_to_char(self.selection.head);
		if forward {
			self.search.jump_to_nearest(ci + 1);
		} else {
			// Jump to the match just before current pos
			if !self.search.matches.is_empty() {
				let n = self.search.matches.len();
				self.search.current_match = (0..n)
					.rev()
					.find(|&i| self.search.matches[i].char_start < ci)
					.unwrap_or(n - 1);
			}
		}
		self.jump_to_current_match();
	}

	/// Replace the character under the cursor with `ch`, leaving the cursor on it.
	pub fn replace_char(&mut self, ch: char) {
		let pos = self.selection.head;
		if pos.col >= self.line_len(pos.line) {
			return;
		}
		self.save_undo(EditKind::Delete);
		let ci = self.pos_to_char(pos);
		self.replace_range(ci, ci + 1, &ch.to_string());
		let new_pos = if ch == '\n' {
			CursorPos::new(pos.line + 1, 0)
		} else {
			pos
		};
		self.selection = Selection::caret(new_pos);
		self.post_edit();
	}

	/// Return the word (alphanumeric + `_`) under the cursor, or `None`.
	pub fn word_under_cursor(&self) -> Option<String> {
		let pos = self.selection.head;
		let text = self.line_text(pos.line);
		let chars: Vec<char> = text.chars().collect();
		if pos.col >= chars.len() {
			return None;
		}
		let is_word = |c: char| c.is_alphanumeric() || c == '_';
		if !is_word(chars[pos.col]) {
			return None;
		}
		let mut start = pos.col;
		while start > 0 && is_word(chars[start - 1]) {
			start -= 1;
		}
		let mut end = pos.col + 1;
		while end < chars.len() && is_word(chars[end]) {
			end += 1;
		}
		Some(chars[start..end].iter().collect())
	}

	// ── Folding ───────────────────────────────────────────────────────────

	pub fn toggle_fold(&mut self, line: usize) {
		self.folds.toggle(line);
		self.refresh_visual_lines();
	}

	// ── Wrapping ──────────────────────────────────────────────────────────

	pub fn set_wrap(&mut self, enabled: bool) {
		self.wrap_config.enabled = enabled;
		self.refresh_visual_lines();
	}

	pub fn set_wrap_col(&mut self, col: usize) {
		self.wrap_config.wrap_col = col;
		if self.wrap_config.enabled {
			self.refresh_visual_lines();
		}
	}

	// ── Navigation ────────────────────────────────────────────────────────

	pub fn move_left(&mut self, extend: bool) {
		self.desired_col = None;
		if !extend && !self.selection.is_caret() {
			let (s, _) = self.selection.ordered();
			self.selection = Selection::caret(s);
			self.refresh_bracket_match();
			return;
		}
		let p = self.selection.head;
		let n = if p.col > 0 {
			CursorPos::new(p.line, p.col - 1)
		} else if p.line > 0 {
			CursorPos::new(p.line - 1, self.line_len(p.line - 1))
		} else {
			p
		};
		self.set_head(n, extend);
	}

	pub fn move_right(&mut self, extend: bool) {
		self.desired_col = None;
		if !extend && !self.selection.is_caret() {
			let (_, e) = self.selection.ordered();
			self.selection = Selection::caret(e);
			self.refresh_bracket_match();
			return;
		}
		let p = self.selection.head;
		let ll = self.line_len(p.line);
		let n = if p.col < ll {
			CursorPos::new(p.line, p.col + 1)
		} else if p.line < self.line_count() - 1 {
			CursorPos::new(p.line + 1, 0)
		} else {
			p
		};
		self.set_head(n, extend);
	}

	pub fn move_up(&mut self, extend: bool) {
		let p = self.selection.head;
		if p.line == 0 {
			return;
		}
		let tc = self.desired_col.unwrap_or(p.col);
		// Skip folded lines
		let mut nl = p.line - 1;
		while nl > 0 && self.folds.is_hidden(nl) {
			nl -= 1;
		}
		let nc = tc.min(self.line_len(nl));
		self.set_head(CursorPos::new(nl, nc), extend);
		self.desired_col = Some(tc);
	}

	pub fn move_down(&mut self, extend: bool) {
		let p = self.selection.head;
		if p.line >= self.line_count() - 1 {
			return;
		}
		let tc = self.desired_col.unwrap_or(p.col);
		let mut nl = p.line + 1;
		let max = self.line_count() - 1;
		while nl < max && self.folds.is_hidden(nl) {
			nl += 1;
		}
		let nc = tc.min(self.line_len(nl));
		self.set_head(CursorPos::new(nl, nc), extend);
		self.desired_col = Some(tc);
	}

	pub fn move_home(&mut self, extend: bool) {
		self.desired_col = None;
		let p = self.selection.head;
		let first = self
			.line_text(p.line)
			.chars()
			.position(|c| !c.is_whitespace())
			.unwrap_or(0);
		let nc = if p.col <= first && p.col != 0 {
			0
		} else {
			first
		};
		self.set_head(CursorPos::new(p.line, nc), extend);
	}

	pub fn move_end(&mut self, extend: bool) {
		self.desired_col = None;
		let p = self.selection.head;
		self.set_head(CursorPos::new(p.line, self.line_len(p.line)), extend);
	}

	pub fn move_word_left(&mut self, extend: bool) {
		self.desired_col = None;
		self.set_head(self.word_boundary_left(self.selection.head), extend);
	}

	pub fn move_word_right(&mut self, extend: bool) {
		self.desired_col = None;
		self.set_head(self.word_boundary_right(self.selection.head), extend);
	}

	pub fn move_to_start(&mut self, extend: bool) {
		self.desired_col = None;
		self.set_head(CursorPos::zero(), extend);
	}

	pub fn move_to_end(&mut self, extend: bool) {
		self.desired_col = None;
		let l = self.line_count().saturating_sub(1);
		self.set_head(CursorPos::new(l, self.line_len(l)), extend);
	}

	pub fn page_up(&mut self, vis: usize, extend: bool) {
		let p = self.selection.head;
		let tc = self.desired_col.unwrap_or(p.col);
		let nl = p.line.saturating_sub(vis);
		self.set_head(CursorPos::new(nl, tc.min(self.line_len(nl))), extend);
		self.desired_col = Some(tc);
	}

	pub fn page_down(&mut self, vis: usize, extend: bool) {
		let p = self.selection.head;
		let tc = self.desired_col.unwrap_or(p.col);
		let nl = (p.line + vis).min(self.line_count().saturating_sub(1));
		self.set_head(CursorPos::new(nl, tc.min(self.line_len(nl))), extend);
		self.desired_col = Some(tc);
	}

	pub fn select_all(&mut self) {
		let l = self.line_count().saturating_sub(1);
		self.selection = Selection {
			anchor: CursorPos::zero(),
			head: CursorPos::new(l, self.line_len(l)),
		};
	}

	pub fn select_word_at(&mut self, p: CursorPos) {
		let p = self.clamp_pos(p);
		let text = self.line_text(p.line);
		if text.is_empty() {
			self.selection = Selection::caret(p);
			return;
		}
		let chars: Vec<char> = text.chars().collect();
		let col = p.col.min(chars.len().saturating_sub(1));
		let is_w = |c: char| c.is_alphanumeric() || c == '_';
		if !is_w(chars[col]) {
			self.selection = Selection {
				anchor: CursorPos::new(p.line, col),
				head: CursorPos::new(p.line, col + 1),
			};
			return;
		}
		let mut s = col;
		while s > 0 && is_w(chars[s - 1]) {
			s -= 1;
		}
		let mut e = col;
		while e < chars.len() && is_w(chars[e]) {
			e += 1;
		}
		self.selection = Selection {
			anchor: CursorPos::new(p.line, s),
			head: CursorPos::new(p.line, e),
		};
	}

	pub fn select_line(&mut self, line: usize) {
		let l = line.min(self.line_count().saturating_sub(1));
		self.selection = Selection {
			anchor: CursorPos::new(l, 0),
			head: CursorPos::new(l, self.line_len(l)),
		};
	}

	fn set_head(&mut self, p: CursorPos, extend: bool) {
		if extend {
			self.selection.head = p;
		} else {
			self.selection = Selection::caret(p);
		}
		self.refresh_bracket_match();
	}

	pub fn click_to_pos(&self, line: usize, col: usize) -> CursorPos {
		self.clamp_pos(CursorPos::new(line, col))
	}

	// ── Word boundaries ───────────────────────────────────────────────────

	fn word_boundary_left(&self, p: CursorPos) -> CursorPos {
		if p.col == 0 {
			if p.line == 0 {
				return p;
			}
			let pl = p.line - 1;
			return CursorPos::new(pl, self.line_len(pl));
		}
		let chars: Vec<char> = self.line_text(p.line).chars().collect();
		let mut c = p.col.min(chars.len());
		let is_w = |ch: char| ch.is_alphanumeric() || ch == '_';
		while c > 0 && chars[c - 1].is_whitespace() {
			c -= 1;
		}
		if c > 0 && is_w(chars[c - 1]) {
			while c > 0 && is_w(chars[c - 1]) {
				c -= 1;
			}
		} else if c > 0 {
			c -= 1;
		}
		CursorPos::new(p.line, c)
	}

	fn word_boundary_right(&self, p: CursorPos) -> CursorPos {
		let ll = self.line_len(p.line);
		if p.col >= ll {
			if p.line >= self.line_count() - 1 {
				return p;
			}
			return CursorPos::new(p.line + 1, 0);
		}
		let chars: Vec<char> = self.line_text(p.line).chars().collect();
		let mut c = p.col;
		let is_w = |ch: char| ch.is_alphanumeric() || ch == '_';
		if c < chars.len() && is_w(chars[c]) {
			while c < chars.len() && is_w(chars[c]) {
				c += 1;
			}
		} else if c < chars.len() && !chars[c].is_whitespace() {
			c += 1;
		}
		while c < chars.len() && chars[c].is_whitespace() {
			c += 1;
		}
		CursorPos::new(p.line, c)
	}

	// ── Bracket matching ──────────────────────────────────────────────────

	fn update_bracket_match(&mut self) {
		self.matched_bracket = None;
		let p = self.selection.head;
		let text = self.line_text(p.line);
		let chars: Vec<char> = text.chars().collect();
		for &col in &[p.col, p.col.wrapping_sub(1)] {
			if col < chars.len() {
				let ch = chars[col];
				if is_open_bracket(ch) {
					if let Some((ml, mc)) = self.find_close(p.line, col, ch) {
						self.matched_bracket = Some(BracketPair {
							open_line: p.line,
							open_col: col,
							close_line: ml,
							close_col: mc,
						});
						return;
					}
				} else if is_close_bracket(ch) {
					if let Some((ml, mc)) = self.find_open(p.line, col, ch) {
						self.matched_bracket = Some(BracketPair {
							open_line: ml,
							open_col: mc,
							close_line: p.line,
							close_col: col,
						});
						return;
					}
				}
			}
		}
	}

	fn find_close(&self, sl: usize, sc: usize, open: char) -> Option<(usize, usize)> {
		let close = matching_close(open)?;
		let mut d = 0i32;
		for l in sl..self.line_count() {
			let cs: Vec<char> = self.line_text(l).chars().collect();
			for c in (if l == sl { sc } else { 0 })..cs.len() {
				if cs[c] == open {
					d += 1;
				} else if cs[c] == close {
					d -= 1;
					if d == 0 {
						return Some((l, c));
					}
				}
			}
		}
		None
	}

	fn find_open(&self, sl: usize, sc: usize, close: char) -> Option<(usize, usize)> {
		let open = matching_open(close)?;
		let mut d = 0i32;
		for l in (0..=sl).rev() {
			let cs: Vec<char> = self.line_text(l).chars().collect();
			let end = if l == sl {
				sc
			} else {
				cs.len().saturating_sub(1)
			};
			for c in (0..=end).rev() {
				if c >= cs.len() {
					continue;
				}
				if cs[c] == close {
					d += 1;
				} else if cs[c] == open {
					d -= 1;
					if d == 0 {
						return Some((l, c));
					}
				}
			}
		}
		None
	}

	// ── Diagnostics ───────────────────────────────────────────────────────

	fn collect_diagnostics(&mut self) {
		self.diagnostics.clear();
		match self.highlighter.language {
			SyntaxLanguage::Rust => {
				let tree = self.highlighter.tree().cloned();
				if let Some(tree) = tree {
					self.walk_errors(tree.root_node());
				}
			}
			SyntaxLanguage::Sql => self.collect_sql_diagnostics(),
		}
	}

	/// Lightweight structural diagnostics for the manual SQL token stream.
	/// Detects: misspelled SQL keywords (via edit distance), unmatched `)`, unclosed `(`.
	fn collect_sql_diagnostics(&mut self) {
		let tokens = self.highlighter.tokens.clone();
		let text = self.rope.to_string();
		let mut paren_stack: Vec<(usize, usize)> = Vec::new();
		let mut at_stmt_start = true;

		for tok in &tokens {
			if tok.byte_range.start >= text.len() {
				continue;
			}
			let slice = match text.get(tok.byte_range.clone()) {
				Some(s) => s,
				None => continue,
			};
			match tok.kind {
				// Comments are transparent — don't affect statement state.
				TokenKind::Comment => {}
				TokenKind::Punctuation => match slice {
					"(" => {
						let (line, col) = document::byte_to_char_col(&self.rope, tok.byte_range.start);
						paren_stack.push((line, col));
						at_stmt_start = false;
					}
					")" => {
						at_stmt_start = false;
						if paren_stack.pop().is_none() {
							let (line, col) =
								document::byte_to_char_col(&self.rope, tok.byte_range.start);
							self.diagnostics.push(Diagnostic {
								line,
								col_start: col,
								col_end: col + 1,
								message: "Unmatched `)`".into(),
							});
						}
					}
					";" => at_stmt_start = true,
					_ => at_stmt_start = false,
				},
				TokenKind::Keyword => at_stmt_start = false,
				TokenKind::Identifier if at_stmt_start => {
					// First word of a statement is not a recognized keyword.
					let (line, col) = document::byte_to_char_col(&self.rope, tok.byte_range.start);
					let msg = match sql_keyword_near_miss(slice) {
						Some(kw) => format!(
							"Unrecognized SQL command `{}`, did you mean `{}`?",
							slice, kw
						),
						None => format!("Unrecognized SQL command `{}`", slice),
					};
					self.diagnostics.push(Diagnostic {
						line,
						col_start: col,
						col_end: col + slice.chars().count(),
						message: msg,
					});
					at_stmt_start = false;
				}
				TokenKind::Identifier => {
					// Mid-statement: flag all-uppercase identifiers that look like
					// mistyped SQL keywords (edit distance ≤ 1, including transpositions).
					if let Some(kw) = sql_keyword_near_miss(slice) {
						let (line, col) = document::byte_to_char_col(&self.rope, tok.byte_range.start);
						self.diagnostics.push(Diagnostic {
							line,
							col_start: col,
							col_end: col + slice.chars().count(),
							message: format!("Did you mean `{}`?", kw),
						});
					}
					at_stmt_start = false;
				}
				_ => at_stmt_start = false,
			}
		}

		for (line, col) in paren_stack {
			self.diagnostics.push(Diagnostic {
				line,
				col_start: col,
				col_end: col + 1,
				message: "Unclosed `(`".into(),
			});
		}
	}

	fn walk_errors<'t>(&mut self, node: tree_sitter::Node<'t>) {
		if node.is_error() || node.is_missing() {
			let start =
				document::point_to_char_pos(&self.rope, node.start_position(), |line| self.line_text(line));
			let end =
				document::point_to_char_pos(&self.rope, node.end_position(), |line| self.line_text(line));
			let snippet = if start.line < self.line_count() {
				let end_col = if start.line == end.line {
					end.col
				} else {
					self.line_len(start.line)
				};
				let snippet = self.line_slice(start.line, start.col, end_col);
				if snippet.is_empty() {
					format!("`{}`", node.kind())
				} else {
					format!("`{}`", snippet)
				}
			} else {
				format!("`{}`", node.kind())
			};
			let msg = if node.is_missing() {
				format!("Missing token near {}", snippet)
			} else {
				format!("Unexpected {}", snippet)
			};
			self.diagnostics.push(Diagnostic {
				line: start.line,
				col_start: start.col,
				col_end: if start.line == end.line {
					end.col.max(start.col + 1)
				} else {
					self.line_len(start.line).max(start.col + 1)
				},
				message: msg,
			});
		}
		for i in 0..node.child_count() {
			if let Some(c) = node.child(i as u32) {
				self.walk_errors(c);
			}
		}
	}

	// ── Indent guides ─────────────────────────────────────────────────────

	/// Returns visual column positions of indent guides for this line.
	pub fn indent_guides(&self, line: usize) -> Vec<usize> {
		let text = self.line_text(line);
		// Count leading whitespace in visual columns (tabs = TAB_WIDTH, spaces = 1).
		let mut vcol = 0usize;
		for ch in text.chars() {
			match ch {
				'\t' => vcol = (vcol / TAB_WIDTH + 1) * TAB_WIDTH,
				' ' => vcol += 1,
				_ => break,
			}
		}
		let mut g = Vec::new();
		let mut c = TAB_WIDTH;
		while c <= vcol {
			g.push(c);
			c += TAB_WIDTH;
		}
		g
	}

	// ── Vim :substitute ───────────────────────────────────────────────────

	/// Apply a vim-style substitution to lines `first..=last`.
	/// `pattern` is a Rust regex. `replacement` supports vim escapes:
	/// `&` = whole match, `\1`–`\9` = capture groups, `\t` = tab, `\n` = newline, `\\` = backslash.
	/// Returns the number of lines changed.
	pub fn substitute(
		&mut self,
		first: usize,
		last: usize,
		pattern: &str,
		replacement: &str,
		global: bool,
		case_insensitive: bool,
	) -> usize {
		let re = match RegexBuilder::new(pattern)
			.case_insensitive(case_insensitive)
			.build()
		{
			Ok(r) => r,
			Err(_) => return 0,
		};

		let rep = replacement.to_string();
		let last = last.min(self.line_count().saturating_sub(1));

		self.save_undo(EditKind::Other);

		let mut changed = 0usize;
		// Process bottom-to-top so rope char indices above stay valid.
		for line in (first..=last).rev() {
			let text = self.line_text(line);
			let new_text = if global {
				re.replace_all(&text, |caps: &Captures| apply_vim_replacement(&rep, caps))
					.into_owned()
			} else {
				re.replace(&text, |caps: &Captures| apply_vim_replacement(&rep, caps))
					.into_owned()
			};
			if new_text == text {
				continue;
			}

			// Splice just the content portion of the line (leave the newline).
			let line_start = self.rope.line_to_char(line);
			let content_end = line_start + self.line_text(line).chars().count();
			self.replace_range(line_start, content_end, &new_text);
			changed += 1;
		}

		if changed > 0 {
			self.post_edit();
		}
		changed
	}
}

// ── Vim replacement helper ─────────────────────────────────────────────────────

fn apply_vim_replacement(rep: &str, caps: &Captures) -> String {
	let mut out = String::new();
	let mut chars = rep.chars().peekable();
	while let Some(ch) = chars.next() {
		match ch {
			'\\' => match chars.next() {
				Some('t') => out.push('\t'),
				Some('n') => out.push('\n'),
				Some('\\') => out.push('\\'),
				Some(d) if d.is_ascii_digit() => {
					let idx = d.to_digit(10).unwrap() as usize;
					out.push_str(caps.get(idx).map_or("", |m| m.as_str()));
				}
				Some(c) => {
					out.push('\\');
					out.push(c);
				}
				None => out.push('\\'),
			},
			'&' => out.push_str(caps.get(0).map_or("", |m| m.as_str())),
			c => out.push(c),
		}
	}
	out
}

// ── SQL keyword typo detection ─────────────────────────────────────────────────

/// Returns the closest SQL keyword if `word` is an all-uppercase identifier
/// within edit distance 1 (including transpositions) of a known keyword.
/// Only words of 3+ characters are checked to avoid noisy short-word matches.
fn sql_keyword_near_miss(word: &str) -> Option<&'static str> {
	if word.len() < 3 || !word.bytes().all(|b| b.is_ascii_uppercase()) {
		return None;
	}
	const KEYWORDS: &[&str] = &[
		"SELECT",
		"FROM",
		"WHERE",
		"INSERT",
		"UPDATE",
		"DELETE",
		"CREATE",
		"DROP",
		"ALTER",
		"TABLE",
		"INDEX",
		"INTO",
		"VALUES",
		"JOIN",
		"LEFT",
		"RIGHT",
		"INNER",
		"OUTER",
		"CROSS",
		"FULL",
		"ORDER",
		"GROUP",
		"HAVING",
		"LIMIT",
		"OFFSET",
		"UNION",
		"EXCEPT",
		"INTERSECT",
		"DISTINCT",
		"EXISTS",
		"BETWEEN",
		"PARTITION",
		"OVER",
		"MATERIALIZED",
		"VIEW",
		"WITH",
		"RETURNING",
		"TRUNCATE",
		"VACUUM",
		"ANALYZE",
		"EXPLAIN",
		"COMMIT",
		"ROLLBACK",
		"BEGIN",
		"TRANSACTION",
		"GRANT",
		"REVOKE",
	];
	for &kw in KEYWORDS {
		if word == kw {
			return None;
		} // exact match → recognized, not an error
		if osa_distance(word.as_bytes(), kw.as_bytes()) == 1 {
			return Some(kw);
		}
	}
	None
}

/// Optimal String Alignment distance (edit distance + adjacent transpositions).
/// Transpositions count as 1 edit, so FORM↔FROM and WHER↔WHERE both score 1.
fn osa_distance(a: &[u8], b: &[u8]) -> usize {
	let (m, n) = (a.len(), b.len());
	// Length gap > 1 means distance ≥ 2; return early to skip the allocation.
	if m.abs_diff(n) > 1 {
		return m.abs_diff(n);
	}
	let mut dp = vec![vec![0usize; n + 1]; m + 1];
	for i in 0..=m {
		dp[i][0] = i;
	}
	for j in 0..=n {
		dp[0][j] = j;
	}
	for i in 1..=m {
		for j in 1..=n {
			let cost = (a[i - 1] != b[j - 1]) as usize;
			dp[i][j] = (dp[i - 1][j] + 1)
				.min(dp[i][j - 1] + 1)
				.min(dp[i - 1][j - 1] + cost);
			if i > 1 && j > 1 && a[i - 1] == b[j - 2] && a[i - 2] == b[j - 1] {
				dp[i][j] = dp[i][j].min(dp[i - 2][j - 2] + 1);
			}
		}
	}
	dp[m][n]
}
