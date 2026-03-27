pub mod core;
pub mod normal;
pub mod visual;
pub mod command;
pub mod operator;

pub use self::core::{VimMode, NormalEdit, parse_substitute};
