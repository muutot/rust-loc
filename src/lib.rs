//! rust-loc: count Rust source lines with test code excluded.
//!
//! The library exposes the same pipeline the CLI uses: parse the command line,
//! discover Rust files, analyse them in parallel, reduce them to business-code
//! counters and render the result as text, JSON, CSV or Markdown.

pub mod badge;
pub mod baseline;
pub mod cli;
pub mod clones;
pub mod cocomo;
pub mod config;
pub mod gate;
pub mod generated;
pub mod git;
#[cfg(windows)]
pub mod gui;
pub mod languages;
pub mod lexer;
pub mod metrics;
pub mod options;
pub mod png;
pub mod report;
pub mod scan;
pub mod svg;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
