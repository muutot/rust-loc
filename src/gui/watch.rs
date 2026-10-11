//! Filesystem watcher for the scan roots.
//!
//! One worker thread per root parks in `ReadDirectoryChangesW` and posts a
//! message to the window when something under the root changes, so the report
//! can refresh on its own. Build output is ignored: compiling the project
//! churns `target`, and rescanning for every object file would never settle.

use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};
use std::sync::Arc;

use crate::gui::sys::*;

/// Message posted to the window when a watched root changes.
pub const WM_WATCH: u32 = WM_APP + 1;

/// Directory names whose churn never changes the report.
const NOISE: [&str; 6] = ["target", "node_modules", ".git", ".idea", "dist", "build"];

/// A running watch over one or more roots. Dropping it stops the workers.
pub struct Watcher {
    stop: Arc<AtomicBool>,
    /// Directory handles, so `drop` can close the one each worker is parked
    /// on and let it return.
    handles: Arc<Vec<AtomicIsize>>,
}

impl Drop for Watcher {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        for slot in self.handles.iter() {
            close(slot);
        }
    }
}

/// Start watching `roots`; a change posts `WM_WATCH` to `hwnd`.
pub fn start(hwnd: HWND, roots: &[PathBuf]) -> Watcher {
    let stop = Arc::new(AtomicBool::new(false));
    let handles: Arc<Vec<AtomicIsize>> =
        Arc::new((0..roots.len()).map(|_| AtomicIsize::new(0)).collect());
    let window = hwnd as isize;
    for (index, root) in roots.iter().enumerate() {
        let stop = Arc::clone(&stop);
        let handles = Arc::clone(&handles);
        let root = root.clone();
        std::thread::spawn(move || watch_root(window, &root, &stop, &handles, index));
    }
    Watcher { stop, handles }
}

/// Park in the change notification for one root until it is closed.
fn watch_root(
    window: isize,
    root: &Path,
    stop: &Arc<AtomicBool>,
    handles: &Arc<Vec<AtomicIsize>>,
    index: usize,
) {
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
            FILE_FLAG_BACKUP_SEMANTICS,
            std::ptr::null_mut(),
        )
    };
    if handle.is_null() || handle == INVALID_HANDLE_VALUE {
        return;
    }
    handles[index].store(handle as isize, Ordering::SeqCst);

    let filter = FILE_NOTIFY_CHANGE_FILE_NAME
        | FILE_NOTIFY_CHANGE_DIR_NAME
        | FILE_NOTIFY_CHANGE_LAST_WRITE
        | FILE_NOTIFY_CHANGE_SIZE;
    let mut buffer = vec![0u8; 16 * 1024];
    while !stop.load(Ordering::SeqCst) {
        let mut returned: u32 = 0;
        let ok = unsafe {
            ReadDirectoryChangesW(
                handle,
                buffer.as_mut_ptr().cast(),
                buffer.len() as u32,
                1,
                filter,
                &mut returned,
                std::ptr::null_mut(),
                None,
            )
        };
        if ok == 0 || stop.load(Ordering::SeqCst) {
            break;
        }
        if returned == 0 {
            continue;
        }
        if !only_noise(&buffer[..returned as usize]) {
            unsafe { PostMessageW(window as HWND, WM_WATCH, 0, 0) };
        }
    }
    close(&handles[index]);
}

/// Close a handle slot, but only once: whoever takes the non-zero value
/// closes it, the loser sees zero.
fn close(slot: &AtomicIsize) {
    let raw = slot.swap(0, Ordering::SeqCst);
    if raw != 0 {
        unsafe { CloseHandle(raw as HANDLE) };
    }
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

    #[test]
    fn noise_matches_a_whole_path_component() {
        assert!(is_noise("a\\.git\\HEAD"));
        assert!(is_noise("src/target/x.rs"));
        assert!(!is_noise("src\\targets.rs"));
    }
}
