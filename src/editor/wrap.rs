#[derive(Debug, Clone, Copy)]
pub struct VisualLine {
	pub doc_line: usize,
	/// Byte offset within the document line where this visual line starts.
	pub col_start: usize,
	/// Byte offset within the document line where this visual line ends (exclusive).
	pub col_end: usize,
	/// Whether this is the first visual line of the doc line (for line number display).
	pub is_first: bool,
}

/// Configuration for line wrapping.
#[derive(Debug, Clone, Copy)]
pub struct WrapConfig {
	pub enabled: bool,
	/// Maximum number of columns before wrapping.
	pub wrap_col: usize,
}

impl Default for WrapConfig {
	fn default() -> Self {
		Self {
			enabled: false,
			wrap_col: 120,
		}
	}
}

/// Compute visual lines for the entire document.
pub fn compute_visual_lines(
	line_count: usize,
	line_text: &dyn Fn(usize) -> String,
	is_hidden: &dyn Fn(usize) -> bool,
	config: &WrapConfig,
) -> Vec<VisualLine> {
	let mut visual = Vec::new();

	for doc_line in 0..line_count {
		if is_hidden(doc_line) {
			continue;
		}

		let text = line_text(doc_line);
		if !config.enabled || text.len() <= config.wrap_col {
			visual.push(VisualLine {
				doc_line,
				col_start: 0,
				col_end: text.len(),
				is_first: true,
			});
		} else {
			// Wrap at word boundaries when possible
			let mut col = 0;
			let mut first = true;
			while col < text.len() {
				let remaining = text.len() - col;
				let chunk_end = if remaining <= config.wrap_col {
					text.len()
				} else {
					// Try to find a good break point (space, comma, paren)
					let max_end = col + config.wrap_col;
					let slice = &text[col..max_end];
					// Search backwards for a break character
					let break_pos = slice
						.rfind(|c: char| c == ' ' || c == ',' || c == '(' || c == ')')
						.map(|p| col + p + 1) // include the break char
						.unwrap_or(max_end); // hard break if no good spot
					break_pos
				};

				visual.push(VisualLine {
					doc_line,
					col_start: col,
					col_end: chunk_end,
					is_first: first,
				});
				first = false;
				col = chunk_end;
			}
			// Handle empty lines
			if col == 0 {
				visual.push(VisualLine {
					doc_line,
					col_start: 0,
					col_end: 0,
					is_first: true,
				});
			}
		}
	}

	visual
}
