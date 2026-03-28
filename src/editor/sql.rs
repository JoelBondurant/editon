pub fn current_statement_range(text: &str, cursor: usize) -> (usize, usize) {
	(
		find_statement_start(text, cursor),
		find_statement_end(text, cursor),
	)
}

pub fn current_statement_text(text: &str, cursor: usize) -> String {
	let (start, end) = current_statement_range(text, cursor);
	text[start..end].trim().to_string()
}

pub fn newline_extra_indent(before_cursor: &str, after_cursor: &str, indent: &str) -> &'static str {
	let trimmed = before_cursor.trim_end();
	let next_trimmed = after_cursor.trim_start();
	if trimmed.ends_with('(') && !next_trimmed.starts_with(')') {
		if indent.contains('\t') && !indent.contains(' ') {
			"\t"
		} else {
			"    "
		}
	} else {
		""
	}
}

fn find_statement_start(text: &str, cursor: usize) -> usize {
	let chars: Vec<char> = text.chars().collect();
	let cursor = cursor.min(chars.len());
	let mut in_single = false;
	let mut in_double = false;
	let mut in_line_comment = false;
	let mut in_block_comment = false;
	let mut last_boundary = 0usize;
	let mut i = 0usize;

	while i < cursor {
		let ch = chars[i];
		let next = chars.get(i + 1).copied();

		if in_line_comment {
			if ch == '\n' {
				in_line_comment = false;
			}
			i += 1;
			continue;
		}
		if in_block_comment {
			if ch == '*' && next == Some('/') {
				in_block_comment = false;
				i += 2;
			} else {
				i += 1;
			}
			continue;
		}
		if in_single {
			if ch == '\'' {
				if next == Some('\'') {
					i += 2;
					continue;
				}
				in_single = false;
			}
			i += 1;
			continue;
		}
		if in_double {
			if ch == '"' {
				in_double = false;
			}
			i += 1;
			continue;
		}

		match (ch, next) {
			('-', Some('-')) => {
				in_line_comment = true;
				i += 2;
			}
			('/', Some('*')) => {
				in_block_comment = true;
				i += 2;
			}
			('\'', _) => {
				in_single = true;
				i += 1;
			}
			('"', _) => {
				in_double = true;
				i += 1;
			}
			(';', _) => {
				last_boundary = i + 1;
				i += 1;
			}
			_ => i += 1,
		}
	}

	while last_boundary < chars.len() && chars[last_boundary].is_whitespace() {
		last_boundary += 1;
	}
	char_offset_to_byte_idx(text, last_boundary)
}

fn find_statement_end(text: &str, cursor: usize) -> usize {
	let chars: Vec<char> = text.chars().collect();
	let mut i = cursor.min(chars.len());
	let mut in_single = false;
	let mut in_double = false;
	let mut in_line_comment = false;
	let mut in_block_comment = false;

	while i < chars.len() {
		let ch = chars[i];
		let next = chars.get(i + 1).copied();

		if in_line_comment {
			if ch == '\n' {
				in_line_comment = false;
			}
			i += 1;
			continue;
		}
		if in_block_comment {
			if ch == '*' && next == Some('/') {
				in_block_comment = false;
				i += 2;
			} else {
				i += 1;
			}
			continue;
		}
		if in_single {
			if ch == '\'' {
				if next == Some('\'') {
					i += 2;
					continue;
				}
				in_single = false;
			}
			i += 1;
			continue;
		}
		if in_double {
			if ch == '"' {
				in_double = false;
			}
			i += 1;
			continue;
		}

		match (ch, next) {
			('-', Some('-')) => {
				in_line_comment = true;
				i += 2;
			}
			('/', Some('*')) => {
				in_block_comment = true;
				i += 2;
			}
			('\'', _) => {
				in_single = true;
				i += 1;
			}
			('"', _) => {
				in_double = true;
				i += 1;
			}
			(';', _) => return char_offset_to_byte_idx(text, i + 1),
			_ => i += 1,
		}
	}

	text.len()
}

fn char_offset_to_byte_idx(text: &str, offset: usize) -> usize {
	text.char_indices()
		.nth(offset)
		.map(|(idx, _)| idx)
		.unwrap_or(text.len())
}
