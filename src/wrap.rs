/// Soft line wrapping: computes visual line breaks without modifying the buffer.

/// Represents one visual line (a sub-range of a document line).
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
                        .unwrap_or(max_end);   // hard break if no good spot
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

/// Convert a document (line, col) to a visual line index.
pub fn doc_to_visual(visual_lines: &[VisualLine], doc_line: usize, doc_col: usize) -> usize {
    for (i, vl) in visual_lines.iter().enumerate() {
        if vl.doc_line == doc_line && doc_col >= vl.col_start && doc_col <= vl.col_end {
            return i;
        }
        // If past the target doc line, use the last visual line of that doc line
        if vl.doc_line > doc_line {
            return i.saturating_sub(1);
        }
    }
    visual_lines.len().saturating_sub(1)
}

/// Convert a visual line index back to (doc_line, col_within_visual_line).
pub fn visual_to_doc(visual_lines: &[VisualLine], visual_idx: usize) -> (usize, usize) {
    if let Some(vl) = visual_lines.get(visual_idx) {
        (vl.doc_line, vl.col_start)
    } else if let Some(last) = visual_lines.last() {
        (last.doc_line, last.col_start)
    } else {
        (0, 0)
    }
}
