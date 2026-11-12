//! rust-loc: count Rust source lines with test code excluded.
//!
//! The library exposes the same pipeline the CLI uses: parse the command line,
//! discover Rust files, analyse them in parallel, reduce them to business-code
//! counters and render the result as text, JSON, CSV or Markdown.

pub mod activity;
pub mod authors;
pub mod badge;
pub mod baseline;
pub mod cli;
pub mod clones;
pub mod cocomo;
pub mod config;
pub mod coupling;
pub mod gate;
pub mod generated;
pub mod generic;
pub mod git;
#[cfg(all(windows, feature = "gui"))]
pub mod gui;
pub mod history;
pub mod html;
pub mod i18n;
pub mod json;
pub mod languages;
pub mod lexer;
pub mod mcp;
pub mod metrics;
pub mod options;
pub mod png;
pub mod pylexer;
pub mod report;
pub mod scan;
pub mod settings;
pub mod sources;
pub mod svg;
pub mod vdiff;
#[cfg(feature = "web")]
pub mod web;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
