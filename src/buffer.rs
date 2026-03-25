use ropey::Rope;
use crate::folding::FoldState;
use crate::highlight::{Highlighter, SyntaxLanguage, SyntaxToken};
use crate::search::SearchState;
use crate::wrap::{self, VisualLine, WrapConfig};

// ─── Diagnostic ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct Diagnostic {
    pub line: usize,
    pub col_start: usize,
    pub col_end: usize,
    pub message: String,
}

// ─── Bracket matching ─────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BracketPair {
    pub open_line: usize,
    pub open_col: usize,
    pub close_line: usize,
    pub close_col: usize,
}

// ─── Cursor & Selection ───────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CursorPos {
    pub line: usize,
    pub col: usize,
}

impl CursorPos {
    pub fn new(line: usize, col: usize) -> Self { Self { line, col } }
    pub fn zero() -> Self { Self { line: 0, col: 0 } }
}

impl PartialOrd for CursorPos {
    fn partial_cmp(&self, o: &Self) -> Option<std::cmp::Ordering> { Some(self.cmp(o)) }
}
impl Ord for CursorPos {
    fn cmp(&self, o: &Self) -> std::cmp::Ordering {
        self.line.cmp(&o.line).then(self.col.cmp(&o.col))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Selection {
    pub anchor: CursorPos,
    pub head: CursorPos,
}

impl Selection {
    pub fn caret(p: CursorPos) -> Self { Self { anchor: p, head: p } }
    pub fn is_caret(&self) -> bool { self.anchor == self.head }
    pub fn ordered(&self) -> (CursorPos, CursorPos) {
        if self.anchor <= self.head { (self.anchor, self.head) } else { (self.head, self.anchor) }
    }
}

// ─── Undo with smart command grouping ─────────────────────────────────────────

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum EditKind {
    Insert,
    Delete,
    Newline,
    Paste,
    Other,
}

#[derive(Clone)]
struct Snapshot {
    text: String,
    selection: Selection,
    kind: EditKind,
    timestamp_ms: u64,
}

/// Configurable undo history.
pub struct UndoConfig {
    /// Max number of undo entries.
    pub max_history: usize,
    /// Consecutive edits of the same kind within this window (ms) are grouped.
    pub group_timeout_ms: u64,
}

impl Default for UndoConfig {
    fn default() -> Self {
        Self {
            max_history: 500,
            group_timeout_ms: 800,
        }
    }
}

struct UndoStack {
    history: Vec<Snapshot>,
    index: usize,
    config: UndoConfig,
}

impl UndoStack {
    fn new(config: UndoConfig) -> Self {
        Self { history: Vec::new(), index: 0, config }
    }

    fn now_ms() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64
    }

    fn push(&mut self, text: String, selection: Selection, kind: EditKind) {
        let now = Self::now_ms();

        // Smart grouping: if the last entry is the same kind and within timeout,
        // replace it instead of adding a new entry.
        if self.index > 0 && !self.history.is_empty() {
            let last = &self.history[self.index - 1];
            if last.kind == kind
                && kind == EditKind::Insert
                && now - last.timestamp_ms < self.config.group_timeout_ms
            {
                // Update the entry at current position instead of pushing
                // (we keep the old anchor text but update the head text)
                // Actually for grouping we want the *pre-edit* state of the group,
                // so we just skip saving.
                return;
            }
        }

        self.history.truncate(self.index);
        self.history.push(Snapshot {
            text,
            selection,
            kind,
            timestamp_ms: now,
        });
        if self.history.len() > self.config.max_history {
            self.history.remove(0);
        }
        self.index = self.history.len();
    }

    /// Force a new undo boundary regardless of grouping.
    fn force_boundary(&mut self, text: String, selection: Selection) {
        self.history.truncate(self.index);
        self.history.push(Snapshot {
            text,
            selection,
            kind: EditKind::Other,
            timestamp_ms: Self::now_ms(),
        });
        if self.history.len() > self.config.max_history {
            self.history.remove(0);
        }
        self.index = self.history.len();
    }

    fn undo(&mut self) -> Option<&Snapshot> {
        if self.index > 0 {
            self.index -= 1;
            Some(&self.history[self.index])
        } else {
            None
        }
    }

    fn redo(&mut self) -> Option<&Snapshot> {
        if self.index < self.history.len() {
            let snap = &self.history[self.index];
            self.index += 1;
            Some(snap)
        } else {
            None
        }
    }
}

// ─── Auto-pairs ───────────────────────────────────────────────────────────────

fn matching_close(c: char) -> Option<char> {
    match c { '(' => Some(')'), '[' => Some(']'), '{' => Some('}'), '\'' => Some('\''), '"' => Some('"'), _ => None }
}
fn is_open_bracket(c: char) -> bool { matches!(c, '(' | '[' | '{') }
fn is_close_bracket(c: char) -> bool { matches!(c, ')' | ']' | '}') }
fn matching_open(c: char) -> Option<char> {
    match c { ')' => Some('('), ']' => Some('['), '}' => Some('{'), _ => None }
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
        let mut undo = UndoStack::new(undo_config);
        undo.force_boundary(text.to_string(), sel);

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
        };
        buf.post_edit();
        buf
    }

    // ── Accessors ─────────────────────────────────────────────────────────

    pub fn tokens(&self) -> &[SyntaxToken] { &self.highlighter.tokens }
    pub fn language(&self) -> SyntaxLanguage { self.highlighter.language }

    pub fn set_language(&mut self, lang: SyntaxLanguage) {
        self.highlighter = Highlighter::new(lang);
        let text = self.rope.to_string();
        self.highlighter.parse(&text);
        self.post_edit();
    }

    pub fn line_count(&self) -> usize { self.rope.len_lines().max(1) }

    pub fn line_len(&self, line: usize) -> usize {
        if line >= self.rope.len_lines() { return 0; }
        let s: String = self.rope.line(line).chars().collect();
        s.trim_end_matches('\n').trim_end_matches('\r').len()
    }

    pub fn clamp_pos(&self, p: CursorPos) -> CursorPos {
        let l = p.line.min(self.line_count().saturating_sub(1));
        CursorPos::new(l, p.col.min(self.line_len(l)))
    }

    fn pos_to_char(&self, p: CursorPos) -> usize {
        let c = self.clamp_pos(p);
        self.rope.line_to_char(c.line) + c.col
    }

    pub fn line_text(&self, line: usize) -> String {
        if line >= self.rope.len_lines() { return String::new(); }
        let s: String = self.rope.line(line).chars().collect();
        s.trim_end_matches('\n').trim_end_matches('\r').to_string()
    }

    pub fn full_text(&self) -> String { self.rope.to_string() }

    pub fn selected_text(&self) -> String {
        if self.selection.is_caret() { return String::new(); }
        let (s, e) = self.selection.ordered();
        self.rope.slice(self.pos_to_char(s)..self.pos_to_char(e)).to_string()
    }

    fn line_indent(&self, line: usize) -> String {
        self.line_text(line).chars().take_while(|c| c.is_whitespace()).collect()
    }

    fn char_at(&self, p: CursorPos) -> Option<char> {
        self.line_text(self.clamp_pos(p).line).chars().nth(self.clamp_pos(p).col)
    }

    fn char_before(&self, p: CursorPos) -> Option<char> {
        if p.col == 0 { None } else { self.line_text(p.line).chars().nth(p.col - 1) }
    }

    // ── Post-edit refresh ─────────────────────────────────────────────────

    fn post_edit(&mut self) {
        let text = self.rope.to_string();
        self.highlighter.parse(&text);
        self.collect_diagnostics();
        self.update_bracket_match();
        let line_count = self.line_count();
        let lines: Vec<String> = (0..line_count).map(|l| self.line_text(l)).collect();
        let tree = self.highlighter.tree().cloned();
        self.folds.detect_regions(
            tree.as_ref(),
            line_count,
            &|l| lines[l].clone(),
        );
        self.recompute_visual_lines();
        if self.search.is_open {
            self.search.find_all(&self.rope);
        }
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
        self.undo_stack.push(self.rope.to_string(), self.selection, kind);
    }

    fn save_undo_boundary(&mut self) {
        self.undo_stack.force_boundary(self.rope.to_string(), self.selection);
    }

    pub fn undo(&mut self) {
        if let Some(snap) = self.undo_stack.undo().cloned() {
            self.rope = Rope::from_str(&snap.text);
            self.selection = snap.selection;
            self.post_edit();
        }
    }

    pub fn redo(&mut self) {
        if let Some(snap) = self.undo_stack.redo().cloned() {
            self.rope = Rope::from_str(&snap.text);
            self.selection = snap.selection;
            self.post_edit();
        }
    }

    // ── Clipboard ─────────────────────────────────────────────────────────

    pub fn copy(&mut self) -> String {
        let text = self.selected_text();
        if !text.is_empty() {
            self.clipboard = text.clone();
        }
        text
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
        if text.is_empty() { return; }
        self.save_undo(EditKind::Paste);
        self.delete_selection_inner();
        self.desired_col = None;
        let pos = self.selection.head;
        let ci = self.pos_to_char(pos);
        self.rope.insert(ci, text);

        let newlines = text.chars().filter(|c| *c == '\n').count();
        let new_pos = if newlines > 0 {
            let after = &text[text.rfind('\n').unwrap() + 1..];
            CursorPos::new(pos.line + newlines, after.len())
        } else {
            CursorPos::new(pos.line, pos.col + text.len())
        };
        self.selection = Selection::caret(new_pos);
        self.post_edit();
    }

    // ── Editing ───────────────────────────────────────────────────────────

    pub fn insert_char(&mut self, ch: char) {
        self.save_undo(EditKind::Insert);
        self.delete_selection_inner();
        self.desired_col = None;
        let pos = self.selection.head;
        let ci = self.pos_to_char(pos);
        self.rope.insert_char(ci, ch);
        let new = if ch == '\n' {
            CursorPos::new(pos.line + 1, 0)
        } else {
            CursorPos::new(pos.line, pos.col + ch.len_utf8())
        };
        self.selection = Selection::caret(new);
        self.post_edit();
    }

    pub fn insert_str(&mut self, text: &str) {
        if text.is_empty() { return; }
        self.save_undo(EditKind::Insert);
        self.delete_selection_inner();
        self.desired_col = None;
        let pos = self.selection.head;
        let ci = self.pos_to_char(pos);
        self.rope.insert(ci, text);
        let newlines = text.chars().filter(|c| *c == '\n').count();
        let new = if newlines > 0 {
            let after = &text[text.rfind('\n').unwrap() + 1..];
            CursorPos::new(pos.line + newlines, after.len())
        } else {
            CursorPos::new(pos.line, pos.col + text.len())
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
                if trimmed.ends_with('(') || trimmed.ends_with('{') || trimmed.ends_with('[')
                    || trimmed.to_uppercase().ends_with(" AS")
                    || trimmed.to_uppercase().ends_with(" BEGIN")
                    || trimmed.to_uppercase().ends_with(" THEN")
                { "    " } else { "" }
            }
            SyntaxLanguage::Rust => {
                if trimmed.ends_with('{') || trimmed.ends_with('(') || trimmed.ends_with('[')
                    || trimmed.ends_with("=>")
                { "    " } else { "" }
            }
        };

        let ins = format!("\n{}{}", indent, extra);
        let ci = self.pos_to_char(pos);
        self.rope.insert(ci, &ins);
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
                self.update_bracket_match();
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
            self.rope.insert(ci, &format!("{}{}", ch, close));
            self.selection = Selection::caret(CursorPos::new(p.line, p.col + ch.len_utf8()));
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
        if p.line == 0 && p.col == 0 { return; }
        self.save_undo(EditKind::Delete);

        // Auto-pair removal
        if p.col > 0 {
            if let Some(prev) = self.char_before(p) {
                if let Some(exp) = matching_close(prev) {
                    if self.char_at(p) == Some(exp) {
                        let cs = self.pos_to_char(CursorPos::new(p.line, p.col - 1));
                        self.rope.remove(cs..cs + 2);
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
        self.rope.remove(ds..de);
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
        if ci >= self.rope.len_chars() { return; }
        self.save_undo(EditKind::Delete);
        self.rope.remove(ci..ci + 1);
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
        if p.line == 0 && p.col == 0 { return; }
        self.save_undo(EditKind::Delete);
        let t = self.word_boundary_left(p);
        self.rope.remove(self.pos_to_char(t)..self.pos_to_char(p));
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
        if self.pos_to_char(p) >= self.rope.len_chars() { return; }
        self.save_undo(EditKind::Delete);
        let t = self.word_boundary_right(p);
        self.rope.remove(self.pos_to_char(p)..self.pos_to_char(t));
        self.post_edit();
    }

    pub fn duplicate_line(&mut self) {
        self.save_undo_boundary();
        let l = self.selection.head.line;
        let t = self.line_text(l);
        let ls = self.rope.line_to_char(l);
        let lc = self.rope.line(l).len_chars();
        let at = ls + lc;
        let ins = if at >= self.rope.len_chars() { format!("\n{}", t) } else { format!("{}\n", t) };
        self.rope.insert(at, &ins);
        self.selection = Selection::caret(CursorPos::new(l + 1, self.selection.head.col));
        self.post_edit();
    }

    fn delete_selection_inner(&mut self) {
        if self.selection.is_caret() { return; }
        let (s, e) = self.selection.ordered();
        self.rope.remove(self.pos_to_char(s)..self.pos_to_char(e));
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
        if self.search.replace_current(&mut self.rope).is_some() {
            self.post_edit();
        }
    }

    pub fn search_replace_all(&mut self) {
        self.save_undo_boundary();
        if self.search.replace_all(&mut self.rope) > 0 {
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

    // ── Folding ───────────────────────────────────────────────────────────

    pub fn toggle_fold(&mut self, line: usize) {
        self.folds.toggle(line);
        self.recompute_visual_lines();
    }

    // ── Wrapping ──────────────────────────────────────────────────────────

    pub fn set_wrap(&mut self, enabled: bool) {
        self.wrap_config.enabled = enabled;
        self.recompute_visual_lines();
    }

    pub fn set_wrap_col(&mut self, col: usize) {
        self.wrap_config.wrap_col = col;
        if self.wrap_config.enabled {
            self.recompute_visual_lines();
        }
    }

    // ── Navigation ────────────────────────────────────────────────────────

    pub fn move_left(&mut self, extend: bool) {
        self.desired_col = None;
        if !extend && !self.selection.is_caret() {
            let (s, _) = self.selection.ordered();
            self.selection = Selection::caret(s);
            self.update_bracket_match();
            return;
        }
        let p = self.selection.head;
        let n = if p.col > 0 {
            CursorPos::new(p.line, p.col - 1)
        } else if p.line > 0 {
            CursorPos::new(p.line - 1, self.line_len(p.line - 1))
        } else { p };
        self.set_head(n, extend);
    }

    pub fn move_right(&mut self, extend: bool) {
        self.desired_col = None;
        if !extend && !self.selection.is_caret() {
            let (_, e) = self.selection.ordered();
            self.selection = Selection::caret(e);
            self.update_bracket_match();
            return;
        }
        let p = self.selection.head;
        let ll = self.line_len(p.line);
        let n = if p.col < ll {
            CursorPos::new(p.line, p.col + 1)
        } else if p.line < self.line_count() - 1 {
            CursorPos::new(p.line + 1, 0)
        } else { p };
        self.set_head(n, extend);
    }

    pub fn move_up(&mut self, extend: bool) {
        let p = self.selection.head;
        if p.line == 0 { return; }
        let tc = self.desired_col.unwrap_or(p.col);
        // Skip folded lines
        let mut nl = p.line - 1;
        while nl > 0 && self.folds.is_hidden(nl) { nl -= 1; }
        let nc = tc.min(self.line_len(nl));
        self.set_head(CursorPos::new(nl, nc), extend);
        self.desired_col = Some(tc);
    }

    pub fn move_down(&mut self, extend: bool) {
        let p = self.selection.head;
        if p.line >= self.line_count() - 1 { return; }
        let tc = self.desired_col.unwrap_or(p.col);
        let mut nl = p.line + 1;
        let max = self.line_count() - 1;
        while nl < max && self.folds.is_hidden(nl) { nl += 1; }
        let nc = tc.min(self.line_len(nl));
        self.set_head(CursorPos::new(nl, nc), extend);
        self.desired_col = Some(tc);
    }

    pub fn move_home(&mut self, extend: bool) {
        self.desired_col = None;
        let p = self.selection.head;
        let first = self.line_text(p.line).chars().position(|c| !c.is_whitespace()).unwrap_or(0);
        let nc = if p.col <= first && p.col != 0 { 0 } else { first };
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
        self.selection = Selection { anchor: CursorPos::zero(), head: CursorPos::new(l, self.line_len(l)) };
    }

    pub fn select_word_at(&mut self, p: CursorPos) {
        let p = self.clamp_pos(p);
        let text = self.line_text(p.line);
        if text.is_empty() { self.selection = Selection::caret(p); return; }
        let chars: Vec<char> = text.chars().collect();
        let col = p.col.min(chars.len().saturating_sub(1));
        let is_w = |c: char| c.is_alphanumeric() || c == '_';
        if !is_w(chars[col]) {
            self.selection = Selection { anchor: CursorPos::new(p.line, col), head: CursorPos::new(p.line, col + 1) };
            return;
        }
        let mut s = col;
        while s > 0 && is_w(chars[s - 1]) { s -= 1; }
        let mut e = col;
        while e < chars.len() && is_w(chars[e]) { e += 1; }
        self.selection = Selection { anchor: CursorPos::new(p.line, s), head: CursorPos::new(p.line, e) };
    }

    pub fn select_line(&mut self, line: usize) {
        let l = line.min(self.line_count().saturating_sub(1));
        self.selection = Selection { anchor: CursorPos::new(l, 0), head: CursorPos::new(l, self.line_len(l)) };
    }

    fn set_head(&mut self, p: CursorPos, extend: bool) {
        if extend { self.selection.head = p; } else { self.selection = Selection::caret(p); }
        self.update_bracket_match();
    }

    pub fn click_to_pos(&self, line: usize, col: usize) -> CursorPos {
        self.clamp_pos(CursorPos::new(line, col))
    }

    // ── Word boundaries ───────────────────────────────────────────────────

    fn word_boundary_left(&self, p: CursorPos) -> CursorPos {
        if p.col == 0 {
            if p.line == 0 { return p; }
            let pl = p.line - 1;
            return CursorPos::new(pl, self.line_len(pl));
        }
        let chars: Vec<char> = self.line_text(p.line).chars().collect();
        let mut c = p.col.min(chars.len());
        let is_w = |ch: char| ch.is_alphanumeric() || ch == '_';
        while c > 0 && chars[c - 1].is_whitespace() { c -= 1; }
        if c > 0 && is_w(chars[c - 1]) {
            while c > 0 && is_w(chars[c - 1]) { c -= 1; }
        } else if c > 0 { c -= 1; }
        CursorPos::new(p.line, c)
    }

    fn word_boundary_right(&self, p: CursorPos) -> CursorPos {
        let ll = self.line_len(p.line);
        if p.col >= ll {
            if p.line >= self.line_count() - 1 { return p; }
            return CursorPos::new(p.line + 1, 0);
        }
        let chars: Vec<char> = self.line_text(p.line).chars().collect();
        let mut c = p.col;
        let is_w = |ch: char| ch.is_alphanumeric() || ch == '_';
        if c < chars.len() && is_w(chars[c]) {
            while c < chars.len() && is_w(chars[c]) { c += 1; }
        } else if c < chars.len() && !chars[c].is_whitespace() { c += 1; }
        while c < chars.len() && chars[c].is_whitespace() { c += 1; }
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
                        self.matched_bracket = Some(BracketPair { open_line: p.line, open_col: col, close_line: ml, close_col: mc });
                        return;
                    }
                } else if is_close_bracket(ch) {
                    if let Some((ml, mc)) = self.find_open(p.line, col, ch) {
                        self.matched_bracket = Some(BracketPair { open_line: ml, open_col: mc, close_line: p.line, close_col: col });
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
                if cs[c] == open { d += 1; } else if cs[c] == close { d -= 1; if d == 0 { return Some((l, c)); } }
            }
        }
        None
    }

    fn find_open(&self, sl: usize, sc: usize, close: char) -> Option<(usize, usize)> {
        let open = matching_open(close)?;
        let mut d = 0i32;
        for l in (0..=sl).rev() {
            let cs: Vec<char> = self.line_text(l).chars().collect();
            let end = if l == sl { sc } else { cs.len().saturating_sub(1) };
            for c in (0..=end).rev() {
                if c >= cs.len() { continue; }
                if cs[c] == close { d += 1; } else if cs[c] == open { d -= 1; if d == 0 { return Some((l, c)); } }
            }
        }
        None
    }

    // ── Diagnostics ───────────────────────────────────────────────────────

    fn collect_diagnostics(&mut self) {
        self.diagnostics.clear();
        let tree = self.highlighter.tree().cloned();
        if let Some(tree) = tree {
            self.walk_errors(tree.root_node());
        }
    }

    fn walk_errors<'t>(&mut self, node: tree_sitter::Node<'t>) {
        if node.is_error() || node.is_missing() {
            let s = node.start_position();
            let e = node.end_position();
            let snippet = if s.row < self.line_count() {
                let lt = self.line_text(s.row);
                let a = s.column.min(lt.len());
                let b = if s.row == e.row { e.column.min(lt.len()) } else { lt.len() };
                if a < b { format!("`{}`", &lt[a..b]) } else { format!("`{}`", node.kind()) }
            } else { format!("`{}`", node.kind()) };
            let msg = if node.is_missing() { format!("Missing token near {}", snippet) } else { format!("Unexpected {}", snippet) };
            self.diagnostics.push(Diagnostic {
                line: s.row,
                col_start: s.column,
                col_end: if s.row == e.row { e.column.max(s.column + 1) } else { self.line_len(s.row).max(s.column + 1) },
                message: msg,
            });
        }
        for i in 0..node.child_count() {
            if let Some(c) = node.child(i as u32) { self.walk_errors(c); }
        }
    }

    // ── Indent guides ─────────────────────────────────────────────────────

    pub fn indent_guides(&self, line: usize) -> Vec<usize> {
        let spaces = self.line_text(line).chars().take_while(|c| *c == ' ').count();
        let mut g = Vec::new();
        let mut c = 4;
        while c <= spaces { g.push(c); c += 4; }
        g
    }
}
