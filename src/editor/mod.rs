/// DejaVu Sans Mono font bytes — pass to `.font()` on your iced app builder
/// so the editor's whitespace glyphs (▸ ␣ ¬) render correctly.
pub const DEJAVU_SANS_MONO: &[u8] = include_bytes!("../../fonts/DejaVuSansMono.ttf");

pub mod buffer;
pub mod folding;
pub mod highlight;
pub mod search;
pub mod theme;
pub mod widget;
pub mod wrap;

mod code_editor;
pub mod vim;

#[allow(unused_imports)]
pub use code_editor::{CodeEditor, EditorMsg};
#[allow(unused_imports)]
pub use vim::VimMode;
