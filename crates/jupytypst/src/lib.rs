pub mod kernel;
pub mod repl;

mod cell;
mod output;
mod persist;
mod session;

#[doc(hidden)]
pub mod testkit;

pub const CODE_DISPLAY_NAME: &str = "Typst (Code Mode)";
pub const MARKUP_DISPLAY_NAME: &str = "Typst";
pub const DEFAULT_PAGE_SETUP: &str = "set page(width: auto, height: auto, margin: 16pt)";
