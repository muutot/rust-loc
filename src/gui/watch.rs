//! Filesystem watcher for the scan roots.
//!
//! One worker thread per root parks in `ReadDirectoryChangesW` and posts a
//! message to the window when something under the root changes, so the report
//! can refresh on its own. Build output is ignored: compiling the project
//! churns `target`, and rescanning for every object file would never settle.
//!
//! The reads are overlapped, and that is not an optimisation. A synchronous
//! read cannot be cancelled: closing its handle from another thread waits for
//! the read to return, and a read parked on a quiet directory only returns
//! when that directory moves again. Changing the root list restarts the watch
//! from the window thread, so every add or remove froze the window until the
//! tree it stopped watching happened to change. A cancelled overlapped read
//! ends at once instead, and each worker releases its own handle only after
//! that read has drained.

use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use crate::gui::sys::*;

/// Message posted to the window when a watched root changes.
pub const WM_WATCH: u32 = WM_APP + 1;

/// Directory names whose churn never changes the report.
const NOISE: [&str; 6] = ["target", "node_modules", ".git", ".idea", "dist", "build"];

/// How long a parked worker waits for a change before it checks the stop flag
/// again, so a dropped watch lets go of its handle within one poll.
const STOP_POLL_MS: u32 = 50;

/// A running watch over one or more roots. Dropping it stops the workers.
pub struct Watcher {
    stop: Arc<AtomicBool>,
}

impl Drop for Watcher {
    fn drop(&mut self) {
        // Only a flag: the workers finish their own read and close their own
        // handle, so the thread that drops the watch never waits on one.
        self.stop.store(true, Ordering::SeqCst);
    }
}

/// Start watching `roots`; a change posts `WM_WATCH` to `hwnd`.
pub fn start(hwnd: HWND, roots: &[PathBuf]) -> Watcher {
    let stop = Arc::new(AtomicBool::new(false));
    let window = hwnd as isize;
    for root in roots {
        let stop = Arc::clone(&stop);
        let root = root.clone();
        std::thread::spawn(move || watch_root(window, &root, &stop));
    }
    Watcher { stop }
}

/// Park in the change notification for one root until it is stopped.
fn watch_root(window: isize, root: &Path, stop: &Arc<AtomicBool>) {
    let wide: Vec<u16> = root
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let handle = unsafe {
        CreateFileW(
            wide.as_ptr(),
            FILE_LIST_DIRECTORY,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            std::ptr::null(),
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OVERLAPPED,
            std::ptr::null_mut(),
        )
    };
    if handle.is_null() || handle == INVALID_HANDLE_VALUE {
        return;
    }

    let filter = FILE_NOTIFY_CHANGE_FILE_NAME
        | FILE_NOTIFY_CHANGE_DIR_NAME
        | FILE_NOTIFY_CHANGE_LAST_WRITE
        | FILE_NOTIFY_CHANGE_SIZE;
    let mut buffer = vec![0u8; 16 * 1024];
    let mut overlapped = OVERLAPPED::default();
    let mut parked = false;
    while !stop.load(Ordering::SeqCst) {
        if !parked {
            overlapped = OVERLAPPED::default();
            let issued = unsafe {
                ReadDirectoryChangesW(
                    handle,
                    buffer.as_mut_ptr().cast(),
                    buffer.len() as u32,
                    1,
                    filter,
                    std::ptr::null_mut(),
                    &mut overlapped,
                    None,
                )
            };
            if issued == 0 && last_error() != ERROR_IO_PENDING {
                break;
            }
            parked = true;
        }
        // Stop between issuing the read and taking the flag above, and the
        // read below would sit there till the tree moved: cancel it, and wait
        // for the cancellation to land before this thread lets the handle go.
        if stop.load(Ordering::SeqCst) {
            let mut cancelled: u32 = 0;
            unsafe { CancelIoEx(handle, &overlapped) };
            unsafe { GetOverlappedResult(handle, &overlapped, &mut cancelled, 1) };
            break;
        }
        let mut returned: u32 = 0;
        let ready =
            unsafe { GetOverlappedResultEx(handle, &overlapped, &mut returned, STOP_POLL_MS, 0) };
        if ready == 0 {
            if last_error() == WAIT_TIMEOUT {
                continue;
            }
            break;
        }
        if returned > 0 && !only_noise(&buffer[..returned as usize]) {
            unsafe { PostMessageW(window as HWND, WM_WATCH, 0, 0) };
        }
        parked = false;
    }
    unsafe { CloseHandle(handle) };
}

/// The error of the call that just failed.
fn last_error() -> u32 {
    unsafe { GetLastError() }
}

/// True when every change in a notification batch happened under a noise
/// directory. An empty or unparsable batch counts as a real change.
fn only_noise(buffer: &[u8]) -> bool {
    let mut offset = 0usize;
    let mut any = false;
    while offset + 12 <= buffer.len() {
        let next = read_u32(buffer, offset) as usize;
        let length = read_u32(buffer, offset + 8) as usize;
        let start = offset + 12;
        if start + length > buffer.len() {
            break;
        }
        any = true;
        if !is_noise(&utf16(&buffer[start..start + length])) {
            return false;
        }
        if next == 0 {
            break;
        }
        offset += next;
    }
    any
}

fn read_u32(buffer: &[u8], at: usize) -> u32 {
    u32::from_ne_bytes([buffer[at], buffer[at + 1], buffer[at + 2], buffer[at + 3]])
}

fn utf16(bytes: &[u8]) -> String {
    let units = bytes
        .chunks_exact(2)
        .map(|pair| u16::from_ne_bytes([pair[0], pair[1]]));
    char::decode_utf16(units)
        .map(|unit| unit.unwrap_or(char::REPLACEMENT_CHARACTER))
        .collect()
}

/// The relative path of a change; a name that runs through any noise
/// directory is not interesting.
fn is_noise(name: &str) -> bool {
    name.split(['\\', '/'])
        .any(|part| NOISE.iter().any(|noise| part.eq_ignore_ascii_case(noise)))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build the byte layout ReadDirectoryChangesW fills in.
    fn batch(names: &[&str]) -> Vec<u8> {
        let mut out = Vec::new();
        for (index, name) in names.iter().enumerate() {
            let units: Vec<u16> = name.encode_utf16().collect();
            let start = out.len();
            out.extend_from_slice(&0u32.to_ne_bytes());
            out.extend_from_slice(&1u32.to_ne_bytes());
            out.extend_from_slice(&((units.len() * 2) as u32).to_ne_bytes());
            for unit in units {
                out.extend_from_slice(&unit.to_ne_bytes());
            }
            if index + 1 < names.len() {
                let next = (out.len() - start) as u32;
                out[start..start + 4].copy_from_slice(&next.to_ne_bytes());
            }
        }
        out
    }

    #[test]
    fn build_output_is_noise() {
        assert!(only_noise(&batch(&["target\\debug\\app.exe"])));
        assert!(only_noise(&batch(&["node_modules\\react\\index.js"])));
    }

    #[test]
    fn source_changes_are_not_noise() {
        assert!(!only_noise(&batch(&["src\\lib.rs"])));
        assert!(!only_noise(&batch(&["target\\x", "src\\lib.rs"])));
    }

    #[test]
    fn an_empty_batch_counts_as_a_change() {
        assert!(!only_noise(&[]));
    }

    /// Dropping a watch must not wait for a change. The window thread drops
    /// the old watch whenever the root list changes, and a read parked on a
    /// quiet tree would hold it there until that tree moved again.
    #[test]
    fn dropping_a_watch_returns_without_a_change() {
        let dir = std::env::temp_dir().join("rust-loc-watch-drop");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let watcher = start(std::ptr::null_mut(), std::slice::from_ref(&dir));
        // Give the worker time to park in its read.
        std::thread::sleep(std::time::Duration::from_millis(200));
        let (sender, receiver) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            drop(watcher);
            let _ = sender.send(());
        });
        assert!(receiver
            .recv_timeout(std::time::Duration::from_secs(5))
            .is_ok());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
