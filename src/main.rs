use iced::keyboard::{self, Key};
use iced::widget::{column, container, row, text, Space};
use iced::{event, Element, Length, Subscription, Task, Theme};

use editon::buffer::{Buffer, CursorPos, Selection, UndoConfig};
use editon::highlight::SyntaxLanguage;
use editon::theme::EditorTheme;
use editon::widget::{self, EditorAction, SqlEditor};

const DEJAVU_SANS_MONO: &[u8] = include_bytes!("../fonts/DejaVuSansMono.ttf");

fn main() -> iced::Result {
    iced::application(App::new, App::update, App::view)
        .title("Code Editor")
        .subscription(App::subscription)
        .theme(|_: &App| Theme::Dark)
        .window_size((1200.0, 800.0))
        .antialiasing(true)
        .font(DEJAVU_SANS_MONO)
        .run()
}

const SAMPLE_SQL: &str = r#"-- SQL + Rust polyglot editor demo
SELECT
    u.id,
    u.username,
    u.email,
    COUNT(o.id) AS order_count,
    SUM(o.total_amount) AS total_spent
FROM users u
LEFT JOIN orders o ON o.user_id = u.id
WHERE u.is_active = TRUE
    AND u.created_at >= '2024-01-01'
GROUP BY u.id, u.username, u.email
HAVING COUNT(o.id) > 5
ORDER BY total_spent DESC
LIMIT 100;

-- Materialized view
CREATE MATERIALIZED VIEW monthly_revenue AS
SELECT
    DATE_TRUNC('month', o.created_at) AS month,
    p.category,
    SUM(oi.quantity * oi.unit_price) AS revenue
FROM orders o
JOIN order_items oi ON oi.order_id = o.id
JOIN products p ON p.id = oi.product_id
WHERE o.status IN ('completed', 'shipped')
GROUP BY 1, 2
ORDER BY 1 DESC, 3 DESC;

-- Window functions
WITH ranked AS (
    SELECT
        department,
        employee_name,
        salary,
        RANK() OVER (
            PARTITION BY department
            ORDER BY salary DESC
        ) AS rank
    FROM employees
    WHERE salary > 50000
)
SELECT * FROM ranked WHERE rank <= 3;

-- Error for diagnostics
SELEC * FORM broken WHER id = ;
"#;

const SAMPLE_RUST: &str = r#"use std::collections::HashMap;

/// A generic repository trait for data access.
pub trait Repository<T: Clone + Send + Sync> {
    fn find_by_id(&self, id: u64) -> Option<&T>;
    fn find_all(&self) -> Vec<&T>;
    fn insert(&mut self, item: T) -> u64;
    fn delete(&mut self, id: u64) -> bool;
}

#[derive(Debug, Clone)]
pub struct User {
    pub id: u64,
    pub name: String,
    pub email: String,
    pub role: UserRole,
}

#[derive(Debug, Clone, PartialEq)]
pub enum UserRole {
    Admin,
    Editor,
    Viewer,
}

pub struct InMemoryRepo<T: Clone + Send + Sync> {
    store: HashMap<u64, T>,
    next_id: u64,
}

impl<T: Clone + Send + Sync> InMemoryRepo<T> {
    pub fn new() -> Self {
        Self {
            store: HashMap::new(),
            next_id: 1,
        }
    }
}

impl<T: Clone + Send + Sync> Repository<T> for InMemoryRepo<T> {
    fn find_by_id(&self, id: u64) -> Option<&T> {
        self.store.get(&id)
    }

    fn find_all(&self) -> Vec<&T> {
        self.store.values().collect()
    }

    fn insert(&mut self, item: T) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        self.store.insert(id, item);
        id
    }

    fn delete(&mut self, id: u64) -> bool {
        self.store.remove(&id).is_some()
    }
}

// Async handler with lifetime
async fn process_users<'a>(
    repo: &'a dyn Repository<User>,
    filter: impl Fn(&User) -> bool + 'a,
) -> Vec<&'a User> {
    repo.find_all()
        .into_iter()
        .filter(|u| filter(u))
        .collect()
}

macro_rules! create_user {
    ($name:expr, $email:expr, $role:expr) => {
        User {
            id: 0,
            name: $name.to_string(),
            email: $email.to_string(),
            role: $role,
        }
    };
}

fn main() {
    let mut repo = InMemoryRepo::new();

    let users = vec![
        create_user!("Alice", "alice@example.com", UserRole::Admin),
        create_user!("Bob", "bob@example.com", UserRole::Editor),
        create_user!("Charlie", "charlie@example.com", UserRole::Viewer),
    ];

    for user in users {
        let id = repo.insert(user);
        println!("Inserted user with id: {}", id);
    }

    // Find admins
    let admins: Vec<_> = repo
        .find_all()
        .into_iter()
        .filter(|u| u.role == UserRole::Admin)
        .collect();

    println!("Found {} admin(s)", admins.len());
}
"#;

#[derive(Debug, Clone, PartialEq)]
enum VimMode { Normal, Insert, Visual, VisualLine, Command }

struct App {
    buffer: Buffer,
    theme: EditorTheme,
    scroll_y: f32,
    scroll_x: f32,
    status: String,
    bounds_h: f32,
    is_dragging: bool,
    click_count: u32,
    show_minimap: bool,
    show_whitespace: bool,
    vim_mode: VimMode,
    vim_command: String,
    pending_g: bool,
    /// Digit characters accumulated before a motion/operator (e.g. "12" for 12j).
    vim_count: String,
    /// Pending operator: 'd' for `d`, 'y' for `y` (waiting for second char).
    pending_op: Option<char>,
}

#[derive(Debug, Clone)]
enum Msg {
    Action(EditorAction),
    Key(Key, keyboard::Modifiers, Option<String>),
    Scroll(f32, f32),
    MouseMove(iced::Point),
    MouseUp,
}

impl App {
    fn new() -> (Self, Task<Msg>) {
        let undo_cfg = UndoConfig {
            max_history: 1000,
            group_timeout_ms: 600,
        };
        let buffer = Buffer::with_undo_config(SAMPLE_SQL, SyntaxLanguage::Sql, undo_cfg);
        let dc = buffer.diagnostics.len();
        (Self {
            buffer,
            theme: EditorTheme::dark(),
            scroll_y: 0.0, scroll_x: 0.0,
            status: format!("NOR | Ln 1, Col 1 | {} diag", dc),
            bounds_h: 750.0,
            is_dragging: false,
            click_count: 0,
            show_minimap: true,
            show_whitespace: true,
            vim_mode: VimMode::Normal,
            vim_command: String::new(),
            pending_g: false,
            vim_count: String::new(),
            pending_op: None,
        }, Task::none())
    }

    fn subscription(&self) -> Subscription<Msg> {
        event::listen_with(|event, _status, _id| match event {
            iced::Event::Keyboard(keyboard::Event::KeyPressed { key, modifiers, text, .. }) => {
                Some(Msg::Key(key, modifiers, text.map(|t| t.to_string())))
            }
            iced::Event::Mouse(iced::mouse::Event::WheelScrolled { delta }) => {
                let (dx, dy) = match delta {
                    iced::mouse::ScrollDelta::Lines { x, y } => (-x * 40.0, -y * 40.0),
                    iced::mouse::ScrollDelta::Pixels { x, y } => (-x, -y),
                };
                Some(Msg::Scroll(dx, dy))
            }
            iced::Event::Mouse(iced::mouse::Event::CursorMoved { position }) => {
                Some(Msg::MouseMove(position))
            }
            iced::Event::Mouse(iced::mouse::Event::ButtonReleased(iced::mouse::Button::Left)) => {
                Some(Msg::MouseUp)
            }
            _ => None,
        })
    }

    fn update(&mut self, msg: Msg) -> Task<Msg> {
        match msg {
            Msg::Action(EditorAction::MouseDown(pos)) => {
                let cursor_pos = self.pos_from_pixel(pos);
                self.buffer.selection.anchor = cursor_pos;
                self.buffer.selection.head = cursor_pos;
                self.is_dragging = true;
                self.click_count = 1;
                self.update_status();
            }
            Msg::Action(_) => {}

            Msg::MouseMove(pos) => {
                if self.is_dragging && self.click_count == 1 {
                    let target = self.pos_from_pixel(pos);
                    self.buffer.selection.head = target;
                    self.update_status();
                }
            }
            Msg::MouseUp => { self.is_dragging = false; }

            Msg::Key(key, mods, text) => {
                // ── Vim command bar captures all input ────────────────
                if self.vim_mode == VimMode::Command {
                    return self.handle_vim_command_key(key, text);
                }
                // ── Vim normal mode ───────────────────────────────────
                if self.vim_mode == VimMode::Normal {
                    return self.handle_vim_normal_key(key, mods, text);
                }
                // ── Vim visual modes ──────────────────────────────────
                if self.vim_mode == VimMode::Visual || self.vim_mode == VimMode::VisualLine {
                    return self.handle_vim_visual_key(key, mods, text);
                }
                // ── Insert mode: Escape → Normal ──────────────────────
                if matches!(&key, Key::Named(keyboard::key::Named::Escape))
                    && !self.buffer.search.is_open
                {
                    self.vim_mode = VimMode::Normal;
                    // Vim convention: cursor steps back one on leaving insert mode
                    if self.buffer.selection.head.col > 0 {
                        self.buffer.move_left(false);
                    }
                    self.update_status();
                    return Task::none();
                }

                let shift = mods.shift();
                let ctrl = mods.command();

                // Search mode input handling
                if self.buffer.search.is_open {
                    match key {
                        Key::Named(keyboard::key::Named::Escape) => {
                            self.buffer.search_close();
                            self.update_status();
                            return Task::none();
                        }
                        Key::Named(keyboard::key::Named::Enter) if ctrl && shift => {
                            self.buffer.search_replace_all();
                            self.update_status();
                            return Task::none();
                        }
                        Key::Named(keyboard::key::Named::Enter) if shift => {
                            self.buffer.search_prev();
                            self.ensure_cursor_visible();
                            self.update_status();
                            return Task::none();
                        }
                        Key::Named(keyboard::key::Named::Enter) => {
                            self.buffer.search_next();
                            self.ensure_cursor_visible();
                            self.update_status();
                            return Task::none();
                        }
                        _ => {}
                    }
                }

                match key {
                    // ── Language switching ─────────────────────────────
                    Key::Named(keyboard::key::Named::F5) => {
                        let new_lang = match self.buffer.language() {
                            SyntaxLanguage::Sql => SyntaxLanguage::Rust,
                            SyntaxLanguage::Rust => SyntaxLanguage::Sql,
                        };
                        let sample = match new_lang {
                            SyntaxLanguage::Sql => SAMPLE_SQL,
                            SyntaxLanguage::Rust => SAMPLE_RUST,
                        };
                        self.buffer = Buffer::new(sample, new_lang);
                        self.scroll_y = 0.0;
                        self.scroll_x = 0.0;
                    }

                    // ── Search ────────────────────────────────────────
                    Key::Character(ref ch) if ctrl && ch.as_str() == "f" => {
                        self.buffer.search_open();
                    }
                    Key::Character(ref ch) if ctrl && shift && ch.as_str() == "h" => {
                        self.buffer.search_replace_current();
                    }

                    // ── Folding ───────────────────────────────────────
                    Key::Character(ref ch) if ctrl && shift && ch.as_str() == "[" => {
                        let l = self.buffer.selection.head.line;
                        self.buffer.toggle_fold(l);
                    }
                    Key::Character(ref ch) if ctrl && shift && ch.as_str() == "]" => {
                        let l = self.buffer.selection.head.line;
                        self.buffer.toggle_fold(l);
                    }

                    // ── Wrap toggle ───────────────────────────────────
                    Key::Character(ref ch) if ctrl && ch.as_str() == "w" => {
                        let enabled = !self.buffer.wrap_config.enabled;
                        self.buffer.set_wrap(enabled);
                    }

                    // ── Minimap toggle ────────────────────────────────
                    Key::Character(ref ch) if ctrl && ch.as_str() == "m" => {
                        self.show_minimap = !self.show_minimap;
                    }

                    // ── Whitespace visibility toggle ──────────────────
                    Key::Character(ref ch) if ctrl && ch.as_str() == "l" => {
                        self.show_whitespace = !self.show_whitespace;
                    }

                    // ── Navigation ────────────────────────────────────
                    Key::Named(keyboard::key::Named::ArrowLeft) if ctrl => self.buffer.move_word_left(shift),
                    Key::Named(keyboard::key::Named::ArrowRight) if ctrl => self.buffer.move_word_right(shift),
                    Key::Named(keyboard::key::Named::ArrowLeft) => self.buffer.move_left(shift),
                    Key::Named(keyboard::key::Named::ArrowRight) => self.buffer.move_right(shift),
                    Key::Named(keyboard::key::Named::ArrowUp) => self.buffer.move_up(shift),
                    Key::Named(keyboard::key::Named::ArrowDown) => self.buffer.move_down(shift),
                    Key::Named(keyboard::key::Named::Home) if ctrl => self.buffer.move_to_start(shift),
                    Key::Named(keyboard::key::Named::End) if ctrl => self.buffer.move_to_end(shift),
                    Key::Named(keyboard::key::Named::Home) => self.buffer.move_home(shift),
                    Key::Named(keyboard::key::Named::End) => self.buffer.move_end(shift),
                    Key::Named(keyboard::key::Named::PageUp) => {
                        let v = widget::visible_line_count(self.bounds_h);
                        self.buffer.page_up(v, shift);
                    }
                    Key::Named(keyboard::key::Named::PageDown) => {
                        let v = widget::visible_line_count(self.bounds_h);
                        self.buffer.page_down(v, shift);
                    }

                    // ── Editing ───────────────────────────────────────
                    Key::Named(keyboard::key::Named::Backspace) if ctrl => self.buffer.delete_word_back(),
                    Key::Named(keyboard::key::Named::Delete) if ctrl => self.buffer.delete_word_forward(),
                    Key::Named(keyboard::key::Named::Backspace) => self.buffer.backspace(),
                    Key::Named(keyboard::key::Named::Delete) => self.buffer.delete(),
                    Key::Named(keyboard::key::Named::Enter) => self.buffer.insert_newline(),
                    Key::Named(keyboard::key::Named::Tab) if shift => self.buffer.dedent_lines(),
                    Key::Named(keyboard::key::Named::Tab) => {
                        if self.buffer.selection.is_caret() {
                            self.buffer.insert_char('\t');
                        } else {
                            self.buffer.indent_lines();
                        }
                    }

                    Key::Character(ref ch) => {
                        let s = ch.as_str();
                        if ctrl {
                            match s {
                                "a" => self.buffer.select_all(),
                                "z" if shift => self.buffer.redo(),
                                "z" => self.buffer.undo(),
                                "y" => self.buffer.redo(),
                                "d" => self.buffer.duplicate_line(),
                                "c" => {
                                    let copied = self.buffer.copy();
                                    if !copied.is_empty() {
                                        return iced::clipboard::write(copied);
                                    }
                                }
                                "x" => { let _ = self.buffer.cut(); }
                                "v" => {
                                    let clip = self.buffer.clipboard.clone();
                                    if !clip.is_empty() { self.buffer.paste(&clip); }
                                }
                                _ => {}
                            }
                        } else {
                            // Use the event's `text` field so shift, AltGr, etc. are
                            // applied correctly by the platform before we see them.
                            let insert = text.as_deref().unwrap_or(s);
                            for c in insert.chars() {
                                self.buffer.insert_char_auto_pair(c);
                            }
                        }
                    }
                    // Space is Key::Named in iced, not Key::Character
                    Key::Named(keyboard::key::Named::Space) if !mods.command() => {
                        self.buffer.insert_char_auto_pair(' ');
                    }
                    _ => {}
                }
                self.update_status();
                self.ensure_cursor_visible();
            }

            Msg::Scroll(dx, dy) => {
                let sp = if self.buffer.search.is_open { widget::search_panel_height() } else { 0.0 };
                let eh = self.bounds_h - sp;
                let max_y = (self.buffer.line_count() as f32 * widget::line_height() + widget::top_pad() * 2.0 - eh).max(0.0);
                self.scroll_y = (self.scroll_y + dy).clamp(0.0, max_y);
                self.scroll_x = (self.scroll_x + dx).max(0.0);
            }

        }
        Task::none()
    }

    fn view(&'_ self) -> Element<'_, Msg> {
        let editor = SqlEditor::new(&self.buffer, &self.theme, Msg::Action)
            .scroll_y(self.scroll_y)
            .scroll_x(self.scroll_x)
            .show_minimap(self.show_minimap)
            .show_whitespace(self.show_whitespace)
            .block_cursor(self.vim_mode == VimMode::Normal);

        let sc = iced::Color::from_rgb(0.55, 0.58, 0.62);
        let sep = iced::Color::from_rgb(0.35, 0.37, 0.40);
        let lang = self.buffer.language().display_name();
        let wrap_status = if self.buffer.wrap_config.enabled { "Wrap:On" } else { "Wrap:Off" };

        let status_bar = container(
            row![
                text(&self.status).size(13).color(sc),
                Space::new().width(Length::Fill),
                text(wrap_status).size(13).color(sc),
                text("  ·  ").size(13).color(sep),
                text("UTF-8").size(13).color(sc),
                text("  ·  ").size(13).color(sep),
                text(lang).size(13).color(sc),
                text("  ·  ").size(13).color(sep),
                text("F5=lang  C-l=ws").size(11).color(sep),
            ]
            .padding(6).spacing(4),
        )
        .style(|_: &Theme| container::Style {
            background: Some(iced::Background::Color(iced::Color::from_rgb(0.09, 0.10, 0.12))),
            ..Default::default()
        })
        .width(Length::Fill);

        let cmd_bar_color = iced::Color::from_rgb(0.90, 0.92, 0.95);
        let cmd_bar = container(
            row![
                text(":").size(14).color(cmd_bar_color),
                text(&self.vim_command).size(14).color(cmd_bar_color),
                text("█").size(14).color(iced::Color::from_rgba(0.90, 0.92, 0.95, 0.7)),
            ]
            .padding(iced::Padding { top: 4.0, bottom: 4.0, left: 8.0, right: 8.0 })
            .spacing(0),
        )
        .style(|_: &Theme| container::Style {
            background: Some(iced::Background::Color(iced::Color::from_rgb(0.11, 0.12, 0.16))),
            ..Default::default()
        })
        .width(Length::Fill);

        if self.vim_mode == VimMode::Command {
            column![
                container(Element::from(editor)).width(Length::Fill).height(Length::Fill),
                cmd_bar,
                status_bar,
            ].into()
        } else {
            column![
                container(Element::from(editor)).width(Length::Fill).height(Length::Fill),
                status_bar,
            ].into()
        }
    }

    fn pos_from_pixel(&self, pixel: iced::Point) -> CursorPos {
        let gw = widget::gutter_width(self.buffer.line_count());
        let bounds = iced::Rectangle { x: 0.0, y: 0.0, width: 1200.0, height: self.bounds_h };
        widget::pixel_to_pos(&self.buffer, &bounds, gw, self.scroll_x, self.scroll_y, pixel.x, pixel.y)
    }

    fn update_status(&mut self) {
        let mode = match self.vim_mode {
            VimMode::Normal     => "NOR",
            VimMode::Insert     => "INS",
            VimMode::Visual     => "VIS",
            VimMode::VisualLine => "V-LINE",
            VimMode::Command    => "CMD",
        };
        let p = self.buffer.selection.head;
        let dc = self.buffer.diagnostics.len();
        let sel = if !self.buffer.selection.is_caret() {
            let (s, e) = self.buffer.selection.ordered();
            let cs = self.buffer.rope.line_to_char(s.line) + s.col;
            let ce = self.buffer.rope.line_to_char(e.line) + e.col;
            format!(" | {} sel ({} ln)", ce.saturating_sub(cs), e.line - s.line + 1)
        } else { String::new() };
        let search = if self.buffer.search.is_open {
            format!(" | Search: {}/{}", self.buffer.search.current_match + 1, self.buffer.search.match_count())
        } else { String::new() };
        self.status = format!(
            "{} | Ln {}, Col {}{}{} | {} diag",
            mode, p.line + 1, p.col + 1, sel, search, dc,
        );
    }

    fn ensure_cursor_visible(&mut self) {
        let sp = if self.buffer.search.is_open { widget::search_panel_height() } else { 0.0 };
        let vh = self.bounds_h - widget::top_pad() * 2.0 - sp;
        let cy = self.buffer.selection.head.line as f32 * widget::line_height();
        if cy < self.scroll_y { self.scroll_y = cy; }
        else if cy + widget::line_height() > self.scroll_y + vh {
            self.scroll_y = cy + widget::line_height() - vh;
        }
        let head = self.buffer.selection.head;
        let hlt = self.buffer.line_text(head.line);
        let vcol = editon::buffer::visual_col_of(&hlt, head.col);
        let cx = vcol as f32 * widget::CHAR_W;
        let gw = widget::gutter_width(self.buffer.line_count());
        let mm = if self.show_minimap { widget::minimap_width() } else { 0.0 };
        let vw = 1200.0 - gw - widget::scrollbar_width() - mm;
        if cx < self.scroll_x { self.scroll_x = cx; }
        else if cx + 9.6 > self.scroll_x + vw { self.scroll_x = cx + 9.6 - vw; }
    }

    // ── Vim normal mode key handler ───────────────────────────────────────────

    fn handle_vim_normal_key(&mut self, key: Key, mods: keyboard::Modifiers, text: Option<String>) -> Task<Msg> {
        use keyboard::key::Named;
        let shift = mods.shift();
        let ctrl  = mods.command();
        let was_g = self.pending_g;
        self.pending_g = false;

        // ── `r` (replace char) consumes the very next key as the replacement ──
        if self.pending_op == Some('r') {
            self.pending_op = None;
            let ch = match &key {
                Key::Named(Named::Space)     => Some(' '),
                Key::Named(Named::Tab)       => Some('\t'),
                Key::Named(Named::Enter)     => Some('\n'),
                Key::Named(Named::Escape)    => None, // cancel
                Key::Character(_)            => text.as_deref().and_then(|t| t.chars().next()),
                _                            => None,
            };
            if let Some(c) = ch {
                let count = self.vim_count.parse::<usize>().unwrap_or(1).max(1);
                self.vim_count.clear();
                for _ in 0..count { self.buffer.replace_char(c); }
            } else {
                self.vim_count.clear();
            }
            self.update_status();
            self.ensure_cursor_visible();
            return Task::none();
        }

        match key {
            Key::Named(Named::Escape) => {
                if self.buffer.search.is_open { self.buffer.search_close(); }
                self.buffer.selection.anchor = self.buffer.selection.head;
                self.vim_count.clear();
                self.pending_op = None;
            }
            Key::Named(Named::ArrowLeft)  if ctrl => self.buffer.move_word_left(shift),
            Key::Named(Named::ArrowRight) if ctrl => self.buffer.move_word_right(shift),
            Key::Named(Named::ArrowLeft)           => self.buffer.move_left(shift),
            Key::Named(Named::ArrowRight)          => self.buffer.move_right(shift),
            Key::Named(Named::ArrowUp)             => self.buffer.move_up(shift),
            Key::Named(Named::ArrowDown)           => self.buffer.move_down(shift),
            Key::Named(Named::Home) if ctrl => self.buffer.move_to_start(shift),
            Key::Named(Named::End)  if ctrl => self.buffer.move_to_end(shift),
            Key::Named(Named::Home)         => self.buffer.move_home(shift),
            Key::Named(Named::End)          => self.buffer.move_end(shift),
            Key::Named(Named::PageUp) => {
                let v = widget::visible_line_count(self.bounds_h);
                self.buffer.page_up(v, false);
            }
            Key::Named(Named::PageDown) => {
                let v = widget::visible_line_count(self.bounds_h);
                self.buffer.page_down(v, false);
            }
            Key::Named(Named::F5) => {
                let new_lang = match self.buffer.language() {
                    SyntaxLanguage::Sql  => SyntaxLanguage::Rust,
                    SyntaxLanguage::Rust => SyntaxLanguage::Sql,
                };
                let sample = match new_lang {
                    SyntaxLanguage::Sql  => SAMPLE_SQL,
                    SyntaxLanguage::Rust => SAMPLE_RUST,
                };
                self.buffer = Buffer::new(sample, new_lang);
                self.scroll_y = 0.0;
                self.scroll_x = 0.0;
            }

            Key::Character(_) => {
                let ch = text.as_deref().unwrap_or("");

                if ctrl {
                    match ch {
                        "f" | "F" => self.buffer.search_open(),
                        "w" | "W" => { let e = !self.buffer.wrap_config.enabled; self.buffer.set_wrap(e); }
                        "m" | "M" => self.show_minimap = !self.show_minimap,
                        "l" | "L" => self.show_whitespace = !self.show_whitespace,
                        "r" | "R" => self.buffer.redo(),
                        _ => {}
                    }
                } else {
                    // ─ Count prefix digits ─────────────────────────────────
                    let is_count_digit = ch.len() == 1
                        && ch.chars().next().map_or(false, |c| c.is_ascii_digit())
                        && (ch != "0" || !self.vim_count.is_empty());
                    if is_count_digit {
                        self.vim_count.push_str(ch);
                        self.update_status();
                        return Task::none();
                    }

                    let count = self.vim_count.parse::<usize>().unwrap_or(1).max(1);
                    self.vim_count.clear();

                    // ─ Pending operator + motion/doubling ──────────────────
                    if let Some(op) = self.pending_op.take() {
                        // `>` and `<` double to mean current-line indent/dedent
                        if (op == '>' && ch == ">") || (op == '<' && ch == "<") {
                            let line = self.buffer.selection.head.line;
                            let last = (line + count - 1).min(self.buffer.line_count().saturating_sub(1));
                            self.buffer.selection = Selection {
                                anchor: CursorPos::new(line, 0),
                                head:   CursorPos::new(last, self.buffer.line_len(last)),
                            };
                            if op == '>' { self.buffer.indent_lines(); } else { self.buffer.dedent_lines(); }
                            self.buffer.selection = Selection::caret(CursorPos::new(line, 0));
                            self.update_status();
                            self.ensure_cursor_visible();
                            return Task::none();
                        }
                        // `g` inside dg / yg / cg — wait for second `g`
                        if ch == "g" && !was_g {
                            self.pending_op = Some(op);
                            self.pending_g = true;
                            return Task::none();
                        }
                        // Linewise doubles: dd / yy / cc
                        let task = match (op, ch) {
                            ('d', "d") => {
                                let line = self.buffer.selection.head.line;
                                let yanked = self.buffer.yank_lines(line, count);
                                self.buffer.delete_lines(line, count);
                                self.update_status(); self.ensure_cursor_visible();
                                iced::clipboard::write(yanked)
                            }
                            ('y', "y") => {
                                let line = self.buffer.selection.head.line;
                                let yanked = self.buffer.yank_lines(line, count);
                                self.update_status();
                                iced::clipboard::write(yanked)
                            }
                            ('c', "c") => {
                                // cc: clear line content, stay on line, enter insert
                                let line = self.buffer.selection.head.line;
                                let len  = self.buffer.line_len(line);
                                self.buffer.selection = Selection {
                                    anchor: CursorPos::new(line, 0),
                                    head:   CursorPos::new(line, len),
                                };
                                let _ = self.buffer.cut();
                                self.buffer.selection = Selection::caret(CursorPos::new(line, 0));
                                self.vim_mode = VimMode::Insert;
                                self.update_status(); self.ensure_cursor_visible();
                                Task::none()
                            }
                            // Operator + motion
                            (op, motion) => {
                                let motion_str = if was_g { "gg" } else { motion };
                                self.exec_operator_motion(op, motion_str, count)
                            }
                        };
                        return task;
                    }

                    match ch {
                        // ─ Enter insert mode ───────────────────────────────
                        "i" => self.vim_mode = VimMode::Insert,
                        "I" => { self.buffer.move_home(false); self.vim_mode = VimMode::Insert; }
                        "a" => { self.buffer.move_right(false); self.vim_mode = VimMode::Insert; }
                        "A" => { self.buffer.move_end(false);  self.vim_mode = VimMode::Insert; }
                        "o" => {
                            self.buffer.move_end(false);
                            self.buffer.insert_newline();
                            self.vim_mode = VimMode::Insert;
                        }
                        "O" => {
                            self.buffer.move_home(false);
                            self.buffer.insert_newline();
                            self.buffer.move_up(false);
                            self.vim_mode = VimMode::Insert;
                        }
                        // ─ Visual modes ────────────────────────────────────
                        "v" => {
                            self.vim_mode = VimMode::Visual;
                            self.buffer.selection.anchor = self.buffer.selection.head;
                        }
                        "V" => {
                            self.vim_mode = VimMode::VisualLine;
                            self.buffer.select_lines(count);
                        }
                        // ─ Operators ───────────────────────────────────────
                        "d" => self.pending_op = Some('d'),
                        "y" => self.pending_op = Some('y'),
                        "c" => self.pending_op = Some('c'),
                        "r" => self.pending_op = Some('r'),
                        ">" => self.pending_op = Some('>'),
                        "<" => self.pending_op = Some('<'),
                        // ─ C = c$ (change to end of line) ──────────────────
                        "C" => { return self.exec_operator_motion('c', "$", 1); }
                        // ─ Paste ───────────────────────────────────────────
                        "p" => {
                            if self.buffer.clipboard_is_line {
                                for _ in 0..count { self.buffer.paste_line_below(); }
                            } else {
                                let clip = self.buffer.clipboard.clone();
                                if !clip.is_empty() {
                                    self.buffer.move_right(false);
                                    for _ in 0..count { self.buffer.paste(&clip); }
                                }
                            }
                        }
                        "P" => {
                            if self.buffer.clipboard_is_line {
                                for _ in 0..count { self.buffer.paste_line_above(); }
                            } else {
                                let clip = self.buffer.clipboard.clone();
                                if !clip.is_empty() {
                                    for _ in 0..count { self.buffer.paste(&clip); }
                                }
                            }
                        }
                        // ─ ~ toggle case ───────────────────────────────────
                        "~" => {
                            for _ in 0..count {
                                let pos = self.buffer.selection.head;
                                let lt  = self.buffer.line_text(pos.line);
                                if let Some(c) = lt.chars().nth(pos.col) {
                                    let toggled = if c.is_uppercase() {
                                        c.to_lowercase().next().unwrap_or(c)
                                    } else {
                                        c.to_uppercase().next().unwrap_or(c)
                                    };
                                    self.buffer.replace_char(toggled);
                                    self.buffer.move_right(false);
                                }
                            }
                        }
                        // ─ * / # search word under cursor ──────────────────
                        "*" => {
                            if let Some(word) = self.buffer.word_under_cursor() {
                                self.buffer.search_star(&word, true);
                                self.ensure_cursor_visible();
                            }
                        }
                        "#" => {
                            if let Some(word) = self.buffer.word_under_cursor() {
                                self.buffer.search_star(&word, false);
                                self.ensure_cursor_visible();
                            }
                        }
                        // ─ Enter command bar ───────────────────────────────
                        ":" => self.vim_mode = VimMode::Command,
                        // ─ Motions (count-aware) ───────────────────────────
                        "h" => { for _ in 0..count { self.buffer.move_left(false); } }
                        "j" => { for _ in 0..count { self.buffer.move_down(false); } }
                        "k" => { for _ in 0..count { self.buffer.move_up(false); } }
                        "l" => { for _ in 0..count { self.buffer.move_right(false); } }
                        "w" => { for _ in 0..count { self.buffer.move_word_right(false); } }
                        "b" => { for _ in 0..count { self.buffer.move_word_left(false); } }
                        "e" => { for _ in 0..count { self.buffer.move_word_right(false); } }
                        "0" => self.buffer.move_home(false),
                        "$" => self.buffer.move_end(false),
                        "^" => self.buffer.move_home(false),
                        "g" if was_g => self.buffer.move_to_start(false),
                        "g"          => self.pending_g = true,
                        "G"          => self.buffer.move_to_end(false),
                        // ─ Editing ─────────────────────────────────────────
                        "x" => { for _ in 0..count { self.buffer.delete(); } }
                        "X" => { for _ in 0..count { self.buffer.backspace(); } }
                        "u" => self.buffer.undo(),
                        // ─ Search navigation ───────────────────────────────
                        "n" => { self.buffer.search_next(); }
                        "N" => { self.buffer.search_prev(); }
                        _ => {}
                    }
                }
            }
            _ => {}
        }

        self.update_status();
        self.ensure_cursor_visible();
        Task::none()
    }

    // ── Operator + motion engine ──────────────────────────────────────────────

    fn exec_operator_motion(&mut self, op: char, motion: &str, count: usize) -> Task<Msg> {
        // Anchor at current cursor; motion extends head.
        let origin = self.buffer.selection.head;
        self.buffer.selection.anchor = origin;

        match motion {
            "h"       => { for _ in 0..count { self.buffer.move_left(true); } }
            "j"       => { for _ in 0..count { self.buffer.move_down(true); } }
            "k"       => { for _ in 0..count { self.buffer.move_up(true); } }
            "l"       => { for _ in 0..count { self.buffer.move_right(true); } }
            "w" | "e" => { for _ in 0..count { self.buffer.move_word_right(true); } }
            "b"       => { for _ in 0..count { self.buffer.move_word_left(true); } }
            "0" | "^" => self.buffer.move_home(true),
            "$"       => self.buffer.move_end(true),
            "G"       => self.buffer.move_to_end(true),
            "gg"      => self.buffer.move_to_start(true),
            _ => {
                // Unknown motion — reset and bail
                self.buffer.selection = Selection::caret(origin);
                self.update_status();
                return Task::none();
            }
        }

        match op {
            'd' => {
                let yanked = self.buffer.cut();
                self.buffer.selection = Selection::caret(self.buffer.selection.head);
                self.buffer.clipboard_is_line = false;
                self.update_status(); self.ensure_cursor_visible();
                if !yanked.is_empty() { return iced::clipboard::write(yanked); }
            }
            'y' => {
                let yanked = self.buffer.copy();
                // Cursor returns to the start of the yanked region
                let start = origin.min(self.buffer.selection.head);
                self.buffer.selection = Selection::caret(start);
                self.buffer.clipboard_is_line = false;
                self.update_status(); self.ensure_cursor_visible();
                if !yanked.is_empty() { return iced::clipboard::write(yanked); }
            }
            'c' => {
                let _ = self.buffer.cut();
                self.buffer.selection = Selection::caret(self.buffer.selection.head);
                self.vim_mode = VimMode::Insert;
                self.update_status(); self.ensure_cursor_visible();
            }
            _ => {}
        }
        Task::none()
    }

    // ── Vim visual mode key handler ───────────────────────────────────────────

    fn handle_vim_visual_key(&mut self, key: Key, mods: keyboard::Modifiers, text: Option<String>) -> Task<Msg> {
        use keyboard::key::Named;
        let ctrl = mods.command();
        let is_line = self.vim_mode == VimMode::VisualLine;

        // Count digit accumulation
        if let Key::Character(_) = &key {
            let ch = text.as_deref().unwrap_or("");
            let is_count_digit = ch.len() == 1
                && ch.chars().next().map_or(false, |c| c.is_ascii_digit())
                && (ch != "0" || !self.vim_count.is_empty());
            if is_count_digit {
                self.vim_count.push_str(ch);
                return Task::none();
            }
        }
        let count = self.vim_count.parse::<usize>().unwrap_or(1).max(1);
        self.vim_count.clear();

        match key {
            Key::Named(Named::Escape) => {
                self.buffer.selection.anchor = self.buffer.selection.head;
                self.vim_mode = VimMode::Normal;
            }
            Key::Named(Named::ArrowLeft)  => self.buffer.move_left(true),
            Key::Named(Named::ArrowRight) => self.buffer.move_right(true),
            Key::Named(Named::ArrowUp)    => self.buffer.move_up(true),
            Key::Named(Named::ArrowDown)  => self.buffer.move_down(true),
            Key::Character(_) => {
                let ch = text.as_deref().unwrap_or("");
                if ctrl {
                    match ch {
                        "f" | "F" => self.buffer.search_open(),
                        _ => {}
                    }
                } else {
                    match ch {
                        // ─ Motions extend the selection ─────────────────────
                        "h" => { for _ in 0..count { self.buffer.move_left(true); } }
                        "j" => { for _ in 0..count { self.buffer.move_down(true); } }
                        "k" => { for _ in 0..count { self.buffer.move_up(true); } }
                        "l" => { for _ in 0..count { self.buffer.move_right(true); } }
                        "w" => { for _ in 0..count { self.buffer.move_word_right(true); } }
                        "b" => { for _ in 0..count { self.buffer.move_word_left(true); } }
                        "0" | "^" => self.buffer.move_home(true),
                        "$"       => self.buffer.move_end(true),
                        "G"       => self.buffer.move_to_end(true),
                        "g"       => self.buffer.move_to_start(true),
                        // ─ Indent / dedent selection ────────────────────────
                        ">" => {
                            self.buffer.indent_lines();
                            self.vim_mode = VimMode::Normal;
                            self.buffer.selection.anchor = self.buffer.selection.head;
                        }
                        "<" => {
                            self.buffer.dedent_lines();
                            self.vim_mode = VimMode::Normal;
                            self.buffer.selection.anchor = self.buffer.selection.head;
                        }
                        // ─ Operators ───────────────────────────────────────
                        "y" => {
                            let yanked = if is_line {
                                let (s, e) = self.buffer.selection.ordered();
                                self.buffer.yank_lines(s.line, e.line - s.line + 1)
                            } else {
                                self.buffer.copy()
                            };
                            self.buffer.selection.anchor = self.buffer.selection.head;
                            self.vim_mode = VimMode::Normal;
                            self.update_status(); self.ensure_cursor_visible();
                            if !yanked.is_empty() { return iced::clipboard::write(yanked); }
                            return Task::none();
                        }
                        "d" | "x" => {
                            let yanked = if is_line {
                                let (s, e) = self.buffer.selection.ordered();
                                let lcount = e.line - s.line + 1;
                                let y = self.buffer.yank_lines(s.line, lcount);
                                self.buffer.delete_lines(s.line, lcount);
                                y
                            } else {
                                self.buffer.cut()
                            };
                            self.vim_mode = VimMode::Normal;
                            self.update_status(); self.ensure_cursor_visible();
                            if !yanked.is_empty() { return iced::clipboard::write(yanked); }
                            return Task::none();
                        }
                        "c" => {
                            // Change: delete selection and enter insert mode
                            let _ = self.buffer.cut();
                            self.vim_mode = VimMode::Insert;
                            self.update_status(); self.ensure_cursor_visible();
                            return Task::none();
                        }
                        // ─ Switch visual sub-mode ───────────────────────────
                        "v" => {
                            self.vim_mode = if is_line { VimMode::Visual } else { VimMode::Normal };
                            if self.vim_mode == VimMode::Normal {
                                self.buffer.selection.anchor = self.buffer.selection.head;
                            }
                        }
                        "V" => {
                            if is_line {
                                self.vim_mode = VimMode::Normal;
                                self.buffer.selection.anchor = self.buffer.selection.head;
                            } else {
                                self.vim_mode = VimMode::VisualLine;
                                let (s, e) = self.buffer.selection.ordered();
                                self.buffer.select_lines(e.line - s.line + 1);
                            }
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }

        // V-LINE: snap selection to whole lines
        if self.vim_mode == VimMode::VisualLine {
            let (s, e) = self.buffer.selection.ordered();
            if self.buffer.selection.head >= self.buffer.selection.anchor {
                self.buffer.selection.anchor = CursorPos::new(s.line, 0);
                self.buffer.selection.head   = CursorPos::new(e.line, self.buffer.line_len(e.line));
            } else {
                self.buffer.selection.head   = CursorPos::new(s.line, 0);
                self.buffer.selection.anchor = CursorPos::new(e.line, self.buffer.line_len(e.line));
            }
        }

        self.update_status();
        self.ensure_cursor_visible();
        Task::none()
    }

    // ── Vim command bar key handler ───────────────────────────────────────────

    fn handle_vim_command_key(&mut self, key: Key, text: Option<String>) -> Task<Msg> {
        use keyboard::key::Named;
        match key {
            Key::Named(Named::Escape) => {
                self.vim_mode = VimMode::Normal;
                self.vim_command.clear();
            }
            Key::Named(Named::Enter) => {
                self.execute_vim_command();
                self.vim_mode = VimMode::Normal;
                self.vim_command.clear();
            }
            Key::Named(Named::Backspace) => {
                if self.vim_command.pop().is_none() {
                    self.vim_mode = VimMode::Normal;
                }
            }
            Key::Named(Named::Space) => { self.vim_command.push(' '); }
            Key::Character(_) => {
                if let Some(t) = text { self.vim_command.push_str(&t); }
            }
            _ => {}
        }
        self.update_status();
        Task::none()
    }

    // ── Vim command execution ─────────────────────────────────────────────────

    fn execute_vim_command(&mut self) {
        let cmd = self.vim_command.trim().to_string();

        // :N  →  jump to line N (1-indexed)
        if let Ok(n) = cmd.parse::<usize>() {
            let line = n.saturating_sub(1).min(self.buffer.line_count().saturating_sub(1));
            self.buffer.selection.anchor = CursorPos { line, col: 0 };
            self.buffer.selection.head   = CursorPos { line, col: 0 };
            self.ensure_cursor_visible();
            return;
        }

        // :[range]s/pattern/replacement/[flags]
        if let Some((first, last, pat, rep, global, icase)) =
            parse_substitute(&cmd, self.buffer.selection.head.line, self.buffer.line_count().saturating_sub(1))
        {
            let changed = self.buffer.substitute(first, last, &pat, &rep, global, icase);
            // Move cursor to first changed line
            if changed > 0 {
                let line = first.min(self.buffer.line_count().saturating_sub(1));
                self.buffer.selection.anchor = CursorPos { line, col: 0 };
                self.buffer.selection.head   = CursorPos { line, col: 0 };
                self.ensure_cursor_visible();
            }
            self.update_status();
            return;
        }

        match cmd.as_str() {
            "noh" | "nohl" | "nohlsearch" => self.buffer.search_close(),
            "q" | "q!" | "wq"             => { /* quit: no file I/O in demo */ }
            "w"                            => { /* save:  no file I/O in demo */ }
            _                              => {}
        }
    }
}

// ── Vim :substitute command parser ────────────────────────────────────────────

/// Parse `[range]s<sep>pattern<sep>replacement[<sep>[flags]]`.
/// Returns `(first_line, last_line, pattern, replacement, global, case_insensitive)`.
fn parse_substitute(
    cmd: &str,
    current_line: usize,
    last_line: usize,
) -> Option<(usize, usize, String, String, bool, bool)> {
    // Consume optional range characters: digits, %, ., $, ,
    let mut i = 0;
    let bytes = cmd.as_bytes();
    while i < bytes.len() && matches!(bytes[i], b'0'..=b'9' | b'%' | b'.' | b'$' | b',') {
        i += 1;
    }
    let range_str = &cmd[..i];

    // Next must be 's'
    if bytes.get(i) != Some(&b's') { return None; }
    i += 1;

    // Separator character (usually '/')
    let sep = *bytes.get(i)? as char;
    i += 1;

    // Split the remainder into at most 3 parts by sep
    let rest = &cmd[i..];
    let sep_str = sep.to_string();
    let mut parts = rest.splitn(3, sep_str.as_str());
    let pattern     = parts.next().unwrap_or("");
    let replacement = parts.next().unwrap_or("");
    let flags       = parts.next().unwrap_or("");

    if pattern.is_empty() { return None; }

    let (first, last) = parse_vim_range(range_str, current_line, last_line);
    let global = flags.contains('g');
    let icase  = flags.contains('i');

    Some((first, last, pattern.to_string(), replacement.to_string(), global, icase))
}

fn parse_vim_range(range: &str, current: usize, last: usize) -> (usize, usize) {
    match range.trim() {
        "" | "." => (current, current),
        "%"      => (0, last),
        "$"      => (last, last),
        s => {
            if let Some((a, b)) = s.split_once(',') {
                (parse_line_addr(a, current, last), parse_line_addr(b, current, last))
            } else {
                let n = parse_line_addr(s, current, last);
                (n, n)
            }
        }
    }
}

fn parse_line_addr(s: &str, current: usize, last: usize) -> usize {
    match s.trim() {
        "." => current,
        "$" => last,
        n   => n.parse::<usize>().map(|n| n.saturating_sub(1).min(last)).unwrap_or(current),
    }
}

impl Default for App {
    fn default() -> Self { App::new().0 }
}
