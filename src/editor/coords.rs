use std::cmp::Ordering;

pub const TAB_WIDTH: usize = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CursorPos {
	pub line: usize,
	pub col: usize,
}

impl CursorPos {
	pub fn new(line: usize, col: usize) -> Self {
		Self { line, col }
	}

	pub fn zero() -> Self {
		Self { line: 0, col: 0 }
	}
}

impl PartialOrd for CursorPos {
	fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
		Some(self.cmp(other))
	}
}

impl Ord for CursorPos {
	fn cmp(&self, other: &Self) -> Ordering {
		self.line.cmp(&other.line).then(self.col.cmp(&other.col))
	}
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Selection {
	pub anchor: CursorPos,
	pub head: CursorPos,
}

impl Selection {
	pub fn caret(p: CursorPos) -> Self {
		Self { anchor: p, head: p }
	}

	pub fn is_caret(&self) -> bool {
		self.anchor == self.head
	}

	pub fn ordered(&self) -> (CursorPos, CursorPos) {
		if self.anchor <= self.head {
			(self.anchor, self.head)
		} else {
			(self.head, self.anchor)
		}
	}
}

pub mod line {
	use super::TAB_WIDTH;

	/// Logical col -> visual col. Tabs expand to the next TAB_WIDTH boundary.
	pub fn visual_col_of(line: &str, logical_col: usize) -> usize {
		let mut vcol = 0usize;
		for (i, ch) in line.chars().enumerate() {
			if i >= logical_col {
				break;
			}
			if ch == '\t' {
				vcol = (vcol / TAB_WIDTH + 1) * TAB_WIDTH;
			} else {
				vcol += 1;
			}
		}
		vcol
	}

	/// Visual col -> logical col. Snaps to the nearest character boundary.
	pub fn logical_col_of(line: &str, target_vcol: usize) -> usize {
		let mut vcol = 0usize;
		for (i, ch) in line.chars().enumerate() {
			if vcol >= target_vcol {
				return i;
			}
			if ch == '\t' {
				let next = (vcol / TAB_WIDTH + 1) * TAB_WIDTH;
				if target_vcol < next {
					return i;
				}
				vcol = next;
			} else {
				vcol += 1;
			}
		}
		line.chars().count()
	}

	/// Iterate over line characters together with each character's starting visual column.
	pub fn chars_with_vcols(line: &str) -> impl Iterator<Item = (char, usize)> + '_ {
		let mut vcol = 0usize;
		line.chars().map(move |ch| {
			let start = vcol;
			vcol = if ch == '\t' {
				(vcol / TAB_WIDTH + 1) * TAB_WIDTH
			} else {
				vcol + 1
			};
			(ch, start)
		})
	}

	pub fn slice_chars(text: &str, start_col: usize, end_col: usize) -> String {
		text.chars()
			.skip(start_col)
			.take(end_col.saturating_sub(start_col))
			.collect()
	}

	pub fn char_to_byte_idx(text: &str, char_idx: usize) -> usize {
		text.char_indices()
			.nth(char_idx)
			.map(|(idx, _)| idx)
			.unwrap_or(text.len())
	}

	pub fn byte_to_char_idx(text: &str, byte_idx: usize) -> usize {
		let mut char_count = 0;
		for (i, _) in text.char_indices() {
			if i >= byte_idx {
				return char_count;
			}
			char_count += 1;
		}
		char_count
	}
}

pub mod document {
	use ropey::Rope;
	use tree_sitter::Point;

	use super::{CursorPos, line};

	pub fn clamp_pos(rope: &Rope, p: CursorPos) -> CursorPos {
		let line_count = rope.len_lines().max(1);
		let line = p.line.min(line_count.saturating_sub(1));
		let col = p.col.min(line_len(rope, line));
		CursorPos::new(line, col)
	}

	pub fn pos_to_char(rope: &Rope, p: CursorPos) -> usize {
		let clamped = clamp_pos(rope, p);
		rope.line_to_char(clamped.line) + clamped.col
	}

	pub fn byte_to_char_col(rope: &Rope, byte: usize) -> (usize, usize) {
		let char_idx = rope.byte_to_char(byte);
		let line = rope.char_to_line(char_idx);
		let col = char_idx - rope.line_to_char(line);
		(line, col)
	}

	pub fn point_to_char_pos<F>(rope: &Rope, point: Point, mut line_text: F) -> CursorPos
	where
		F: FnMut(usize) -> String,
	{
		let line_count = rope.len_lines().max(1);
		if point.row >= line_count {
			return CursorPos::new(line_count.saturating_sub(1), 0);
		}
		let line = point.row;
		let text = line_text(line);
		let byte_col = point.column.min(text.len());
		CursorPos::new(line, line::byte_to_char_idx(&text, byte_col))
	}

	fn line_len(rope: &Rope, line: usize) -> usize {
		if line >= rope.len_lines() {
			return 0;
		}
		let text: String = rope.line(line).chars().collect();
		text.trim_end_matches('\n')
			.trim_end_matches('\r')
			.chars()
			.count()
	}
}
