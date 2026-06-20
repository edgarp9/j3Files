use std::ptr::null;

use windows_sys::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreateMenu, CreatePopupMenu, DestroyMenu, DrawMenuBar, PostMessageW,
    SetForegroundWindow, SetMenu, TrackPopupMenu, MF_CHECKED, MF_POPUP, MF_SEPARATOR, MF_STRING,
    MF_UNCHECKED, TPM_RETURNCMD, TPM_RIGHTBUTTON, WM_NULL,
};

use crate::domain::{ExplorerError, ExplorerResult};

use super::{str_to_wide_null, windows_api_error, MenuHandle, ScreenPoint, WindowHandle};

#[derive(Debug)]
pub struct OwnedMenu {
    handle: MenuHandle,
}

impl OwnedMenu {
    pub fn menu_bar() -> ExplorerResult<Self> {
        Ok(Self {
            handle: create_menu_bar()?,
        })
    }

    pub fn popup() -> ExplorerResult<Self> {
        Ok(Self {
            handle: create_popup_menu()?,
        })
    }

    pub fn handle(&self) -> MenuHandle {
        self.handle
    }

    pub fn release(mut self) -> MenuHandle {
        let handle = self.handle;
        self.handle = MenuHandle::null();
        handle
    }
}

impl Drop for OwnedMenu {
    fn drop(&mut self) {
        if !self.handle.is_null() {
            let _ = destroy_menu(self.handle);
        }
    }
}

pub fn create_menu_bar() -> ExplorerResult<MenuHandle> {
    // SAFETY: CreateMenu has no preconditions and returns an owned menu handle.
    let menu = unsafe { CreateMenu() };
    if menu.is_null() {
        return Err(windows_api_error("create menu", "CreateMenu"));
    }

    Ok(MenuHandle::from_sys(menu))
}

pub fn create_popup_menu() -> ExplorerResult<MenuHandle> {
    // SAFETY: CreatePopupMenu has no preconditions and returns an owned menu handle.
    let menu = unsafe { CreatePopupMenu() };
    if menu.is_null() {
        return Err(windows_api_error("create popup menu", "CreatePopupMenu"));
    }

    Ok(MenuHandle::from_sys(menu))
}

pub fn append_menu_item(menu: MenuHandle, id: u16, text: &str) -> ExplorerResult<()> {
    let text = str_to_wide_null(text);
    // SAFETY: menu is an owned or window-attached menu handle; text lives through the call.
    let succeeded = unsafe { AppendMenuW(menu.raw(), MF_STRING, id as usize, text.as_ptr()) };
    if succeeded == 0 {
        return Err(windows_api_error("append menu item", "AppendMenuW"));
    }

    Ok(())
}

pub fn append_checked_menu_item(
    menu: MenuHandle,
    id: u16,
    text: &str,
    checked: bool,
) -> ExplorerResult<()> {
    let text = str_to_wide_null(text);
    let flags = MF_STRING | if checked { MF_CHECKED } else { MF_UNCHECKED };

    // SAFETY: menu is an owned or window-attached menu handle; text lives through the call.
    let succeeded = unsafe { AppendMenuW(menu.raw(), flags, id as usize, text.as_ptr()) };
    if succeeded == 0 {
        return Err(windows_api_error("append menu item", "AppendMenuW"));
    }

    Ok(())
}

pub fn append_menu_separator(menu: MenuHandle) -> ExplorerResult<()> {
    // SAFETY: menu is a valid menu handle; separator does not read the text pointer.
    let succeeded = unsafe { AppendMenuW(menu.raw(), MF_SEPARATOR, 0, null()) };
    if succeeded == 0 {
        return Err(windows_api_error("append menu separator", "AppendMenuW"));
    }

    Ok(())
}

pub fn append_menu_popup(menu: MenuHandle, popup: MenuHandle, text: &str) -> ExplorerResult<()> {
    let text = str_to_wide_null(text);
    // SAFETY: both menu handles are valid and text lives through the call.
    let succeeded =
        unsafe { AppendMenuW(menu.raw(), MF_POPUP, popup.raw() as usize, text.as_ptr()) };
    if succeeded == 0 {
        return Err(windows_api_error("append popup menu", "AppendMenuW"));
    }

    Ok(())
}

pub fn append_owned_menu_popup(
    menu: MenuHandle,
    popup: OwnedMenu,
    text: &str,
) -> ExplorerResult<()> {
    append_menu_popup(menu, popup.handle(), text)?;
    popup.release();
    Ok(())
}

pub fn track_popup_menu(
    owner: WindowHandle,
    menu: MenuHandle,
    point: ScreenPoint,
) -> ExplorerResult<Option<u16>> {
    if owner.is_null() || menu.is_null() {
        return Err(ExplorerError::state_conflict(
            "팝업 메뉴를 표시할 창 또는 메뉴가 없습니다.",
        ));
    }

    // SAFETY: owner is the top-level application window for this popup menu.
    unsafe {
        SetForegroundWindow(owner.raw());
    }

    // SAFETY: menu is a valid popup menu and owner is a valid window on this UI thread. A null
    // RECT permits the normal popup placement behavior.
    let command = unsafe {
        TrackPopupMenu(
            menu.raw(),
            TPM_RETURNCMD | TPM_RIGHTBUTTON,
            point.x,
            point.y,
            0,
            owner.raw(),
            null(),
        )
    };

    // SAFETY: posting WM_NULL completes the standard TrackPopupMenu foreground handling sequence.
    unsafe {
        PostMessageW(owner.raw(), WM_NULL, 0, 0);
    }

    if command == 0 {
        return Ok(None);
    }

    let command = u16::try_from(command)
        .map_err(|_| ExplorerError::state_conflict("팝업 메뉴 명령 ID가 너무 큽니다."))?;
    Ok(Some(command))
}

pub fn set_window_menu(hwnd: WindowHandle, menu: MenuHandle) -> ExplorerResult<()> {
    // SAFETY: hwnd is a top-level window handle and menu is a valid menu bar handle.
    let succeeded = unsafe { SetMenu(hwnd.raw(), menu.raw()) };
    if succeeded == 0 {
        return Err(windows_api_error("set window menu", "SetMenu"));
    }

    Ok(())
}

pub fn draw_menu_bar(hwnd: WindowHandle) -> ExplorerResult<()> {
    // SAFETY: hwnd is a top-level window handle whose menu may have just changed.
    let succeeded = unsafe { DrawMenuBar(hwnd.raw()) };
    if succeeded == 0 {
        return Err(windows_api_error("draw menu bar", "DrawMenuBar"));
    }

    Ok(())
}

pub fn destroy_menu(menu: MenuHandle) -> ExplorerResult<()> {
    // SAFETY: menu is a detached menu handle owned by the application.
    let succeeded = unsafe { DestroyMenu(menu.raw()) };
    if succeeded == 0 {
        return Err(windows_api_error("destroy menu", "DestroyMenu"));
    }

    Ok(())
}
