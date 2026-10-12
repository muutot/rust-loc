//! Background scan worker.
//!
//! The GUI never shells out: the report is produced in-process on a worker
//! thread so nothing can flash a console window.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver, Sender};

use crate::options::Options;
use crate::report::{self, Report};

pub enum Msg {
    Progress {
        stage: crate::scan::Stage,
        done: usize,
        total: usize,
        path: String,
    },
    Done(Box<Report>),
    /// The author pass, which runs on its own after the scan.
    Authors(Box<crate::authors::Authors>),
}

/// Kick off a scan and stream progress back to the UI thread.
pub fn start(roots: Vec<PathBuf>, options: Options, cwd: PathBuf) -> Receiver<Msg> {
    let (sender, receiver) = channel();
    std::thread::spawn(move || {
        let progress_sender: Sender<Msg> = sender.clone();
        let progress = move |stage: crate::scan::Stage, done: usize, total: usize, path: &Path| {
            let _ = progress_sender.send(Msg::Progress {
                stage,
                done,
                total,
                path: path.to_string_lossy().into_owned(),
            });
        };
        let result = report::scan(&roots, &options, &cwd, Some(&progress));
        let _ = sender.send(Msg::Done(Box::new(result)));
    });
    receiver
}

/// Blame the business files of a finished scan.
///
/// One `git blame` per file costs more than reading the whole tree, so the
/// author pass does not belong to the scan: it runs here, on its own, and only
/// when the author pane is the one in front of the user. The targets are a copy
/// of what the pass needs, so the report stays on the UI thread.
pub fn start_authors(targets: Vec<crate::authors::Target>, options: Options) -> Receiver<Msg> {
    let (sender, receiver) = channel();
    std::thread::spawn(move || {
        let progress_sender: Sender<Msg> = sender.clone();
        let progress = move |stage: crate::scan::Stage, done: usize, total: usize, path: &Path| {
            let _ = progress_sender.send(Msg::Progress {
                stage,
                done,
                total,
                path: path.to_string_lossy().into_owned(),
            });
        };
        let authors = crate::authors::Authors::collect_targets(&targets, &options, Some(&progress));
        let _ = sender.send(Msg::Authors(Box::new(authors)));
    });
    receiver
}
