//! rust-loc-mcp: the MCP server, one JSON-RPC message per line of stdin.
//!
//! Nothing is written to stdout except answers, so an editor can launch the
//! binary as a stdio server and speak the protocol straight down the pipe.

use std::io::{BufRead, Write};
use std::path::PathBuf;

const USAGE: &str = "rust-loc-mcp - serve rust-loc over the Model Context Protocol

USAGE:
    rust-loc-mcp [ROOT]

ROOT is the directory relative scan roots are resolved against
(default: the working directory). The server reads one JSON-RPC
message per line on stdin and writes one answer per line to stdout.";

fn main() {
    if let Some(code) = early_exit() {
        std::process::exit(code);
    }
    let cwd = workspace();
    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        let Some(answer) = rust_loc::mcp::handle(&line, &cwd) else {
            continue;
        };
        if writeln!(out, "{}", answer).is_err() || out.flush().is_err() {
            break;
        }
    }
}

/// Help, version and unknown switches leave before any protocol traffic: a
/// caller asking for them is a person, not a client.
fn early_exit() -> Option<i32> {
    for argument in std::env::args().skip(1) {
        match argument.as_str() {
            "-h" | "--help" => {
                println!("{}", USAGE);
                return Some(0);
            }
            "-V" | "--version" => {
                println!("rust-loc-mcp {}", rust_loc::VERSION);
                return Some(0);
            }
            other if other.starts_with('-') => {
                eprintln!("rust-loc-mcp: unknown option: {}", other);
                return Some(2);
            }
            _ => {}
        }
    }
    None
}

/// The directory relative scan roots resolve against: the optional argument,
/// made absolute so the paths in an answer read the same from anywhere.
fn workspace() -> PathBuf {
    let chosen: Option<PathBuf> = std::env::args_os()
        .skip(1)
        .map(PathBuf::from)
        .find(|argument| !argument.to_string_lossy().starts_with('-'));
    let root = match chosen {
        Some(path) if path.is_absolute() => path,
        Some(path) => absolute(&path),
        None => absolute(std::path::Path::new(".")),
    };
    if !root.is_dir() {
        eprintln!("rust-loc-mcp: not a directory: {}", root.display());
        std::process::exit(1);
    }
    root
}

fn absolute(path: &std::path::Path) -> PathBuf {
    let base = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let joined = if path.is_absolute() {
        path.to_path_buf()
    } else {
        base.join(path)
    };
    // Collapse `.` so the printed roots stay readable.
    let mut out = PathBuf::new();
    for part in joined.components() {
        match part {
            std::path::Component::CurDir => {}
            other => out.push(other),
        }
    }
    out
}
