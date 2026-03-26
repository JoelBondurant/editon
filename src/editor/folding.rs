use std::collections::BTreeMap;

use super::highlight::SyntaxLanguage;

/// A foldable region in the document.
#[derive(Debug, Clone)]
pub struct FoldRegion {
	pub start_line: usize,
	pub end_line: usize,
	pub kind: FoldKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FoldKind {
	Block,     // { ... }
	Paren,     // ( ... ) spanning multiple lines
	Comment,   // multi-line comments
	Indent,    // indentation-based (for SQL subqueries etc.)
	Statement, // top-level SQL statement (SELECT…, CREATE…, etc.)
}

/// Top-level SQL keywords that begin a foldable statement.
const SQL_STATEMENT_KEYWORDS: &[&str] = &[
	"SELECT", "INSERT", "UPDATE", "DELETE", "CREATE", "DROP", "ALTER",
	"WITH", "MERGE", "TRUNCATE", "GRANT", "REVOKE", "EXPLAIN",
];

/// Manages fold state for the editor.
pub struct FoldState {
	/// Detected foldable regions (start_line → region).
	pub regions: BTreeMap<usize, FoldRegion>,
	/// Lines that are currently collapsed (start_line of the fold).
	pub collapsed: BTreeMap<usize, usize>, // start_line → end_line
}

impl FoldState {
	pub fn new() -> Self {
		Self {
			regions: BTreeMap::new(),
			collapsed: BTreeMap::new(),
		}
	}

	/// Detect foldable regions from the tree-sitter tree and line text.
	pub fn detect_regions(
		&mut self,
		tree: Option<&tree_sitter::Tree>,
		language: SyntaxLanguage,
		line_count: usize,
		line_text: &dyn Fn(usize) -> String,
	) {
		self.regions.clear();

		// SQL: statement-level folds take priority; run first so or_insert
		// below won't overwrite them.
		if language == SyntaxLanguage::Sql {
			self.detect_sql_statement_folds(line_count, line_text);
		}

		// Tree-sitter based: walk for multi-line nodes (Rust blocks, etc.)
		if let Some(tree) = tree {
			self.walk_node(tree.root_node());
		}

		// Indentation-based: sub-blocks within statements
		self.detect_indent_folds(line_count, line_text);

		// Remove any collapsed regions that no longer exist
		self.collapsed
			.retain(|start, _| self.regions.contains_key(start));
	}

	/// Detect top-level SQL statements as foldable regions.
	/// A statement starts at a line whose first non-whitespace token is a SQL
	/// keyword and ends at the line containing the closing `;`, or just before
	/// the next statement keyword / end of file.
	fn detect_sql_statement_folds(
		&mut self,
		line_count: usize,
		line_text: &dyn Fn(usize) -> String,
	) {
		// Collect all lines, pre-trimmed.
		let lines: Vec<String> = (0..line_count).map(|l| line_text(l)).collect();

		// Find every line that starts a new top-level statement.
		let starts: Vec<usize> = lines
			.iter()
			.enumerate()
			.filter(|(_, t)| is_sql_statement_start(t))
			.map(|(i, _)| i)
			.collect();

		for (idx, &start) in starts.iter().enumerate() {
			// The candidate end is either just before the next statement start
			// or the last line of the file.
			let search_end = starts.get(idx + 1).copied().unwrap_or(line_count);

			// Walk backward from search_end to find the last line that is part
			// of this statement, skipping trailing blank lines and comments
			// (which belong to the next statement, not this one).
			let mut end = start + 1;
			for li in (start + 1..search_end).rev() {
				let t = lines[li].trim();
				if !t.is_empty() && !t.starts_with("--") {
					end = li;
					break;
				}
			}

			// Only create a fold if there is at least one line to collapse.
			if end > start {
				self.regions.insert(
					start,
					FoldRegion {
						start_line: start,
						end_line: end,
						kind: FoldKind::Statement,
					},
				);
			}
		}
	}

	fn walk_node(&mut self, node: tree_sitter::Node) {
		let start = node.start_position();
		let end = node.end_position();

		if end.row > start.row + 1 {
			let kind = match node.kind() {
				"block_comment" | "comment" => FoldKind::Comment,
				k if k.contains("block") || k.contains("body") => FoldKind::Block,
				_ => {
					let first_child = node.child(0);
					match first_child.map(|c| c.kind()) {
						Some("{") => FoldKind::Block,
						Some("(") => FoldKind::Paren,
						_ => FoldKind::Block,
					}
				}
			};

			self.regions.entry(start.row).or_insert(FoldRegion {
				start_line: start.row,
				end_line: end.row,
				kind,
			});
		}

		for i in 0..node.child_count() {
			if let Some(child) = node.child(i as u32) {
				self.walk_node(child);
			}
		}
	}

	fn detect_indent_folds(&mut self, line_count: usize, line_text: &dyn Fn(usize) -> String) {
		let mut i = 0;
		while i < line_count {
			let text = line_text(i);
			let base_indent = indent_level(&text);
			if base_indent == 0 || text.trim().is_empty() {
				i += 1;
				continue;
			}

			let mut end = i + 1;
			while end < line_count {
				let t = line_text(end);
				if t.trim().is_empty() {
					end += 1;
					continue;
				}
				if indent_level(&t) <= base_indent {
					break;
				}
				end += 1;
			}

			if end > i + 2 && !self.regions.contains_key(&i) {
				self.regions.insert(
					i,
					FoldRegion {
						start_line: i,
						end_line: end - 1,
						kind: FoldKind::Indent,
					},
				);
			}
			i = end;
		}
	}

	/// Toggle fold at a given line.
	pub fn toggle(&mut self, line: usize) {
		if self.collapsed.contains_key(&line) {
			self.collapsed.remove(&line);
		} else if let Some(region) = self.regions.get(&line) {
			self.collapsed.insert(line, region.end_line);
		}
	}

	/// Check if a line is hidden (inside a collapsed fold).
	pub fn is_hidden(&self, line: usize) -> bool {
		for (&start, &end) in &self.collapsed {
			if line > start && line <= end {
				return true;
			}
		}
		false
	}

	/// Check if a line is the start of a collapsed fold.
	pub fn is_collapsed_start(&self, line: usize) -> bool {
		self.collapsed.contains_key(&line)
	}

	/// Check if a line has a foldable region starting here.
	pub fn is_foldable(&self, line: usize) -> bool {
		self.regions.contains_key(&line)
	}

	/// Number of hidden lines in a collapsed region starting at `line`.
	pub fn hidden_count(&self, line: usize) -> usize {
		self.collapsed.get(&line).map(|end| end - line).unwrap_or(0)
	}

	/// Map a visible line index to an actual document line, accounting for folds.
	pub fn visible_to_doc_line(&self, visible: usize, total_lines: usize) -> usize {
		let mut doc = 0;
		let mut vis = 0;
		while doc < total_lines && vis < visible {
			if self.is_hidden(doc) {
				doc += 1;
				continue;
			}
			vis += 1;
			doc += 1;
		}
		while doc < total_lines && self.is_hidden(doc) {
			doc += 1;
		}
		doc
	}

	/// Total number of visible lines.
	pub fn visible_line_count(&self, total_lines: usize) -> usize {
		(0..total_lines).filter(|&l| !self.is_hidden(l)).count()
	}
}

/// Returns true if the line begins a top-level SQL statement.
fn is_sql_statement_start(line: &str) -> bool {
	let trimmed = line.trim();
	if trimmed.is_empty() || trimmed.starts_with("--") {
		return false;
	}
	// Must start at column 0 (no leading whitespace = top-level).
	if line.starts_with(|c: char| c.is_whitespace()) {
		return false;
	}
	let first_word: String = trimmed
		.chars()
		.take_while(|c| c.is_alphabetic() || *c == '_')
		.collect::<String>()
		.to_uppercase();
	SQL_STATEMENT_KEYWORDS.contains(&first_word.as_str())
}

fn indent_level(text: &str) -> usize {
	let spaces = text.chars().take_while(|c| *c == ' ').count();
	let tabs = text.chars().take_while(|c| *c == '\t').count();
	spaces / 4 + tabs
}
