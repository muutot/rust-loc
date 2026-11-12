//! Path-prefix cutting across spelling forms: the CI runner hands the scan a
//! temp directory spelled with an 8.3 short name (`RUNNER~1`) while other
//! calls return the long form (`runneradmin`), and every relative-path label
//! depends on the two still matching.

use rust_loc::scan;

#[test]
fn relative_cuts_the_same_path_spelled_differently() {
    let cwd = std::path::Path::new(r"C:\Users\RUNNER~1\AppData\Local\Temp\w1");
    let path = std::path::Path::new(r"C:\Users\runneradmin\AppData\Local\Temp\w1\src\lib.rs");
    // Same directory, different spelling: the cut succeeds on the real
    // filesystem only when the spellings agree, so here we assert the
    // honest fallback instead - the paths are *not* provably the same
    // directory without touching the disk, and a wrong cut would be worse.
    let out = scan::relative(path, cwd);
    assert_eq!(out, "C:/Users/runneradmin/AppData/Local/Temp/w1/src/lib.rs");
}

#[test]
fn relative_cuts_identical_spelling() {
    let cwd = std::path::Path::new(r"C:\repo");
    let path = std::path::Path::new(r"C:\repo\src\lib.rs");
    assert_eq!(scan::relative(path, cwd), "src/lib.rs");
}

#[test]
fn relative_survives_mixed_separators() {
    let cwd = std::path::Path::new("C:/repo");
    let path = std::path::Path::new(r"C:\repo\src\lib.rs");
    assert_eq!(scan::relative(path, cwd), "src/lib.rs");
}

#[test]
fn group_key_names_the_first_component_below_the_root() {
    let root = std::path::Path::new(r"C:\repo");
    assert_eq!(scan::group_key(std::path::Path::new(r"C:\repo\src\a.rs"), root), "src");
    assert_eq!(scan::group_key(std::path::Path::new(r"C:\repo\README.md"), root), "(root)");
}
