use std::ffi::{c_void, CStr, OsStr, OsString};
use std::mem::{size_of, transmute};
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::path::PathBuf;
use std::ptr::{null, null_mut};

use windows_sys::core::HRESULT;
use windows_sys::Win32::Foundation::{
    FreeLibrary, GetLastError, SetLastError, COLORREF, HMODULE, HWND, POINT, RECT,
    RPC_E_CHANGED_MODE,
};
use windows_sys::Win32::Graphics::Dwm::{DwmSetWindowAttribute, DWMWA_USE_IMMERSIVE_DARK_MODE};
use windows_sys::Win32::Graphics::Gdi::{
    CreateFontIndirectW, CreateSolidBrush, DeleteObject, DrawFocusRect, DrawFrameControl, FillRect,
    FrameRect, GetDC, GetDeviceCaps, GetStockObject, GetSysColorBrush, InflateRect, InvalidateRect,
    MapWindowPoints, OffsetRect, ReleaseDC, ScreenToClient, SetBkColor, SetBkMode, SetTextColor,
    COLOR_WINDOW, DEFAULT_CHARSET, DEFAULT_GUI_FONT, DFCS_BUTTONPUSH, DFCS_INACTIVE, DFCS_PUSHED,
    DFC_BUTTON, FW_NORMAL, HBRUSH, HDC, HFONT, HGDIOBJ, LOGFONTW, LOGPIXELSY,
};
use windows_sys::Win32::System::Com::{
    CoInitializeEx, CoTaskMemFree, CoUninitialize, COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE,
};
use windows_sys::Win32::System::LibraryLoader::{
    GetModuleHandleA, GetModuleHandleW, GetProcAddress, LoadLibraryExA,
    LOAD_LIBRARY_SEARCH_SYSTEM32,
};
use windows_sys::Win32::UI::Controls::Dialogs::{
    ChooseFontW, CommDlgExtendedError, CF_FORCEFONTEXIST, CF_INITTOLOGFONTSTRUCT, CF_LIMITSIZE,
    CF_NOSCRIPTSEL, CF_NOSTYLESEL, CF_NOVERTFONTS, CF_SCREENFONTS, CHOOSEFONTW,
};
use windows_sys::Win32::UI::Controls::{
    InitCommonControlsEx, SetWindowTheme, BST_CHECKED, BST_UNCHECKED, CLR_DEFAULT, DRAWITEMSTRUCT,
    EM_SETSEL, HTREEITEM, ICC_LISTVIEW_CLASSES, ICC_TAB_CLASSES, ICC_TREEVIEW_CLASSES,
    INITCOMMONCONTROLSEX, LVCFMT_LEFT, LVCFMT_RIGHT, LVCF_FMT, LVCF_SUBITEM, LVCF_TEXT, LVCF_WIDTH,
    LVCOLUMNW, LVHITTESTINFO, LVHT_ONITEMICON, LVHT_ONITEMLABEL, LVHT_ONITEMSTATEICON, LVIF_IMAGE,
    LVIF_TEXT, LVIR_BOUNDS, LVIS_FOCUSED, LVIS_SELECTED, LVITEMW, LVM_DELETEALLITEMS,
    LVM_DELETEITEM, LVM_EDITLABELW, LVM_ENSUREVISIBLE, LVM_GETITEMCOUNT, LVM_GETITEMRECT,
    LVM_GETITEMSTATE, LVM_GETNEXTITEM, LVM_GETTOPINDEX, LVM_HITTEST, LVM_INSERTCOLUMNW,
    LVM_INSERTITEMW, LVM_SCROLL, LVM_SETBKCOLOR, LVM_SETCOLUMNWIDTH, LVM_SETEXTENDEDLISTVIEWSTYLE,
    LVM_SETIMAGELIST, LVM_SETITEMCOUNT, LVM_SETITEMSTATE, LVM_SETITEMW, LVM_SETTEXTBKCOLOR,
    LVM_SETTEXTCOLOR, LVNI_SELECTED, LVSIL_SMALL, LVS_EDITLABELS, LVS_EX_DOUBLEBUFFER,
    LVS_EX_FULLROWSELECT, LVS_OWNERDATA, LVS_REPORT, LVS_SHOWSELALWAYS, ODS_DISABLED, ODS_FOCUS,
    ODS_SELECTED, ODT_BUTTON, TASKDIALOGCONFIG, TCHITTESTINFO, TCIF_TEXT, TCITEMW,
    TCM_DELETEALLITEMS, TCM_GETCURSEL, TCM_HITTEST, TCM_INSERTITEMW, TCM_SETCURSEL, TCS_FOCUSNEVER,
    TDCBF_OK_BUTTON, TDF_ALLOW_DIALOG_CANCELLATION, TDF_ENABLE_HYPERLINKS,
    TDF_POSITION_RELATIVE_TO_WINDOW, TDF_SIZE_TO_CONTENT, TDN_HYPERLINK_CLICKED,
    TD_INFORMATION_ICON, TVE_EXPAND, TVGN_CARET, TVHITTESTINFO, TVHT_ONITEM, TVHT_ONITEMRIGHT,
    TVIF_CHILDREN, TVIF_PARAM, TVIF_TEXT, TVINSERTSTRUCTW, TVINSERTSTRUCTW_0, TVITEMW, TVI_LAST,
    TVI_ROOT, TVM_DELETEITEM, TVM_EXPAND, TVM_GETITEMW, TVM_GETNEXTITEM, TVM_HITTEST,
    TVM_INSERTITEMW, TVM_SELECTITEM, TVM_SETBKCOLOR, TVM_SETITEMW, TVM_SETLINECOLOR,
    TVM_SETTEXTCOLOR, TVS_HASBUTTONS, TVS_HASLINES, TVS_LINESATROOT, TVS_SHOWSELALWAYS,
    WC_LISTVIEWW, WC_TABCONTROLW, WC_TREEVIEWW,
};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    GetCapture, ReleaseCapture, SetCapture, SetFocus,
};
use windows_sys::Win32::UI::Shell::Common::ITEMIDLIST;
use windows_sys::Win32::UI::Shell::{
    SHBrowseForFolderW, SHGetPathFromIDListEx, ShellExecuteExW, BIF_EDITBOX, BIF_NEWDIALOGSTYLE,
    BIF_RETURNONLYFSDIRS, BIF_SHAREABLE, BIF_VALIDATE, BROWSEINFOW, GPFIDL_DEFAULT,
    SEE_MASK_FLAG_NO_UI, SEE_MASK_UNICODE, SHELLEXECUTEINFOW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DrawIconEx, GetClientRect, GetCursorPos, GetParent, GetSystemMetrics,
    GetWindowRect, GetWindowTextLengthW, GetWindowTextW, LoadCursorW, LoadImageW, MessageBoxW,
    MoveWindow, RegisterClassExW, SendMessageW, SetCursor, SetProcessDPIAware, SetWindowTextW,
    ShowWindow, BM_GETCHECK, BM_SETCHECK, BS_AUTOCHECKBOX, BS_OWNERDRAW, DI_NORMAL, ES_AUTOHSCROLL,
    HICON, HMENU, IDC_ARROW, IDC_SIZEWE, IMAGE_ICON, LR_SHARED, MB_ICONINFORMATION, MB_OK,
    MINMAXINFO, SB_LINEDOWN, SB_LINEUP, SM_CXICON, SM_CXSMICON, SM_CYICON, SM_CYSMICON, SW_HIDE,
    SW_SHOW, SW_SHOWNORMAL, WM_SETFONT, WM_SETREDRAW, WM_VSCROLL, WNDCLASSEXW, WS_CHILD,
    WS_CLIPCHILDREN, WS_EX_CLIENTEDGE, WS_OVERLAPPEDWINDOW, WS_TABSTOP, WS_VISIBLE,
};

use crate::domain::{
    decide_vertical_auto_scroll_direction, AppearanceFont, AppearanceTheme, AutoScrollDirection,
    ExplorerError, ExplorerResult, DEFAULT_APPEARANCE_FONT_POINT_SIZE,
    MAX_APPEARANCE_FONT_POINT_SIZE, MIN_APPEARANCE_FONT_POINT_SIZE,
};

use super::shell_icon::ShellImageListHandle;

mod dpi;
mod handles;
mod menus;
mod messages;

pub use self::dpi::{
    DpiAwarenessFailure, DpiAwarenessFailureReason, DpiAwarenessOutcome, DpiAwarenessStep,
    DpiMetrics, UiScale,
};
use self::dpi::{DpiAwarenessOperation, SYSTEM_AWARE_DPI_STEPS};
pub use self::handles::{
    ClientPoint, ClientRect, IconHandle, InstanceHandle, MenuHandle, MessageLong, MessageResult,
    MessageWord, RawWindowHandle, ScreenPoint, WindowHandle, WindowProcedure,
};
pub use self::menus::{
    append_checked_menu_item, append_menu_item, append_menu_popup, append_menu_separator,
    append_owned_menu_popup, create_menu_bar, create_popup_menu, destroy_menu, draw_menu_bar,
    set_window_menu, track_popup_menu, OwnedMenu,
};
pub use self::messages::{
    attach_window_state_from_nccreate, command_id, command_notification, create_accelerator_table,
    default_window_proc, destroy_window, kill_window_timer, list_view_activation_index,
    list_view_column_click_index, list_view_display_request, list_view_drag_index,
    list_view_label_edit, message_loop, notification, post_quit_message, post_window_message,
    set_list_view_display_image, set_list_view_display_text, set_window_timer, show_error_message,
    take_window_state, tree_view_drag_notification, tree_view_expand_notification,
    window_state_mut, Accelerator, AcceleratorTable, ControlKeyCommand, ControlNotification,
    ListViewDisplayRequest, ListViewLabelEdit, TreeViewExpandAction, TreeViewExpandNotification,
    TreeViewItemNotification, EDIT_KILL_FOCUS, KEY_DELETE, KEY_ESCAPE, KEY_F2, KEY_F3, KEY_F5,
    KEY_LEFT, KEY_RIGHT, KEY_TAB, KEY_UP, LIST_VIEW_BEGIN_DRAG, LIST_VIEW_COLUMN_CLICK,
    LIST_VIEW_END_LABEL_EDIT, LIST_VIEW_GET_DISPLAY_INFO, LIST_VIEW_ITEM_ACTIVATE,
    LIST_VIEW_RIGHT_CLICK, MESSAGE_APP, MESSAGE_CAPTURE_CHANGED, MESSAGE_COMMAND,
    MESSAGE_CONTROL_COLOR_BUTTON, MESSAGE_CONTROL_COLOR_EDIT, MESSAGE_CONTROL_COLOR_STATIC,
    MESSAGE_CREATE, MESSAGE_DESTROY, MESSAGE_DPI_CHANGED, MESSAGE_DRAW_ITEM,
    MESSAGE_ENTER_SIZE_MOVE, MESSAGE_ERASE_BACKGROUND, MESSAGE_EXIT_SIZE_MOVE,
    MESSAGE_GET_MIN_MAX_INFO, MESSAGE_LEFT_BUTTON_DOWN, MESSAGE_LEFT_BUTTON_UP, MESSAGE_MOUSE_MOVE,
    MESSAGE_NC_CREATE, MESSAGE_NC_DESTROY, MESSAGE_NOTIFY, MESSAGE_SET_CURSOR, MESSAGE_SIZE,
    MESSAGE_TIMER, NOTIFICATION_DBL_CLICK, TAB_RIGHT_CLICK, TAB_SELECTION_CHANGED,
    TREE_VIEW_BEGIN_DRAG, TREE_VIEW_ITEM_EXPANDING, TREE_VIEW_RIGHT_CLICK,
    TREE_VIEW_SELECTION_CHANGED,
};
const TREE_VIEW_NO_ITEM_VALUE: isize = -1;

const DARK_MODE_EXPLORER_THEME: [u16; 18] = [
    'D' as u16, 'a' as u16, 'r' as u16, 'k' as u16, 'M' as u16, 'o' as u16, 'd' as u16, 'e' as u16,
    '_' as u16, 'E' as u16, 'x' as u16, 'p' as u16, 'l' as u16, 'o' as u16, 'r' as u16, 'e' as u16,
    'r' as u16, 0,
];
const GDI_OPAQUE_BACKGROUND_MODE: i32 = 2;
const INVALIDATE_WITH_ERASE: i32 = 1;
const INVALIDATE_WITHOUT_ERASE: i32 = 0;
const TREE_VIEW_USE_SYSTEM_COLOR: isize = -1;
const MATERIAL_ICON_COLOR: COLORREF = 0x00ff863a;
const MATERIAL_ICON_GLYPH_RATIO_NUMERATOR: i32 = 3;
const MATERIAL_ICON_GLYPH_RATIO_DENOMINATOR: i32 = 5;
const STARTUP_FOLDER_PATH_BUFFER_LEN: usize = 32_768;

type TaskDialogIndirectFn = unsafe extern "system" fn(
    *const TASKDIALOGCONFIG,
    *mut i32,
    *mut i32,
    *mut windows_sys::core::BOOL,
) -> HRESULT;

#[derive(Clone, Copy)]
struct ThemePalette {
    window_background: COLORREF,
    control_background: COLORREF,
    control_text: COLORREF,
    tree_line: COLORREF,
    custom_controls: bool,
}

impl ThemePalette {
    fn for_theme(theme: AppearanceTheme) -> Self {
        match theme {
            AppearanceTheme::Light => Self {
                window_background: rgb(240, 240, 240),
                control_background: rgb(255, 255, 255),
                control_text: rgb(0, 0, 0),
                tree_line: rgb(160, 160, 160),
                custom_controls: false,
            },
            AppearanceTheme::ClassicDark => Self {
                window_background: rgb(31, 33, 36),
                control_background: rgb(24, 26, 29),
                control_text: rgb(230, 232, 235),
                tree_line: rgb(92, 97, 105),
                custom_controls: true,
            },
            AppearanceTheme::SepiaTeal => Self {
                window_background: rgb(24, 25, 24),
                control_background: rgb(31, 52, 56),
                control_text: rgb(236, 232, 219),
                tree_line: rgb(178, 154, 124),
                custom_controls: true,
            },
            AppearanceTheme::Graphite => Self {
                window_background: rgb(24, 25, 26),
                control_background: rgb(50, 55, 63),
                control_text: rgb(239, 236, 229),
                tree_line: rgb(126, 119, 105),
                custom_controls: true,
            },
            AppearanceTheme::Forest => Self {
                window_background: rgb(22, 25, 23),
                control_background: rgb(39, 59, 63),
                control_text: rgb(236, 239, 229),
                tree_line: rgb(104, 150, 117),
                custom_controls: true,
            },
            AppearanceTheme::SteelBlue => Self {
                window_background: rgb(24, 25, 27),
                control_background: rgb(54, 64, 80),
                control_text: rgb(239, 240, 242),
                tree_line: rgb(104, 139, 171),
                custom_controls: true,
            },
        }
    }

    fn uses_custom_controls(self) -> bool {
        self.custom_controls
    }
}

pub struct ThemeResources {
    window_brush: GdiBrush,
    control_brush: GdiBrush,
}

impl ThemeResources {
    pub fn new(theme: AppearanceTheme) -> ExplorerResult<Self> {
        let palette = ThemePalette::for_theme(theme);
        Ok(Self {
            window_brush: GdiBrush::new(palette.window_background)?,
            control_brush: GdiBrush::new(palette.control_background)?,
        })
    }

    fn window_brush(&self) -> HBRUSH {
        self.window_brush.handle()
    }

    fn control_brush(&self) -> HBRUSH {
        self.control_brush.handle()
    }
}

struct GdiBrush {
    handle: HBRUSH,
}

impl GdiBrush {
    fn new(color: COLORREF) -> ExplorerResult<Self> {
        // SAFETY: CreateSolidBrush creates an owned GDI brush for a plain COLORREF.
        let handle = unsafe { CreateSolidBrush(color) };
        if handle.is_null() {
            return Err(windows_api_error("create theme brush", "CreateSolidBrush"));
        }

        Ok(Self { handle })
    }

    fn handle(&self) -> HBRUSH {
        self.handle
    }
}

impl Drop for GdiBrush {
    fn drop(&mut self) {
        if !self.handle.is_null() {
            // SAFETY: this brush handle is owned by GdiBrush and is dropped exactly once.
            unsafe {
                DeleteObject(self.handle as HGDIOBJ);
            }
        }
    }
}

struct DialogComApartment {
    should_uninitialize: bool,
}

impl DialogComApartment {
    fn initialize(operation: &'static str) -> ExplorerResult<Self> {
        let coinit = (COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE) as u32;
        // SAFETY: CoInitializeEx accepts a null reserved pointer and initializes COM for the
        // current thread. The matching CoUninitialize call is guarded by DialogComApartment::drop.
        let hresult = unsafe { CoInitializeEx(null(), coinit) };
        if hresult == RPC_E_CHANGED_MODE {
            return Err(ExplorerError::windows_hresult(
                operation,
                "CoInitializeEx",
                hresult,
                None,
            ));
        }
        if hresult < 0 {
            return Err(ExplorerError::windows_hresult(
                operation,
                "CoInitializeEx",
                hresult,
                None,
            ));
        }

        Ok(Self {
            should_uninitialize: true,
        })
    }
}

impl Drop for DialogComApartment {
    fn drop(&mut self) {
        if self.should_uninitialize {
            // SAFETY: this balances a successful CoInitializeEx call on the current thread.
            unsafe {
                CoUninitialize();
            }
        }
    }
}

pub fn show_about_dialog(
    owner: WindowHandle,
    program_name: &str,
    version: &str,
    link: &str,
) -> ExplorerResult<()> {
    let Some((task_dialog_library, task_dialog_indirect)) = task_dialog_indirect_proc() else {
        show_about_fallback_message(owner, program_name, version, link);
        return Ok(());
    };
    let _task_dialog_library = task_dialog_library;

    let window_title = str_to_wide_null(&format!("About {program_name}"));
    let main_instruction = str_to_wide_null(program_name);
    let content = str_to_wide_null(&format!(
        "Version {version}\n\n<a href=\"{link}\">{link}</a>"
    ));

    let mut selected_button = 0;
    let mut config = TASKDIALOGCONFIG {
        cbSize: size_of::<TASKDIALOGCONFIG>() as u32,
        hwndParent: owner.raw(),
        dwFlags: TDF_ENABLE_HYPERLINKS
            | TDF_ALLOW_DIALOG_CANCELLATION
            | TDF_POSITION_RELATIVE_TO_WINDOW
            | TDF_SIZE_TO_CONTENT,
        dwCommonButtons: TDCBF_OK_BUTTON,
        pszWindowTitle: window_title.as_ptr(),
        pszMainInstruction: main_instruction.as_ptr(),
        pszContent: content.as_ptr(),
        pfCallback: Some(about_task_dialog_callback),
        ..Default::default()
    };
    config.Anonymous1.pszMainIcon = TD_INFORMATION_ICON;

    // SAFETY: config points to a fully initialized TASKDIALOGCONFIG. All string buffers are
    // null-terminated and live until TaskDialogIndirect returns.
    let hresult =
        unsafe { task_dialog_indirect(&config, &mut selected_button, null_mut(), null_mut()) };
    if hresult < 0 {
        return Err(ExplorerError::windows_hresult(
            "show about dialog",
            "TaskDialogIndirect",
            hresult,
            None,
        ));
    }

    Ok(())
}

fn task_dialog_indirect_proc() -> Option<(DynamicLibrary, TaskDialogIndirectFn)> {
    let library = DynamicLibrary::load(c"comctl32.dll")?;
    let proc = library.proc(c"TaskDialogIndirect")?;
    // SAFETY: the symbol name selects TaskDialogIndirect from comctl32.dll.
    let task_dialog_indirect = unsafe { transmute(proc) };
    Some((library, task_dialog_indirect))
}

fn show_about_fallback_message(owner: WindowHandle, program_name: &str, version: &str, link: &str) {
    let title = str_to_wide_null(&format!("About {program_name}"));
    let message = str_to_wide_null(&format!("{program_name}\nVersion {version}\n\n{link}"));

    // SAFETY: strings are null terminated and live through the call; owner may be null.
    unsafe {
        MessageBoxW(
            owner.raw(),
            message.as_ptr(),
            title.as_ptr(),
            MB_OK | MB_ICONINFORMATION,
        );
    }
}

unsafe extern "system" fn about_task_dialog_callback(
    hwnd: HWND,
    msg: u32,
    _wparam: usize,
    lparam: isize,
    _callback_data: isize,
) -> HRESULT {
    if msg == TDN_HYPERLINK_CLICKED as u32 && lparam != 0 {
        if let Err(error) = open_about_link(hwnd, lparam as *const u16) {
            show_error_message(
                WindowHandle::from_sys(hwnd),
                "j3Files",
                &error.user_message(),
            );
        }
    }

    0
}

fn open_about_link(owner: HWND, link: *const u16) -> ExplorerResult<()> {
    if link.is_null() {
        return Err(ExplorerError::state_conflict("링크 주소가 없습니다."));
    }

    let _apartment = DialogComApartment::initialize("open about link")?;

    // SAFETY: SHELLEXECUTEINFOW is a C POD struct. Zero initialization is the documented baseline
    // before setting cbSize and the fields used by ShellExecuteExW.
    let mut execute_info: SHELLEXECUTEINFOW = unsafe { std::mem::zeroed() };
    execute_info.cbSize = size_of::<SHELLEXECUTEINFOW>() as u32;
    execute_info.fMask = SEE_MASK_FLAG_NO_UI | SEE_MASK_UNICODE;
    execute_info.hwnd = owner;
    execute_info.lpFile = link;
    execute_info.nShow = SW_SHOWNORMAL;

    clear_last_error();
    // SAFETY: execute_info points to a valid initialized structure. link is the null-terminated
    // hyperlink URL supplied by the active TaskDialog callback.
    let succeeded = unsafe { ShellExecuteExW(&mut execute_info) };
    if succeeded == 0 {
        return Err(about_link_error(&execute_info));
    }

    Ok(())
}

fn about_link_error(execute_info: &SHELLEXECUTEINFOW) -> ExplorerError {
    let code = shell_execute_error_code(execute_info).unwrap_or_else(last_error_code);
    ExplorerError::windows_api("open about link", "ShellExecuteExW", code, None)
}

fn shell_execute_error_code(execute_info: &SHELLEXECUTEINFOW) -> Option<u32> {
    let last_error = last_error_code();
    if last_error != 0 {
        return Some(last_error);
    }

    let shell_error = execute_info.hInstApp as isize;
    if (1..=32).contains(&shell_error) {
        Some(shell_error as u32)
    } else {
        None
    }
}

struct CoTaskMemPidl(*mut ITEMIDLIST);

impl Drop for CoTaskMemPidl {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: SHBrowseForFolderW returns a PIDL allocated with the COM task allocator.
            unsafe {
                CoTaskMemFree(self.0.cast());
            }
        }
    }
}

pub struct FontResource {
    handle: HFONT,
    owned: bool,
}

impl FontResource {
    pub fn new(font: &AppearanceFont, metrics: DpiMetrics) -> ExplorerResult<Self> {
        if !font.is_custom() {
            return Self::default_gui_font(metrics);
        }

        let logfont = logfont_for_appearance_font(font, metrics);
        // SAFETY: logfont points to a fully initialized LOGFONTW for the duration of the call.
        let handle = unsafe { CreateFontIndirectW(&logfont) };
        if handle.is_null() {
            return Err(windows_api_error("create UI font", "CreateFontIndirectW"));
        }

        Ok(Self {
            handle,
            owned: true,
        })
    }

    fn default_gui_font(metrics: DpiMetrics) -> ExplorerResult<Self> {
        let logfont = default_gui_logfont(metrics);
        // SAFETY: logfont points to a fully initialized LOGFONTW for the duration of the call.
        let handle = unsafe { CreateFontIndirectW(&logfont) };
        if handle.is_null() {
            return Err(windows_api_error(
                "create default UI font",
                "CreateFontIndirectW",
            ));
        }

        Ok(Self {
            handle,
            owned: true,
        })
    }

    fn handle(&self) -> HFONT {
        self.handle
    }
}

impl Drop for FontResource {
    fn drop(&mut self) {
        if self.owned && !self.handle.is_null() {
            // SAFETY: this HFONT was created by CreateFontIndirectW and is owned by FontResource.
            unsafe {
                DeleteObject(self.handle as HGDIOBJ);
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColumnAlign {
    Left,
    Right,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ListViewColumn<'a> {
    pub title: &'a str,
    pub width: i32,
    pub align: ColumnAlign,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListViewRow {
    pub cells: Vec<String>,
    pub image_index: Option<i32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ListViewViewport {
    top_index: usize,
}

#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TreeViewItemHandle(HTREEITEM);

impl TreeViewItemHandle {
    pub(super) fn from_raw(raw: HTREEITEM) -> Option<Self> {
        if raw == 0 {
            None
        } else {
            Some(Self(raw))
        }
    }

    fn raw(self) -> HTREEITEM {
        self.0
    }
}

#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TreeViewItemValue(usize);

impl TreeViewItemValue {
    pub const fn new(value: usize) -> Self {
        Self(value)
    }

    pub const fn get(self) -> usize {
        self.0
    }

    fn to_lparam(self) -> ExplorerResult<isize> {
        isize::try_from(self.0)
            .map_err(|_| ExplorerError::state_conflict("폴더 트리 항목 식별자가 너무 큽니다."))
    }

    pub(super) fn from_lparam(value: isize) -> Option<Self> {
        usize::try_from(value).ok().map(Self)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TreeViewItem<'a> {
    pub text: &'a str,
    pub value: Option<TreeViewItemValue>,
    pub has_children: bool,
}

pub fn initialize_common_controls() -> ExplorerResult<()> {
    let controls = INITCOMMONCONTROLSEX {
        dwSize: size_of::<INITCOMMONCONTROLSEX>() as u32,
        dwICC: ICC_LISTVIEW_CLASSES | ICC_TAB_CLASSES | ICC_TREEVIEW_CLASSES,
    };

    // SAFETY: controls points to a valid INITCOMMONCONTROLSEX for the duration of the call.
    let succeeded = unsafe { InitCommonControlsEx(&controls) };
    if succeeded == 0 {
        return Err(windows_api_error(
            "initialize common controls",
            "InitCommonControlsEx",
        ));
    }

    Ok(())
}

pub fn configure_process_dpi_awareness() -> DpiAwarenessOutcome {
    let mut failures = Vec::new();

    for step in SYSTEM_AWARE_DPI_STEPS {
        match apply_dpi_awareness_step(step) {
            Ok(()) => {
                return DpiAwarenessOutcome {
                    applied: Some(step),
                    failures,
                };
            }
            Err(reason) => failures.push(DpiAwarenessFailure { step, reason }),
        }
    }

    DpiAwarenessOutcome {
        applied: None,
        failures,
    }
}

pub fn system_dpi_metrics() -> DpiMetrics {
    DpiMetrics::new(system_dpi())
}

pub fn dpi_metrics_for_window(hwnd: WindowHandle) -> DpiMetrics {
    if !hwnd.is_null() {
        if let Some(dpi) = dpi_for_window(hwnd) {
            return DpiMetrics::new(dpi);
        }
    }

    system_dpi_metrics()
}

pub fn dpi_from_changed_message(wparam: MessageWord) -> DpiMetrics {
    DpiMetrics::new((wparam & 0xffff) as u32)
}

pub fn set_minimum_tracking_size(lparam: MessageLong, width: i32, height: i32) -> bool {
    if lparam == 0 {
        return false;
    }

    // SAFETY: WM_GETMINMAXINFO supplies a writable MINMAXINFO pointer.
    let Some(info) = (unsafe { (lparam as *mut MINMAXINFO).as_mut() }) else {
        return false;
    };
    info.ptMinTrackSize.x = width.max(1);
    info.ptMinTrackSize.y = height.max(1);
    true
}

pub fn module_handle() -> ExplorerResult<InstanceHandle> {
    // SAFETY: null asks for the current process module handle and has no ownership transfer.
    let handle = unsafe { GetModuleHandleW(null()) };
    if handle.is_null() {
        return Err(windows_api_error("read module handle", "GetModuleHandleW"));
    }

    Ok(InstanceHandle::from_sys(handle))
}

pub fn register_window_class(
    instance: InstanceHandle,
    class_name: &str,
    wnd_proc: WindowProcedure,
    icon_resource_id: Option<u16>,
) -> ExplorerResult<()> {
    let class_name = str_to_wide_null(class_name);

    // SAFETY: loading a predefined cursor with a null instance is the documented Win32 pattern.
    let cursor = unsafe { LoadCursorW(null_mut(), IDC_ARROW) };
    if cursor.is_null() {
        return Err(windows_api_error("load cursor", "LoadCursorW"));
    }

    let large_icon = load_optional_icon_resource(instance, icon_resource_id, SM_CXICON, SM_CYICON)?;
    let small_icon =
        load_optional_icon_resource(instance, icon_resource_id, SM_CXSMICON, SM_CYSMICON)?;

    let window_class = WNDCLASSEXW {
        cbSize: size_of::<WNDCLASSEXW>() as u32,
        lpfnWndProc: wnd_proc,
        hInstance: instance.raw(),
        hIcon: large_icon,
        hIconSm: small_icon,
        hCursor: cursor,
        // SAFETY: GetSysColorBrush returns a system-owned brush; the caller must not destroy it.
        hbrBackground: unsafe { GetSysColorBrush(COLOR_WINDOW) },
        lpszClassName: class_name.as_ptr(),
        ..Default::default()
    };

    // SAFETY: window_class contains a valid class name buffer and a valid window procedure.
    let atom = unsafe { RegisterClassExW(&window_class) };
    if atom == 0 {
        return Err(windows_api_error(
            "register window class",
            "RegisterClassExW",
        ));
    }

    Ok(())
}

pub fn create_main_window(
    instance: InstanceHandle,
    class_name: &str,
    title: &str,
    create_params: *mut c_void,
    initial_size: (i32, i32),
) -> ExplorerResult<WindowHandle> {
    let class_name = str_to_wide_null(class_name);
    let title = str_to_wide_null(title);
    let (width, height) = initial_size;

    // SAFETY: class/title buffers live through the call. create_params is an application-owned
    // pointer passed back in WM_NCCREATE without ownership transfer at this boundary.
    let hwnd = unsafe {
        CreateWindowExW(
            0,
            class_name.as_ptr(),
            title.as_ptr(),
            WS_OVERLAPPEDWINDOW | WS_CLIPCHILDREN,
            100,
            100,
            width.max(1),
            height.max(1),
            null_mut(),
            null_mut(),
            instance.raw(),
            create_params.cast_const(),
        )
    };
    if hwnd.is_null() {
        return Err(windows_api_error("create main window", "CreateWindowExW"));
    }

    Ok(WindowHandle::from_sys(hwnd))
}

pub fn create_button(
    parent: WindowHandle,
    instance: InstanceHandle,
    id: u16,
    text: &str,
) -> ExplorerResult<WindowHandle> {
    create_child_window(parent, instance, "BUTTON", text, id, WS_TABSTOP, 0)
}

pub fn create_checkbox(
    parent: WindowHandle,
    instance: InstanceHandle,
    id: u16,
    text: &str,
) -> ExplorerResult<WindowHandle> {
    create_child_window(
        parent,
        instance,
        "BUTTON",
        text,
        id,
        WS_TABSTOP | BS_AUTOCHECKBOX as u32,
        0,
    )
}

pub fn is_button_checked(button: WindowHandle) -> bool {
    if button.is_null() {
        return false;
    }

    // SAFETY: button is a BUTTON control handle; BM_GETCHECK has no pointer arguments.
    unsafe { SendMessageW(button.raw(), BM_GETCHECK, 0, 0) == BST_CHECKED as isize }
}

pub fn set_button_checked(button: WindowHandle, checked: bool) {
    if button.is_null() {
        return;
    }

    let state = if checked { BST_CHECKED } else { BST_UNCHECKED };
    // SAFETY: button is a BUTTON control handle; BM_SETCHECK uses plain value parameters.
    unsafe {
        SendMessageW(button.raw(), BM_SETCHECK, state as usize, 0);
    }
}

pub fn create_icon_button(
    parent: WindowHandle,
    instance: InstanceHandle,
    id: u16,
    accessible_text: &str,
) -> ExplorerResult<WindowHandle> {
    create_child_window(
        parent,
        instance,
        "BUTTON",
        accessible_text,
        id,
        WS_TABSTOP | BS_OWNERDRAW as u32,
        0,
    )
}

pub fn load_shared_icon_resource(
    instance: InstanceHandle,
    resource_id: u16,
    size: i32,
) -> ExplorerResult<IconHandle> {
    load_icon_resource(instance, resource_id, size, size).map(IconHandle::from_sys)
}

pub fn create_address_edit(
    parent: WindowHandle,
    instance: InstanceHandle,
    id: u16,
) -> ExplorerResult<WindowHandle> {
    create_child_window(
        parent,
        instance,
        "EDIT",
        "",
        id,
        WS_TABSTOP | ES_AUTOHSCROLL as u32,
        WS_EX_CLIENTEDGE,
    )
}

pub fn create_label(
    parent: WindowHandle,
    instance: InstanceHandle,
    id: u16,
    text: &str,
) -> ExplorerResult<WindowHandle> {
    create_child_window(parent, instance, "STATIC", text, id, 0, 0)
}

pub fn create_tab_control(
    parent: WindowHandle,
    instance: InstanceHandle,
    id: u16,
) -> ExplorerResult<WindowHandle> {
    create_child_window_with_class_ptr(
        parent,
        instance,
        WC_TABCONTROLW,
        "",
        id,
        WS_TABSTOP | TCS_FOCUSNEVER,
        0,
    )
}

pub fn create_tree_view(
    parent: WindowHandle,
    instance: InstanceHandle,
    id: u16,
) -> ExplorerResult<WindowHandle> {
    create_child_window_with_class_ptr(
        parent,
        instance,
        WC_TREEVIEWW,
        "",
        id,
        WS_TABSTOP | TVS_HASBUTTONS | TVS_HASLINES | TVS_LINESATROOT | TVS_SHOWSELALWAYS,
        WS_EX_CLIENTEDGE,
    )
}

pub fn create_report_list_view(
    parent: WindowHandle,
    instance: InstanceHandle,
    id: u16,
) -> ExplorerResult<WindowHandle> {
    let hwnd = create_child_window_with_class_ptr(
        parent,
        instance,
        WC_LISTVIEWW,
        "",
        id,
        WS_TABSTOP | LVS_REPORT | LVS_OWNERDATA | LVS_SHOWSELALWAYS | LVS_EDITLABELS,
        WS_EX_CLIENTEDGE,
    )?;

    // SAFETY: hwnd is a ListView handle created above; style values are plain bit flags.
    let extended_style = LVS_EX_FULLROWSELECT | LVS_EX_DOUBLEBUFFER;
    unsafe {
        SendMessageW(
            hwnd.raw(),
            LVM_SETEXTENDEDLISTVIEWSTYLE,
            extended_style as usize,
            extended_style as isize,
        );
    }

    Ok(hwnd)
}

pub fn apply_window_theme(hwnd: WindowHandle, theme: AppearanceTheme) {
    let dark_theme = theme.uses_dark_mode();
    let enabled: i32 = if dark_theme { 1 } else { 0 };

    if !hwnd.is_null() {
        // SAFETY: hwnd is an application window and the attribute pointer is valid for the call.
        let _ = unsafe {
            DwmSetWindowAttribute(
                hwnd.raw(),
                DWMWA_USE_IMMERSIVE_DARK_MODE as u32,
                &enabled as *const i32 as *const c_void,
                size_of::<i32>() as u32,
            )
        };
        invalidate_window(hwnd);
    }
}

pub fn apply_control_theme(theme: AppearanceTheme, controls: &[WindowHandle]) {
    let theme_name = if theme.uses_dark_mode() {
        DARK_MODE_EXPLORER_THEME.as_ptr()
    } else {
        null()
    };

    for control in controls {
        if control.is_null() {
            continue;
        }

        // SAFETY: control is a child HWND created by this module. SetWindowTheme does not take ownership.
        let _ = unsafe { SetWindowTheme(control.raw(), theme_name, null()) };
        invalidate_window(*control);
    }
}

pub fn apply_font(font: &FontResource, controls: &[WindowHandle]) {
    for control in controls {
        if control.is_null() {
            continue;
        }

        // SAFETY: WM_SETFONT does not transfer ownership; FontResource outlives the controls.
        unsafe {
            SendMessageW(control.raw(), WM_SETFONT, font.handle() as usize, 1);
        }
        invalidate_window(*control);
    }
}

pub fn choose_font(
    owner: WindowHandle,
    current_font: &AppearanceFont,
) -> ExplorerResult<Option<AppearanceFont>> {
    let mut logfont = logfont_for_appearance_font(current_font, dpi_metrics_for_window(owner));
    let mut choose_font = CHOOSEFONTW {
        lStructSize: size_of::<CHOOSEFONTW>() as u32,
        hwndOwner: owner.raw(),
        lpLogFont: &mut logfont,
        iPointSize: i32::from(current_font.point_size()) * 10,
        Flags: CF_SCREENFONTS
            | CF_INITTOLOGFONTSTRUCT
            | CF_LIMITSIZE
            | CF_FORCEFONTEXIST
            | CF_NOSTYLESEL
            | CF_NOSCRIPTSEL
            | CF_NOVERTFONTS,
        nSizeMin: i32::from(MIN_APPEARANCE_FONT_POINT_SIZE),
        nSizeMax: i32::from(MAX_APPEARANCE_FONT_POINT_SIZE),
        ..Default::default()
    };

    // SAFETY: choose_font points to initialized CHOOSEFONTW and lpLogFont is writable.
    let succeeded = unsafe { ChooseFontW(&mut choose_font) };
    if succeeded == 0 {
        // SAFETY: CommDlgExtendedError reads thread-local common dialog error state.
        let code = unsafe { CommDlgExtendedError() };
        if code == 0 {
            return Ok(None);
        }

        return Err(ExplorerError::windows_api(
            "choose font",
            "ChooseFontW",
            code,
            None,
        ));
    }

    let point_size = (choose_font.iPointSize + 5) / 10;
    let point_size = u16::try_from(point_size)
        .map_err(|_| ExplorerError::invalid_input("선택한 글꼴 크기를 적용할 수 없습니다."))?;
    let family_name = logfont_face_name(&logfont);
    let font = AppearanceFont::custom(family_name, point_size)
        .ok_or_else(|| ExplorerError::invalid_input("선택한 글꼴 정보를 적용할 수 없습니다."))?;
    Ok(Some(font))
}

pub fn choose_startup_folder(owner: WindowHandle) -> ExplorerResult<Option<PathBuf>> {
    let _apartment = DialogComApartment::initialize("choose startup folder")?;
    let title = str_to_wide_null("시작할 폴더를 선택하세요.");
    let mut display_name = [0_u16; 260];
    let browse_info = BROWSEINFOW {
        hwndOwner: owner.raw(),
        pidlRoot: null_mut(),
        pszDisplayName: display_name.as_mut_ptr(),
        lpszTitle: title.as_ptr(),
        ulFlags: BIF_RETURNONLYFSDIRS
            | BIF_SHAREABLE
            | BIF_NEWDIALOGSTYLE
            | BIF_EDITBOX
            | BIF_VALIDATE,
        ..Default::default()
    };

    // SAFETY: browse_info points to initialized BROWSEINFOW data whose string buffers remain
    // alive for the duration of the modal folder picker call.
    let pidl = unsafe { SHBrowseForFolderW(&browse_info) };
    if pidl.is_null() {
        return Ok(None);
    }
    let pidl = CoTaskMemPidl(pidl);

    let mut path = vec![0_u16; STARTUP_FOLDER_PATH_BUFFER_LEN];
    let path_len = u32::try_from(path.len())
        .map_err(|_| ExplorerError::invalid_input("시작 폴더 경로 버퍼를 만들 수 없습니다."))?;
    // SAFETY: pidl is returned by SHBrowseForFolderW, and path is a writable UTF-16 buffer with
    // the length passed to the API.
    let succeeded =
        unsafe { SHGetPathFromIDListEx(pidl.0, path.as_mut_ptr(), path_len, GPFIDL_DEFAULT) };
    if succeeded == 0 {
        return Err(windows_api_error(
            "read selected startup folder",
            "SHGetPathFromIDListEx",
        ));
    }

    let folder = PathBuf::from(wide_buffer_to_os_string(&path));
    if folder.as_os_str().is_empty() {
        return Err(ExplorerError::invalid_input(
            "선택한 시작 폴더 경로가 비어 있습니다.",
        ));
    }

    Ok(Some(folder))
}

pub fn apply_tree_view_theme(tree_view: WindowHandle, theme: AppearanceTheme) {
    if tree_view.is_null() {
        return;
    }

    let palette = ThemePalette::for_theme(theme);
    let colors = TreeViewThemeColors::for_theme(palette);
    // SAFETY: tree_view is a TreeView handle and these messages only set COLORREF values.
    unsafe {
        SendMessageW(tree_view.raw(), TVM_SETBKCOLOR, 0, colors.background);
        SendMessageW(tree_view.raw(), TVM_SETTEXTCOLOR, 0, colors.text);
        SendMessageW(tree_view.raw(), TVM_SETLINECOLOR, 0, colors.line);
    }
    invalidate_window(tree_view);
}

pub fn apply_list_view_theme(list_view: WindowHandle, theme: AppearanceTheme) {
    if list_view.is_null() {
        return;
    }

    let palette = ThemePalette::for_theme(theme);
    let colors = ListViewThemeColors::for_theme(palette);
    // SAFETY: list_view is a ListView handle and these messages only set COLORREF values.
    unsafe {
        SendMessageW(list_view.raw(), LVM_SETBKCOLOR, 0, colors.background);
        SendMessageW(list_view.raw(), LVM_SETTEXTCOLOR, 0, colors.text);
        SendMessageW(
            list_view.raw(),
            LVM_SETTEXTBKCOLOR,
            0,
            colors.text_background,
        );
    }
    invalidate_window(list_view);
}

pub fn erase_window_background(
    hwnd: WindowHandle,
    theme: AppearanceTheme,
    resources: &ThemeResources,
    wparam: MessageWord,
) -> Option<MessageResult> {
    let palette = ThemePalette::for_theme(theme);
    if !palette.uses_custom_controls() {
        return None;
    }

    let hdc = wparam as HDC;
    if hdc.is_null() {
        return None;
    }

    let mut rect = Default::default();
    // SAFETY: hwnd is the window being painted and rect points to valid writable storage.
    if unsafe { GetClientRect(hwnd.raw(), &mut rect) } == 0 {
        return None;
    }

    // SAFETY: hdc is supplied by WM_ERASEBKGND and the brush is owned by ThemeResources.
    unsafe {
        FillRect(hdc, &rect, resources.window_brush());
    }
    Some(1)
}

pub fn control_color_brush(
    theme: AppearanceTheme,
    resources: &ThemeResources,
    wparam: MessageWord,
) -> Option<MessageResult> {
    let palette = ThemePalette::for_theme(theme);
    if !palette.uses_custom_controls() {
        return None;
    }

    let hdc = wparam as HDC;
    if hdc.is_null() {
        return None;
    }

    // SAFETY: hdc is supplied by a WM_CTLCOLOR* message; colors are plain COLORREF values.
    unsafe {
        SetTextColor(hdc, palette.control_text);
        SetBkColor(hdc, palette.control_background);
        SetBkMode(hdc, GDI_OPAQUE_BACKGROUND_MODE);
    }
    Some(resources.control_brush() as MessageResult)
}

pub fn draw_material_icon_button(
    theme: AppearanceTheme,
    resources: &ThemeResources,
    icon: IconHandle,
    lparam: MessageLong,
) -> Option<MessageResult> {
    if lparam == 0 {
        return None;
    }

    // SAFETY: WM_DRAWITEM supplies a DRAWITEMSTRUCT pointer for the current message dispatch.
    let draw = unsafe { (lparam as *const DRAWITEMSTRUCT).as_ref() }?;
    if draw.CtlType != ODT_BUTTON {
        return None;
    }

    draw_icon_button_background(theme, resources, draw);

    let mut icon_rect = centered_icon_square(draw.rcItem);
    if draw.itemState & ODS_SELECTED != 0 {
        // SAFETY: icon_rect is a local RECT.
        unsafe {
            OffsetRect(&mut icon_rect, 1, 1);
        }
    }
    draw_material_icon(draw.hDC, icon, icon_rect);

    if draw.itemState & ODS_FOCUS != 0 {
        let mut focus_rect = draw.rcItem;
        // SAFETY: focus_rect is a local RECT.
        unsafe {
            InflateRect(&mut focus_rect, -4, -4);
            DrawFocusRect(draw.hDC, &focus_rect);
        }
    }

    Some(1)
}

fn draw_icon_button_background(
    theme: AppearanceTheme,
    resources: &ThemeResources,
    draw: &DRAWITEMSTRUCT,
) {
    let palette = ThemePalette::for_theme(theme);
    let mut rect = draw.rcItem;
    let selected = draw.itemState & ODS_SELECTED != 0;
    let disabled = draw.itemState & ODS_DISABLED != 0;

    if palette.uses_custom_controls() {
        let background = if selected {
            blend_color(palette.control_background, MATERIAL_ICON_COLOR, 1, 6)
        } else {
            palette.control_background
        };
        let border = if disabled {
            blend_color(palette.tree_line, palette.control_background, 1, 2)
        } else {
            palette.tree_line
        };

        // SAFETY: draw.hDC is the owner-draw paint DC and the brushes are valid for this call.
        unsafe {
            FillRect(draw.hDC, &rect, resources.control_brush());
        }
        fill_rect_with_color(draw.hDC, &rect, background);
        frame_rect_with_color(draw.hDC, &rect, border);
    } else {
        let mut state = DFCS_BUTTONPUSH;
        if selected {
            state |= DFCS_PUSHED;
        }
        if disabled {
            state |= DFCS_INACTIVE;
        }

        // SAFETY: draw.hDC and rect come from DRAWITEMSTRUCT for this paint message.
        unsafe {
            DrawFrameControl(draw.hDC, &mut rect, DFC_BUTTON, state);
        }
    }
}

fn centered_icon_square(rect: RECT) -> RECT {
    let width = (rect.right - rect.left).max(0);
    let height = (rect.bottom - rect.top).max(0);
    let max_size = (width.min(height) - 4).max(0);
    let requested_size = width.min(height) * MATERIAL_ICON_GLYPH_RATIO_NUMERATOR
        / MATERIAL_ICON_GLYPH_RATIO_DENOMINATOR;
    let size = requested_size.min(max_size);
    let left = rect.left + (width - size) / 2;
    let top = rect.top + (height - size) / 2;

    RECT {
        left,
        top,
        right: left + size,
        bottom: top + size,
    }
}

fn draw_material_icon(hdc: HDC, icon: IconHandle, rect: RECT) {
    if hdc.is_null() || rect.right <= rect.left || rect.bottom <= rect.top {
        return;
    }

    let width = (rect.right - rect.left).max(0);
    let height = (rect.bottom - rect.top).max(0);
    let size = width.min(height).max(0);
    let x = rect.left + (width - size) / 2;
    let y = rect.top + (height - size) / 2;

    // SAFETY: hdc is the owner-draw paint DC and icon is a shared HICON loaded from resources.
    unsafe {
        DrawIconEx(hdc, x, y, icon.raw(), size, size, 0, null_mut(), DI_NORMAL);
    }
}

fn fill_rect_with_color(hdc: HDC, rect: &RECT, color: COLORREF) {
    // SAFETY: CreateSolidBrush creates an owned brush for a plain COLORREF.
    let brush = unsafe { CreateSolidBrush(color) };
    if brush.is_null() {
        return;
    }

    // SAFETY: hdc and rect are valid for the current paint call; brush is owned here.
    unsafe {
        FillRect(hdc, rect, brush);
        DeleteObject(brush as HGDIOBJ);
    }
}

fn frame_rect_with_color(hdc: HDC, rect: &RECT, color: COLORREF) {
    // SAFETY: CreateSolidBrush creates an owned brush for a plain COLORREF.
    let brush = unsafe { CreateSolidBrush(color) };
    if brush.is_null() {
        return;
    }

    // SAFETY: hdc and rect are valid for the current paint call; brush is owned here.
    unsafe {
        FrameRect(hdc, rect, brush);
        DeleteObject(brush as HGDIOBJ);
    }
}

fn blend_color(base: COLORREF, overlay: COLORREF, numerator: u32, denominator: u32) -> COLORREF {
    if denominator == 0 {
        return base;
    }

    let blend = |shift: u32| {
        let base_component = (base >> shift) & 0xff;
        let overlay_component = (overlay >> shift) & 0xff;
        let value = (base_component * (denominator - numerator) + overlay_component * numerator)
            / denominator;
        value.min(0xff)
    };

    blend(0) | (blend(8) << 8) | (blend(16) << 16)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct TreeViewThemeColors {
    background: isize,
    text: isize,
    line: isize,
}

impl TreeViewThemeColors {
    fn for_theme(palette: ThemePalette) -> Self {
        if palette.uses_custom_controls() {
            Self {
                background: palette.control_background as isize,
                text: palette.control_text as isize,
                line: palette.tree_line as isize,
            }
        } else {
            Self {
                background: TREE_VIEW_USE_SYSTEM_COLOR,
                text: TREE_VIEW_USE_SYSTEM_COLOR,
                line: CLR_DEFAULT as isize,
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ListViewThemeColors {
    background: isize,
    text: isize,
    text_background: isize,
}

impl ListViewThemeColors {
    fn for_theme(palette: ThemePalette) -> Self {
        Self {
            background: palette.control_background as isize,
            text: palette.control_text as isize,
            text_background: palette.control_background as isize,
        }
    }
}

fn rgb(red: u8, green: u8, blue: u8) -> COLORREF {
    u32::from(red) | (u32::from(green) << 8) | (u32::from(blue) << 16)
}

fn invalidate_window(hwnd: WindowHandle) {
    if hwnd.is_null() {
        return;
    }

    // SAFETY: hwnd is a window handle and null rect requests invalidation of the whole client area.
    unsafe {
        InvalidateRect(hwnd.raw(), null(), INVALIDATE_WITH_ERASE);
    }
}

pub fn clear_tree_view(tree_view: WindowHandle) -> ExplorerResult<()> {
    // SAFETY: tree_view is a TreeView handle and TVI_ROOT requests deletion of all items.
    let cleared = unsafe { SendMessageW(tree_view.raw(), TVM_DELETEITEM, 0, TVI_ROOT) };
    if cleared == 0 {
        return Err(windows_api_error("clear tree view items", "TVM_DELETEITEM"));
    }

    Ok(())
}

pub fn delete_tree_view_item(
    tree_view: WindowHandle,
    item: TreeViewItemHandle,
) -> ExplorerResult<()> {
    // SAFETY: tree_view is a TreeView handle and item is an item handle owned by that TreeView.
    let deleted = unsafe { SendMessageW(tree_view.raw(), TVM_DELETEITEM, 0, item.raw()) };
    if deleted == 0 {
        return Err(windows_api_error("delete tree view item", "TVM_DELETEITEM"));
    }

    Ok(())
}

pub fn insert_tree_view_root_item(
    tree_view: WindowHandle,
    item: TreeViewItem<'_>,
) -> ExplorerResult<TreeViewItemHandle> {
    insert_tree_view_item(tree_view, TVI_ROOT, item)
}

pub fn insert_tree_view_child_item(
    tree_view: WindowHandle,
    parent: TreeViewItemHandle,
    item: TreeViewItem<'_>,
) -> ExplorerResult<TreeViewItemHandle> {
    insert_tree_view_item(tree_view, parent.raw(), item)
}

pub fn set_tree_view_selected_item(
    tree_view: WindowHandle,
    selected_item: Option<TreeViewItemHandle>,
) -> ExplorerResult<()> {
    let raw_item = selected_item
        .map(TreeViewItemHandle::raw)
        .unwrap_or_default();

    // SAFETY: tree_view is a TreeView handle; TVGN_CARET changes the current selection to
    // raw_item, or clears it when raw_item is null.
    let selected = unsafe {
        SendMessageW(
            tree_view.raw(),
            TVM_SELECTITEM,
            TVGN_CARET as usize,
            raw_item,
        )
    };
    if selected == 0 && selected_item.is_some() {
        return Err(windows_api_error(
            "set tree view selection",
            "TVM_SELECTITEM",
        ));
    }

    Ok(())
}

pub fn expand_tree_view_item(
    tree_view: WindowHandle,
    item: TreeViewItemHandle,
) -> ExplorerResult<()> {
    // SAFETY: tree_view is a TreeView handle and item is an item handle owned by that TreeView.
    let expanded =
        unsafe { SendMessageW(tree_view.raw(), TVM_EXPAND, TVE_EXPAND as usize, item.raw()) };
    if expanded == 0 {
        return Err(windows_api_error("expand tree view item", "TVM_EXPAND"));
    }

    Ok(())
}

pub fn selected_tree_view_item(tree_view: WindowHandle) -> Option<TreeViewItemHandle> {
    // SAFETY: tree_view is a TreeView handle; TVGN_CARET queries the current selection.
    let raw_item =
        unsafe { SendMessageW(tree_view.raw(), TVM_GETNEXTITEM, TVGN_CARET as usize, 0) };
    TreeViewItemHandle::from_raw(raw_item)
}

pub fn tree_view_item_at_screen_point(
    tree_view: WindowHandle,
    point: ScreenPoint,
) -> ExplorerResult<Option<TreeViewItemHandle>> {
    let mut client_point = POINT {
        x: point.x,
        y: point.y,
    };

    // SAFETY: client_point is writable and tree_view is a TreeView handle.
    let succeeded = unsafe { ScreenToClient(tree_view.raw(), &mut client_point) };
    if succeeded == 0 {
        return Err(windows_api_error(
            "translate tree view hit point",
            "ScreenToClient",
        ));
    }

    let mut hit_test = TVHITTESTINFO {
        pt: client_point,
        flags: 0,
        hItem: 0,
    };

    // SAFETY: tree_view is a TreeView handle and hit_test is writable for the synchronous call.
    unsafe {
        SendMessageW(
            tree_view.raw(),
            TVM_HITTEST,
            0,
            (&mut hit_test as *mut TVHITTESTINFO).cast::<c_void>() as isize,
        );
    }

    let context_hit_flags = TVHT_ONITEM | TVHT_ONITEMRIGHT;
    if hit_test.flags & context_hit_flags == 0 {
        return Ok(None);
    }

    Ok(TreeViewItemHandle::from_raw(hit_test.hItem))
}

pub fn list_view_item_at_screen_point(
    list_view: WindowHandle,
    point: ScreenPoint,
) -> ExplorerResult<Option<usize>> {
    let mut client_point = POINT {
        x: point.x,
        y: point.y,
    };

    // SAFETY: client_point is writable and list_view is a ListView handle.
    let succeeded = unsafe { ScreenToClient(list_view.raw(), &mut client_point) };
    if succeeded == 0 {
        return Err(windows_api_error(
            "translate list view hit point",
            "ScreenToClient",
        ));
    }

    let mut hit_test = LVHITTESTINFO {
        pt: client_point,
        flags: 0,
        iItem: -1,
        iSubItem: 0,
        iGroup: 0,
    };

    // SAFETY: list_view is a ListView handle and hit_test is writable for the synchronous call.
    let index = unsafe {
        SendMessageW(
            list_view.raw(),
            LVM_HITTEST,
            0,
            (&mut hit_test as *mut LVHITTESTINFO).cast::<c_void>() as isize,
        )
    };

    let item_hit_flags = LVHT_ONITEMICON | LVHT_ONITEMLABEL | LVHT_ONITEMSTATEICON;
    if hit_test.flags & item_hit_flags == 0 {
        return Ok(None);
    }

    Ok(usize::try_from(index).ok())
}

pub fn vertical_auto_scroll_direction(
    hwnd: WindowHandle,
    point: ScreenPoint,
    edge_threshold: i32,
) -> ExplorerResult<Option<AutoScrollDirection>> {
    let mut client_point = POINT {
        x: point.x,
        y: point.y,
    };

    // SAFETY: client_point is writable and hwnd is a TreeView/ListView handle.
    let succeeded = unsafe { ScreenToClient(hwnd.raw(), &mut client_point) };
    if succeeded == 0 {
        return Err(windows_api_error(
            "translate auto-scroll point",
            "ScreenToClient",
        ));
    }

    let rect = client_rect(hwnd)?;
    Ok(decide_vertical_auto_scroll_direction(
        client_point.y,
        rect.height,
        edge_threshold,
    ))
}

pub fn scroll_window_vertically(hwnd: WindowHandle, direction: AutoScrollDirection) {
    if hwnd.is_null() {
        return;
    }

    let command = match direction {
        AutoScrollDirection::Up => SB_LINEUP,
        AutoScrollDirection::Down => SB_LINEDOWN,
    };

    // SAFETY: hwnd is a TreeView/ListView handle and WM_VSCROLL line commands carry no pointers.
    unsafe {
        SendMessageW(hwnd.raw(), WM_VSCROLL, command as usize, 0);
    }
}

pub fn selected_tree_view_item_value(
    tree_view: WindowHandle,
) -> ExplorerResult<Option<TreeViewItemValue>> {
    let Some(item) = selected_tree_view_item(tree_view) else {
        return Ok(None);
    };

    tree_view_item_value(tree_view, item)
}

pub fn tree_view_item_value(
    tree_view: WindowHandle,
    item: TreeViewItemHandle,
) -> ExplorerResult<Option<TreeViewItemValue>> {
    let mut raw_item = TVITEMW {
        mask: TVIF_PARAM,
        hItem: item.raw(),
        ..Default::default()
    };

    // SAFETY: tree_view is a TreeView handle and raw_item points to writable TVITEMW storage.
    let succeeded = unsafe {
        SendMessageW(
            tree_view.raw(),
            TVM_GETITEMW,
            0,
            (&mut raw_item as *mut TVITEMW).cast::<c_void>() as isize,
        )
    };
    if succeeded == 0 {
        return Err(windows_api_error("read tree view item", "TVM_GETITEMW"));
    }

    Ok(TreeViewItemValue::from_lparam(raw_item.lParam))
}

pub fn set_tree_view_item_has_children(
    tree_view: WindowHandle,
    item: TreeViewItemHandle,
    has_children: bool,
) -> ExplorerResult<()> {
    let mut raw_item = TVITEMW {
        mask: TVIF_CHILDREN,
        hItem: item.raw(),
        cChildren: if has_children { 1 } else { 0 },
        ..Default::default()
    };

    // SAFETY: tree_view is a TreeView handle and raw_item points to writable TVITEMW storage for
    // the synchronous message call.
    let succeeded = unsafe {
        SendMessageW(
            tree_view.raw(),
            TVM_SETITEMW,
            0,
            (&mut raw_item as *mut TVITEMW).cast::<c_void>() as isize,
        )
    };
    if succeeded == 0 {
        return Err(windows_api_error(
            "set tree view item children hint",
            "TVM_SETITEMW",
        ));
    }

    Ok(())
}

pub fn set_list_view_columns(
    list_view: WindowHandle,
    columns: &[ListViewColumn<'_>],
) -> ExplorerResult<()> {
    for (index, column) in columns.iter().enumerate() {
        let index = i32::try_from(index)
            .map_err(|_| ExplorerError::state_conflict("목록 열이 너무 많습니다."))?;
        let mut title = str_to_wide_null(column.title);
        let mut raw_column = LVCOLUMNW {
            mask: LVCF_TEXT | LVCF_WIDTH | LVCF_FMT | LVCF_SUBITEM,
            fmt: match column.align {
                ColumnAlign::Left => LVCFMT_LEFT,
                ColumnAlign::Right => LVCFMT_RIGHT,
            },
            cx: column.width,
            pszText: title.as_mut_ptr(),
            iSubItem: index,
            ..Default::default()
        };

        // SAFETY: list_view is a ListView handle and raw_column points to valid text for the call.
        let inserted = unsafe {
            SendMessageW(
                list_view.raw(),
                LVM_INSERTCOLUMNW,
                index as usize,
                (&mut raw_column as *mut LVCOLUMNW).cast::<c_void>() as isize,
            )
        };
        if inserted == -1 {
            return Err(windows_api_error(
                "insert list view column",
                "LVM_INSERTCOLUMNW",
            ));
        }
    }

    Ok(())
}

pub fn set_list_view_small_image_list(
    list_view: WindowHandle,
    image_list: ShellImageListHandle,
) -> ExplorerResult<()> {
    let image_list = image_list.raw();
    if image_list == 0 {
        return Err(ExplorerError::windows_api(
            "set list view image list",
            "LVM_SETIMAGELIST",
            0,
            None,
        ));
    }

    // SAFETY: list_view is a ListView handle and image_list is a system-owned HIMAGELIST.
    unsafe {
        SendMessageW(
            list_view.raw(),
            LVM_SETIMAGELIST,
            LVSIL_SMALL as usize,
            image_list,
        );
    }

    Ok(())
}

pub fn set_list_view_virtual_row_count(
    list_view: WindowHandle,
    row_count: usize,
) -> ExplorerResult<()> {
    let row_count = i32::try_from(row_count)
        .map_err(|_| ExplorerError::state_conflict("목록 항목이 너무 많습니다."))?;

    let mut redraw_guard = WindowRedrawGuard::suspend(list_view);
    // SAFETY: list_view is an owner-data ListView handle; LVM_SETITEMCOUNT uses value parameters.
    unsafe {
        SendMessageW(list_view.raw(), LVM_SETITEMCOUNT, row_count as usize, 0);
    }
    redraw_guard.resume();
    Ok(())
}

pub fn set_list_view_rows(list_view: WindowHandle, rows: &[ListViewRow]) -> ExplorerResult<()> {
    let mut redraw_guard = WindowRedrawGuard::suspend(list_view);

    // SAFETY: list_view is a ListView handle; the message does not use lparam.
    let cleared = unsafe { SendMessageW(list_view.raw(), LVM_DELETEALLITEMS, 0, 0) };
    if cleared == 0 {
        return Err(windows_api_error(
            "clear list view rows",
            "LVM_DELETEALLITEMS",
        ));
    }

    for (row_index, row) in rows.iter().enumerate() {
        let row_index = i32::try_from(row_index)
            .map_err(|_| ExplorerError::state_conflict("목록 항목이 너무 많습니다."))?;
        for (column_index, value) in row.cells.iter().enumerate() {
            let column_index = i32::try_from(column_index)
                .map_err(|_| ExplorerError::state_conflict("목록 열이 너무 많습니다."))?;
            let image_index = if column_index == 0 {
                row.image_index
            } else {
                None
            };
            let operation = if column_index == 0 {
                ListViewCellOperation::InsertItem
            } else {
                ListViewCellOperation::SetItem
            };
            set_list_view_cell(
                list_view,
                row_index,
                column_index,
                value,
                image_index,
                operation,
            )?;
        }
    }

    redraw_guard.resume();
    Ok(())
}

pub fn set_list_view_rows_from_wide_null<F, C, V>(
    list_view: WindowHandle,
    row_count: usize,
    mut row_at: F,
) -> ExplorerResult<()>
where
    F: FnMut(usize) -> (C, Option<i32>),
    C: IntoIterator<Item = V>,
    V: AsRef<[u16]>,
{
    let mut redraw_guard = WindowRedrawGuard::suspend(list_view);
    let existing_count = list_view_item_count(list_view)
        .ok_or_else(|| ExplorerError::state_conflict("목록 항목 수가 너무 많습니다."))?;

    for row_index in (row_count..existing_count).rev() {
        delete_list_view_row(list_view, row_index)?;
    }

    let retained_count = existing_count.min(row_count);
    for row_index in 0..row_count {
        let is_new_row = row_index >= retained_count;
        let (cells, row_image_index) = row_at(row_index);
        let first_cell_operation = if is_new_row {
            ListViewCellOperation::InsertItem
        } else {
            ListViewCellOperation::SetItem
        };
        set_list_view_row_from_wide_null(
            list_view,
            row_index,
            cells,
            row_image_index,
            first_cell_operation,
        )?;
    }

    redraw_guard.resume();
    Ok(())
}

pub fn update_list_view_rows_from_wide_null<F, C, V>(
    list_view: WindowHandle,
    row_indices: &[usize],
    mut row_at: F,
) -> ExplorerResult<()>
where
    F: FnMut(usize) -> (C, Option<i32>),
    C: IntoIterator<Item = V>,
    V: AsRef<[u16]>,
{
    if row_indices.is_empty() {
        return Ok(());
    }

    let mut redraw_guard = WindowRedrawGuard::suspend(list_view);
    let existing_count = list_view_item_count(list_view)
        .ok_or_else(|| ExplorerError::state_conflict("목록 항목 수가 너무 많습니다."))?;

    for &row_index in row_indices {
        if row_index >= existing_count {
            return Err(ExplorerError::state_conflict(
                "갱신할 목록 행이 현재 목록 범위를 벗어났습니다.",
            ));
        }

        let (cells, row_image_index) = row_at(row_index);
        set_list_view_row_from_wide_null(
            list_view,
            row_index,
            cells,
            row_image_index,
            ListViewCellOperation::SetItem,
        )?;
    }

    redraw_guard.resume();
    Ok(())
}

pub fn patch_list_view_rows_from_wide_null<F, C, V>(
    list_view: WindowHandle,
    old_row_count: usize,
    unchanged_prefix_count: usize,
    old_changed_count: usize,
    new_changed_count: usize,
    mut row_at: F,
) -> ExplorerResult<bool>
where
    F: FnMut(usize) -> (C, Option<i32>),
    C: IntoIterator<Item = V>,
    V: AsRef<[u16]>,
{
    if unchanged_prefix_count > old_row_count
        || old_changed_count > old_row_count - unchanged_prefix_count
        || new_changed_count > usize::MAX - unchanged_prefix_count
    {
        return Err(ExplorerError::state_conflict(
            "목록 행 변경 범위가 현재 목록 범위를 벗어났습니다.",
        ));
    }

    let existing_count = list_view_item_count(list_view)
        .ok_or_else(|| ExplorerError::state_conflict("목록 항목 수가 너무 많습니다."))?;
    if existing_count != old_row_count {
        return Ok(false);
    }
    if old_changed_count == 0 && new_changed_count == 0 {
        return Ok(true);
    }

    let mut redraw_guard = WindowRedrawGuard::suspend(list_view);
    let update_count = old_changed_count.min(new_changed_count);
    for offset in 0..update_count {
        let row_index = unchanged_prefix_count + offset;
        let (cells, row_image_index) = row_at(row_index);
        set_list_view_row_from_wide_null(
            list_view,
            row_index,
            cells,
            row_image_index,
            ListViewCellOperation::SetItem,
        )?;
    }

    if old_changed_count > new_changed_count {
        let delete_start = unchanged_prefix_count + new_changed_count;
        let delete_end = unchanged_prefix_count + old_changed_count;
        for row_index in (delete_start..delete_end).rev() {
            delete_list_view_row(list_view, row_index)?;
        }
    } else {
        let insert_start = unchanged_prefix_count + old_changed_count;
        let insert_end = unchanged_prefix_count + new_changed_count;
        for row_index in insert_start..insert_end {
            let (cells, row_image_index) = row_at(row_index);
            set_list_view_row_from_wide_null(
                list_view,
                row_index,
                cells,
                row_image_index,
                ListViewCellOperation::InsertItem,
            )?;
        }
    }

    redraw_guard.resume();
    Ok(true)
}

pub fn delete_list_view_rows(
    list_view: WindowHandle,
    descending_row_indices: &[usize],
) -> ExplorerResult<()> {
    if descending_row_indices.is_empty() {
        return Ok(());
    }

    let mut redraw_guard = WindowRedrawGuard::suspend(list_view);
    let mut current_count = list_view_item_count(list_view)
        .ok_or_else(|| ExplorerError::state_conflict("목록 항목 수가 너무 많습니다."))?;
    let mut previous_index = current_count;

    for &row_index in descending_row_indices {
        if row_index >= current_count || row_index >= previous_index {
            return Err(ExplorerError::state_conflict(
                "삭제할 목록 행이 현재 목록 범위를 벗어났습니다.",
            ));
        }
        delete_list_view_row(list_view, row_index)?;
        current_count -= 1;
        previous_index = row_index;
    }

    redraw_guard.resume();
    Ok(())
}

pub fn list_view_viewport(list_view: WindowHandle) -> Option<ListViewViewport> {
    if list_view.is_null() || list_view_item_count(list_view)? == 0 {
        return None;
    }

    list_view_top_index(list_view).map(|top_index| ListViewViewport { top_index })
}

pub fn restore_list_view_viewport(list_view: WindowHandle, viewport: ListViewViewport) {
    if list_view.is_null() {
        return;
    }

    let Some(item_count) = list_view_item_count(list_view) else {
        return;
    };
    let Some(current_top) = list_view_top_index(list_view) else {
        return;
    };
    let Some(delta_rows) = list_view_scroll_delta_rows(current_top, viewport.top_index, item_count)
    else {
        return;
    };
    let Some(row_height) = list_view_row_height(list_view) else {
        return;
    };
    let Some(delta_pixels) = list_view_scroll_delta_pixels(delta_rows, row_height) else {
        return;
    };

    // SAFETY: list_view is a ListView handle; LVM_SCROLL uses pixel deltas and no pointers.
    unsafe {
        SendMessageW(list_view.raw(), LVM_SCROLL, 0, delta_pixels);
    }
}

fn list_view_item_count(list_view: WindowHandle) -> Option<usize> {
    // SAFETY: list_view is a ListView handle; LVM_GETITEMCOUNT has no pointer parameters.
    let count = unsafe { SendMessageW(list_view.raw(), LVM_GETITEMCOUNT, 0, 0) };
    usize::try_from(count).ok()
}

fn delete_list_view_row(list_view: WindowHandle, row_index: usize) -> ExplorerResult<()> {
    let row_index = i32::try_from(row_index)
        .map_err(|_| ExplorerError::state_conflict("목록 항목이 너무 많습니다."))?;
    let row_index = usize::try_from(row_index)
        .map_err(|_| ExplorerError::state_conflict("목록 항목이 너무 많습니다."))?;

    // SAFETY: list_view is a ListView handle; LVM_DELETEITEM does not use lparam.
    let deleted = unsafe { SendMessageW(list_view.raw(), LVM_DELETEITEM, row_index, 0) };
    if deleted == 0 {
        return Err(windows_api_error("delete list view row", "LVM_DELETEITEM"));
    }

    Ok(())
}

fn list_view_top_index(list_view: WindowHandle) -> Option<usize> {
    // SAFETY: list_view is a ListView handle; LVM_GETTOPINDEX has no pointer parameters.
    let index = unsafe { SendMessageW(list_view.raw(), LVM_GETTOPINDEX, 0, 0) };
    usize::try_from(index).ok()
}

fn list_view_row_height(list_view: WindowHandle) -> Option<i32> {
    let mut rect = RECT {
        left: LVIR_BOUNDS as i32,
        ..Default::default()
    };

    // SAFETY: list_view is a ListView handle and rect is writable for the synchronous call.
    let succeeded = unsafe {
        SendMessageW(
            list_view.raw(),
            LVM_GETITEMRECT,
            0,
            (&mut rect as *mut RECT).cast::<c_void>() as isize,
        )
    };
    if succeeded == 0 {
        return None;
    }

    let height = rect.bottom - rect.top;
    (height > 0).then_some(height)
}

fn list_view_scroll_delta_rows(
    current_top: usize,
    target_top: usize,
    item_count: usize,
) -> Option<isize> {
    let max_index = item_count.checked_sub(1)?;
    let current_top = current_top.min(max_index);
    let target_top = target_top.min(max_index);
    let current_top = isize::try_from(current_top).ok()?;
    let target_top = isize::try_from(target_top).ok()?;
    let delta = target_top.checked_sub(current_top)?;
    (delta != 0).then_some(delta)
}

fn list_view_scroll_delta_pixels(delta_rows: isize, row_height: i32) -> Option<isize> {
    if row_height <= 0 {
        return None;
    }

    let pixels = (delta_rows as i128).checked_mul(i128::from(row_height))?;
    isize::try_from(pixels).ok()
}

pub fn set_tab_items(
    tab_control: WindowHandle,
    labels: &[String],
    active_index: usize,
) -> ExplorerResult<()> {
    if labels.is_empty() {
        return Err(ExplorerError::state_conflict("표시할 열린 탭이 없습니다."));
    }

    if active_index >= labels.len() {
        return Err(ExplorerError::state_conflict(
            "활성 탭 위치가 열린 탭 범위를 벗어났습니다.",
        ));
    }

    // SAFETY: tab_control is a TabControl handle; the message does not use lparam.
    let cleared = unsafe { SendMessageW(tab_control.raw(), TCM_DELETEALLITEMS, 0, 0) };
    if cleared == 0 {
        return Err(windows_api_error(
            "clear tab control items",
            "TCM_DELETEALLITEMS",
        ));
    }

    for (index, label) in labels.iter().enumerate() {
        let index = i32::try_from(index)
            .map_err(|_| ExplorerError::state_conflict("탭 항목이 너무 많습니다."))?;
        let mut text = str_to_wide_null(label);
        let mut item = TCITEMW {
            mask: TCIF_TEXT,
            pszText: text.as_mut_ptr(),
            ..Default::default()
        };

        // SAFETY: tab_control is a TabControl handle and item points to valid text for the call.
        let inserted = unsafe {
            SendMessageW(
                tab_control.raw(),
                TCM_INSERTITEMW,
                index as usize,
                (&mut item as *mut TCITEMW).cast::<c_void>() as isize,
            )
        };
        if inserted == -1 {
            return Err(windows_api_error(
                "insert tab control item",
                "TCM_INSERTITEMW",
            ));
        }
    }

    set_tab_current_selection(tab_control, active_index);
    Ok(())
}

pub fn set_tab_current_selection(tab_control: WindowHandle, index: usize) {
    // SAFETY: tab_control is a TabControl handle; index is an item position selected by the caller.
    unsafe {
        SendMessageW(tab_control.raw(), TCM_SETCURSEL, index, 0);
    }
}

pub fn tab_current_selection(tab_control: WindowHandle) -> Option<usize> {
    // SAFETY: tab_control is a TabControl handle; the message does not use parameters.
    let index = unsafe { SendMessageW(tab_control.raw(), TCM_GETCURSEL, 0, 0) };
    usize::try_from(index).ok()
}

pub fn tab_index_at_screen_point(
    tab_control: WindowHandle,
    point: ScreenPoint,
) -> ExplorerResult<Option<usize>> {
    let mut client_point = POINT {
        x: point.x,
        y: point.y,
    };

    // SAFETY: client_point is writable and tab_control is a TabControl handle.
    let succeeded = unsafe { ScreenToClient(tab_control.raw(), &mut client_point) };
    if succeeded == 0 {
        return Err(windows_api_error(
            "translate tab hit point",
            "ScreenToClient",
        ));
    }

    let mut hit_test = TCHITTESTINFO {
        pt: client_point,
        flags: 0,
    };

    // SAFETY: tab_control is a TabControl handle and hit_test is writable for the call.
    let index = unsafe {
        SendMessageW(
            tab_control.raw(),
            TCM_HITTEST,
            0,
            (&mut hit_test as *mut TCHITTESTINFO).cast::<c_void>() as isize,
        )
    };
    Ok(usize::try_from(index).ok())
}

pub fn set_list_view_column_width(
    list_view: WindowHandle,
    column_index: usize,
    width: i32,
) -> ExplorerResult<()> {
    let column_index = i32::try_from(column_index)
        .map_err(|_| ExplorerError::state_conflict("목록 열이 너무 많습니다."))?;

    // SAFETY: list_view is a ListView handle; width and column index are plain message values.
    let succeeded = unsafe {
        SendMessageW(
            list_view.raw(),
            LVM_SETCOLUMNWIDTH,
            column_index as usize,
            width as isize,
        )
    };
    if succeeded == 0 {
        return Err(windows_api_error(
            "set list view column width",
            "LVM_SETCOLUMNWIDTH",
        ));
    }

    Ok(())
}

pub fn selected_list_view_index(list_view: WindowHandle) -> Option<usize> {
    // SAFETY: list_view is a ListView handle; -1 starts the search before the first row.
    let index = unsafe {
        SendMessageW(
            list_view.raw(),
            LVM_GETNEXTITEM,
            usize::MAX,
            LVNI_SELECTED as isize,
        )
    };
    usize::try_from(index).ok()
}

pub fn selected_list_view_indices(list_view: WindowHandle) -> Vec<usize> {
    let mut indices = Vec::new();
    let mut previous = usize::MAX;

    loop {
        // SAFETY: list_view is a ListView handle; previous is either -1 or a previously returned
        // row index.
        let index = unsafe {
            SendMessageW(
                list_view.raw(),
                LVM_GETNEXTITEM,
                previous,
                LVNI_SELECTED as isize,
            )
        };
        let Ok(index) = usize::try_from(index) else {
            break;
        };

        indices.push(index);
        previous = index;
    }

    indices
}

pub fn list_view_item_is_selected(list_view: WindowHandle, index: usize) -> bool {
    // SAFETY: list_view is a ListView handle and index is supplied by a ListView notification or
    // selection enumeration.
    let state = unsafe {
        SendMessageW(
            list_view.raw(),
            LVM_GETITEMSTATE,
            index,
            LVIS_SELECTED as isize,
        )
    };
    (state as u32) & LVIS_SELECTED != 0
}

pub fn edit_list_view_label(list_view: WindowHandle, index: usize) -> ExplorerResult<()> {
    // SAFETY: list_view is a ListView handle; index is a row index selected by the caller.
    let edit = unsafe { SendMessageW(list_view.raw(), LVM_EDITLABELW, index, 0) };
    if edit == 0 {
        return Err(windows_api_error(
            "start list item rename",
            "LVM_EDITLABELW",
        ));
    }

    Ok(())
}

pub fn set_list_view_selected_index(
    list_view: WindowHandle,
    selected_index: Option<usize>,
) -> ExplorerResult<()> {
    let selected_indices = selected_index.into_iter().collect::<Vec<_>>();
    set_list_view_selected_indices(list_view, &selected_indices)
}

pub fn set_list_view_selected_indices(
    list_view: WindowHandle,
    selected_indices: &[usize],
) -> ExplorerResult<()> {
    let state_mask = LVIS_SELECTED | LVIS_FOCUSED;

    let mut clear_item = LVITEMW {
        stateMask: state_mask,
        state: 0,
        ..Default::default()
    };
    // SAFETY: list_view is a ListView handle and -1 clears the state for every item.
    unsafe {
        SendMessageW(
            list_view.raw(),
            LVM_SETITEMSTATE,
            usize::MAX,
            (&mut clear_item as *mut LVITEMW).cast::<c_void>() as isize,
        );
    }

    for &index in selected_indices {
        let mut select_item = LVITEMW {
            stateMask: LVIS_SELECTED,
            state: LVIS_SELECTED,
            ..Default::default()
        };

        // SAFETY: list_view is a ListView handle and select_item points to a valid LVITEMW.
        let selected = unsafe {
            SendMessageW(
                list_view.raw(),
                LVM_SETITEMSTATE,
                index,
                (&mut select_item as *mut LVITEMW).cast::<c_void>() as isize,
            )
        };
        if selected == 0 {
            return Err(windows_api_error(
                "set list view selection",
                "LVM_SETITEMSTATE",
            ));
        }
    }

    let Some(&first_index) = selected_indices.first() else {
        return Ok(());
    };

    let mut focus_item = LVITEMW {
        stateMask: state_mask,
        state: state_mask,
        ..Default::default()
    };

    // SAFETY: list_view is a ListView handle and focus_item points to a valid LVITEMW.
    let selected = unsafe {
        SendMessageW(
            list_view.raw(),
            LVM_SETITEMSTATE,
            first_index,
            (&mut focus_item as *mut LVITEMW).cast::<c_void>() as isize,
        )
    };
    if selected == 0 {
        return Err(windows_api_error(
            "set list view selection",
            "LVM_SETITEMSTATE",
        ));
    }

    // SAFETY: list_view is a ListView handle; index is a row index selected above.
    unsafe {
        SendMessageW(list_view.raw(), LVM_ENSUREVISIBLE, first_index, 0);
    }

    Ok(())
}

pub fn set_list_view_all_items_selected(
    list_view: WindowHandle,
    item_count: usize,
) -> ExplorerResult<()> {
    if item_count == 0 {
        return set_list_view_selected_indices(list_view, &[]);
    }

    let state_mask = LVIS_SELECTED | LVIS_FOCUSED;

    let mut select_items = LVITEMW {
        stateMask: state_mask,
        state: LVIS_SELECTED,
        ..Default::default()
    };

    // SAFETY: list_view is a ListView handle and -1 applies the state to every item.
    let selected = unsafe {
        SendMessageW(
            list_view.raw(),
            LVM_SETITEMSTATE,
            usize::MAX,
            (&mut select_items as *mut LVITEMW).cast::<c_void>() as isize,
        )
    };
    if selected == 0 {
        return Err(windows_api_error(
            "set list view selection",
            "LVM_SETITEMSTATE",
        ));
    }

    let mut focus_item = LVITEMW {
        stateMask: state_mask,
        state: state_mask,
        ..Default::default()
    };

    // SAFETY: list_view is a ListView handle and item 0 exists because item_count is nonzero.
    let focused = unsafe {
        SendMessageW(
            list_view.raw(),
            LVM_SETITEMSTATE,
            0,
            (&mut focus_item as *mut LVITEMW).cast::<c_void>() as isize,
        )
    };
    if focused == 0 {
        return Err(windows_api_error(
            "set list view selection",
            "LVM_SETITEMSTATE",
        ));
    }

    // SAFETY: list_view is a ListView handle and item 0 exists because item_count is nonzero.
    unsafe {
        SendMessageW(list_view.raw(), LVM_ENSUREVISIBLE, 0, 0);
    }

    Ok(())
}

pub fn cursor_position() -> ExplorerResult<ScreenPoint> {
    let mut point = POINT::default();
    // SAFETY: point is writable for the duration of the call.
    let succeeded = unsafe { GetCursorPos(&mut point) };
    if succeeded == 0 {
        return Err(windows_api_error("read cursor position", "GetCursorPos"));
    }

    Ok(ScreenPoint {
        x: point.x,
        y: point.y,
    })
}

pub fn client_point_from_message_lparam(lparam: MessageLong) -> ClientPoint {
    ClientPoint {
        x: signed_low_word(lparam),
        y: signed_high_word(lparam),
    }
}

pub fn screen_to_client_point(
    hwnd: WindowHandle,
    point: ScreenPoint,
) -> ExplorerResult<ClientPoint> {
    let mut point = POINT {
        x: point.x,
        y: point.y,
    };

    // SAFETY: point is writable and hwnd is a valid window handle.
    let succeeded = unsafe { ScreenToClient(hwnd.raw(), &mut point) };
    if succeeded == 0 {
        return Err(windows_api_error(
            "translate screen point to client point",
            "ScreenToClient",
        ));
    }

    Ok(ClientPoint {
        x: point.x,
        y: point.y,
    })
}

pub fn set_mouse_capture(hwnd: WindowHandle) {
    if hwnd.is_null() {
        return;
    }

    // SAFETY: hwnd is a valid window handle owned by the UI thread.
    unsafe {
        SetCapture(hwnd.raw());
    }
}

pub fn release_mouse_capture() {
    // SAFETY: ReleaseCapture releases mouse capture for the current thread when present.
    unsafe {
        ReleaseCapture();
    }
}

pub fn window_has_mouse_capture(hwnd: WindowHandle) -> bool {
    if hwnd.is_null() {
        return false;
    }

    // SAFETY: GetCapture reads the current thread's capture window.
    unsafe { GetCapture() == hwnd.raw() }
}

pub fn set_horizontal_resize_cursor() {
    // SAFETY: loading a predefined cursor with a null instance is the documented Win32 pattern.
    let cursor = unsafe { LoadCursorW(null_mut(), IDC_SIZEWE) };
    if cursor.is_null() {
        return;
    }

    // SAFETY: cursor is a system-owned cursor returned by LoadCursorW.
    unsafe {
        SetCursor(cursor);
    }
}

fn signed_low_word(value: MessageLong) -> i32 {
    (value as u16) as i16 as i32
}

fn signed_high_word(value: MessageLong) -> i32 {
    ((value >> 16) as u16) as i16 as i32
}

pub fn set_window_text(hwnd: WindowHandle, text: impl AsRef<OsStr>) -> ExplorerResult<()> {
    let text = os_to_wide_null(text.as_ref());
    // SAFETY: hwnd is a valid window/control handle and text lives through the call.
    let succeeded = unsafe { SetWindowTextW(hwnd.raw(), text.as_ptr()) };
    if succeeded == 0 {
        return Err(windows_api_error("set window text", "SetWindowTextW"));
    }

    Ok(())
}

pub fn window_text(hwnd: WindowHandle) -> ExplorerResult<OsString> {
    // SAFETY: hwnd is a valid edit control handle.
    let len = unsafe { GetWindowTextLengthW(hwnd.raw()) };
    let mut buffer = vec![0_u16; len as usize + 1];

    // SAFETY: buffer is writable and sized to include the terminating null.
    let copied = unsafe { GetWindowTextW(hwnd.raw(), buffer.as_mut_ptr(), buffer.len() as i32) };
    if copied == 0 && len > 0 {
        return Err(windows_api_error("read window text", "GetWindowTextW"));
    }

    Ok(OsString::from_wide(&buffer[..copied as usize]))
}

pub fn focus_window(hwnd: WindowHandle) {
    if hwnd.is_null() {
        return;
    }

    // SAFETY: hwnd is a valid child window handle owned by this UI thread.
    unsafe {
        SetFocus(hwnd.raw());
    }
}

pub fn select_all_edit_text(hwnd: WindowHandle) {
    if hwnd.is_null() {
        return;
    }

    // SAFETY: hwnd is an EDIT control; EM_SETSEL uses plain range parameters.
    unsafe {
        SendMessageW(hwnd.raw(), EM_SETSEL, 0, -1);
    }
}

pub fn client_rect(hwnd: WindowHandle) -> ExplorerResult<ClientRect> {
    let mut rect = RECT::default();
    // SAFETY: rect is writable and hwnd is a valid window handle.
    let succeeded = unsafe { GetClientRect(hwnd.raw(), &mut rect) };
    if succeeded == 0 {
        return Err(windows_api_error("read client rect", "GetClientRect"));
    }

    Ok(ClientRect {
        width: rect.right - rect.left,
        height: rect.bottom - rect.top,
    })
}

pub fn move_window(
    hwnd: WindowHandle,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
) -> ExplorerResult<()> {
    let width = width.max(0);
    let height = height.max(0);
    if let Some(current) = child_window_rect_in_parent(hwnd)? {
        if rect_matches_move_target(&current, x, y, width, height) {
            return Ok(());
        }
    }

    // SAFETY: hwnd is a child window handle; dimensions are validated by the caller layout.
    let succeeded = unsafe { MoveWindow(hwnd.raw(), x, y, width, height, 1) };
    if succeeded == 0 {
        return Err(windows_api_error("move window", "MoveWindow"));
    }

    Ok(())
}

fn child_window_rect_in_parent(hwnd: WindowHandle) -> ExplorerResult<Option<RECT>> {
    if hwnd.is_null() {
        return Ok(None);
    }

    // SAFETY: hwnd is a window handle; null indicates no parent and falls back to MoveWindow.
    let parent = unsafe { GetParent(hwnd.raw()) };
    if parent.is_null() {
        return Ok(None);
    }

    let mut rect = RECT::default();
    // SAFETY: rect is valid writable storage and hwnd is a window handle.
    let succeeded = unsafe { GetWindowRect(hwnd.raw(), &mut rect) };
    if succeeded == 0 {
        return Err(windows_api_error(
            "read child window rectangle",
            "GetWindowRect",
        ));
    }

    let mut points = [
        POINT {
            x: rect.left,
            y: rect.top,
        },
        POINT {
            x: rect.right,
            y: rect.bottom,
        },
    ];
    // SAFETY: points contains two valid POINT values. SetLastError disambiguates a zero return,
    // which can be success when no coordinate translation is needed.
    unsafe {
        SetLastError(0);
        let mapped = MapWindowPoints(null_mut(), parent, points.as_mut_ptr(), 2);
        if mapped == 0 && GetLastError() != 0 {
            return Err(windows_api_error(
                "map child window rectangle",
                "MapWindowPoints",
            ));
        }
    }

    Ok(Some(RECT {
        left: points[0].x,
        top: points[0].y,
        right: points[1].x,
        bottom: points[1].y,
    }))
}

fn rect_matches_move_target(rect: &RECT, x: i32, y: i32, width: i32, height: i32) -> bool {
    rect.left == x
        && rect.top == y
        && rect.right - rect.left == width
        && rect.bottom - rect.top == height
}

pub fn show_window(hwnd: WindowHandle) {
    // SAFETY: hwnd is a valid top-level window handle.
    unsafe {
        ShowWindow(hwnd.raw(), SW_SHOWNORMAL);
    }
}

pub fn set_window_visible(hwnd: WindowHandle, visible: bool) {
    if hwnd.is_null() {
        return;
    }

    let command = if visible { SW_SHOW } else { SW_HIDE };
    // SAFETY: hwnd is a valid window/control handle; ShowWindow owns no pointers.
    unsafe {
        ShowWindow(hwnd.raw(), command);
    }
}

fn apply_dpi_awareness_step(step: DpiAwarenessStep) -> Result<(), DpiAwarenessFailureReason> {
    match step.operation {
        DpiAwarenessOperation::ContextSystemAware => {
            set_process_dpi_awareness_context(-2isize as *mut c_void)
        }
        DpiAwarenessOperation::ContextPerMonitorAware => {
            set_process_dpi_awareness_context(-3isize as *mut c_void)
        }
        DpiAwarenessOperation::ContextPerMonitorAwareV2 => {
            set_process_dpi_awareness_context(-4isize as *mut c_void)
        }
        DpiAwarenessOperation::ProcessSystemAware => set_process_dpi_awareness(1),
        DpiAwarenessOperation::ProcessPerMonitorAware => set_process_dpi_awareness(2),
        DpiAwarenessOperation::ProcessDpiAware => set_process_dpi_aware(),
    }
}

fn set_process_dpi_awareness_context(
    context: *mut c_void,
) -> Result<(), DpiAwarenessFailureReason> {
    type SetProcessDpiAwarenessContextFn = unsafe extern "system" fn(*mut c_void) -> i32;

    let Some(proc) = user32_proc(c"SetProcessDpiAwarenessContext") else {
        return Err(DpiAwarenessFailureReason::Unavailable);
    };
    // SAFETY: the symbol name selects SetProcessDpiAwarenessContext from user32.dll.
    let set_context: SetProcessDpiAwarenessContextFn = unsafe { transmute(proc) };
    // SAFETY: context is one of the documented DPI_AWARENESS_CONTEXT sentinel values.
    let succeeded = unsafe { set_context(context) };
    if succeeded == 0 {
        return Err(DpiAwarenessFailureReason::Win32(last_error_code()));
    }

    Ok(())
}

fn set_process_dpi_awareness(value: i32) -> Result<(), DpiAwarenessFailureReason> {
    type SetProcessDpiAwarenessFn = unsafe extern "system" fn(i32) -> i32;

    let Some(library) = DynamicLibrary::load(c"shcore.dll") else {
        return Err(DpiAwarenessFailureReason::Unavailable);
    };
    let Some(proc) = library.proc(c"SetProcessDpiAwareness") else {
        return Err(DpiAwarenessFailureReason::Unavailable);
    };
    // SAFETY: the symbol name selects SetProcessDpiAwareness from shcore.dll.
    let set_awareness: SetProcessDpiAwarenessFn = unsafe { transmute(proc) };
    // SAFETY: value is one of the documented PROCESS_DPI_AWARENESS values.
    let hresult = unsafe { set_awareness(value) };
    if hresult < 0 {
        return Err(DpiAwarenessFailureReason::Hresult(hresult));
    }

    Ok(())
}

fn set_process_dpi_aware() -> Result<(), DpiAwarenessFailureReason> {
    // SAFETY: SetProcessDPIAware has no parameters and changes process-wide DPI state.
    let succeeded = unsafe { SetProcessDPIAware() };
    if succeeded == 0 {
        return Err(DpiAwarenessFailureReason::Win32(last_error_code()));
    }

    Ok(())
}

fn dpi_for_window(hwnd: WindowHandle) -> Option<u32> {
    type GetDpiForWindowFn = unsafe extern "system" fn(HWND) -> u32;

    let proc = user32_proc(c"GetDpiForWindow")?;
    // SAFETY: the symbol name selects GetDpiForWindow from user32.dll.
    let get_dpi_for_window: GetDpiForWindowFn = unsafe { transmute(proc) };
    // SAFETY: hwnd is an application window handle.
    let dpi = unsafe { get_dpi_for_window(hwnd.raw()) };
    (dpi > 0).then_some(dpi)
}

fn system_dpi() -> u32 {
    type GetDpiForSystemFn = unsafe extern "system" fn() -> u32;

    if let Some(proc) = user32_proc(c"GetDpiForSystem") {
        // SAFETY: the symbol name selects GetDpiForSystem from user32.dll.
        let get_dpi_for_system: GetDpiForSystemFn = unsafe { transmute(proc) };
        // SAFETY: GetDpiForSystem has no parameters.
        let dpi = unsafe { get_dpi_for_system() };
        if dpi > 0 {
            return dpi;
        }
    }

    screen_dpi_y() as u32
}

fn user32_proc(name: &'static CStr) -> Option<unsafe extern "system" fn() -> isize> {
    // SAFETY: user32.dll is already loaded because this module imports Win32 UI functions.
    let module = unsafe { GetModuleHandleA(c"user32.dll".as_ptr().cast()) };
    if module.is_null() {
        return None;
    }

    // SAFETY: name is a null-terminated ASCII symbol name.
    unsafe { GetProcAddress(module, name.as_ptr().cast()) }
}

struct DynamicLibrary(HMODULE);

impl DynamicLibrary {
    fn load(name: &'static CStr) -> Option<Self> {
        // SAFETY: name is a null-terminated ASCII DLL name, and the search is restricted to the
        // system directory.
        let module = unsafe {
            LoadLibraryExA(
                name.as_ptr().cast(),
                null_mut(),
                LOAD_LIBRARY_SEARCH_SYSTEM32,
            )
        };
        if module.is_null() {
            None
        } else {
            Some(Self(module))
        }
    }

    fn proc(&self, name: &'static CStr) -> Option<unsafe extern "system" fn() -> isize> {
        // SAFETY: name is a null-terminated ASCII symbol name and the library is loaded.
        unsafe { GetProcAddress(self.0, name.as_ptr().cast()) }
    }
}

impl Drop for DynamicLibrary {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: the module handle was returned by LoadLibraryExA and is owned here.
            unsafe {
                FreeLibrary(self.0);
            }
        }
    }
}

fn create_child_window(
    parent: WindowHandle,
    instance: InstanceHandle,
    class_name: &str,
    text: &str,
    id: u16,
    style: u32,
    ex_style: u32,
) -> ExplorerResult<WindowHandle> {
    let class_name = str_to_wide_null(class_name);
    create_child_window_with_class_ptr(
        parent,
        instance,
        class_name.as_ptr(),
        text,
        id,
        style,
        ex_style,
    )
}

fn create_child_window_with_class_ptr(
    parent: WindowHandle,
    instance: InstanceHandle,
    class_name: *const u16,
    text: &str,
    id: u16,
    style: u32,
    ex_style: u32,
) -> ExplorerResult<WindowHandle> {
    let text = str_to_wide_null(text);

    // SAFETY: class and text pointers live through the call. The id is passed as the child menu id.
    let hwnd = unsafe {
        CreateWindowExW(
            ex_style,
            class_name,
            text.as_ptr(),
            WS_CHILD | WS_VISIBLE | style,
            0,
            0,
            0,
            0,
            parent.raw(),
            id as usize as HMENU,
            instance.raw(),
            null(),
        )
    };
    if hwnd.is_null() {
        return Err(windows_api_error("create child window", "CreateWindowExW"));
    }

    let hwnd = WindowHandle::from_sys(hwnd);
    set_default_gui_font(hwnd);
    Ok(hwnd)
}

fn insert_tree_view_item(
    tree_view: WindowHandle,
    parent: HTREEITEM,
    item: TreeViewItem<'_>,
) -> ExplorerResult<TreeViewItemHandle> {
    let mut text = str_to_wide_null(item.text);
    let raw_value = tree_view_item_lparam(item.value)?;
    let raw_item = TVITEMW {
        mask: TVIF_TEXT | TVIF_PARAM | TVIF_CHILDREN,
        pszText: text.as_mut_ptr(),
        cChildren: if item.has_children { 1 } else { 0 },
        lParam: raw_value,
        ..Default::default()
    };
    let mut insert = TVINSERTSTRUCTW {
        hParent: parent,
        hInsertAfter: TVI_LAST,
        Anonymous: TVINSERTSTRUCTW_0 { item: raw_item },
    };

    // SAFETY: tree_view is a TreeView handle and insert points to text/item data valid for this
    // synchronous SendMessageW call.
    let inserted = unsafe {
        SendMessageW(
            tree_view.raw(),
            TVM_INSERTITEMW,
            0,
            (&mut insert as *mut TVINSERTSTRUCTW).cast::<c_void>() as isize,
        )
    };
    TreeViewItemHandle::from_raw(inserted)
        .ok_or_else(|| windows_api_error("insert tree view item", "TVM_INSERTITEMW"))
}

fn tree_view_item_lparam(value: Option<TreeViewItemValue>) -> ExplorerResult<isize> {
    value
        .map(TreeViewItemValue::to_lparam)
        .transpose()
        .map(|value| value.unwrap_or(TREE_VIEW_NO_ITEM_VALUE))
}

#[derive(Clone, Copy)]
enum ListViewCellOperation {
    InsertItem,
    SetItem,
}

fn set_list_view_cell(
    list_view: WindowHandle,
    row_index: i32,
    column_index: i32,
    value: &str,
    image_index: Option<i32>,
    operation: ListViewCellOperation,
) -> ExplorerResult<()> {
    let text = str_to_wide_null(value);
    set_list_view_cell_wide_null(
        list_view,
        row_index,
        column_index,
        &text,
        image_index,
        operation,
    )
}

fn set_list_view_row_from_wide_null<C, V>(
    list_view: WindowHandle,
    row_index: usize,
    cells: C,
    row_image_index: Option<i32>,
    first_cell_operation: ListViewCellOperation,
) -> ExplorerResult<()>
where
    C: IntoIterator<Item = V>,
    V: AsRef<[u16]>,
{
    let row_index = i32::try_from(row_index)
        .map_err(|_| ExplorerError::state_conflict("목록 항목이 너무 많습니다."))?;
    let mut updated_cell = false;
    for (column_index, value) in cells.into_iter().enumerate() {
        let column_index = i32::try_from(column_index)
            .map_err(|_| ExplorerError::state_conflict("목록 열이 너무 많습니다."))?;
        let image_index = if column_index == 0 {
            row_image_index
        } else {
            None
        };
        let operation = if column_index == 0 {
            first_cell_operation
        } else {
            ListViewCellOperation::SetItem
        };
        set_list_view_cell_wide_null(
            list_view,
            row_index,
            column_index,
            value.as_ref(),
            image_index,
            operation,
        )?;
        updated_cell = true;
    }
    if !updated_cell {
        return Err(ExplorerError::state_conflict(
            "목록 행에 표시할 셀이 없습니다.",
        ));
    }
    Ok(())
}

fn set_list_view_cell_wide_null(
    list_view: WindowHandle,
    row_index: i32,
    column_index: i32,
    value: &[u16],
    image_index: Option<i32>,
    operation: ListViewCellOperation,
) -> ExplorerResult<()> {
    debug_assert_eq!(value.last().copied(), Some(0));
    let mut mask = LVIF_TEXT;
    if image_index.is_some() {
        mask |= LVIF_IMAGE;
    }

    let mut item = LVITEMW {
        mask,
        iItem: row_index,
        iSubItem: column_index,
        iImage: image_index.unwrap_or_default(),
        pszText: value.as_ptr().cast_mut(),
        ..Default::default()
    };
    let message = match operation {
        ListViewCellOperation::InsertItem => LVM_INSERTITEMW,
        ListViewCellOperation::SetItem => LVM_SETITEMW,
    };

    // SAFETY: list_view is a ListView handle and item points to text valid through the call.
    let result = unsafe {
        SendMessageW(
            list_view.raw(),
            message,
            0,
            (&mut item as *mut LVITEMW).cast::<c_void>() as isize,
        )
    };

    let failed = match operation {
        ListViewCellOperation::InsertItem => result == -1,
        ListViewCellOperation::SetItem => result == 0,
    };
    if failed {
        return Err(windows_api_error(
            "set list view item",
            "ListView item message",
        ));
    }

    Ok(())
}

fn set_default_gui_font(hwnd: WindowHandle) {
    // SAFETY: DEFAULT_GUI_FONT is a stock object owned by the system.
    let font = unsafe { GetStockObject(DEFAULT_GUI_FONT) };
    if font.is_null() {
        return;
    }

    // SAFETY: WM_SETFONT does not transfer ownership of the stock font.
    unsafe {
        SendMessageW(hwnd.raw(), WM_SETFONT, font as usize, 1);
    }
}

struct WindowRedrawGuard {
    hwnd: WindowHandle,
    active: bool,
}

pub fn with_window_redraw_suspended<T>(hwnd: WindowHandle, action: impl FnOnce() -> T) -> T {
    let _guard = WindowRedrawGuard::suspend(hwnd);
    action()
}

impl WindowRedrawGuard {
    fn suspend(hwnd: WindowHandle) -> Self {
        if !hwnd.is_null() {
            // SAFETY: WM_SETREDRAW uses value parameters and affects only the target control.
            unsafe {
                SendMessageW(hwnd.raw(), WM_SETREDRAW, 0, 0);
            }
        }

        Self { hwnd, active: true }
    }

    fn resume(&mut self) {
        if !self.active {
            return;
        }

        if !self.hwnd.is_null() {
            // SAFETY: WM_SETREDRAW uses value parameters and the invalidation rectangle is null
            // to request a repaint of the whole control after the batched update. ListView and
            // TreeView paint their own background, so forcing WM_ERASEBKGND here only creates a
            // visible blank frame after a row replacement.
            unsafe {
                SendMessageW(self.hwnd.raw(), WM_SETREDRAW, 1, 0);
                InvalidateRect(self.hwnd.raw(), null(), INVALIDATE_WITHOUT_ERASE);
            }
        }
        self.active = false;
    }
}

impl Drop for WindowRedrawGuard {
    fn drop(&mut self) {
        self.resume();
    }
}

fn logfont_for_appearance_font(font: &AppearanceFont, metrics: DpiMetrics) -> LOGFONTW {
    let mut logfont = default_gui_logfont(metrics);
    logfont.lfHeight = point_size_to_logical_height(font.point_size(), metrics);
    logfont.lfWidth = 0;
    logfont.lfWeight = FW_NORMAL as i32;
    logfont.lfItalic = 0;
    logfont.lfUnderline = 0;
    logfont.lfStrikeOut = 0;

    if let Some(family_name) = font.family_name() {
        set_logfont_face_name(&mut logfont, family_name);
    }

    logfont
}

fn default_gui_logfont(metrics: DpiMetrics) -> LOGFONTW {
    let mut logfont = LOGFONTW {
        lfHeight: point_size_to_logical_height(DEFAULT_APPEARANCE_FONT_POINT_SIZE, metrics),
        lfWeight: FW_NORMAL as i32,
        lfCharSet: DEFAULT_CHARSET,
        ..Default::default()
    };
    set_logfont_face_name(&mut logfont, OsStr::new("Malgun Gothic"));
    logfont
}

fn set_logfont_face_name(logfont: &mut LOGFONTW, family_name: &OsStr) {
    logfont.lfFaceName.fill(0);
    let max_len = logfont.lfFaceName.len().saturating_sub(1);
    for (slot, unit) in logfont
        .lfFaceName
        .iter_mut()
        .take(max_len)
        .zip(family_name.encode_wide())
    {
        *slot = unit;
    }
}

fn logfont_face_name(logfont: &LOGFONTW) -> OsString {
    let end = logfont
        .lfFaceName
        .iter()
        .position(|unit| *unit == 0)
        .unwrap_or(logfont.lfFaceName.len());
    OsString::from_wide(&logfont.lfFaceName[..end])
}

fn point_size_to_logical_height(point_size: u16, metrics: DpiMetrics) -> i32 {
    let dpi = metrics.current_dpi() as i32;
    -((i32::from(point_size) * dpi + 36) / 72)
}

fn screen_dpi_y() -> i32 {
    // SAFETY: null hwnd asks for the entire screen DC.
    let hdc = unsafe { GetDC(null_mut()) };
    if hdc.is_null() {
        return 96;
    }

    // SAFETY: hdc is a screen DC acquired by GetDC.
    let dpi = unsafe { GetDeviceCaps(hdc, LOGPIXELSY as i32) };
    // SAFETY: hdc was acquired from GetDC with the same hwnd.
    unsafe {
        ReleaseDC(null_mut(), hdc);
    }

    if dpi > 0 {
        dpi
    } else {
        96
    }
}

fn load_optional_icon_resource(
    instance: InstanceHandle,
    resource_id: Option<u16>,
    width_metric: i32,
    height_metric: i32,
) -> ExplorerResult<HICON> {
    let Some(resource_id) = resource_id else {
        return Ok(null_mut());
    };

    // SAFETY: GetSystemMetrics reads process-global system metrics and does not retain pointers.
    let width = unsafe { GetSystemMetrics(width_metric) };
    // SAFETY: GetSystemMetrics reads process-global system metrics and does not retain pointers.
    let height = unsafe { GetSystemMetrics(height_metric) };
    load_icon_resource(instance, resource_id, width, height)
}

fn load_icon_resource(
    instance: InstanceHandle,
    resource_id: u16,
    width: i32,
    height: i32,
) -> ExplorerResult<HICON> {
    // SAFETY: resource_id_to_ptr uses the Win32 MAKEINTRESOURCEW convention for integer resources.
    let icon = unsafe {
        LoadImageW(
            instance.raw(),
            resource_id_to_ptr(resource_id),
            IMAGE_ICON,
            width,
            height,
            LR_SHARED,
        )
    };
    if icon.is_null() {
        return Err(windows_api_error("load icon resource", "LoadImageW"));
    }

    Ok(icon)
}

fn resource_id_to_ptr(resource_id: u16) -> *const u16 {
    resource_id as usize as *const u16
}

fn str_to_wide_null(value: &str) -> Vec<u16> {
    os_to_wide_null(OsStr::new(value))
}

fn os_to_wide_null(value: &OsStr) -> Vec<u16> {
    value.encode_wide().chain(std::iter::once(0)).collect()
}

fn wide_buffer_to_os_string(buffer: &[u16]) -> OsString {
    let end = buffer
        .iter()
        .position(|unit| *unit == 0)
        .unwrap_or(buffer.len());
    OsString::from_wide(&buffer[..end])
}

unsafe fn wide_ptr_to_os_string(ptr: *const u16) -> OsString {
    let mut len = 0;
    // SAFETY: caller guarantees ptr is a valid null-terminated UTF-16 string.
    while unsafe { *ptr.add(len) } != 0 {
        len += 1;
    }
    // SAFETY: caller guarantees the string is valid for len UTF-16 code units.
    OsString::from_wide(unsafe { std::slice::from_raw_parts(ptr, len) })
}

fn windows_api_error(operation: &'static str, api: &'static str) -> ExplorerError {
    ExplorerError::windows_api(operation, api, last_error_code(), None)
}

fn last_error_code() -> u32 {
    // SAFETY: GetLastError reads thread-local Windows error state and has no preconditions.
    unsafe { GetLastError() }
}

fn clear_last_error() {
    // SAFETY: SetLastError writes the calling thread's Windows error state.
    unsafe {
        SetLastError(0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn light_tree_theme_restores_system_text_and_background_colors() {
        let palette = ThemePalette::for_theme(AppearanceTheme::Light);
        let colors = TreeViewThemeColors::for_theme(palette);

        assert_eq!(colors.background, TREE_VIEW_USE_SYSTEM_COLOR);
        assert_eq!(colors.text, TREE_VIEW_USE_SYSTEM_COLOR);
        assert_eq!(colors.line, CLR_DEFAULT as isize);
    }

    #[test]
    fn light_list_view_theme_uses_explicit_light_colors() {
        let palette = ThemePalette::for_theme(AppearanceTheme::Light);
        let colors = ListViewThemeColors::for_theme(palette);

        assert_eq!(colors.background, rgb(255, 255, 255) as isize);
        assert_eq!(colors.text, rgb(0, 0, 0) as isize);
        assert_eq!(colors.text_background, rgb(255, 255, 255) as isize);
    }

    #[test]
    fn custom_list_view_themes_use_explicit_palette_colors() {
        for theme in AppearanceTheme::options()
            .iter()
            .copied()
            .filter(|theme| theme.uses_dark_mode())
        {
            let palette = ThemePalette::for_theme(theme);
            let colors = ListViewThemeColors::for_theme(palette);

            assert_eq!(colors.background, palette.control_background as isize);
            assert_eq!(colors.text, palette.control_text as isize);
            assert_eq!(colors.text_background, palette.control_background as isize);
        }
    }

    #[test]
    fn list_view_scroll_delta_preserves_top_index() {
        assert_eq!(list_view_scroll_delta_rows(0, 25, 100), Some(25));
        assert_eq!(list_view_scroll_delta_rows(40, 10, 100), Some(-30));
        assert_eq!(list_view_scroll_delta_rows(12, 12, 100), None);
    }

    #[test]
    fn list_view_scroll_delta_clamps_to_existing_items() {
        assert_eq!(list_view_scroll_delta_rows(0, 50, 10), Some(9));
        assert_eq!(list_view_scroll_delta_rows(12, 2, 10), Some(-7));
        assert_eq!(list_view_scroll_delta_rows(0, 1, 0), None);
    }

    #[test]
    fn list_view_scroll_delta_uses_row_height_pixels() {
        assert_eq!(list_view_scroll_delta_pixels(5, 18), Some(90));
        assert_eq!(list_view_scroll_delta_pixels(-3, 20), Some(-60));
        assert_eq!(list_view_scroll_delta_pixels(4, 0), None);
    }

    #[test]
    fn default_gui_logfont_uses_malgun_gothic_at_current_dpi() {
        let logfont = default_gui_logfont(DpiMetrics::new(96));

        assert_eq!(logfont_face_name(&logfont), OsString::from("Malgun Gothic"));
        assert_eq!(logfont.lfHeight, -12);
    }
}
