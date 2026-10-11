//! Native Win32 front end for rust-loc.
#![cfg_attr(windows, windows_subsystem = "windows")]

fn main() {
    std::process::exit(rust_loc::gui::run());
}
