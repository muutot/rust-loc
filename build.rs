//! Embed a Common Controls v6 manifest so the GUI uses modern themed widgets.

fn main() {
    println!("cargo:rerun-if-changed=gui/rust-loc-gui.manifest");
    if std::env::var("CARGO_CFG_WINDOWS").is_ok() {
        let root = std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR");
        let manifest = std::path::Path::new(&root)
            .join("gui")
            .join("rust-loc-gui.manifest");
        println!("cargo:rustc-link-arg-bins=/MANIFEST:EMBED");
        println!(
            "cargo:rustc-link-arg-bins=/MANIFESTINPUT:{}",
            manifest.display()
        );
    }
}
