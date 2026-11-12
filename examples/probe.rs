//! Manual probe: print what the churn helper sees for a given root, so a
//! "no git history" report can be traced without a debugger.

use rust_loc::scan;

fn main() {
    let root = std::env::args()
        .nth(1)
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().unwrap());
    println!("root: {}", root.display());
    println!("run_git rev-parse: {:?}", scan::run_git_probe(&root));
    match rust_loc::git::churn(&root) {
        Some((toplevel, churn)) => {
            println!("toplevel: {}", toplevel.display());
            println!("entries: {}", churn.len());
            let mut rows: Vec<_> = churn.iter().collect();
            rows.sort_by(|a, b| b.1.lines().cmp(&a.1.lines()));
            for (path, change) in rows.iter().take(10) {
                println!("  {:40} added={:?} deleted={:?} commits={:?}", path, change.added, change.deleted, change.commits);
            }
        }
        None => println!("churn: None"),
    }
}
