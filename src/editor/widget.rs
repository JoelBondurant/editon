use iced::advanced::layout::{self, Layout};
use iced::advanced::renderer;
use iced::advanced::text::Renderer as TextRenderer;
use iced::advanced::widget::{self, Widget};
use iced::advanced::{Clipboard, Renderer as _, Shell};
use iced::keyboard;
use iced::mouse;
use iced::{Color, Element, Event, Length, Pixels, Point, Rectangle, Renderer, Size, Theme};

use super::buffer::{self, Buffer, CursorPos, TAB_WIDTH};
use super::highlight::TokenKind;
use super::theme::EditorTheme;
use super::wrap::VisualLine;

// ─── Constants ────────────────────────────────────────────────────────────────

pub const CHAR_W: f32 = 9.6;

pub const EDITOR_FONT: iced::Font = iced::Font {
	family: iced::font::Family::Name("DejaVu Sans Mono"),
	weight: iced::font::Weight::Normal,
	stretch: iced::font::Stretch::Normal,
	style: iced::font::Style::Normal,
};
const LINE_H: f32 = 22.0;
const GUTTER_PAD: f32 = 16.0;
const FOLD_COL_W: f32 = 16.0;
const LEFT_PAD: f32 = 12.0;
const TOP_PAD: f32 = 8.0;
const FONT_SZ: f32 = 15.0;
const CURSOR_W: f32 = 2.0;
const ERR_THICK: f32 = 2.0;
const SCROLL_W: f32 = 10.0;
const INDENT_W: f32 = 1.0;
const BRACKET_BW: f32 = 1.5;
const MINIMAP_W: f32 = 80.0;
const MINIMAP_LINE_H: f32 = 2.5;
const MINIMAP_CHAR_W: f32 = 1.2;
const SEARCH_PANEL_H: f32 = 40.0;

// ─── Actions ──────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub enum EditorAction {
	Edit,
	CursorMoved,
	MouseDown(iced::Point),
	DoubleClick(iced::Point),
	/// Fired when the widget's pixel bounds change (e.g. window resize).
	Resize(f32, f32),
	/// Toggle the fold at the given document line.
	ToggleFold(usize),
}

// ─── Widget ───────────────────────────────────────────────────────────────────

pub struct SqlEditor<'a, Message> {
	buffer: &'a Buffer,
	theme: &'a EditorTheme,
	on_action: Box<dyn Fn(EditorAction) -> Message + 'a>,
	scroll_y: f32,
	scroll_x: f32,
	show_minimap: bool,
	block_cursor: bool,
	show_whitespace: bool,
	/// Visual block selection: (top_line, bottom_line, left_col, right_col inclusive)
	visual_block: Option<(usize, usize, usize, usize)>,
}

impl<'a, Message> SqlEditor<'a, Message> {
	pub fn new(
		buffer: &'a Buffer,
		theme: &'a EditorTheme,
		on_action: impl Fn(EditorAction) -> Message + 'a,
	) -> Self {
		Self {
			buffer,
			theme,
			on_action: Box::new(on_action),
			scroll_y: 0.0,
			scroll_x: 0.0,
			show_minimap: true,
			block_cursor: false,
			show_whitespace: true,
			visual_block: None,
		}
	}

	pub fn scroll_y(mut self, v: f32) -> Self {
		self.scroll_y = v;
		self
	}
	pub fn scroll_x(mut self, v: f32) -> Self {
		self.scroll_x = v;
		self
	}
	pub fn show_minimap(mut self, v: bool) -> Self {
		self.show_minimap = v;
		self
	}
	pub fn block_cursor(mut self, v: bool) -> Self {
		self.block_cursor = v;
		self
	}
	pub fn show_whitespace(mut self, v: bool) -> Self {
		self.show_whitespace = v;
		self
	}
	pub fn visual_block(mut self, v: Option<(usize, usize, usize, usize)>) -> Self {
		self.visual_block = v;
		self
	}

	fn gutter_w(&self) -> f32 {
		let d = format!("{}", self.buffer.line_count()).len().max(3) as f32;
		d * CHAR_W + GUTTER_PAD * 2.0 + FOLD_COL_W
	}

	fn text_x(&self) -> f32 {
		self.gutter_w() + LEFT_PAD
	}
	fn minimap_x(&self, bounds: &Rectangle) -> f32 {
		bounds.x + bounds.width
			- if self.show_minimap {
				MINIMAP_W + SCROLL_W
			} else {
				SCROLL_W
			}
	}

	/// Find the index into `buffer.visual_lines` for the cursor position.
	fn cursor_visual_line_idx(&self) -> usize {
		let head = self.buffer.selection.head;
		self.buffer
			.visual_lines
			.iter()
			.position(|vl| {
				vl.doc_line == head.line && vl.col_start <= head.col && head.col <= vl.col_end
			})
			.or_else(|| {
				self.buffer
					.visual_lines
					.iter()
					.position(|vl| vl.doc_line == head.line)
			})
			.unwrap_or(head.line)
	}

	fn pixel_to_pos(&self, bounds: &Rectangle, px: f32, py: f32) -> CursorPos {
		let ry = py - bounds.y - TOP_PAD + self.scroll_y;
		let vl_idx = ((ry / LINE_H).floor().max(0.0) as usize)
			.min(self.buffer.visual_lines.len().saturating_sub(1));
		if let Some(vl) = self.buffer.visual_lines.get(vl_idx) {
			let lt = self.buffer.line_text(vl.doc_line);
			let vl_vcol_off = buffer::visual_col_of(&lt, vl.col_start);
			let rx = px - bounds.x - self.text_x() + self.scroll_x;
			let vcol = (rx / CHAR_W).round().max(0.0) as usize + vl_vcol_off;
			let logical = buffer::logical_col_of(&lt, vcol);
			self.buffer.click_to_pos(vl.doc_line, logical)
		} else {
			CursorPos::new(self.buffer.line_count().saturating_sub(1), 0)
		}
	}
}

// ─── State ────────────────────────────────────────────────────────────────────

pub struct EditorState {
	pub is_focused: bool,
	is_dragging: bool,
	last_click: std::time::Instant,
	click_count: u32,
	hover_diag: Option<usize>,
	last_bounds: Rectangle,
}

impl Default for EditorState {
	fn default() -> Self {
		Self {
			is_focused: false,
			is_dragging: false,
			last_click: std::time::Instant::now(),
			click_count: 0,
			hover_diag: None,
			last_bounds: Rectangle::default(),
		}
	}
}

// ─── Private draw helpers ─────────────────────────────────────────────────────

impl<'a, Message> SqlEditor<'a, Message> {
	fn draw_background(&self, renderer: &mut Renderer, b: Rectangle, gw: f32) {
		let th = self.theme;
		fill(renderer, b, th.background);
		fill(
			renderer,
			Rectangle {
				x: b.x,
				y: b.y,
				width: gw,
				height: b.height,
			},
			th.gutter_bg,
		);
		fill(
			renderer,
			Rectangle {
				x: b.x + gw - 1.0,
				y: b.y,
				width: 1.0,
				height: b.height,
			},
			th.gutter_border,
		);
	}

	/// Draw the line gutter for one visual line.
	/// `is_first`: true for the first visual line of a doc line; continuation lines get a blank gutter.
	fn draw_line_gutter(
		&self,
		renderer: &mut Renderer,
		b: Rectangle,
		gw: f32,
		li: usize,
		y: f32,
		active: usize,
		is_first: bool,
	) {
		if !is_first {
			return;
		}
		let th = self.theme;
		let num = format!("{}", li + 1);
		let nc = if li == active {
			th.gutter_active_text
		} else {
			th.gutter_text
		};
		draw_text(
			renderer,
			&num,
			b.x + gw - FOLD_COL_W - GUTTER_PAD - (num.len() as f32 * CHAR_W),
			y,
			nc,
			gw,
		);
		if self.buffer.folds.is_foldable(li) {
			let collapsed = self.buffer.folds.is_collapsed_start(li);
			draw_text(
				renderer,
				if collapsed { "▶" } else { "▼" },
				b.x + gw - FOLD_COL_W + 2.0,
				y,
				th.fold_indicator,
				FOLD_COL_W,
			);
		}
		if self.buffer.folds.is_collapsed_start(li) {
			fill(
				renderer,
				Rectangle {
					x: b.x + gw,
					y,
					width: b.width - gw,
					height: LINE_H,
				},
				th.fold_collapsed_bg,
			);
		}
		if self.buffer.diagnostics.iter().any(|d| d.line == li) {
			fill_r(
				renderer,
				Rectangle {
					x: b.x + 4.0,
					y: y + LINE_H / 2.0 - 3.0,
					width: 6.0,
					height: 6.0,
				},
				th.error_gutter_marker,
				3.0,
			);
		}
	}

	fn draw_line_highlights(
		&self,
		renderer: &mut Renderer,
		b: Rectangle,
		gw: f32,
		tx: f32,
		li: usize,
		y: f32,
		active: usize,
		st: &EditorState,
		vl: &VisualLine,
	) {
		let th = self.theme;
		let lt = self.buffer.line_text(li);
		// Visual column offset of the start of this visual line within the doc line.
		let vl_vcol_off = buffer::visual_col_of(&lt, vl.col_start);

		if li == active && st.is_focused {
			fill(
				renderer,
				Rectangle {
					x: b.x + gw,
					y,
					width: b.width
						- gw - SCROLL_W - if self.show_minimap { MINIMAP_W } else { 0.0 },
					height: LINE_H,
				},
				th.current_line_bg,
			);
		}

		// Indent guides: only relevant on first visual line of a doc line.
		if vl.is_first {
			for &vcol in &self.buffer.indent_guides(li) {
				let guide_abs = vcol.saturating_sub(TAB_WIDTH);
				if guide_abs >= vl_vcol_off {
					let gx = b.x + tx + ((guide_abs - vl_vcol_off) as f32 * CHAR_W) - self.scroll_x;
					let c = if li == active {
						th.indent_guide_active
					} else {
						th.indent_guide
					};
					fill(
						renderer,
						Rectangle {
							x: gx,
							y,
							width: INDENT_W,
							height: LINE_H,
						},
						c,
					);
				}
			}
		}

		// Search matches clipped to this visual line's byte range.
		if self.buffer.search.is_open {
			for (i, m) in self.buffer.search.matches.iter().enumerate() {
				if m.line == li && m.col_start < vl.col_end && m.col_end > vl.col_start {
					let ms = m.col_start.max(vl.col_start).min(lt.len());
					let me = m.col_end.min(vl.col_end).min(lt.len());
					let mvs = buffer::visual_col_of(&lt, ms).saturating_sub(vl_vcol_off);
					let mve = buffer::visual_col_of(&lt, me).saturating_sub(vl_vcol_off);
					let mx = b.x + tx + (mvs as f32 * CHAR_W) - self.scroll_x;
					let mw = ((mve - mvs) as f32 * CHAR_W).max(CHAR_W);
					let c = if i == self.buffer.search.current_match {
						th.search_current_bg
					} else {
						th.search_match_bg
					};
					fill(
						renderer,
						Rectangle {
							x: mx,
							y,
							width: mw,
							height: LINE_H,
						},
						c,
					);
				}
			}
		}

		if let Some((top, bottom, left_col, right_col)) = self.visual_block {
			if li >= top && li <= bottom {
				if left_col < vl.col_end && right_col + 1 > vl.col_start {
					let vcs = buffer::visual_col_of(&lt, left_col.max(vl.col_start).min(lt.len()))
						.saturating_sub(vl_vcol_off);
					let vce =
						buffer::visual_col_of(&lt, (right_col + 1).min(vl.col_end).min(lt.len()))
							.saturating_sub(vl_vcol_off);
					let sx = b.x + tx + (vcs as f32 * CHAR_W) - self.scroll_x;
					let sw = ((vce - vcs) as f32 * CHAR_W).max(CHAR_W * 0.5);
					fill(
						renderer,
						Rectangle {
							x: sx,
							y,
							width: sw,
							height: LINE_H,
						},
						th.selection,
					);
				}
			}
		} else if !self.buffer.selection.is_caret() {
			let (ss, se) = self.buffer.selection.ordered();
			if li >= ss.line && li <= se.line {
				let raw_start = if li == ss.line { ss.col } else { 0 };
				let raw_end = if li == se.line { se.col } else { lt.len() };

				// Check that the selection overlaps this visual line's byte range.
				if raw_start < vl.col_end && raw_end > vl.col_start {
					let clip_start = raw_start.max(vl.col_start).min(lt.len());
					let clip_end = raw_end.min(vl.col_end).min(lt.len());
					let vcs = buffer::visual_col_of(&lt, clip_start).saturating_sub(vl_vcol_off);
					// If the selection extends past the end of this VL (to next VL or next line),
					// cover the full width of this VL; add +1 only when crossing a doc-line boundary.
					let vce = if raw_end > vl.col_end {
						let end_abs = buffer::visual_col_of(&lt, vl.col_end.min(lt.len()))
							.saturating_sub(vl_vcol_off);
						end_abs + if li < se.line { 1 } else { 0 }
					} else {
						buffer::visual_col_of(&lt, clip_end).saturating_sub(vl_vcol_off)
					};
					if vce > vcs {
						let sx = b.x + tx + (vcs as f32 * CHAR_W) - self.scroll_x;
						let sw = ((vce - vcs) as f32 * CHAR_W).max(CHAR_W * 0.5);
						fill(
							renderer,
							Rectangle {
								x: sx,
								y,
								width: sw,
								height: LINE_H,
							},
							th.selection,
						);
					}
				}
			}
		}

		// Bracket matching: only when bracket is within this visual line's byte range.
		if let Some(ref bm) = self.buffer.matched_bracket {
			for &(bl, bc) in &[(bm.open_line, bm.open_col), (bm.close_line, bm.close_col)] {
				if bl == li && bc >= vl.col_start && bc < vl.col_end {
					let blt = self.buffer.line_text(bl);
					let bvcol = buffer::visual_col_of(&blt, bc).saturating_sub(vl_vcol_off);
					let bx = b.x + tx + (bvcol as f32 * CHAR_W) - self.scroll_x;
					fill(
						renderer,
						Rectangle {
							x: bx,
							y,
							width: CHAR_W,
							height: LINE_H,
						},
						th.bracket_match_bg,
					);
					for rect in [
						Rectangle {
							x: bx,
							y,
							width: CHAR_W,
							height: BRACKET_BW,
						},
						Rectangle {
							x: bx,
							y: y + LINE_H - BRACKET_BW,
							width: CHAR_W,
							height: BRACKET_BW,
						},
						Rectangle {
							x: bx,
							y,
							width: BRACKET_BW,
							height: LINE_H,
						},
						Rectangle {
							x: bx + CHAR_W - BRACKET_BW,
							y,
							width: BRACKET_BW,
							height: LINE_H,
						},
					] {
						fill(renderer, rect, th.bracket_match_border);
					}
				}
			}
		}
	}

	fn draw_line_tokens(
		&self,
		renderer: &mut Renderer,
		b: Rectangle,
		tx: f32,
		li: usize,
		y: f32,
		vl: &VisualLine,
	) {
		let th = self.theme;
		let lt = self.buffer.line_text(li);
		let vl_vcol_off = buffer::visual_col_of(&lt, vl.col_start);
		let render_start = vl.col_start;
		let render_end = vl.col_end.min(lt.len());
		let lbs = self.buffer.rope.line_to_byte(li);
		let lbe = lbs + lt.len();

		// Build token spans clipped to this visual line's byte range.
		let mut spans: Vec<(usize, usize, TokenKind)> = Vec::new();
		for tok in self.buffer.tokens() {
			if tok.byte_range.end <= lbs || tok.byte_range.start >= lbe {
				continue;
			}
			let s = (tok.byte_range.start.max(lbs) - lbs).max(render_start);
			let e = (tok.byte_range.end.min(lbe) - lbs).min(render_end);
			if s >= e {
				continue;
			}
			spans.push((s, e, tok.kind));
		}
		spans.sort_by_key(|s| s.0);
		let mut render: Vec<(usize, usize, TokenKind)> = Vec::new();
		let mut cur = render_start;
		for &(s, e, k) in &spans {
			if s > cur {
				render.push((cur, s, TokenKind::Plain));
			}
			render.push((s, e, k));
			cur = e;
		}
		if cur < render_end {
			render.push((cur, render_end, TokenKind::Plain));
		}

		let ws_color = Color {
			a: 0.35,
			..th.gutter_text
		};
		let trail_start = lt.trim_end().len();

		for &(start, end, kind) in &render {
			if start >= render_end {
				break;
			}
			let sl = &lt[start..end.min(lt.len())];
			if sl.is_empty() {
				continue;
			}
			let color = token_color(&kind, th);
			// Use absolute visual col for tab-stop math; subtract vl_vcol_off for screen X.
			let mut vcol = buffer::visual_col_of(&lt, start);
			let mut seg = String::new();
			let mut seg_vcol = vcol;
			let mut byte_pos = start;
			for ch in sl.chars() {
				if ch == '\t' {
					if !seg.is_empty() {
						draw_text(
							renderer,
							&seg,
							b.x + tx + ((seg_vcol.saturating_sub(vl_vcol_off)) as f32 * CHAR_W)
								- self.scroll_x,
							y,
							color,
							b.width,
						);
						seg.clear();
					}
					if self.show_whitespace {
						draw_text(
							renderer,
							"▸",
							b.x + tx + ((vcol.saturating_sub(vl_vcol_off)) as f32 * CHAR_W)
								- self.scroll_x,
							y,
							ws_color,
							CHAR_W,
						);
					}
					vcol = (vcol / TAB_WIDTH + 1) * TAB_WIDTH;
					seg_vcol = vcol;
				} else if self.show_whitespace && ch == ' ' {
					if !seg.is_empty() {
						draw_text(
							renderer,
							&seg,
							b.x + tx + ((seg_vcol.saturating_sub(vl_vcol_off)) as f32 * CHAR_W)
								- self.scroll_x,
							y,
							color,
							b.width,
						);
						seg.clear();
					}
					let glyph = if byte_pos >= trail_start { "~" } else { "␣" };
					draw_text(
						renderer,
						glyph,
						b.x + tx + ((vcol.saturating_sub(vl_vcol_off)) as f32 * CHAR_W)
							- self.scroll_x,
						y,
						ws_color,
						CHAR_W,
					);
					vcol += 1;
					seg_vcol = vcol;
				} else {
					seg.push(ch);
					vcol += 1;
				}
				byte_pos += ch.len_utf8();
			}
			if !seg.is_empty() {
				draw_text(
					renderer,
					&seg,
					b.x + tx + ((seg_vcol.saturating_sub(vl_vcol_off)) as f32 * CHAR_W)
						- self.scroll_x,
					y,
					color,
					b.width,
				);
			}
		}

		// EOL marker, fold indicator, and diagnostics only on the last visual line for this doc line.
		let is_last_vl = vl.col_end >= lt.len();
		if is_last_vl {
			if self.show_whitespace {
				let eol_vcol = buffer::chars_with_vcols(&lt)
					.last()
					.map_or(0, |(_, vc)| vc + 1);
				draw_text(
					renderer,
					"¬",
					b.x + tx + ((eol_vcol.saturating_sub(vl_vcol_off)) as f32 * CHAR_W)
						- self.scroll_x,
					y,
					Color {
						a: 0.18,
						..th.gutter_text
					},
					CHAR_W,
				);
			}
			if self.buffer.folds.is_collapsed_start(li) {
				let hc = self.buffer.folds.hidden_count(li);
				if hc > 0 {
					let eol_vcol = buffer::chars_with_vcols(&lt)
						.last()
						.map_or(0, |(_, vc)| vc + 1);
					draw_text(
						renderer,
						&format!(" ⋯ {} lines", hc),
						b.x + tx + ((eol_vcol.saturating_sub(vl_vcol_off)) as f32 * CHAR_W) + 8.0
							- self.scroll_x,
						y,
						th.comment,
						200.0,
					);
				}
			}
		}

		for diag in &self.buffer.diagnostics {
			if diag.line == li && diag.col_start < vl.col_end && diag.col_end > vl.col_start {
				let ds = diag.col_start.max(vl.col_start).min(lt.len());
				let de = diag.col_end.min(render_end).min(lt.len());
				if ds < de {
					let uvs = buffer::visual_col_of(&lt, ds).saturating_sub(vl_vcol_off);
					let uve = buffer::visual_col_of(&lt, de).saturating_sub(vl_vcol_off);
					let ux = b.x + tx + (uvs as f32 * CHAR_W) - self.scroll_x;
					let uw = ((uve - uvs) as f32 * CHAR_W).max(CHAR_W);
					let uy = y + LINE_H - ERR_THICK - 1.0;
					let seg: f32 = 4.0;
					let mut sx = ux;
					let mut up = true;
					while sx < ux + uw {
						fill(
							renderer,
							Rectangle {
								x: sx,
								y: if up { uy - 1.0 } else { uy + 1.0 },
								width: seg.min(ux + uw - sx),
								height: ERR_THICK,
							},
							th.error_underline,
						);
						sx += seg;
						up = !up;
					}
				}
			}
		}
	}

	fn draw_cursor(
		&self,
		renderer: &mut Renderer,
		b: Rectangle,
		tx: f32,
		editor_h: f32,
		st: &EditorState,
	) {
		if !st.is_focused {
			return;
		}
		let th = self.theme;
		let head = self.buffer.selection.head;
		let clt = self.buffer.line_text(head.line);
		let cvcol_abs = buffer::visual_col_of(&clt, head.col);

		// Find visual line index for cursor position.
		let vl_idx = self.cursor_visual_line_idx();
		let vl_vcol_off = self
			.buffer
			.visual_lines
			.get(vl_idx)
			.map(|vl| buffer::visual_col_of(&clt, vl.col_start))
			.unwrap_or(0);

		let cy = b.y + TOP_PAD + (vl_idx as f32 * LINE_H) - self.scroll_y;
		let cx =
			b.x + tx + ((cvcol_abs.saturating_sub(vl_vcol_off)) as f32 * CHAR_W) - self.scroll_x;
		if cy > b.y - LINE_H && cy < b.y + editor_h {
			if self.block_cursor {
				fill(
					renderer,
					Rectangle {
						x: cx,
						y: cy,
						width: CHAR_W,
						height: LINE_H,
					},
					Color {
						a: 0.55,
						..th.cursor
					},
				);
			} else {
				fill(
					renderer,
					Rectangle {
						x: cx,
						y: cy,
						width: CURSOR_W,
						height: LINE_H,
					},
					th.cursor,
				);
			}
		}
	}

	fn draw_minimap(&self, renderer: &mut Renderer, b: Rectangle, editor_h: f32) {
		let th = self.theme;
		let mx = self.minimap_x(&b);
		fill(
			renderer,
			Rectangle {
				x: mx,
				y: b.y,
				width: MINIMAP_W,
				height: editor_h,
			},
			th.minimap_bg,
		);
		let total_h = self.buffer.line_count() as f32 * MINIMAP_LINE_H;
		if total_h > 0.0 {
			let scale = MINIMAP_LINE_H / LINE_H;
			let vp_h = (editor_h * scale).min(editor_h).max(20.0);
			let vp_y = b.y + self.scroll_y * scale;
			fill(
				renderer,
				Rectangle {
					x: mx,
					y: vp_y,
					width: MINIMAP_W,
					height: vp_h,
				},
				th.minimap_viewport,
			);
		}
		for li in 0..self.buffer.line_count() {
			if self.buffer.folds.is_hidden(li) {
				continue;
			}
			let my = b.y + li as f32 * MINIMAP_LINE_H;
			if my > b.y + editor_h {
				break;
			}
			let lt = self.buffer.line_text(li);
			if lt.trim().is_empty() {
				continue;
			}
			let lbs = self.buffer.rope.line_to_byte(li);
			let lbe = lbs + lt.len();
			for tok in self.buffer.tokens() {
				if tok.byte_range.end <= lbs || tok.byte_range.start >= lbe {
					continue;
				}
				let s = tok.byte_range.start.max(lbs) - lbs;
				let e = tok.byte_range.end.min(lbe) - lbs;
				let tw = (e - s) as f32 * MINIMAP_CHAR_W;
				if tw > 0.5 {
					let c = token_color(&tok.kind, th);
					fill(
						renderer,
						Rectangle {
							x: mx + 4.0 + s as f32 * MINIMAP_CHAR_W,
							y: my,
							width: tw.min(MINIMAP_W - 8.0),
							height: MINIMAP_LINE_H,
						},
						Color::from_rgba(c.r, c.g, c.b, 0.35),
					);
				}
			}
		}
	}

	fn draw_scrollbar(&self, renderer: &mut Renderer, b: Rectangle, editor_h: f32) {
		let th = self.theme;
		let sb_x = b.x + b.width - SCROLL_W;
		fill(
			renderer,
			Rectangle {
				x: sb_x,
				y: b.y,
				width: SCROLL_W,
				height: editor_h,
			},
			th.scrollbar_track,
		);
		let total = self.buffer.visual_lines.len() as f32 * LINE_H + TOP_PAD * 2.0;
		if total > editor_h {
			let th_h = ((editor_h / total) * editor_h).max(24.0);
			let th_y = b.y + (self.scroll_y / (total - editor_h)) * (editor_h - th_h);
			fill_r(
				renderer,
				Rectangle {
					x: sb_x + 2.0,
					y: th_y,
					width: SCROLL_W - 4.0,
					height: th_h,
				},
				th.scrollbar_thumb,
				3.0,
			);
		}
	}

	fn draw_search_panel(&self, renderer: &mut Renderer, b: Rectangle) {
		let th = self.theme;
		let sp_y = b.y + b.height - SEARCH_PANEL_H;
		fill(
			renderer,
			Rectangle {
				x: b.x,
				y: sp_y,
				width: b.width,
				height: SEARCH_PANEL_H,
			},
			th.search_panel_bg,
		);
		fill(
			renderer,
			Rectangle {
				x: b.x,
				y: sp_y,
				width: b.width,
				height: 1.0,
			},
			th.gutter_border,
		);
		let info = format!(
            "Find: \"{}\"  {} of {}   Replace: \"{}\"   [Enter=next] [Shift+Enter=prev] [Ctrl+Shift+H=replace] [Ctrl+Shift+Enter=all]",
            self.buffer.search.query,
            if self.buffer.search.matches.is_empty() { 0 } else { self.buffer.search.current_match + 1 },
            self.buffer.search.match_count(),
            self.buffer.search.replacement,
        );
		draw_text(
			renderer,
			&info,
			b.x + 12.0,
			sp_y + 4.0,
			th.tooltip_text,
			b.width - 24.0,
		);
	}

	fn draw_tooltip(&self, renderer: &mut Renderer, b: Rectangle, tx: f32, st: &EditorState) {
		let th = self.theme;
		if let Some(di) = st.hover_diag {
			if let Some(diag) = self.buffer.diagnostics.get(di) {
				let diag_vl_idx = self
					.buffer
					.visual_lines
					.iter()
					.position(|vl| {
						vl.doc_line == diag.line
							&& vl.col_start <= diag.col_start
							&& diag.col_start <= vl.col_end
					})
					.unwrap_or(diag.line);
				let dy =
					b.y + TOP_PAD + (diag_vl_idx as f32 * LINE_H) - self.scroll_y + LINE_H + 4.0;
				let dx = b.x + tx + (diag.col_start as f32 * CHAR_W) - self.scroll_x;
				let tw = (diag.message.len() as f32 * CHAR_W * 0.62)
					.min(400.0)
					.max(150.0);
				let th2 = 28.0;
				fill_r(
					renderer,
					Rectangle {
						x: dx + 1.0,
						y: dy + 1.0,
						width: tw,
						height: th2,
					},
					th.tooltip_shadow,
					4.0,
				);
				fill_r(
					renderer,
					Rectangle {
						x: dx,
						y: dy,
						width: tw,
						height: th2,
					},
					th.tooltip_bg,
					4.0,
				);
				for rect in [
					Rectangle {
						x: dx,
						y: dy,
						width: tw,
						height: 1.0,
					},
					Rectangle {
						x: dx,
						y: dy + th2 - 1.0,
						width: tw,
						height: 1.0,
					},
					Rectangle {
						x: dx,
						y: dy,
						width: 1.0,
						height: th2,
					},
					Rectangle {
						x: dx + tw - 1.0,
						y: dy,
						width: 1.0,
						height: th2,
					},
				] {
					fill(renderer, rect, th.tooltip_border);
				}
				let msg = if diag.message.len() > 55 {
					format!("{}…", &diag.message[..54])
				} else {
					diag.message.clone()
				};
				draw_text(renderer, &msg, dx + 8.0, dy, th.tooltip_text, tw - 16.0);
			}
		}
	}
}

// ─── Widget impl ──────────────────────────────────────────────────────────────

impl<'a, Message: Clone> Widget<Message, Theme, Renderer> for SqlEditor<'a, Message> {
	fn tag(&self) -> widget::tree::Tag {
		widget::tree::Tag::of::<EditorState>()
	}
	fn state(&self) -> widget::tree::State {
		widget::tree::State::new(EditorState::default())
	}
	fn size(&self) -> Size<Length> {
		Size {
			width: Length::Fill,
			height: Length::Fill,
		}
	}

	fn layout(
		&mut self,
		_t: &mut widget::Tree,
		_r: &Renderer,
		lim: &layout::Limits,
	) -> layout::Node {
		layout::Node::new(lim.width(Length::Fill).height(Length::Fill).max())
	}

	fn draw(
		&self,
		tree: &widget::Tree,
		renderer: &mut Renderer,
		_theme: &Theme,
		_style: &renderer::Style,
		layout: Layout<'_>,
		_cursor: mouse::Cursor,
		_vp: &Rectangle,
	) {
		let b = layout.bounds();
		let st = tree.state.downcast_ref::<EditorState>();
		let gw = self.gutter_w();
		let tx = self.text_x();
		let editor_h = b.height
			- if self.buffer.search.is_open {
				SEARCH_PANEL_H
			} else {
				0.0
			};

		renderer.start_layer(b);
		{
			self.draw_background(renderer, b, gw);

			// Clip text content to the region left of the minimap/scrollbar.
			let mm_x = self.minimap_x(&b);
			let content_clip = Rectangle {
				x: b.x,
				y: b.y,
				width: mm_x - b.x,
				height: editor_h,
			};
			renderer.start_layer(content_clip);
			{
				let vls = &self.buffer.visual_lines;
				let first = (self.scroll_y / LINE_H).floor() as usize;
				let last = (first + (editor_h / LINE_H).ceil() as usize + 2).min(vls.len());
				let active = self.buffer.selection.head.line;

				for vi in first..last {
					if let Some(vl) = vls.get(vi) {
						let y = b.y + TOP_PAD + (vi as f32 * LINE_H) - self.scroll_y;
						if y + LINE_H < b.y || y > b.y + editor_h {
							continue;
						}
						self.draw_line_gutter(renderer, b, gw, vl.doc_line, y, active, vl.is_first);
						self.draw_line_highlights(
							renderer,
							b,
							gw,
							tx,
							vl.doc_line,
							y,
							active,
							st,
							vl,
						);
						self.draw_line_tokens(renderer, b, tx, vl.doc_line, y, vl);
					}
				}

				self.draw_cursor(renderer, b, tx, editor_h, st);
				self.draw_tooltip(renderer, b, tx, st);
			}
			renderer.end_layer();

			if self.show_minimap {
				self.draw_minimap(renderer, b, editor_h);
			}
			self.draw_scrollbar(renderer, b, editor_h);
			if self.buffer.search.is_open {
				self.draw_search_panel(renderer, b);
			}
		}
		renderer.end_layer();
	}

	fn update(
		&mut self,
		tree: &mut widget::Tree,
		event: &Event,
		layout: Layout<'_>,
		cursor: mouse::Cursor,
		_r: &Renderer,
		_clip: &mut dyn Clipboard,
		shell: &mut Shell<'_, Message>,
		_vp: &Rectangle,
	) {
		let b = layout.bounds();
		let st = tree.state.downcast_mut::<EditorState>();

		match event {
			Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
				if let Some(pos) = cursor.position_over(b) {
					st.is_focused = true;
					let now = std::time::Instant::now();
					if now.duration_since(st.last_click).as_millis() < 400 {
						st.click_count += 1;
					} else {
						st.click_count = 1;
					}
					st.last_click = now;

					let gw = self.gutter_w();
					if pos.x >= b.x + gw - FOLD_COL_W && pos.x <= b.x + gw {
						let ry = pos.y - b.y - TOP_PAD + self.scroll_y;
						let vl_idx = ((ry / LINE_H).floor().max(0.0) as usize)
							.min(self.buffer.visual_lines.len().saturating_sub(1));
						if let Some(vl) = self.buffer.visual_lines.get(vl_idx) {
							let doc_line = vl.doc_line;
							if self.buffer.folds.is_foldable(doc_line) {
								shell.publish((self.on_action)(EditorAction::ToggleFold(doc_line)));
							}
						}
						shell.capture_event();
						return;
					}

					st.is_dragging = true;
					let action = if st.click_count >= 2 {
						EditorAction::DoubleClick(pos)
					} else {
						EditorAction::MouseDown(pos)
					};
					shell.publish((self.on_action)(action));
					shell.capture_event();
					return;
				} else {
					st.is_focused = false;
				}
			}
			Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => {
				st.is_dragging = false;
			}
			Event::Mouse(mouse::Event::CursorMoved { position }) => {
				st.hover_diag = None;
				if cursor.is_over(b) {
					let hp = self.pixel_to_pos(&b, position.x, position.y);
					for (i, d) in self.buffer.diagnostics.iter().enumerate() {
						if d.line == hp.line && hp.col >= d.col_start && hp.col < d.col_end {
							st.hover_diag = Some(i);
							break;
						}
					}
				}
				if st.is_dragging {
					shell.capture_event();
					return;
				}
			}
			Event::Keyboard(keyboard::Event::KeyPressed { .. }) if st.is_focused => {
				shell.capture_event();
				return;
			}
			_ => {}
		}

		// Detect widget resize and notify CodeEditor so it can recompute wrap_col.
		if b.width != st.last_bounds.width || b.height != st.last_bounds.height {
			st.last_bounds = b;
			shell.publish((self.on_action)(EditorAction::Resize(b.width, b.height)));
		}
	}

	fn mouse_interaction(
		&self,
		_t: &widget::Tree,
		layout: Layout<'_>,
		cursor: mouse::Cursor,
		_vp: &Rectangle,
		_r: &Renderer,
	) -> mouse::Interaction {
		let b = layout.bounds();
		if cursor.is_over(b) {
			if let Some(pos) = cursor.position() {
				let gw = self.gutter_w();
				if pos.x > b.x + gw && pos.x < self.minimap_x(&b) {
					return mouse::Interaction::Text;
				}
			}
		}
		mouse::Interaction::default()
	}
}

impl<'a, Message: Clone + 'a> From<SqlEditor<'a, Message>> for Element<'a, Message> {
	fn from(e: SqlEditor<'a, Message>) -> Self {
		Self::new(e)
	}
}

// ─── Drawing helpers ──────────────────────────────────────────────────────────

fn fill(r: &mut Renderer, rect: Rectangle, color: Color) {
	r.fill_quad(
		renderer::Quad {
			bounds: rect,
			border: iced::Border::default(),
			shadow: iced::Shadow::default(),
			snap: true,
		},
		color,
	);
}

fn fill_r(r: &mut Renderer, rect: Rectangle, color: Color, radius: f32) {
	r.fill_quad(
		renderer::Quad {
			bounds: rect,
			border: iced::Border {
				radius: radius.into(),
				..Default::default()
			},
			shadow: iced::Shadow::default(),
			snap: true,
		},
		color,
	);
}

fn draw_text(r: &mut Renderer, content: &str, x: f32, y: f32, color: Color, max_w: f32) {
	let text_y = y + (LINE_H - FONT_SZ) / 2.0;
	r.fill_text(
		iced::advanced::text::Text {
			content: content.to_string().into(),
			bounds: Size::new(max_w, FONT_SZ),
			size: Pixels(FONT_SZ),
			line_height: iced::advanced::text::LineHeight::Relative(1.0),
			font: EDITOR_FONT,
			align_x: iced::advanced::text::Alignment::Left,
			align_y: iced::alignment::Vertical::Top,
			shaping: iced::advanced::text::Shaping::Basic,
			wrapping: iced::advanced::text::Wrapping::None,
		},
		Point::new(x, text_y),
		color,
		Rectangle {
			x,
			y,
			width: max_w,
			height: LINE_H,
		},
	);
}

fn token_color(kind: &TokenKind, th: &EditorTheme) -> Color {
	match kind {
		TokenKind::Keyword => th.keyword,
		TokenKind::Type => th.type_name,
		TokenKind::String => th.string,
		TokenKind::Number => th.number,
		TokenKind::Comment => th.comment,
		TokenKind::Operator => th.operator,
		TokenKind::Punctuation => th.punctuation,
		TokenKind::Identifier => th.identifier,
		TokenKind::Function => th.function,
		TokenKind::Macro => th.macro_color,
		TokenKind::Attribute => th.attribute,
		TokenKind::Lifetime => th.lifetime,
		TokenKind::Error => th.error_underline,
		TokenKind::Plain => th.plain,
	}
}

// ─── Public helpers ───────────────────────────────────────────────────────────

pub fn pixel_to_pos(
	buf: &Buffer,
	bounds: &Rectangle,
	gutter_w: f32,
	scroll_x: f32,
	scroll_y: f32,
	px: f32,
	py: f32,
) -> CursorPos {
	let ry = py - bounds.y - TOP_PAD + scroll_y;
	let vl_idx =
		((ry / LINE_H).floor().max(0.0) as usize).min(buf.visual_lines.len().saturating_sub(1));
	if let Some(vl) = buf.visual_lines.get(vl_idx) {
		let lt = buf.line_text(vl.doc_line);
		let vl_vcol_off = buffer::visual_col_of(&lt, vl.col_start);
		let rx = px - bounds.x - gutter_w - LEFT_PAD + scroll_x;
		let vcol = (rx / CHAR_W).round().max(0.0) as usize + vl_vcol_off;
		let logical = buffer::logical_col_of(&lt, vcol);
		buf.click_to_pos(vl.doc_line, logical)
	} else {
		CursorPos::new(buf.line_count().saturating_sub(1), 0)
	}
}

pub fn gutter_width(line_count: usize) -> f32 {
	format!("{}", line_count).len().max(3) as f32 * CHAR_W + GUTTER_PAD * 2.0 + FOLD_COL_W
}

pub fn visible_line_count(h: f32) -> usize {
	((h - TOP_PAD) / LINE_H).ceil() as usize
}
pub const fn line_height() -> f32 {
	LINE_H
}
pub const fn top_pad() -> f32 {
	TOP_PAD
}
pub const fn left_pad() -> f32 {
	LEFT_PAD
}
pub const fn scrollbar_width() -> f32 {
	SCROLL_W
}
pub const fn minimap_width() -> f32 {
	MINIMAP_W
}
pub const fn search_panel_height() -> f32 {
	SEARCH_PANEL_H
}
