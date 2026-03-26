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
const FOLD_COL_W: f32 = 16.0; // fold indicator column width
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
            buffer, theme,
            on_action: Box::new(on_action),
            scroll_y: 0.0, scroll_x: 0.0,
            show_minimap: true,
            block_cursor: false,
            show_whitespace: true,
            visual_block: None,
        }
    }

    pub fn scroll_y(mut self, v: f32) -> Self { self.scroll_y = v; self }
    pub fn scroll_x(mut self, v: f32) -> Self { self.scroll_x = v; self }
    pub fn show_minimap(mut self, v: bool) -> Self { self.show_minimap = v; self }
    pub fn block_cursor(mut self, v: bool) -> Self { self.block_cursor = v; self }
    pub fn show_whitespace(mut self, v: bool) -> Self { self.show_whitespace = v; self }
    pub fn visual_block(mut self, v: Option<(usize, usize, usize, usize)>) -> Self { self.visual_block = v; self }

    fn gutter_w(&self) -> f32 {
        let d = format!("{}", self.buffer.line_count()).len().max(3) as f32;
        d * CHAR_W + GUTTER_PAD * 2.0 + FOLD_COL_W
    }

    fn text_x(&self) -> f32 { self.gutter_w() + LEFT_PAD }
    fn minimap_x(&self, bounds: &Rectangle) -> f32 {
        bounds.x + bounds.width - if self.show_minimap { MINIMAP_W + SCROLL_W } else { SCROLL_W }
    }
    fn pixel_to_pos(&self, bounds: &Rectangle, px: f32, py: f32) -> CursorPos {
        let rx = px - bounds.x - self.text_x() + self.scroll_x;
        let ry = py - bounds.y - TOP_PAD + self.scroll_y;
        let line = ((ry / LINE_H).floor().max(0.0) as usize)
            .min(self.buffer.line_count().saturating_sub(1));
        let vcol = (rx / CHAR_W).round().max(0.0) as usize;
        let lt = self.buffer.line_text(line);
        let logical = buffer::logical_col_of(&lt, vcol);
        self.buffer.click_to_pos(line, logical)
    }
}

// ─── State ────────────────────────────────────────────────────────────────────

pub struct EditorState {
    pub is_focused: bool,
    is_dragging: bool,
    last_click: std::time::Instant,
    click_count: u32,
    hover_diag: Option<usize>,
}

impl Default for EditorState {
    fn default() -> Self {
        Self {
            is_focused: false, is_dragging: false,
            last_click: std::time::Instant::now(), click_count: 0,
            hover_diag: None,
        }
    }
}

// ─── Widget impl ──────────────────────────────────────────────────────────────

impl<'a, Message: Clone> Widget<Message, Theme, Renderer> for SqlEditor<'a, Message> {
    fn tag(&self) -> widget::tree::Tag { widget::tree::Tag::of::<EditorState>() }
    fn state(&self) -> widget::tree::State { widget::tree::State::new(EditorState::default()) }
    fn size(&self) -> Size<Length> { Size { width: Length::Fill, height: Length::Fill } }

    fn layout(&mut self, _t: &mut widget::Tree, _r: &Renderer, lim: &layout::Limits) -> layout::Node {
        layout::Node::new(lim.width(Length::Fill).height(Length::Fill).max())
    }

    fn draw(
        &self, tree: &widget::Tree, renderer: &mut Renderer,
        _theme: &Theme, _style: &renderer::Style,
        layout: Layout<'_>, _cursor: mouse::Cursor, _vp: &Rectangle,
    ) {
        let b = layout.bounds();
        let st = tree.state.downcast_ref::<EditorState>();
        let th = self.theme;
        let gw = self.gutter_w();
        let tx = self.text_x();

        renderer.start_layer(b);
        {
            // Background
            fill(renderer, b, th.background);
            // Gutter
            fill(renderer,Rectangle { x: b.x, y: b.y, width: gw, height: b.height }, th.gutter_bg);
            fill(renderer,Rectangle { x: b.x + gw - 1.0, y: b.y, width: 1.0, height: b.height }, th.gutter_border);

            // Visible lines
            let editor_h = b.height - if self.buffer.search.is_open { SEARCH_PANEL_H } else { 0.0 };
            let vis = (editor_h / LINE_H).ceil() as usize + 2;
            let first = (self.scroll_y / LINE_H).floor() as usize;
            let last = (first + vis).min(self.buffer.line_count());
            let active = self.buffer.selection.head.line;

            for li in first..last {
                if self.buffer.folds.is_hidden(li) { continue; }
                let y = b.y + TOP_PAD + (li as f32 * LINE_H) - self.scroll_y;
                if y + LINE_H < b.y || y > b.y + editor_h { continue; }

                // Current line
                if li == active && st.is_focused {
                    fill(renderer,Rectangle { x: b.x + gw, y, width: b.width - gw - SCROLL_W - if self.show_minimap { MINIMAP_W } else { 0.0 }, height: LINE_H }, th.current_line_bg);
                }

                // Indent guides (visual cols returned by indent_guides)
                for &vcol in &self.buffer.indent_guides(li) {
                    // Guide at start of this indent level = vcol - TAB_WIDTH
                    let gx = b.x + tx + ((vcol.saturating_sub(TAB_WIDTH)) as f32 * CHAR_W) - self.scroll_x;
                    let c = if li == active { th.indent_guide_active } else { th.indent_guide };
                    fill(renderer,Rectangle { x: gx, y, width: INDENT_W, height: LINE_H }, c);
                }

                // Line number
                let num = format!("{}", li + 1);
                let nc = if li == active { th.gutter_active_text } else { th.gutter_text };
                let nx = b.x + gw - FOLD_COL_W - GUTTER_PAD - (num.len() as f32 * CHAR_W);
                draw_text(renderer,&num, nx, y, nc, gw);

                // Fold indicator
                if self.buffer.folds.is_foldable(li) {
                    let fx = b.x + gw - FOLD_COL_W + 2.0;
                    let _fy = y + LINE_H / 2.0 - 5.0;
                    let collapsed = self.buffer.folds.is_collapsed_start(li);
                    let sym = if collapsed { "▶" } else { "▼" };
                    draw_text(renderer,sym, fx, y, th.fold_indicator, FOLD_COL_W);
                }

                // Collapsed indicator background
                if self.buffer.folds.is_collapsed_start(li) {
                    fill(renderer,Rectangle { x: b.x + gw, y, width: b.width - gw, height: LINE_H }, th.fold_collapsed_bg);
                }

                // Error gutter dot
                if self.buffer.diagnostics.iter().any(|d| d.line == li) {
                    fill_r(renderer,Rectangle { x: b.x + 4.0, y: y + LINE_H / 2.0 - 3.0, width: 6.0, height: 6.0 }, th.error_gutter_marker, 3.0);
                }

                // Search match highlights
                if self.buffer.search.is_open {
                    let slt = self.buffer.line_text(li);
                    for (i, m) in self.buffer.search.matches.iter().enumerate() {
                        if m.line == li {
                            let mvs = buffer::visual_col_of(&slt, m.col_start);
                            let mve = buffer::visual_col_of(&slt, m.col_end);
                            let mx = b.x + tx + (mvs as f32 * CHAR_W) - self.scroll_x;
                            let mw = ((mve - mvs) as f32 * CHAR_W).max(CHAR_W);
                            let c = if i == self.buffer.search.current_match { th.search_current_bg } else { th.search_match_bg };
                            fill(renderer,Rectangle { x: mx, y, width: mw, height: LINE_H }, c);
                        }
                    }
                }

                // Selection
                if let Some((top, bottom, left_col, right_col)) = self.visual_block {
                    if li >= top && li <= bottom {
                        let lt_sel = self.buffer.line_text(li);
                        let vcs = buffer::visual_col_of(&lt_sel, left_col);
                        let vce = buffer::visual_col_of(&lt_sel, right_col + 1);
                        let sx = b.x + tx + (vcs as f32 * CHAR_W) - self.scroll_x;
                        let sw = ((vce - vcs) as f32 * CHAR_W).max(CHAR_W * 0.5);
                        fill(renderer, Rectangle { x: sx, y, width: sw, height: LINE_H }, th.selection);
                    }
                } else if !self.buffer.selection.is_caret() {
                    let (ss, se) = self.buffer.selection.ordered();
                    if li >= ss.line && li <= se.line {
                        let lt_sel = self.buffer.line_text(li);
                        let vcs = buffer::visual_col_of(&lt_sel, if li == ss.line { ss.col } else { 0 });
                        let vce = if li == se.line {
                            buffer::visual_col_of(&lt_sel, se.col)
                        } else {
                            // Extend to end of line + 1 to show selection on blank lines too
                            buffer::visual_col_of(&lt_sel, lt_sel.chars().count()) + 1
                        };
                        let sx = b.x + tx + (vcs as f32 * CHAR_W) - self.scroll_x;
                        let sw = ((vce - vcs) as f32 * CHAR_W).max(CHAR_W * 0.5);
                        fill(renderer,Rectangle { x: sx, y, width: sw, height: LINE_H }, th.selection);
                    }
                }

                // Bracket match
                if let Some(ref bm) = self.buffer.matched_bracket {
                    for &(bl, bc) in &[(bm.open_line, bm.open_col), (bm.close_line, bm.close_col)] {
                        if bl == li {
                            let blt = self.buffer.line_text(bl);
                            let bvcol = buffer::visual_col_of(&blt, bc);
                            let bx = b.x + tx + (bvcol as f32 * CHAR_W) - self.scroll_x;
                            fill(renderer,Rectangle { x: bx, y, width: CHAR_W, height: LINE_H }, th.bracket_match_bg);
                            for rect in [
                                Rectangle { x: bx, y, width: CHAR_W, height: BRACKET_BW },
                                Rectangle { x: bx, y: y + LINE_H - BRACKET_BW, width: CHAR_W, height: BRACKET_BW },
                                Rectangle { x: bx, y, width: BRACKET_BW, height: LINE_H },
                                Rectangle { x: bx + CHAR_W - BRACKET_BW, y, width: BRACKET_BW, height: LINE_H },
                            ] { fill(renderer,rect, th.bracket_match_border); }
                        }
                    }
                }

                // Syntax tokens (tab-aware rendering + whitespace glyphs)
                let lt = self.buffer.line_text(li);
                let lbs = self.buffer.rope.line_to_byte(li);
                let lbe = lbs + lt.len();
                let mut spans: Vec<(usize, usize, TokenKind)> = Vec::new();
                for tok in self.buffer.tokens() {
                    if tok.byte_range.end <= lbs || tok.byte_range.start >= lbe { continue; }
                    let s = tok.byte_range.start.max(lbs) - lbs;
                    let e = tok.byte_range.end.min(lbe) - lbs;
                    spans.push((s, e, tok.kind));
                }
                spans.sort_by_key(|s| s.0);
                let mut render: Vec<(usize, usize, TokenKind)> = Vec::new();
                let mut cur = 0;
                for &(s, e, k) in &spans {
                    if s > cur { render.push((cur, s, TokenKind::Plain)); }
                    render.push((s, e, k));
                    cur = e;
                }
                if cur < lt.len() { render.push((cur, lt.len(), TokenKind::Plain)); }

                // Whitespace indicator color: dimmed version of gutter text
                let ws_color = Color { a: 0.35, ..th.gutter_text };

                // Trailing whitespace start (for ~ indicators)
                let trail_start = lt.trim_end().len(); // byte offset where trailing whitespace begins

                for &(start, end, kind) in &render {
                    if start >= lt.len() { break; }
                    let sl = &lt[start..end.min(lt.len())];
                    if sl.is_empty() { continue; }
                    let color = token_color(&kind, th);

                    // Walk char-by-char to expand tabs and draw whitespace glyphs
                    let mut vcol = buffer::visual_col_of(&lt, start);
                    let mut seg = String::new();
                    let mut seg_vcol = vcol;
                    let mut byte_pos = start;

                    for ch in sl.chars() {
                        if ch == '\t' {
                            // Flush any accumulated text segment
                            if !seg.is_empty() {
                                let px = b.x + tx + (seg_vcol as f32 * CHAR_W) - self.scroll_x;
                                draw_text(renderer, &seg, px, y, color, b.width);
                                seg.clear();
                            }
                            // Draw tab glyph at tab position
                            if self.show_whitespace {
                                let px = b.x + tx + (vcol as f32 * CHAR_W) - self.scroll_x;
                                draw_text(renderer, "▸", px, y, ws_color, CHAR_W);
                            }
                            // Advance to next tabstop
                            vcol = (vcol / TAB_WIDTH + 1) * TAB_WIDTH;
                            seg_vcol = vcol;
                        } else if self.show_whitespace && ch == ' ' {
                            // Flush accumulated text before drawing whitespace glyph
                            if !seg.is_empty() {
                                let px = b.x + tx + (seg_vcol as f32 * CHAR_W) - self.scroll_x;
                                draw_text(renderer, &seg, px, y, color, b.width);
                                seg.clear();
                            }
                            let glyph = if byte_pos >= trail_start { "~" } else { "␣" };
                            let px = b.x + tx + (vcol as f32 * CHAR_W) - self.scroll_x;
                            draw_text(renderer, glyph, px, y, ws_color, CHAR_W);
                            vcol += 1;
                            seg_vcol = vcol;
                        } else {
                            seg.push(ch);
                            vcol += 1;
                        }
                        byte_pos += ch.len_utf8();
                    }
                    // Flush remaining segment
                    if !seg.is_empty() {
                        let px = b.x + tx + (seg_vcol as f32 * CHAR_W) - self.scroll_x;
                        draw_text(renderer, &seg, px, y, color, b.width);
                    }
                }

                // EOL marker ¬
                if self.show_whitespace {
                    let eol_vcol = buffer::visual_col_of(&lt, lt.chars().count());
                    let px = b.x + tx + (eol_vcol as f32 * CHAR_W) - self.scroll_x;
                    draw_text(renderer, "¬", px, y, Color { a: 0.18, ..th.gutter_text }, CHAR_W);
                }

                // Collapsed line count badge
                if self.buffer.folds.is_collapsed_start(li) {
                    let hc = self.buffer.folds.hidden_count(li);
                    if hc > 0 {
                        let badge = format!(" ⋯ {} lines", hc);
                        let eol_vcol = buffer::visual_col_of(&lt, lt.chars().count());
                        let bx = b.x + tx + (eol_vcol as f32 * CHAR_W) + 8.0 - self.scroll_x;
                        draw_text(renderer, &badge, bx, y, th.comment, 200.0);
                    }
                }

                // Error squiggles (visual col positions)
                for diag in &self.buffer.diagnostics {
                    if diag.line == li {
                        let uvs = buffer::visual_col_of(&lt, diag.col_start);
                        let uve = buffer::visual_col_of(&lt, diag.col_end);
                        let ux = b.x + tx + (uvs as f32 * CHAR_W) - self.scroll_x;
                        let uw = ((uve - uvs) as f32 * CHAR_W).max(CHAR_W);
                        let uy = y + LINE_H - ERR_THICK - 1.0;
                        let seg: f32 = 4.0;
                        let mut sx = ux;
                        let mut up = true;
                        while sx < ux + uw {
                            fill(renderer,Rectangle { x: sx, y: if up { uy - 1.0 } else { uy + 1.0 }, width: seg.min(ux + uw - sx), height: ERR_THICK }, th.error_underline);
                            sx += seg;
                            up = !up;
                        }
                    }
                }
            }

            // Cursor
            if st.is_focused {
                let cl = self.buffer.selection.head.line;
                let cc = self.buffer.selection.head.col;
                let clt = self.buffer.line_text(cl);
                let cvcol = buffer::visual_col_of(&clt, cc);
                let cy = b.y + TOP_PAD + (cl as f32 * LINE_H) - self.scroll_y;
                let cx = b.x + tx + (cvcol as f32 * CHAR_W) - self.scroll_x;
                if cy > b.y - LINE_H && cy < b.y + editor_h {
                    if self.block_cursor {
                        // Normal mode: full-width semi-transparent block so the char shows through
                        fill(renderer, Rectangle { x: cx, y: cy, width: CHAR_W, height: LINE_H },
                             Color { a: 0.55, ..th.cursor });
                    } else {
                        fill(renderer, Rectangle { x: cx, y: cy, width: CURSOR_W, height: LINE_H }, th.cursor);
                    }
                }
            }

            // ── Minimap ───────────────────────────────────────────────
            if self.show_minimap {
                let mx = self.minimap_x(&b);
                let mh = editor_h;
                fill(renderer,Rectangle { x: mx, y: b.y, width: MINIMAP_W, height: mh }, th.minimap_bg);

                // Viewport indicator
                let total_h = self.buffer.line_count() as f32 * MINIMAP_LINE_H;
                if total_h > 0.0 {
                    let vp_ratio = self.scroll_y / (self.buffer.line_count() as f32 * LINE_H).max(1.0);
                    let vp_h = (mh / total_h * mh).min(mh).max(20.0);
                    let vp_y = b.y + vp_ratio * (mh - vp_h);
                    fill(renderer,Rectangle { x: mx, y: vp_y, width: MINIMAP_W, height: vp_h }, th.minimap_viewport);
                }

                // Minimap text rendering (simplified color blocks)
                for li in 0..self.buffer.line_count() {
                    if self.buffer.folds.is_hidden(li) { continue; }
                    let my = b.y + li as f32 * MINIMAP_LINE_H;
                    if my > b.y + mh { break; }
                    let lt = self.buffer.line_text(li);
                    if lt.trim().is_empty() { continue; }

                    // Render token-colored blocks
                    let lbs = self.buffer.rope.line_to_byte(li);
                    let lbe = lbs + lt.len();
                    for tok in self.buffer.tokens() {
                        if tok.byte_range.end <= lbs || tok.byte_range.start >= lbe { continue; }
                        let s = tok.byte_range.start.max(lbs) - lbs;
                        let e = tok.byte_range.end.min(lbe) - lbs;
                        let tw = (e - s) as f32 * MINIMAP_CHAR_W;
                        let tkx = mx + 4.0 + s as f32 * MINIMAP_CHAR_W;
                        if tw > 0.5 {
                            let c = token_color(&tok.kind, th);
                            // Dim the color for minimap
                            let dim = Color::from_rgba(c.r, c.g, c.b, 0.35);
                            fill(renderer,Rectangle { x: tkx, y: my, width: tw.min(MINIMAP_W - 8.0), height: MINIMAP_LINE_H }, dim);
                        }
                    }
                }
            }

            // ── Scrollbar ─────────────────────────────────────────────
            let sb_x = b.x + b.width - SCROLL_W;
            fill(renderer,Rectangle { x: sb_x, y: b.y, width: SCROLL_W, height: editor_h }, th.scrollbar_track);
            let total = self.buffer.line_count() as f32 * LINE_H + TOP_PAD * 2.0;
            if total > editor_h {
                let ratio = editor_h / total;
                let th_h = (ratio * editor_h).max(24.0);
                let max_sc = total - editor_h;
                let th_y = b.y + (self.scroll_y / max_sc) * (editor_h - th_h);
                fill_r(renderer,Rectangle { x: sb_x + 2.0, y: th_y, width: SCROLL_W - 4.0, height: th_h }, th.scrollbar_thumb, 3.0);
            }

            // ── Search panel ──────────────────────────────────────────
            if self.buffer.search.is_open {
                let sp_y = b.y + b.height - SEARCH_PANEL_H;
                fill(renderer,Rectangle { x: b.x, y: sp_y, width: b.width, height: SEARCH_PANEL_H }, th.search_panel_bg);
                // Separator line
                fill(renderer,Rectangle { x: b.x, y: sp_y, width: b.width, height: 1.0 }, th.gutter_border);

                let info = format!(
                    "Find: \"{}\"  {} of {}   Replace: \"{}\"   [Enter=next] [Shift+Enter=prev] [Ctrl+Shift+H=replace] [Ctrl+Shift+Enter=all]",
                    self.buffer.search.query,
                    if self.buffer.search.matches.is_empty() { 0 } else { self.buffer.search.current_match + 1 },
                    self.buffer.search.match_count(),
                    self.buffer.search.replacement,
                );
                draw_text(renderer,&info, b.x + 12.0, sp_y + 4.0, th.tooltip_text, b.width - 24.0);
            }

            // ── Diagnostic tooltip ────────────────────────────────────
            if let Some(di) = st.hover_diag {
                if let Some(diag) = self.buffer.diagnostics.get(di) {
                    let dy = b.y + TOP_PAD + (diag.line as f32 * LINE_H) - self.scroll_y + LINE_H + 4.0;
                    let dx = b.x + tx + (diag.col_start as f32 * CHAR_W) - self.scroll_x;
                    let tw = (diag.message.len() as f32 * CHAR_W * 0.62).min(400.0).max(150.0);
                    let th2 = 28.0;
                    fill_r(renderer,Rectangle { x: dx + 1.0, y: dy + 1.0, width: tw, height: th2 }, Color::from_rgba(0.0, 0.0, 0.0, 0.25), 4.0);
                    fill_r(renderer,Rectangle { x: dx, y: dy, width: tw, height: th2 }, th.tooltip_bg, 4.0);
                    for rect in [
                        Rectangle { x: dx, y: dy, width: tw, height: 1.0 },
                        Rectangle { x: dx, y: dy + th2 - 1.0, width: tw, height: 1.0 },
                        Rectangle { x: dx, y: dy, width: 1.0, height: th2 },
                        Rectangle { x: dx + tw - 1.0, y: dy, width: 1.0, height: th2 },
                    ] { fill(renderer,rect, th.tooltip_border); }
                    let msg = if diag.message.len() > 55 { format!("{}…", &diag.message[..54]) } else { diag.message.clone() };
                    draw_text(renderer, &msg, dx + 8.0, dy, th.tooltip_text, tw - 16.0);
                }
            }
        }
        renderer.end_layer();
    }

    fn update(
        &mut self, tree: &mut widget::Tree, event: &Event,
        layout: Layout<'_>, cursor: mouse::Cursor,
        _r: &Renderer, _clip: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>, _vp: &Rectangle,
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

                    // Check fold gutter click
                    let gw = self.gutter_w();
                    if pos.x >= b.x + gw - FOLD_COL_W && pos.x <= b.x + gw {
                        // fold toggle handled by app
                        shell.publish((self.on_action)(EditorAction::CursorMoved));
                        shell.capture_event();
                        return;
                    }

                    st.is_dragging = true;
                    shell.publish((self.on_action)(EditorAction::MouseDown(pos)));
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
                if st.is_dragging { shell.capture_event(); return; }
            }
            Event::Keyboard(keyboard::Event::KeyPressed { .. }) if st.is_focused => {
                shell.capture_event(); return;
            }
            _ => {}
        }
    }

    fn mouse_interaction(
        &self, _t: &widget::Tree, layout: Layout<'_>,
        cursor: mouse::Cursor, _vp: &Rectangle, _r: &Renderer,
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
    fn from(e: SqlEditor<'a, Message>) -> Self { Self::new(e) }
}

// ─── Drawing helpers ──────────────────────────────────────────────────────────

fn fill(r: &mut Renderer, rect: Rectangle, color: Color) {
    r.fill_quad(renderer::Quad { bounds: rect, border: iced::Border::default(), shadow: iced::Shadow::default(), snap: true }, color);
}

fn fill_r(r: &mut Renderer, rect: Rectangle, color: Color, radius: f32) {
    r.fill_quad(renderer::Quad { bounds: rect, border: iced::Border { radius: radius.into(), ..Default::default() }, shadow: iced::Shadow::default(), snap: true }, color);
}

fn draw_text(r: &mut Renderer, content: &str, x: f32, y: f32, color: Color, max_w: f32) {
    // Offset by half the leading so glyphs are visually centered within LINE_H.
    // align_y: Top means position.y is the top of the glyph box — no ambiguity.
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
        Point::new(x, text_y), color,
        Rectangle { x, y, width: max_w, height: LINE_H },
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
    buf: &Buffer, bounds: &Rectangle, gutter_w: f32,
    scroll_x: f32, scroll_y: f32, px: f32, py: f32,
) -> CursorPos {
    let rx = px - bounds.x - gutter_w - LEFT_PAD + scroll_x;
    let ry = py - bounds.y - TOP_PAD + scroll_y;
    let line = ((ry / LINE_H).floor().max(0.0) as usize).min(buf.line_count().saturating_sub(1));
    let vcol = (rx / CHAR_W).round().max(0.0) as usize;
    let lt = buf.line_text(line);
    let logical = buffer::logical_col_of(&lt, vcol);
    buf.click_to_pos(line, logical)
}

pub fn gutter_width(line_count: usize) -> f32 {
    format!("{}", line_count).len().max(3) as f32 * CHAR_W + GUTTER_PAD * 2.0 + FOLD_COL_W
}

pub fn visible_line_count(h: f32) -> usize { ((h - TOP_PAD) / LINE_H).ceil() as usize }
pub const fn line_height() -> f32 { LINE_H }
pub const fn top_pad() -> f32 { TOP_PAD }
pub const fn scrollbar_width() -> f32 { SCROLL_W }
pub const fn minimap_width() -> f32 { MINIMAP_W }
pub const fn search_panel_height() -> f32 { SEARCH_PANEL_H }
