use iced::Color;

#[derive(Debug, Clone)]
pub struct EditorTheme {
    pub background: Color,
    pub gutter_bg: Color,
    pub gutter_text: Color,
    pub gutter_active_text: Color,
    pub gutter_border: Color,

    pub cursor: Color,
    pub selection: Color,
    pub current_line_bg: Color,

    // Syntax (shared)
    pub keyword: Color,
    pub type_name: Color,
    pub string: Color,
    pub number: Color,
    pub comment: Color,
    pub operator: Color,
    pub punctuation: Color,
    pub identifier: Color,
    pub function: Color,
    pub plain: Color,

    // Rust-specific
    pub macro_color: Color,
    pub attribute: Color,
    pub lifetime: Color,

    // Bracket match
    pub bracket_match_bg: Color,
    pub bracket_match_border: Color,

    // Indent guides
    pub indent_guide: Color,
    pub indent_guide_active: Color,

    // Diagnostics
    pub error_underline: Color,
    pub error_gutter_marker: Color,

    // Tooltip
    pub tooltip_bg: Color,
    pub tooltip_border: Color,
    pub tooltip_text: Color,

    // Scrollbar
    pub scrollbar_track: Color,
    pub scrollbar_thumb: Color,
    pub scrollbar_thumb_hover: Color,

    // Search highlights
    pub search_match_bg: Color,
    pub search_current_bg: Color,
    pub search_panel_bg: Color,

    // Fold gutter
    pub fold_indicator: Color,
    pub fold_indicator_hover: Color,
    pub fold_collapsed_bg: Color,

    // Minimap
    pub minimap_bg: Color,
    pub minimap_viewport: Color,
    pub minimap_text: Color,
}

impl EditorTheme {
    pub fn dark() -> Self {
        Self {
            background: Color::from_rgb(0.11, 0.12, 0.14),
            gutter_bg: Color::from_rgb(0.09, 0.10, 0.12),
            gutter_text: Color::from_rgb(0.40, 0.42, 0.46),
            gutter_active_text: Color::from_rgb(0.75, 0.78, 0.82),
            gutter_border: Color::from_rgb(0.18, 0.19, 0.22),

            cursor: Color::from_rgb(0.90, 0.92, 0.95),
            selection: Color::from_rgba(0.26, 0.42, 0.68, 0.45),
            current_line_bg: Color::from_rgba(1.0, 1.0, 1.0, 0.03),

            keyword: Color::from_rgb(0.77, 0.55, 0.96),
            type_name: Color::from_rgb(0.31, 0.79, 0.77),
            string: Color::from_rgb(0.80, 0.90, 0.48),
            number: Color::from_rgb(0.95, 0.68, 0.38),
            comment: Color::from_rgb(0.42, 0.46, 0.50),
            operator: Color::from_rgb(0.56, 0.78, 1.0),
            punctuation: Color::from_rgb(0.60, 0.62, 0.66),
            identifier: Color::from_rgb(0.90, 0.92, 0.95),
            function: Color::from_rgb(0.38, 0.75, 1.0),
            plain: Color::from_rgb(0.82, 0.84, 0.88),

            macro_color: Color::from_rgb(0.95, 0.75, 0.40),
            attribute: Color::from_rgb(0.68, 0.85, 0.45),
            lifetime: Color::from_rgb(1.0, 0.60, 0.60),

            bracket_match_bg: Color::from_rgba(0.40, 0.55, 0.80, 0.25),
            bracket_match_border: Color::from_rgba(0.55, 0.70, 1.0, 0.60),

            indent_guide: Color::from_rgba(1.0, 1.0, 1.0, 0.06),
            indent_guide_active: Color::from_rgba(1.0, 1.0, 1.0, 0.12),

            error_underline: Color::from_rgb(1.0, 0.35, 0.35),
            error_gutter_marker: Color::from_rgb(1.0, 0.35, 0.35),

            tooltip_bg: Color::from_rgb(0.16, 0.17, 0.20),
            tooltip_border: Color::from_rgb(0.28, 0.30, 0.34),
            tooltip_text: Color::from_rgb(0.85, 0.87, 0.90),

            scrollbar_track: Color::from_rgba(1.0, 1.0, 1.0, 0.02),
            scrollbar_thumb: Color::from_rgba(1.0, 1.0, 1.0, 0.10),
            scrollbar_thumb_hover: Color::from_rgba(1.0, 1.0, 1.0, 0.18),

            search_match_bg: Color::from_rgba(0.90, 0.75, 0.20, 0.30),
            search_current_bg: Color::from_rgba(0.90, 0.75, 0.20, 0.65),
            search_panel_bg: Color::from_rgb(0.14, 0.15, 0.18),

            fold_indicator: Color::from_rgb(0.45, 0.48, 0.52),
            fold_indicator_hover: Color::from_rgb(0.70, 0.73, 0.78),
            fold_collapsed_bg: Color::from_rgba(0.40, 0.55, 0.80, 0.10),

            minimap_bg: Color::from_rgb(0.09, 0.10, 0.12),
            minimap_viewport: Color::from_rgba(1.0, 1.0, 1.0, 0.08),
            minimap_text: Color::from_rgba(1.0, 1.0, 1.0, 0.25),
        }
    }

    pub fn light() -> Self {
        Self {
            background: Color::from_rgb(0.98, 0.98, 0.99),
            gutter_bg: Color::from_rgb(0.94, 0.95, 0.96),
            gutter_text: Color::from_rgb(0.60, 0.62, 0.66),
            gutter_active_text: Color::from_rgb(0.20, 0.22, 0.26),
            gutter_border: Color::from_rgb(0.86, 0.88, 0.90),

            cursor: Color::from_rgb(0.05, 0.05, 0.10),
            selection: Color::from_rgba(0.26, 0.52, 0.96, 0.25),
            current_line_bg: Color::from_rgba(0.0, 0.0, 0.0, 0.03),

            keyword: Color::from_rgb(0.55, 0.15, 0.82),
            type_name: Color::from_rgb(0.0, 0.55, 0.55),
            string: Color::from_rgb(0.16, 0.55, 0.16),
            number: Color::from_rgb(0.80, 0.45, 0.10),
            comment: Color::from_rgb(0.55, 0.58, 0.62),
            operator: Color::from_rgb(0.10, 0.35, 0.70),
            punctuation: Color::from_rgb(0.40, 0.42, 0.46),
            identifier: Color::from_rgb(0.10, 0.10, 0.15),
            function: Color::from_rgb(0.10, 0.45, 0.75),
            plain: Color::from_rgb(0.15, 0.15, 0.20),

            macro_color: Color::from_rgb(0.65, 0.45, 0.05),
            attribute: Color::from_rgb(0.30, 0.55, 0.15),
            lifetime: Color::from_rgb(0.80, 0.30, 0.30),

            bracket_match_bg: Color::from_rgba(0.20, 0.40, 0.80, 0.15),
            bracket_match_border: Color::from_rgba(0.20, 0.40, 0.80, 0.50),

            indent_guide: Color::from_rgba(0.0, 0.0, 0.0, 0.06),
            indent_guide_active: Color::from_rgba(0.0, 0.0, 0.0, 0.14),

            error_underline: Color::from_rgb(0.90, 0.15, 0.15),
            error_gutter_marker: Color::from_rgb(0.90, 0.15, 0.15),

            tooltip_bg: Color::from_rgb(0.96, 0.96, 0.97),
            tooltip_border: Color::from_rgb(0.82, 0.84, 0.86),
            tooltip_text: Color::from_rgb(0.15, 0.15, 0.20),

            scrollbar_track: Color::from_rgba(0.0, 0.0, 0.0, 0.02),
            scrollbar_thumb: Color::from_rgba(0.0, 0.0, 0.0, 0.12),
            scrollbar_thumb_hover: Color::from_rgba(0.0, 0.0, 0.0, 0.22),

            search_match_bg: Color::from_rgba(1.0, 0.85, 0.20, 0.30),
            search_current_bg: Color::from_rgba(1.0, 0.85, 0.20, 0.60),
            search_panel_bg: Color::from_rgb(0.92, 0.93, 0.94),

            fold_indicator: Color::from_rgb(0.50, 0.52, 0.56),
            fold_indicator_hover: Color::from_rgb(0.25, 0.28, 0.32),
            fold_collapsed_bg: Color::from_rgba(0.20, 0.40, 0.80, 0.06),

            minimap_bg: Color::from_rgb(0.94, 0.95, 0.96),
            minimap_viewport: Color::from_rgba(0.0, 0.0, 0.0, 0.06),
            minimap_text: Color::from_rgba(0.0, 0.0, 0.0, 0.20),
        }
    }
}
