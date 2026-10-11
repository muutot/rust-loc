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
    CreateDIBSection, CreateFontIndirectW, CreatePen, CreateRoundRectRgn, CreateSolidBrush,
    DeleteDC, DeleteObject, DrawTextW, EndPaint, FillRect, GetTextExtentPoint32W, GetWindowDC,
    InvalidateRect, Polyline, ReleaseDC, RoundRect, ScreenToClient, SelectClipRgn, SelectObject,
    SetBkColor, SetBkMode, SetTextColor, SetWindowRgn, BITMAPINFO, BITMAPINFOHEADER, BI_RGB,
    DIB_RGB_COLORS, HBITMAP, HBRUSH, HDC, HFONT, HGDIOBJ, HRGN, LOGFONTW, PAINTSTRUCT, PS_SOLID,
    SRCCOPY,
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
    ImageList_Create, ImageList_Destroy, InitCommonControlsEx, CDDS_ITEMPOSTPAINT,
    CDDS_ITEMPREPAINT, CDDS_POSTPAINT, CDDS_PREPAINT, CDRF_DODEFAULT, CDRF_NOTIFYITEMDRAW,
    CDRF_NOTIFYPOSTPAINT, COMBOBOXINFO, DRAWITEMSTRUCT, EM_SETCUEBANNER, EM_SETMARGINS,
    HDM_GETITEMRECT, HDN_ITEMCLICKW, HIMAGELIST, ICC_BAR_CLASSES, ICC_LISTVIEW_CLASSES,
    ICC_STANDARD_CLASSES, ILC_COLOR32, INITCOMMONCONTROLSEX, LVCFMT_LEFT, LVCFMT_RIGHT, LVCF_FMT,
    LVCF_TEXT, LVCF_WIDTH, LVCOLUMNW, LVIF_TEXT, LVIR_BOUNDS, LVIS_FOCUSED, LVIS_SELECTED, LVITEMW,
    LVM_DELETEALLITEMS, LVM_DELETECOLUMN, LVM_GETHEADER, LVM_GETITEMRECT, LVM_GETITEMSTATE,
    LVM_GETNEXTITEM, LVM_INSERTCOLUMNW, LVM_INSERTITEMW, LVM_SETBKCOLOR,
    LVM_SETEXTENDEDLISTVIEWSTYLE, LVM_SETIMAGELIST, LVM_SETITEMSTATE, LVM_SETITEMTEXTW,
    LVM_SETTEXTBKCOLOR, LVM_SETTEXTCOLOR, LVNI_SELECTED, LVSIL_SMALL, LVS_EX_DOUBLEBUFFER,
    LVS_EX_FULLROWSELECT, LVS_EX_LABELTIP, LVS_REPORT, LVS_SHOWSELALWAYS, LVS_SINGLESEL,
    MEASUREITEMSTRUCT, NMHDR, NMHEADERW, NMLISTVIEW, NMLVCUSTOMDRAW, NM_CUSTOMDRAW, NM_DBLCLK,
    NM_RCLICK, ODS_DISABLED, ODS_FOCUS, ODS_SELECTED, ODT_BUTTON, ODT_COMBOBOX, ODT_LISTBOX,
    SBARS_SIZEGRIP, SB_SETPARTS, SB_SETTEXTW, WM_MOUSELEAVE,
};
pub use windows_sys::Win32::UI::HiDpi::GetDpiForWindow;
pub use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    EnableWindow, GetFocus, SetFocus, TrackMouseEvent, TME_LEAVE, TRACKMOUSEEVENT, VK_APPS,
    VK_ESCAPE, VK_F10, VK_F5, VK_RETURN, VK_TAB,
};
pub use windows_sys::Win32::UI::Shell::{
    DefSubclassProc, DragQueryFileW, SHBrowseForFolderW, SHGetPathFromIDListW, SetWindowSubclass,
    ShellExecuteW, BIF_EDITBOX, BIF_NEWDIALOGSTYLE, BIF_RETURNONLYFSDIRS, BROWSEINFOW, HDROP,
};

// Windows, the messages they send and the structures those messages carry.
pub use windows_sys::Win32::UI::WindowsAndMessaging::{
    AdjustWindowRectEx, AppendMenuW, CreateAcceleratorTableW, CreatePopupMenu, CreateWindowExW,
    DefWindowProcW, DestroyMenu, DestroyWindow, DispatchMessageW, GetClientRect, GetCursorPos,
    GetMessageW, GetSystemMetrics, GetWindowRect, GetWindowTextLengthW, GetWindowTextW, KillTimer,
    LoadCursorW, MoveWindow, PostMessageW, PostQuitMessage, RegisterClassW, SendMessageW,
    SetCursor, SetForegroundWindow, SetTimer, SetWindowPos, SetWindowTextW, ShowWindow,
    TrackPopupMenu, TranslateAcceleratorW, TranslateMessage, ACCEL, BN_CLICKED, BS_OWNERDRAW,
    CBN_SELCHANGE, CBS_DROPDOWNLIST, CBS_HASSTRINGS, CBS_OWNERDRAWFIXED, CB_ADDSTRING,
    CB_GETCOMBOBOXINFO, CB_GETCURSEL, CB_GETITEMHEIGHT, CB_GETLBTEXT, CB_GETLBTEXTLEN,
    CB_RESETCONTENT, CB_SETCURSEL, CS_HREDRAW, CS_VREDRAW, EC_LEFTMARGIN, EC_RIGHTMARGIN,
    EN_CHANGE, EN_KILLFOCUS, EN_SETFOCUS, ES_AUTOHSCROLL, FCONTROL, FSHIFT, FVIRTKEY, HMENU,
    HTCLIENT, HWND_TOP, IDC_ARROW, IDC_HAND, LBN_KILLFOCUS, LBN_SETFOCUS, LBS_HASSTRINGS,
    LBS_NOINTEGRALHEIGHT, LBS_NOTIFY, LBS_OWNERDRAWFIXED, LB_ADDSTRING, LB_GETCURSEL, LB_GETTEXT,
    LB_RESETCONTENT, LB_SETITEMHEIGHT, MF_GRAYED, MF_SEPARATOR, MF_STRING, MINMAXINFO, MSG,
    SM_CXSCREEN, SM_CYSCREEN, SWP_HIDEWINDOW, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER,
    SWP_SHOWWINDOW, SW_HIDE, SW_SHOW, SW_SHOWNORMAL, TPM_RETURNCMD, WM_APP, WM_CLOSE, WM_COMMAND,
    WM_CONTEXTMENU, WM_CREATE, WM_CTLCOLORBTN, WM_CTLCOLOREDIT, WM_CTLCOLORLISTBOX,
    WM_CTLCOLORSTATIC, WM_DESTROY, WM_DPICHANGED, WM_DRAWITEM, WM_ERASEBKGND, WM_GETMINMAXINFO,
    WM_LBUTTONDOWN, WM_MEASUREITEM, WM_MOUSEMOVE, WM_NCPAINT, WM_NOTIFY, WM_PAINT, WM_SETCURSOR,
    WM_SETFONT, WM_SETREDRAW, WM_SIZE, WM_TIMER, WNDCLASSW, WNDPROC, WS_CHILD, WS_CLIPCHILDREN,
    WS_CLIPSIBLINGS, WS_OVERLAPPEDWINDOW, WS_TABSTOP, WS_VISIBLE, WS_VSCROLL,
};

// The user interface language picked at first run.
pub use windows_sys::Win32::Globalization::GetUserDefaultUILanguage;
