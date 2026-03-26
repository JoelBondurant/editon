use iced::keyboard::{self, Key};
use iced::widget::{column, container, row, text, Space};
use iced::{event, Element, Length, Subscription, Task, Theme};

use editon::buffer::{Buffer, CursorPos, UndoConfig};
use editon::highlight::SyntaxLanguage;
use editon::theme::EditorTheme;
use editon::widget::{self, EditorAction, SqlEditor};

fn main() -> iced::Result {
    iced::application(App::new, App::update, App::view)
        .title("Code Editor")
        .subscription(App::subscription)
        .theme(|_: &App| Theme::Dark)
        .window_size((1200.0, 800.0))
        .antialiasing(true)
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
            status: format!("Ln 1, Col 1 | {} diagnostics | SQL", dc),
            bounds_h: 750.0,
            is_dragging: false,
            click_count: 0,
            show_minimap: true,
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
                    Key::Named(keyboard::key::Named::Tab) if shift => { /* dedent TODO */ }
                    Key::Named(keyboard::key::Named::Tab) => self.buffer.insert_str("    "),

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
            .show_minimap(self.show_minimap);

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
                text("F5=switch lang").size(11).color(sep),
            ]
            .padding(6).spacing(4),
        )
        .style(|_: &Theme| container::Style {
            background: Some(iced::Background::Color(iced::Color::from_rgb(0.09, 0.10, 0.12))),
            ..Default::default()
        })
        .width(Length::Fill);

        column![
            container(Element::from(editor)).width(Length::Fill).height(Length::Fill),
            status_bar,
        ].into()
    }

    fn pos_from_pixel(&self, pixel: iced::Point) -> CursorPos {
        let gw = widget::gutter_width(self.buffer.line_count());
        let bounds = iced::Rectangle { x: 0.0, y: 0.0, width: 1200.0, height: self.bounds_h };
        widget::pixel_to_pos(&self.buffer, &bounds, gw, self.scroll_x, self.scroll_y, pixel.x, pixel.y)
    }

    fn update_status(&mut self) {
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
            "Ln {}, Col {}{}{} | {} diag",
            p.line + 1, p.col + 1, sel, search, dc,
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
        let cx = self.buffer.selection.head.col as f32 * 9.6;
        let gw = widget::gutter_width(self.buffer.line_count());
        let mm = if self.show_minimap { widget::minimap_width() } else { 0.0 };
        let vw = 1200.0 - gw - widget::scrollbar_width() - mm;
        if cx < self.scroll_x { self.scroll_x = cx; }
        else if cx + 9.6 > self.scroll_x + vw { self.scroll_x = cx + 9.6 - vw; }
    }
}

impl Default for App {
    fn default() -> Self { App::new().0 }
}
