//! Build script: embed the web front end's build output into the binary.
//!
//! It walks `web/dist` and writes a manifest of `include_bytes!` entries to
//! `OUT_DIR/web_dist.rs`, which `src/web/assets.rs` pulls in. When no build
//! exists yet the manifest carries a single placeholder page, so a plain
//! `cargo build` never fails for want of an npm install.

use std::fs;
use std::path::Path;

const DIST: &str = "web/dist";

const PLACEHOLDER: &str = r#"<!doctype html>
<html lang="zh">
<head><meta charset="utf-8"><title>rust-loc</title></head>
<body style="font-family: system-ui; max-width: 40em; margin: 4em auto; line-height: 1.6">
<h1>rust-loc — 网页前端尚未构建</h1>
<p>二进制里还没有嵌入页面。先构建前端，再重新编译：</p>
<pre>cd web
npm ci
npm run build
cd ..
cargo build --release</pre>
</body>
</html>
"#;

fn main() {
    println!("cargo:rerun-if-changed={DIST}");
    // include_bytes! resolves relative to OUT_DIR, so the manifest carries
    // absolute source paths built from the manifest directory.
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap_or_default();
    let dist = Path::new(&manifest_dir).join(DIST);
    let mut manifest = String::from("pub static ASSETS: &[(&str, &[u8])] = &[\n");
    let mut found_any = false;
    if dist.is_dir() {
        let mut files: Vec<std::path::PathBuf> = Vec::new();
        collect(&dist, &dist, &mut files);
        files.sort();
        for path in &files {
            let relative = path
                .strip_prefix(&dist)
                .unwrap_or(path)
                .to_string_lossy()
                .replace('\\', "/");
            // Paths go into include_bytes! verbatim; forward slashes and the
            // characters a bundler emits are all safe inside the macro.
            manifest.push_str(&format!(
                "    (\"{relative}\", include_bytes!(\"{}\")),\n",
                path.to_string_lossy().replace('\\', "/")
            ));
            found_any = true;
        }
    }
    manifest.push_str("];\n");
    if found_any {
        manifest.push_str("pub static PLACEHOLDER: &[u8] = b\"\";\n");
    } else {
        println!("cargo:warning=web/dist is empty; run `npm ci && npm run build` in web/ to embed the page");
        // The placeholder page ships in place of the real one.
        manifest = String::from("pub static ASSETS: &[(&str, &[u8])] = &[];\n");
        manifest.push_str(&format!(
            "pub static PLACEHOLDER: &[u8] = include_bytes!(\"{}\");\n",
            placeholder_path()
        ));
    }
    let out_dir = std::env::var("OUT_DIR").unwrap_or_default();
    let target = Path::new(&out_dir).join("web_dist.rs");
    fs::write(&target, manifest).unwrap_or_else(|error| {
        panic!("cannot write {}: {error}", target.display())
    });
}

/// Every file below `dir`, with paths relative to nothing — the caller strips
/// the dist prefix.
fn collect(root: &Path, dir: &Path, files: &mut Vec<std::path::PathBuf>) {
    let Ok(read) = fs::read_dir(dir) else {
        return;
    };
    for entry in read.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect(root, &path, files);
        } else if path.is_file() {
            files.push(path);
        }
    }
}

/// The placeholder lives next to the build script; include_bytes! needs a
/// path relative to a source file, and OUT_DIR files resolve relative to the
/// manifest, so write it out and include that copy.
fn placeholder_path() -> String {
    let out_dir = std::env::var("OUT_DIR").unwrap_or_default();
    let target = Path::new(&out_dir).join("web_placeholder.html");
    fs::write(&target, PLACEHOLDER).expect("cannot write placeholder page");
    target.to_string_lossy().replace('\\', "/")
}
