//! Native shell dialogs.
//!
//! `SHBrowseForFolderW` and `GetSaveFileNameW` are used instead of the COM
//! `IFileDialog` wrappers so the shell keeps zero dependencies.

use crate::gui::sys::*;

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

fn from_buffer(buffer: &[u16]) -> String {
    let length = buffer.iter().position(|code| *code == 0).unwrap_or(0);
    String::from_utf16_lossy(&buffer[..length])
}

/// Ask the user for a folder to scan.
pub fn pick_folder(owner: HWND) -> Option<String> {
    let title = wide("Select a folder to scan");
    let mut display = vec![0u16; 260];
    let info = BROWSEINFOW {
        hwndOwner: owner,
        pidlRoot: std::ptr::null_mut(),
        pszDisplayName: display.as_mut_ptr(),
        lpszTitle: title.as_ptr(),
        ulFlags: BIF_RETURNONLYFSDIRS | BIF_NEWDIALOGSTYLE | BIF_EDITBOX,
        lpfn: None,
        lParam: 0,
        iImage: 0,
    };
    unsafe {
        let pidl = SHBrowseForFolderW(&info);
        if pidl.is_null() {
            return None;
        }
        let mut buffer = vec![0u16; 32768];
        let ok = SHGetPathFromIDListW(pidl as *const _, buffer.as_mut_ptr());
        CoTaskMemFree(pidl.cast());
        if ok == 0 {
            return None;
        }
        Some(from_buffer(&buffer))
    }
}

fn filter_for(name: &str) -> (Vec<u16>, Vec<u16>) {
    let (label, pattern, extension) = match name.rsplit('.').next() {
        Some("json") => ("JSON report", "*.json", "json"),
        Some("csv") => ("CSV report", "*.csv", "csv"),
        Some("md") => ("Markdown report", "*.md", "md"),
        _ => ("Text report", "*.txt", "txt"),
    };
    let mut spec: Vec<u16> = Vec::new();
    for part in [label, pattern, "All files", "*.*"] {
        spec.extend(part.encode_utf16());
        spec.push(0);
    }
    spec.push(0);
    (spec, wide(extension))
}

/// Ask where an exported report should be written.
pub fn save_file(owner: HWND, default_name: &str) -> Option<String> {
    let title = wide("Export report");
    let (filter, extension) = filter_for(default_name);
    let mut file = vec![0u16; 1024];
    for (slot, code) in file.iter_mut().zip(default_name.encode_utf16()) {
        *slot = code;
    }
    let mut info: OPENFILENAMEW = unsafe { std::mem::zeroed() };
    info.lStructSize = std::mem::size_of::<OPENFILENAMEW>() as u32;
    info.hwndOwner = owner;
    info.lpstrFilter = filter.as_ptr();
    info.nFilterIndex = 1;
    info.lpstrFile = file.as_mut_ptr();
    info.nMaxFile = file.len() as u32;
    info.lpstrTitle = title.as_ptr();
    info.lpstrDefExt = extension.as_ptr();
    info.Flags = OFN_OVERWRITEPROMPT | OFN_PATHMUSTEXIST | OFN_EXPLORER | OFN_NOCHANGEDIR;
    let ok = unsafe { GetSaveFileNameW(&mut info) };
    if ok == 0 {
        return None;
    }
    Some(from_buffer(&file))
}

/// Roots sitting on the clipboard.
///
/// A folder copied in Explorer arrives as `CF_HDROP`; "Copy as path" and a
/// plain address arrive as text, so both are accepted.
pub fn clipboard_roots() -> Vec<String> {
    let files = clipboard_files();
    if !files.is_empty() {
        return files;
    }
    clipboard_text()
        .map(|text| crate::gui::model::parse_paths(&text))
        .unwrap_or_default()
}

fn open_clipboard() -> bool {
    unsafe { OpenClipboard(std::ptr::null_mut()) != 0 }
}

fn clipboard_files() -> Vec<String> {
    if unsafe { IsClipboardFormatAvailable(CF_HDROP as u32) } == 0 || !open_clipboard() {
        return Vec::new();
    }
    let mut files = Vec::new();
    unsafe {
        let handle = GetClipboardData(CF_HDROP as u32);
        if !handle.is_null() {
            let drop = handle as HDROP;
            let count = DragQueryFileW(drop, u32::MAX, std::ptr::null_mut(), 0);
            for index in 0..count {
                let length = DragQueryFileW(drop, index, std::ptr::null_mut(), 0);
                if length == 0 {
                    continue;
                }
                let mut buffer = vec![0u16; length as usize + 1];
                DragQueryFileW(drop, index, buffer.as_mut_ptr(), buffer.len() as u32);
                files.push(String::from_utf16_lossy(&buffer[..length as usize]));
            }
        }
        CloseClipboard();
    }
    files
}

fn clipboard_text() -> Option<String> {
    if unsafe { IsClipboardFormatAvailable(CF_UNICODETEXT as u32) } == 0 || !open_clipboard() {
        return None;
    }
    let mut text = None;
    unsafe {
        let handle = GetClipboardData(CF_UNICODETEXT as u32);
        if !handle.is_null() {
            let pointer = GlobalLock(handle) as *const u16;
            if !pointer.is_null() {
                let mut length = 0usize;
                while *pointer.add(length) != 0 {
                    length += 1;
                }
                let slice = std::slice::from_raw_parts(pointer, length);
                text = Some(String::from_utf16_lossy(slice));
                GlobalUnlock(handle);
            }
        }
        CloseClipboard();
    }
    text
}

/// Put `value` on the clipboard as Unicode text.
pub fn copy_text(value: &str) -> bool {
    if !open_clipboard() {
        return false;
    }
    let units: Vec<u16> = value.encode_utf16().chain(std::iter::once(0)).collect();
    let bytes = units.len() * std::mem::size_of::<u16>();
    let mut copied = false;
    unsafe {
        if EmptyClipboard() != 0 {
            let block = GlobalAlloc(GHND, bytes);
            if !block.is_null() {
                let target = GlobalLock(block) as *mut u16;
                if !target.is_null() {
                    std::ptr::copy_nonoverlapping(units.as_ptr(), target, units.len());
                    GlobalUnlock(block);
                    if !SetClipboardData(CF_UNICODETEXT as u32, block).is_null() {
                        copied = true;
                    } else {
                        GlobalFree(block);
                    }
                } else {
                    GlobalFree(block);
                }
            }
        }
        CloseClipboard();
    }
    copied
}
