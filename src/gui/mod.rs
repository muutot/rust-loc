//! Native Win32 front end.
//!
//! The shell is plain Win32 plus hand painted surfaces: no web runtime and no
//! UI toolkit. The one crate involved is Microsoft's `windows-sys`, whose
//! bindings are thin declarations over the system DLLs.

mod app;
mod card;
mod dialog;
mod model;
mod paint;
mod scan;
mod sys;
mod theme;
mod watch;
mod window;

/// The UI language catalog is shared with the web front end, so it lives at
/// the crate root and is re-exported here under its old path.
pub use crate::i18n;

/// Create the main window and pump messages until it closes.
pub fn run() -> i32 {
    window::run()
}
