//! Win32 bindings for the native front end.
//!
//! The declarations come from Microsoft's `windows-rs` project through the
//! `windows-sys` crate, which generates them straight from the Windows SDK
//! metadata. Everything the GUI touches is re-exported here so the rest of the
//! front end names a single module and the raw ABI stays in one place: no web
//! runtime, no UI toolkit, nothing beyond the system DLLs.

pub use windows_sys::Win32::Foundation::{
    CloseHandle, GlobalFree, HANDLE, HINSTANCE, HWND, INVALID_HANDLE_VALUE, LPARAM, LRESULT, POINT,
    RECT, SIZE, WPARAM,
};

// GDI: double buffered painting, brushes, pens, fonts and text measurement.
pub use windows_sys::Win32::Graphics::Gdi::{
    BeginPaint, BitBlt, ClientToScreen, CreateCompatibleBitmap, CreateCompatibleDC,
    CreateFontIndirectW, CreatePen, CreateSolidBrush, DeleteDC, DeleteObject, DrawTextW, EndPaint,
    FillRect, GetTextExtentPoint32W, InvalidateRect, RoundRect, ScreenToClient, SelectObject,
    SetBkColor, SetBkMode, SetTextColor, HBRUSH, HDC, HFONT, HGDIOBJ, LOGFONTW, PAINTSTRUCT,
    PS_SOLID, SRCCOPY,
};

pub use windows_sys::Win32::System::Com::CoTaskMemFree;

// Clipboard access for "Paste path" and "Copy equivalent CLI command".
pub use windows_sys::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, GetClipboardData, IsClipboardFormatAvailable, OpenClipboard,
    SetClipboardData,
};
pub use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;

// Directory watch for automatic rescans.
pub use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, ReadDirectoryChangesW, FILE_FLAG_BACKUP_SEMANTICS, FILE_LIST_DIRECTORY,
    FILE_NOTIFY_CHANGE_DIR_NAME, FILE_NOTIFY_CHANGE_FILE_NAME, FILE_NOTIFY_CHANGE_LAST_WRITE,
    FILE_NOTIFY_CHANGE_SIZE, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
};
pub use windows_sys::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GHND};
pub use windows_sys::Win32::System::Ole::{CF_HDROP, CF_UNICODETEXT};

// List view, status bar and the standard controls the shell is built from.
pub use windows_sys::Win32::UI::Controls::Dialogs::{
    GetSaveFileNameW, OFN_EXPLORER, OFN_NOCHANGEDIR, OFN_OVERWRITEPROMPT, OFN_PATHMUSTEXIST,
    OPENFILENAMEW,
};
pub use windows_sys::Win32::UI::Controls::{
    InitCommonControlsEx, BST_CHECKED, CDDS_ITEMPREPAINT, CDDS_PREPAINT, CDRF_DODEFAULT,
    CDRF_NOTIFYITEMDRAW, DRAWITEMSTRUCT, EM_SETCUEBANNER, HDN_ITEMCLICKW, ICC_BAR_CLASSES,
    ICC_LISTVIEW_CLASSES, ICC_STANDARD_CLASSES, INITCOMMONCONTROLSEX, LVCFMT_LEFT, LVCFMT_RIGHT,
    LVCF_FMT, LVCF_TEXT, LVCF_WIDTH, LVCOLUMNW, LVIF_TEXT, LVIR_BOUNDS, LVIS_FOCUSED,
    LVIS_SELECTED, LVITEMW, LVM_DELETEALLITEMS, LVM_DELETECOLUMN, LVM_GETHEADER, LVM_GETITEMRECT,
    LVM_GETNEXTITEM, LVM_INSERTCOLUMNW, LVM_INSERTITEMW, LVM_SETBKCOLOR,
    LVM_SETEXTENDEDLISTVIEWSTYLE, LVM_SETITEMSTATE, LVM_SETITEMTEXTW, LVM_SETTEXTBKCOLOR,
    LVM_SETTEXTCOLOR, LVNI_SELECTED, LVS_EX_DOUBLEBUFFER, LVS_EX_FULLROWSELECT, LVS_EX_LABELTIP,
    LVS_REPORT, LVS_SHOWSELALWAYS, LVS_SINGLESEL, NMHDR, NMLISTVIEW, NMLVCUSTOMDRAW, NM_CUSTOMDRAW,
    NM_DBLCLK, NM_RCLICK, ODS_DISABLED, ODS_SELECTED, SBARS_SIZEGRIP, SB_SETPARTS, SB_SETTEXTW,
    WM_MOUSELEAVE,
};
pub use windows_sys::Win32::UI::HiDpi::GetDpiForWindow;
pub use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    EnableWindow, GetFocus, SetFocus, TrackMouseEvent, TME_LEAVE, TRACKMOUSEEVENT, VK_APPS,
    VK_ESCAPE, VK_F10, VK_F5, VK_RETURN, VK_TAB,
};
pub use windows_sys::Win32::UI::Shell::{
    DragQueryFileW, SHBrowseForFolderW, SHGetPathFromIDListW, ShellExecuteW, BIF_EDITBOX,
    BIF_NEWDIALOGSTYLE, BIF_RETURNONLYFSDIRS, BROWSEINFOW, HDROP,
};

// Windows, the messages they send and the structures those messages carry.
pub use windows_sys::Win32::UI::WindowsAndMessaging::{
    AdjustWindowRectEx, AppendMenuW, CreateAcceleratorTableW, CreatePopupMenu, CreateWindowExW,
    DefWindowProcW, DestroyMenu, DestroyWindow, DispatchMessageW, GetClientRect, GetCursorPos,
    GetMessageW, GetSystemMetrics, GetWindowRect, GetWindowTextLengthW, GetWindowTextW, KillTimer,
    LoadCursorW, MoveWindow, PostMessageW, PostQuitMessage, RegisterClassW, SendMessageW,
    SetCursor, SetForegroundWindow, SetTimer, SetWindowPos, SetWindowTextW, ShowWindow,
    TrackPopupMenu, TranslateAcceleratorW, TranslateMessage, ACCEL, BM_GETCHECK, BM_SETCHECK,
    BN_CLICKED, BS_AUTOCHECKBOX, BS_OWNERDRAW, CBN_SELCHANGE, CBS_DROPDOWNLIST, CB_ADDSTRING,
    CB_GETCURSEL, CB_RESETCONTENT, CB_SETCURSEL, CS_HREDRAW, CS_VREDRAW, EN_CHANGE, ES_AUTOHSCROLL,
    FCONTROL, FSHIFT, FVIRTKEY, HMENU, HTCLIENT, HWND_TOP, IDC_ARROW, IDC_HAND,
    LBS_NOINTEGRALHEIGHT, LBS_NOTIFY, LB_ADDSTRING, LB_GETCURSEL, LB_RESETCONTENT, MF_GRAYED,
    MF_SEPARATOR, MF_STRING, MINMAXINFO, MSG, SM_CXSCREEN, SM_CYSCREEN, SWP_NOACTIVATE, SWP_NOMOVE,
    SWP_NOSIZE, SW_HIDE, SW_SHOW, SW_SHOWNORMAL, TPM_RETURNCMD, WM_APP, WM_CLOSE, WM_COMMAND,
    WM_CONTEXTMENU, WM_CREATE, WM_CTLCOLORBTN, WM_CTLCOLOREDIT, WM_CTLCOLORLISTBOX,
    WM_CTLCOLORSTATIC, WM_DESTROY, WM_DPICHANGED, WM_DRAWITEM, WM_ERASEBKGND, WM_GETMINMAXINFO,
    WM_LBUTTONDOWN, WM_MOUSEMOVE, WM_NOTIFY, WM_PAINT, WM_SETCURSOR, WM_SETFONT, WM_SETREDRAW,
    WM_SIZE, WM_TIMER, WNDCLASSW, WNDPROC, WS_BORDER, WS_CHILD, WS_CLIPCHILDREN,
    WS_OVERLAPPEDWINDOW, WS_TABSTOP, WS_VISIBLE, WS_VSCROLL,
};

// The user interface language picked at first run.
pub use windows_sys::Win32::Globalization::GetUserDefaultUILanguage;
