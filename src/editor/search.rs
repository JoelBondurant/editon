use ropey::Rope;

/// A single search match in the document.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SearchMatch {
	pub line: usize,
	pub col_start: usize,
	pub col_end: usize,
	/// Char index into the rope for the start of the match.
	pub char_start: usize,
	pub char_end: usize,
}

/// Search state, kept separate from the buffer so the widget can query it.
pub struct SearchState {
	pub query: String,
	pub replacement: String,
	pub matches: Vec<SearchMatch>,
	pub current_match: usize,
	pub case_sensitive: bool,
	pub is_open: bool,
}

impl SearchState {
	pub fn new() -> Self {
		Self {
			query: String::new(),
			replacement: String::new(),
			matches: Vec::new(),
			current_match: 0,
			case_sensitive: false,
			is_open: false,
		}
	}

	/// Recompute all matches against the given rope.
	pub fn find_all(&mut self, rope: &Rope) {
		self.matches.clear();
		if self.query.is_empty() {
			return;
		}

		let text = rope.to_string();
		let (haystack, needle);
		let text_lower;
		let query_lower;

		if self.case_sensitive {
			haystack = text.as_str();
			needle = self.query.as_str();
		} else {
			text_lower = text.to_lowercase();
			query_lower = self.query.to_lowercase();
			haystack = text_lower.as_str();
			needle = query_lower.as_str();
		}

		let mut byte_pos = 0;
		while let Some(rel) = haystack[byte_pos..].find(needle) {
			let match_byte_start = byte_pos + rel;
			let match_byte_end = match_byte_start + self.query.len();

			let char_start = rope.byte_to_char(match_byte_start);
			let char_end = rope.byte_to_char(match_byte_end);
			let line = rope.char_to_line(char_start);
			let line_char_start = rope.line_to_char(line);
			let col_start = char_start - line_char_start;
			let col_end = if rope.char_to_line(char_end) == line {
				char_end - line_char_start
			} else {
				col_start + self.query.len()
			};

			self.matches.push(SearchMatch {
				line,
				col_start,
				col_end,
				char_start,
				char_end,
			});

			byte_pos = match_byte_end;
		}

		// Clamp current match
		if !self.matches.is_empty() {
			self.current_match = self.current_match.min(self.matches.len() - 1);
		} else {
			self.current_match = 0;
		}
	}

	pub fn match_count(&self) -> usize {
		self.matches.len()
	}

	pub fn next_match(&mut self) {
		if !self.matches.is_empty() {
			self.current_match = (self.current_match + 1) % self.matches.len();
		}
	}

	pub fn prev_match(&mut self) {
		if !self.matches.is_empty() {
			self.current_match = if self.current_match == 0 {
				self.matches.len() - 1
			} else {
				self.current_match - 1
			};
		}
	}

	/// Find the nearest match at or after the given char index.
	pub fn jump_to_nearest(&mut self, char_idx: usize) {
		if self.matches.is_empty() {
			return;
		}
		for (i, m) in self.matches.iter().enumerate() {
			if m.char_start >= char_idx {
				self.current_match = i;
				return;
			}
		}
		self.current_match = 0; // wrap
	}

	pub fn current(&self) -> Option<&SearchMatch> {
		self.matches.get(self.current_match)
	}

	/// Replace the current match in-place in the rope. Returns the replacement
	/// length delta so the caller can adjust the cursor.
	pub fn replace_current(&mut self, rope: &mut Rope) -> Option<i64> {
		let m = self.matches.get(self.current_match)?.clone();
		rope.remove(m.char_start..m.char_end);
		rope.insert(m.char_start, &self.replacement);
		let delta = self.replacement.len() as i64 - (m.char_end - m.char_start) as i64;
		Some(delta)
	}

	/// Replace all matches. Returns count replaced.
	pub fn replace_all(&mut self, rope: &mut Rope) -> usize {
		let count = self.matches.len();
		// Replace from end to start so byte offsets stay valid
		for m in self.matches.iter().rev() {
			rope.remove(m.char_start..m.char_end);
			rope.insert(m.char_start, &self.replacement);
		}
		self.matches.clear();
		self.current_match = 0;
		count
	}
}
