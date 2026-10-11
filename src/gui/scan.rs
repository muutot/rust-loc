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
        done: usize,
        total: usize,
        path: String,
    },
    Done(Box<Report>),
}

/// Kick off a scan and stream progress back to the UI thread.
pub fn start(roots: Vec<PathBuf>, options: Options, cwd: PathBuf) -> Receiver<Msg> {
    let (sender, receiver) = channel();
    std::thread::spawn(move || {
        let progress_sender: Sender<Msg> = sender.clone();
        let progress = move |done: usize, total: usize, path: &Path| {
            let _ = progress_sender.send(Msg::Progress {
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
