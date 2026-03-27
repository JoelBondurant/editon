use iced::keyboard::{self, Key};
use iced::{Element, Subscription, Task, Theme};

use crate::editor::highlight::SyntaxLanguage;
use crate::editor::{CodeEditor, DEJAVU_SANS_MONO, EditorMsg};

const SAMPLE_SQL: &str = r#"-- SQL + Rust polyglot editor demo
-- A really long comment line to test line wrapping on really long lines like this one which is really long and otherwise pointless.
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
		Self { store: HashMap::new(), next_id: 1 }
	}
}

impl<T: Clone + Send + Sync> Repository<T> for InMemoryRepo<T> {
	fn find_by_id(&self, id: u64) -> Option<&T> { self.store.get(&id) }
	fn find_all(&self) -> Vec<&T> { self.store.values().collect() }
	fn insert(&mut self, item: T) -> u64 {
		let id = self.next_id;
		self.next_id += 1;
		self.store.insert(id, item);
		id
	}
	fn delete(&mut self, id: u64) -> bool { self.store.remove(&id).is_some() }
}

async fn process_users<'a>(
	repo: &'a dyn Repository<User>,
	filter: impl Fn(&User) -> bool + 'a,
) -> Vec<&'a User> {
	repo.find_all().into_iter().filter(|u| filter(u)).collect()
}

macro_rules! create_user {
	($name:expr, $email:expr, $role:expr) => {
		User { id: 0, name: $name.to_string(), email: $email.to_string(), role: $role }
	};
}

fn main() {
	let mut repo = InMemoryRepo::new();
	let users = vec![
		create_user!("Alice", "alice@example.com", UserRole::Admin),
		create_user!("Bob",   "bob@example.com",   UserRole::Editor),
		create_user!("Charlie", "charlie@example.com", UserRole::Viewer),
	];
	for user in users { repo.insert(user); }
	let admins: Vec<_> = repo.find_all().into_iter()
		.filter(|u| u.role == UserRole::Admin).collect();
	println!("Found {} admin(s)", admins.len());
}
"#;

const SAMPLE_TXT: &str = r#"This is a plain text file.
It has no syntax highlighting.
Repeating delete and retyping here should help determine if the "luminosity flutter"
is related to the asynchronous tree-sitter or regex-based highlighting.

1. This is a list item.
2. This is another one.
   - Sub-item with some indentation.
   - Another sub-item.

Just some more random text to fill the screen and allow for some scrolling if needed.
The quick brown fox jumps over the lazy dog.
THE QUICK BROWN FOX JUMPS OVER THE LAZY DOG.
0123456789
!@#$%^&*()_+
"#;

struct DemoApp {
	editor: CodeEditor,
}

#[derive(Debug, Clone)]
enum DemoMsg {
	Editor(EditorMsg),
}

impl DemoApp {
	fn new() -> (Self, Task<DemoMsg>) {
		let editor = CodeEditor::new(SAMPLE_SQL, SyntaxLanguage::Sql);
		(Self { editor }, Task::none())
	}

	fn update(&mut self, msg: DemoMsg) -> Task<DemoMsg> {
		let DemoMsg::Editor(m) = msg;
		if let EditorMsg::Key(Key::Named(keyboard::key::Named::F5), _, _) = &m {
			let (content, lang) = match self.editor.buffer.language() {
				SyntaxLanguage::Sql => (SAMPLE_RUST, SyntaxLanguage::Rust),
				SyntaxLanguage::Rust => (SAMPLE_TXT, SyntaxLanguage::Txt),
				SyntaxLanguage::Txt => (SAMPLE_SQL, SyntaxLanguage::Sql),
			};
			self.editor.set_content_with_language(content, lang);
			return Task::none();
		}
		if let EditorMsg::Key(Key::Named(keyboard::key::Named::F6), _, _) = &m {
			self.editor.set_vim_enabled(!self.editor.vim_enabled());
			return Task::none();
		}
		self.editor.update(m).map(DemoMsg::Editor)
	}

	fn view(&self) -> Element<'_, DemoMsg> {
		self.editor.view().map(DemoMsg::Editor)
	}

	fn subscription(&self) -> Subscription<DemoMsg> {
		self.editor.subscription().map(DemoMsg::Editor)
	}
}

pub fn run() -> iced::Result {
	iced::application(DemoApp::new, DemoApp::update, DemoApp::view)
		.title("editon")
		.subscription(DemoApp::subscription)
		.theme(|_: &DemoApp| Theme::Dark)
		.window_size((1200.0, 800.0))
		.antialiasing(true)
		.font(DEJAVU_SANS_MONO)
		.run()
}
