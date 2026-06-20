use std::ffi::OsString;
use std::ptr::{copy_nonoverlapping, null_mut};

use windows_sys::Win32::UI::Controls::{
    LVIF_IMAGE, LVIF_TEXT, LVN_BEGINDRAG, LVN_COLUMNCLICK, LVN_ENDLABELEDITW, LVN_GETDISPINFOW,
    LVN_ITEMACTIVATE, NMHDR, NMITEMACTIVATE, NMLISTVIEW, NMLVDISPINFOW, NMTREEVIEWW, NM_DBLCLK,
    NM_RCLICK, TCN_SELCHANGE, TVE_COLLAPSE, TVE_COLLAPSERESET, TVE_EXPAND, TVN_BEGINDRAGW,
    TVN_ITEMEXPANDINGW, TVN_SELCHANGEDW,
};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyState, VK_CONTROL, VK_DELETE, VK_ESCAPE, VK_F2, VK_F3, VK_F5, VK_LEFT, VK_RIGHT, VK_TAB,
    VK_UP,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateAcceleratorTableW, DefWindowProcW, DestroyAcceleratorTable, DestroyWindow,
    DispatchMessageW, GetClassNameW, GetDlgCtrlID, GetMessageW, GetWindowLongPtrW,
    IsDialogMessageW, KillTimer, MessageBoxW, PostMessageW, PostQuitMessage, SetTimer,
    SetWindowLongPtrW, TranslateAcceleratorW, TranslateMessage, ACCEL, EN_KILLFOCUS, FALT,
    FCONTROL, FSHIFT, FVIRTKEY, GWLP_USERDATA, HACCEL, MB_ICONERROR, MB_OK, MSG, WM_APP,
    WM_CAPTURECHANGED, WM_COMMAND, WM_CREATE, WM_CTLCOLORBTN, WM_CTLCOLOREDIT, WM_CTLCOLORSTATIC,
    WM_DESTROY, WM_DPICHANGED, WM_DRAWITEM, WM_ENTERSIZEMOVE, WM_ERASEBKGND, WM_EXITSIZEMOVE,
    WM_GETMINMAXINFO, WM_KEYDOWN, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE, WM_NCCREATE,
    WM_NCDESTROY, WM_NOTIFY, WM_SETCURSOR, WM_SIZE, WM_SYSKEYDOWN, WM_TIMER,
};

use crate::domain::{ExplorerError, ExplorerResult};

use super::{
    str_to_wide_null, wide_ptr_to_os_string, windows_api_error, MessageLong, MessageResult,
    MessageWord, TreeViewItemHandle, TreeViewItemValue, WindowHandle,
};

pub const MESSAGE_NC_CREATE: u32 = WM_NCCREATE;
pub const MESSAGE_CREATE: u32 = WM_CREATE;
pub const MESSAGE_COMMAND: u32 = WM_COMMAND;
pub const MESSAGE_NOTIFY: u32 = WM_NOTIFY;
pub const MESSAGE_SIZE: u32 = WM_SIZE;
pub const MESSAGE_GET_MIN_MAX_INFO: u32 = WM_GETMINMAXINFO;
pub const MESSAGE_DPI_CHANGED: u32 = WM_DPICHANGED;
pub const MESSAGE_ENTER_SIZE_MOVE: u32 = WM_ENTERSIZEMOVE;
pub const MESSAGE_EXIT_SIZE_MOVE: u32 = WM_EXITSIZEMOVE;
pub const MESSAGE_ERASE_BACKGROUND: u32 = WM_ERASEBKGND;
pub const MESSAGE_CONTROL_COLOR_EDIT: u32 = WM_CTLCOLOREDIT;
pub const MESSAGE_CONTROL_COLOR_STATIC: u32 = WM_CTLCOLORSTATIC;
pub const MESSAGE_CONTROL_COLOR_BUTTON: u32 = WM_CTLCOLORBTN;
pub const MESSAGE_DRAW_ITEM: u32 = WM_DRAWITEM;
pub const MESSAGE_SET_CURSOR: u32 = WM_SETCURSOR;
pub const MESSAGE_MOUSE_MOVE: u32 = WM_MOUSEMOVE;
pub const MESSAGE_LEFT_BUTTON_DOWN: u32 = WM_LBUTTONDOWN;
pub const MESSAGE_LEFT_BUTTON_UP: u32 = WM_LBUTTONUP;
pub const MESSAGE_CAPTURE_CHANGED: u32 = WM_CAPTURECHANGED;
pub const MESSAGE_DESTROY: u32 = WM_DESTROY;
pub const MESSAGE_NC_DESTROY: u32 = WM_NCDESTROY;
pub const MESSAGE_TIMER: u32 = WM_TIMER;
pub const MESSAGE_APP: u32 = WM_APP;
pub const EDIT_KILL_FOCUS: u16 = EN_KILLFOCUS as u16;
pub const NOTIFICATION_DBL_CLICK: u32 = NM_DBLCLK;
pub const LIST_VIEW_ITEM_ACTIVATE: u32 = LVN_ITEMACTIVATE;
pub const LIST_VIEW_RIGHT_CLICK: u32 = NM_RCLICK;
pub const LIST_VIEW_END_LABEL_EDIT: u32 = LVN_ENDLABELEDITW;
pub const LIST_VIEW_COLUMN_CLICK: u32 = LVN_COLUMNCLICK;
pub const LIST_VIEW_BEGIN_DRAG: u32 = LVN_BEGINDRAG;
pub const LIST_VIEW_GET_DISPLAY_INFO: u32 = LVN_GETDISPINFOW;
pub const TAB_SELECTION_CHANGED: u32 = TCN_SELCHANGE;
pub const TAB_RIGHT_CLICK: u32 = NM_RCLICK;
pub const TREE_VIEW_RIGHT_CLICK: u32 = NM_RCLICK;
pub const TREE_VIEW_BEGIN_DRAG: u32 = TVN_BEGINDRAGW;
pub const TREE_VIEW_ITEM_EXPANDING: u32 = TVN_ITEMEXPANDINGW;
pub const TREE_VIEW_SELECTION_CHANGED: u32 = TVN_SELCHANGEDW;
pub const KEY_DELETE: u16 = VK_DELETE;
pub const KEY_ESCAPE: u16 = VK_ESCAPE;
pub const KEY_F2: u16 = VK_F2;
pub const KEY_F3: u16 = VK_F3;
pub const KEY_F5: u16 = VK_F5;
pub const KEY_LEFT: u16 = VK_LEFT;
pub const KEY_RIGHT: u16 = VK_RIGHT;
pub const KEY_TAB: u16 = VK_TAB;
pub const KEY_UP: u16 = VK_UP;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Accelerator {
    pub key: u16,
    pub command_id: u16,
    pub control: bool,
    pub alt: bool,
    pub shift: bool,
}

impl Accelerator {
    pub const fn new(key: u16, command_id: u16) -> Self {
        Self {
            key,
            command_id,
            control: false,
            alt: false,
            shift: false,
        }
    }

    pub const fn control(mut self) -> Self {
        self.control = true;
        self
    }

    pub const fn alt(mut self) -> Self {
        self.alt = true;
        self
    }

    pub const fn shift(mut self) -> Self {
        self.shift = true;
        self
    }

    fn virt_flags(self) -> u8 {
        let mut flags = FVIRTKEY;
        if self.control {
            flags |= FCONTROL;
        }
        if self.alt {
            flags |= FALT;
        }
        if self.shift {
            flags |= FSHIFT;
        }
        flags
    }
}

pub struct AcceleratorTable {
    handle: HACCEL,
}

impl AcceleratorTable {
    fn raw(&self) -> HACCEL {
        self.handle
    }
}

impl Drop for AcceleratorTable {
    fn drop(&mut self) {
        if !self.handle.is_null() {
            // SAFETY: handle was created by CreateAcceleratorTableW and is owned by this wrapper.
            unsafe {
                DestroyAcceleratorTable(self.handle);
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ControlKeyCommand {
    pub control_id: u16,
    pub key: u16,
    pub command_id: u16,
}

impl ControlKeyCommand {
    pub const fn new(control_id: u16, key: u16, command_id: u16) -> Self {
        Self {
            control_id,
            key,
            command_id,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TreeViewExpandNotification {
    pub action: TreeViewExpandAction,
    pub item: TreeViewItemHandle,
    pub value: Option<TreeViewItemValue>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TreeViewItemNotification {
    pub item: TreeViewItemHandle,
    pub value: Option<TreeViewItemValue>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TreeViewExpandAction {
    Expand,
    Collapse,
    CollapseReset,
    Other(u32),
}

impl TreeViewExpandAction {
    fn from_raw(value: u32) -> Self {
        match value {
            TVE_EXPAND => Self::Expand,
            TVE_COLLAPSE => Self::Collapse,
            TVE_COLLAPSERESET => Self::CollapseReset,
            other => Self::Other(other),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ControlNotification {
    pub hwnd_from: WindowHandle,
    pub id_from: usize,
    pub code: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListViewLabelEdit {
    pub index: usize,
    pub text: Option<OsString>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ListViewDisplayRequest {
    pub row_index: usize,
    pub column_index: usize,
    pub needs_text: bool,
    pub needs_image: bool,
}

pub fn create_accelerator_table(accelerators: &[Accelerator]) -> ExplorerResult<AcceleratorTable> {
    if accelerators.is_empty() {
        return Err(ExplorerError::invalid_input(
            "단축키 테이블에 항목이 없습니다.",
        ));
    }

    let entries = accelerators
        .iter()
        .map(|accelerator| ACCEL {
            fVirt: accelerator.virt_flags(),
            key: accelerator.key,
            cmd: accelerator.command_id,
        })
        .collect::<Vec<_>>();
    let count = i32::try_from(entries.len())
        .map_err(|_| ExplorerError::state_conflict("단축키 항목이 너무 많습니다."))?;

    // SAFETY: entries points to count ACCEL structures for the duration of the call.
    let handle = unsafe { CreateAcceleratorTableW(entries.as_ptr(), count) };
    if handle.is_null() {
        return Err(windows_api_error(
            "create accelerator table",
            "CreateAcceleratorTableW",
        ));
    }

    Ok(AcceleratorTable { handle })
}

pub fn message_loop(
    dialog_owner: WindowHandle,
    accelerator_table: Option<&AcceleratorTable>,
    control_key_commands: &[ControlKeyCommand],
) -> ExplorerResult<i32> {
    let mut message = MSG::default();

    loop {
        // SAFETY: message is writable and no message filter is used.
        let result = unsafe { GetMessageW(&mut message, null_mut(), 0, 0) };
        if result == -1 {
            return Err(windows_api_error("read window message", "GetMessageW"));
        }

        if result == 0 {
            return Ok(message.wParam as i32);
        }

        if let Some(command_id) = control_key_command(&message, control_key_commands) {
            post_window_message(dialog_owner, WM_COMMAND, usize::from(command_id), 0)?;
            continue;
        }

        if let Some(accelerator_table) = accelerator_table {
            if translate_accelerator(dialog_owner, accelerator_table, &message) {
                continue;
            }
        }

        if !dialog_owner.is_null() {
            // SAFETY: dialog_owner is the application's top-level window and message is the
            // current message returned by GetMessageW. IsDialogMessageW owns no pointers after it
            // returns and performs standard modeless dialog keyboard navigation for child controls.
            let handled = unsafe { IsDialogMessageW(dialog_owner.raw(), &message) };
            if handled != 0 {
                continue;
            }
        }

        // SAFETY: message was produced by GetMessageW.
        unsafe {
            TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }
}

fn control_key_command(message: &MSG, commands: &[ControlKeyCommand]) -> Option<u16> {
    if commands.is_empty() || message.message != WM_KEYDOWN || message.hwnd.is_null() {
        return None;
    }

    let key = message.wParam as u16;
    let control_id = message_control_id(message)?;
    commands
        .iter()
        .find(|command| command.control_id == control_id && command.key == key)
        .map(|command| command.command_id)
}

fn message_control_id(message: &MSG) -> Option<u16> {
    // SAFETY: message.hwnd is the message target supplied by Windows.
    let id = unsafe { GetDlgCtrlID(message.hwnd) };
    if id < 0 {
        return None;
    }

    u16::try_from(id).ok()
}

fn translate_accelerator(
    dialog_owner: WindowHandle,
    accelerator_table: &AcceleratorTable,
    message: &MSG,
) -> bool {
    if dialog_owner.is_null() || accelerator_table.raw().is_null() {
        return false;
    }
    if should_preserve_edit_control_shortcut(message) {
        return false;
    }

    // SAFETY: dialog_owner is the top-level window and message is the current MSG from GetMessageW.
    unsafe { TranslateAcceleratorW(dialog_owner.raw(), accelerator_table.raw(), message) != 0 }
}

fn should_preserve_edit_control_shortcut(message: &MSG) -> bool {
    if message.message != WM_KEYDOWN && message.message != WM_SYSKEYDOWN {
        return false;
    }
    if message.hwnd.is_null() || !is_edit_window(message.hwnd) {
        return false;
    }

    let key = message.wParam as u16;
    if key == VK_DELETE || key == VK_F2 {
        return true;
    }

    key_is_down(VK_CONTROL) && matches!(key, 65 | 67 | 86 | 88 | 90)
}

fn is_edit_window(hwnd: windows_sys::Win32::Foundation::HWND) -> bool {
    let mut class_name = [0_u16; 16];
    // SAFETY: class_name is writable and hwnd is the message target supplied by Windows.
    let len = unsafe { GetClassNameW(hwnd, class_name.as_mut_ptr(), class_name.len() as i32) };
    if len != 4 {
        return false;
    }

    ascii_upper(class_name[0]) == b'E' as u16
        && ascii_upper(class_name[1]) == b'D' as u16
        && ascii_upper(class_name[2]) == b'I' as u16
        && ascii_upper(class_name[3]) == b'T' as u16
}

fn ascii_upper(unit: u16) -> u16 {
    if (b'a' as u16..=b'z' as u16).contains(&unit) {
        unit - 32
    } else {
        unit
    }
}

fn key_is_down(key: u16) -> bool {
    // SAFETY: GetKeyState reads the current thread keyboard state for a virtual-key code.
    unsafe { GetKeyState(i32::from(key)) < 0 }
}

pub fn show_error_message(parent: WindowHandle, title: &str, message: &str) {
    let title = str_to_wide_null(title);
    let message = str_to_wide_null(message);

    // SAFETY: strings are null terminated and live through the call; parent may be null.
    unsafe {
        MessageBoxW(
            parent.raw(),
            message.as_ptr(),
            title.as_ptr(),
            MB_OK | MB_ICONERROR,
        );
    }
}

pub fn destroy_window(hwnd: WindowHandle) {
    // SAFETY: hwnd is a window handle owned by the GUI thread.
    unsafe {
        DestroyWindow(hwnd.raw());
    }
}

pub fn post_quit_message(exit_code: i32) {
    // SAFETY: posts a quit message to the current GUI thread.
    unsafe {
        PostQuitMessage(exit_code);
    }
}

pub fn set_window_timer(
    hwnd: WindowHandle,
    timer_id: usize,
    interval_ms: u32,
) -> ExplorerResult<()> {
    // SAFETY: hwnd is the owner window for this timer and no callback is supplied, so WM_TIMER is
    // delivered through the normal window procedure.
    let created = unsafe { SetTimer(hwnd.raw(), timer_id, interval_ms, None) };
    if created == 0 {
        return Err(windows_api_error("set window timer", "SetTimer"));
    }

    Ok(())
}

pub fn kill_window_timer(hwnd: WindowHandle, timer_id: usize) -> ExplorerResult<()> {
    // SAFETY: hwnd and timer_id identify a timer created by SetTimer for this window.
    let succeeded = unsafe { KillTimer(hwnd.raw(), timer_id) };
    if succeeded == 0 {
        return Err(windows_api_error("kill window timer", "KillTimer"));
    }

    Ok(())
}

pub fn post_window_message(
    hwnd: WindowHandle,
    message: u32,
    wparam: MessageWord,
    lparam: MessageLong,
) -> ExplorerResult<()> {
    // SAFETY: hwnd identifies a window owned by this process and the payload ownership is defined
    // by the caller for the custom message.
    let succeeded = unsafe { PostMessageW(hwnd.raw(), message, wparam, lparam) };
    if succeeded == 0 {
        return Err(windows_api_error("post window message", "PostMessageW"));
    }

    Ok(())
}

pub fn default_window_proc(
    hwnd: WindowHandle,
    message: u32,
    wparam: MessageWord,
    lparam: MessageLong,
) -> MessageResult {
    // SAFETY: forwarding unhandled messages to DefWindowProcW is the documented Win32 behavior.
    unsafe { DefWindowProcW(hwnd.raw(), message, wparam, lparam) }
}

pub fn command_id(wparam: MessageWord) -> u16 {
    (wparam & 0xffff) as u16
}

pub fn command_notification(wparam: MessageWord) -> u16 {
    ((wparam >> 16) & 0xffff) as u16
}

/// Reads a `WM_NOTIFY` payload as an `NMHDR`.
///
/// # Safety
///
/// `lparam` must be the `WM_NOTIFY` lparam supplied by Windows for the current message dispatch.
pub unsafe fn notification(lparam: MessageLong) -> Option<ControlNotification> {
    if lparam == 0 {
        return None;
    }

    // SAFETY: WM_NOTIFY lparam points to an NMHDR-compatible structure for the current message.
    let header = unsafe { (lparam as *const NMHDR).as_ref() }?;
    Some(ControlNotification {
        hwnd_from: WindowHandle::from_sys(header.hwndFrom),
        id_from: header.idFrom,
        code: header.code,
    })
}

/// Reads a ListView activation notification payload and returns the item index.
///
/// # Safety
///
/// `lparam` must point to an `NMITEMACTIVATE` payload supplied by the ListView notification being
/// handled.
pub unsafe fn list_view_activation_index(lparam: MessageLong) -> Option<usize> {
    if lparam == 0 {
        return None;
    }

    // SAFETY: callers use this only for ListView activation notifications where lparam points to
    // NMITEMACTIVATE.
    let activation = unsafe { (lparam as *const NMITEMACTIVATE).as_ref() }?;
    usize::try_from(activation.iItem).ok()
}

/// Reads a ListView column-click notification payload and returns the column index.
///
/// # Safety
///
/// `lparam` must point to an `NMLISTVIEW` payload supplied by a ListView
/// `LVN_COLUMNCLICK` notification for the current message dispatch.
pub unsafe fn list_view_column_click_index(lparam: MessageLong) -> Option<usize> {
    if lparam == 0 {
        return None;
    }

    // SAFETY: callers use this only for LVN_COLUMNCLICK where lparam points to NMLISTVIEW.
    let notification = unsafe { (lparam as *const NMLISTVIEW).as_ref() }?;
    usize::try_from(notification.iSubItem).ok()
}

/// Reads a ListView begin-drag notification payload and returns the item index.
///
/// # Safety
///
/// `lparam` must point to an `NMLISTVIEW` payload supplied by a ListView
/// `LVN_BEGINDRAG` notification for the current message dispatch.
pub unsafe fn list_view_drag_index(lparam: MessageLong) -> Option<usize> {
    if lparam == 0 {
        return None;
    }

    // SAFETY: callers use this only for LVN_BEGINDRAG where lparam points to NMLISTVIEW.
    let notification = unsafe { (lparam as *const NMLISTVIEW).as_ref() }?;
    usize::try_from(notification.iItem).ok()
}

/// Reads a virtual ListView display request payload.
///
/// # Safety
///
/// `lparam` must point to an `NMLVDISPINFOW` payload supplied by a ListView
/// `LVN_GETDISPINFOW` notification for the current message dispatch.
pub unsafe fn list_view_display_request(lparam: MessageLong) -> Option<ListViewDisplayRequest> {
    if lparam == 0 {
        return None;
    }

    // SAFETY: callers use this only for LVN_GETDISPINFOW where lparam points to NMLVDISPINFOW.
    let notification = unsafe { (lparam as *const NMLVDISPINFOW).as_ref() }?;
    Some(ListViewDisplayRequest {
        row_index: usize::try_from(notification.item.iItem).ok()?,
        column_index: usize::try_from(notification.item.iSubItem).ok()?,
        needs_text: notification.item.mask & LVIF_TEXT != 0,
        needs_image: notification.item.mask & LVIF_IMAGE != 0,
    })
}

/// Copies text into a virtual ListView display-info buffer.
///
/// # Safety
///
/// `lparam` must point to the `NMLVDISPINFOW` payload currently being handled. `text` may be
/// null-terminated; if it is longer than the ListView buffer, it is truncated safely.
pub unsafe fn set_list_view_display_text(lparam: MessageLong, text: &[u16]) {
    if lparam == 0 {
        return;
    }

    // SAFETY: callers use this only for LVN_GETDISPINFOW where lparam points to writable
    // NMLVDISPINFOW storage for the duration of the notification.
    let Some(notification) = (unsafe { (lparam as *mut NMLVDISPINFOW).as_mut() }) else {
        return;
    };
    if notification.item.mask & LVIF_TEXT == 0 || notification.item.pszText.is_null() {
        return;
    }
    let Ok(capacity) = usize::try_from(notification.item.cchTextMax) else {
        return;
    };
    if capacity == 0 {
        return;
    }

    let source_len = match text.iter().position(|value| *value == 0) {
        Some(index) => index,
        None => text.len(),
    };
    let copy_len = source_len.min(capacity.saturating_sub(1));

    // SAFETY: pszText points to a writable buffer of cchTextMax UTF-16 code units supplied by the
    // ListView for this synchronous notification.
    unsafe {
        copy_nonoverlapping(text.as_ptr(), notification.item.pszText, copy_len);
        *notification.item.pszText.add(copy_len) = 0;
    }
}

/// Sets the image index for a virtual ListView display-info response.
///
/// # Safety
///
/// `lparam` must point to the `NMLVDISPINFOW` payload currently being handled.
pub unsafe fn set_list_view_display_image(lparam: MessageLong, image_index: i32) {
    if lparam == 0 {
        return;
    }

    // SAFETY: callers use this only for LVN_GETDISPINFOW where lparam points to writable
    // NMLVDISPINFOW storage for the duration of the notification.
    let Some(notification) = (unsafe { (lparam as *mut NMLVDISPINFOW).as_mut() }) else {
        return;
    };
    if notification.item.mask & LVIF_IMAGE != 0 {
        notification.item.iImage = image_index;
    }
}

/// Reads a ListView end-label-edit notification payload.
///
/// # Safety
///
/// `lparam` must point to an `NMLVDISPINFOW` payload supplied by a ListView
/// `LVN_ENDLABELEDITW` notification for the current message dispatch.
pub unsafe fn list_view_label_edit(lparam: MessageLong) -> Option<ListViewLabelEdit> {
    if lparam == 0 {
        return None;
    }

    // SAFETY: callers use this only for LVN_ENDLABELEDITW where lparam points to NMLVDISPINFOW.
    let notification = unsafe { (lparam as *const NMLVDISPINFOW).as_ref() }?;
    let index = usize::try_from(notification.item.iItem).ok()?;
    let text = if notification.item.pszText.is_null() {
        None
    } else {
        // SAFETY: pszText is a null-terminated UTF-16 buffer owned by the notification.
        Some(unsafe { wide_ptr_to_os_string(notification.item.pszText) })
    };

    Some(ListViewLabelEdit { index, text })
}

/// Reads a TreeView item-expanding notification payload.
///
/// # Safety
///
/// `lparam` must point to an `NMTREEVIEWW` payload supplied by a TreeView
/// `TVN_ITEMEXPANDINGW` notification for the current message dispatch.
pub unsafe fn tree_view_expand_notification(
    lparam: MessageLong,
) -> Option<TreeViewExpandNotification> {
    if lparam == 0 {
        return None;
    }

    // SAFETY: callers use this only for TVN_ITEMEXPANDINGW notifications where lparam points to
    // NMTREEVIEWW.
    let notification = unsafe { (lparam as *const NMTREEVIEWW).as_ref() }?;
    Some(TreeViewExpandNotification {
        action: TreeViewExpandAction::from_raw(notification.action),
        item: TreeViewItemHandle::from_raw(notification.itemNew.hItem)?,
        value: TreeViewItemValue::from_lparam(notification.itemNew.lParam),
    })
}

/// Reads a TreeView begin-drag notification payload.
///
/// # Safety
///
/// `lparam` must point to an `NMTREEVIEWW` payload supplied by a TreeView
/// `TVN_BEGINDRAGW` notification for the current message dispatch.
pub unsafe fn tree_view_drag_notification(lparam: MessageLong) -> Option<TreeViewItemNotification> {
    if lparam == 0 {
        return None;
    }

    // SAFETY: callers use this only for TVN_BEGINDRAGW notifications where lparam points to
    // NMTREEVIEWW.
    let notification = unsafe { (lparam as *const NMTREEVIEWW).as_ref() }?;
    Some(TreeViewItemNotification {
        item: TreeViewItemHandle::from_raw(notification.itemNew.hItem)?,
        value: TreeViewItemValue::from_lparam(notification.itemNew.lParam),
    })
}

/// Attaches the application state pointer carried by `WM_NCCREATE` to the window user data.
///
/// # Safety
///
/// `lparam` must be the `WM_NCCREATE` lparam for `hwnd`, and its `lpCreateParams` value must be a
/// `Box<T>` pointer that remains owned by the window until `take_window_state` is called.
pub unsafe fn attach_window_state_from_nccreate<T>(
    hwnd: WindowHandle,
    lparam: MessageLong,
) -> bool {
    if lparam == 0 {
        return false;
    }

    let create = lparam as *const windows_sys::Win32::UI::WindowsAndMessaging::CREATESTRUCTW;
    if create.is_null() {
        return false;
    }

    // SAFETY: WM_NCCREATE supplies a valid CREATESTRUCTW pointer for the current call.
    let state = unsafe { (*create).lpCreateParams as *mut T };
    if state.is_null() {
        return false;
    }

    // SAFETY: state is the pointer provided when creating this window.
    unsafe {
        SetWindowLongPtrW(hwnd.raw(), GWLP_USERDATA, state as isize);
    }
    true
}

/// Returns the mutable state pointer previously attached to the window.
///
/// # Safety
///
/// The caller must ensure the stored pointer was created for `T`, is still owned by `hwnd`, and no
/// other mutable reference to the same value is alive.
pub unsafe fn window_state_mut<T>(hwnd: WindowHandle) -> Option<&'static mut T> {
    // SAFETY: GWLP_USERDATA is only written by attach_window_state_from_nccreate for T.
    let ptr = unsafe { GetWindowLongPtrW(hwnd.raw(), GWLP_USERDATA) as *mut T };
    if ptr.is_null() {
        None
    } else {
        // SAFETY: the pointer is owned by the window until take_window_state is called.
        unsafe { ptr.as_mut() }
    }
}

/// Reclaims the boxed state pointer from the window and clears the user data slot.
///
/// # Safety
///
/// The stored pointer must have been produced by `Box::into_raw` for `T`, and this function must be
/// called at most once for that pointer.
pub unsafe fn take_window_state<T>(hwnd: WindowHandle) -> Option<Box<T>> {
    // SAFETY: GWLP_USERDATA is only written with Box::into_raw for T.
    let ptr = unsafe { GetWindowLongPtrW(hwnd.raw(), GWLP_USERDATA) as *mut T };
    if ptr.is_null() {
        return None;
    }

    // SAFETY: clearing user data prevents a second Box reconstruction.
    unsafe {
        SetWindowLongPtrW(hwnd.raw(), GWLP_USERDATA, 0);
    }

    // SAFETY: ptr came from Box::into_raw and ownership is reclaimed exactly once here.
    Some(unsafe { Box::from_raw(ptr) })
}
