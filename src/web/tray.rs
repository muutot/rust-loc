//! The system tray icon: the product sits here while the browser does the
//! work. A left click opens the page, the context menu offers the same plus
//! quit. Plain Win32 — the message loop lives on the main thread, the web
//! server on its own.
//!
//! Everything here is Windows-only and cfg'd away elsewhere; the server runs
//! without it.

#![cfg(windows)]

use std::sync::OnceLock;

use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Shell::{
    Shell_NotifyIconW, NIF_MESSAGE, NIF_ICON, NIF_TIP, NIM_ADD, NIM_DELETE, NOTIFYICONDATAW,
};
use windows_sys::Win32::UI::WindowsAndMessaging as win;

use crate::web::to_wide;

/// The tray callback message: the shell posts it with the mouse event in
/// lParam. WM_APP is the range Microsoft reserves for exactly this.
const WM_APP_TRAY: u32 = win::WM_APP + 1;
const ID_TRAY_ICON: u32 = 1;
const ID_MENU_OPEN: u32 = 2001;
const ID_MENU_QUIT: u32 = 2002;

/// The page the tray opens; set once before the message loop starts.
static PAGE_URL: OnceLock<String> = OnceLock::new();

/// Show the tray icon and pump messages until the reader quits from the
/// menu. Returns the process exit code.
pub fn run_with_tray(port: u16, token: &str) -> i32 {
    let url = format!("http://127.0.0.1:{port}/?token={token}");
    let _ = PAGE_URL.set(url);

    unsafe {
        let class_name = to_wide("rust_loc_web_tray");
        let class = win::WNDCLASSW {
            style: 0,
            lpfnWndProc: Some(wndproc),
            cbClsExtra: 0,
            cbWndExtra: 0,
            hInstance: GetModuleHandleW(std::ptr::null()),
            hIcon: win::LoadIconW(std::ptr::null_mut(), win::IDI_APPLICATION),
            hCursor: std::ptr::null_mut(),
            hbrBackground: std::ptr::null_mut(),
            lpszMenuName: std::ptr::null(),
            lpszClassName: class_name.as_ptr(),
        };
        win::RegisterClassW(&class);

        let window_name = to_wide(crate::i18n::t().window_title);
        // An invisible top-level window: the tray needs an HWND to deliver
        // its callback messages to, and nothing is ever drawn in this one.
        let hwnd = win::CreateWindowExW(
            0,
            class_name.as_ptr(),
            window_name.as_ptr(),
            0,
            0,
            0,
            0,
            0,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            class.hInstance,
            std::ptr::null(),
        );
        if hwnd.is_null() {
            return 1;
        }
        if !add_icon(hwnd) {
            return 1;
        }

        let mut message = std::mem::zeroed::<win::MSG>();
        while win::GetMessageW(&mut message, std::ptr::null_mut(), 0, 0) > 0 {
            win::TranslateMessage(&message);
            win::DispatchMessageW(&message);
        }
    }
    0
}

unsafe fn add_icon(hwnd: HWND) -> bool {
    let tip = to_wide(crate::i18n::t().window_title);
    let mut data: NOTIFYICONDATAW = std::mem::zeroed();
    data.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
    data.hWnd = hwnd;
    data.uID = ID_TRAY_ICON;
    data.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
    data.uCallbackMessage = WM_APP_TRAY;
    data.hIcon = win::LoadIconW(std::ptr::null_mut(), win::IDI_APPLICATION);
    // szTip is 128 u16s and the copy below truncates rather than overruns:
    // window titles are short, but a reader can rename anything.
    let room = data.szTip.len();
    for (slot, unit) in data.szTip.iter_mut().zip(tip) {
        *slot = unit;
    }
    let _ = room;
    Shell_NotifyIconW(NIM_ADD, &data) != 0
}

unsafe extern "system" fn wndproc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match message {
        WM_APP_TRAY => {
            let event = lparam as u32;
            match event {
                win::WM_LBUTTONUP => open_page(),
                win::WM_RBUTTONUP => show_menu(hwnd),
                win::WM_LBUTTONDBLCLK => open_page(),
                _ => {}
            }
            0
        }
        win::WM_COMMAND => {
            match (wparam & 0xFFFF) as u32 {
                ID_MENU_OPEN => open_page(),
                ID_MENU_QUIT => {
                    win::DestroyWindow(hwnd);
                }
                _ => {}
            }
            0
        }
        win::WM_CLOSE => {
            win::DestroyWindow(hwnd);
            0
        }
        win::WM_DESTROY => {
            let mut data: NOTIFYICONDATAW = std::mem::zeroed();
            data.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
            data.hWnd = hwnd;
            data.uID = ID_TRAY_ICON;
            Shell_NotifyIconW(NIM_DELETE, &data);
            win::PostQuitMessage(0);
            0
        }
        _ => win::DefWindowProcW(hwnd, message, wparam, lparam),
    }
}

unsafe fn open_page() {
    if let Some(url) = PAGE_URL.get() {
        crate::web::open_url(url);
    }
}

unsafe fn show_menu(hwnd: HWND) {
    let menu = win::CreatePopupMenu();
    if menu.is_null() {
        return;
    }
    let open_text = to_wide(crate::i18n::t().tray_open);
    let quit_text = to_wide(crate::i18n::t().tray_quit);
    win::AppendMenuW(menu, win::MF_STRING, ID_MENU_OPEN as usize, open_text.as_ptr());
    win::AppendMenuW(menu, win::MF_STRING, ID_MENU_QUIT as usize, quit_text.as_ptr());
    // The menu must know the window is foreground or it will not dismiss on
    // an outside click (the classic tray-menu quirk, KB135788).
    win::SetForegroundWindow(hwnd);
    let mut point = std::mem::zeroed::<POINT>();
    win::GetCursorPos(&mut point);
    let chosen = win::TrackPopupMenu(
        menu,
        win::TPM_RETURNCMD | win::TPM_RIGHTBUTTON,
        point.x,
        point.y,
        0,
        hwnd,
        std::ptr::null(),
    );
    win::DestroyMenu(menu);
    if chosen != 0 {
        win::PostMessageW(hwnd, win::WM_COMMAND, chosen as WPARAM, 0);
    }
    win::PostMessageW(hwnd, win::WM_NULL, 0, 0);
}
