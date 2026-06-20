use std::mem::size_of;
use std::path::PathBuf;
use std::ptr::null_mut;

use windows_sys::Win32::Foundation::{GetLastError, HWND};
use windows_sys::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, GetClipboardData, IsClipboardFormatAvailable, OpenClipboard,
    SetClipboardData,
};
use windows_sys::Win32::System::Memory::{GlobalLock, GlobalUnlock};
use windows_sys::Win32::UI::Shell::HDROP;

use crate::domain::{ExplorerError, ExplorerResult};

use super::hdrop::{self, FileDropUsage, OwnedHglobal};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClipboardFileOperation {
    Copy,
    Move,
}

impl ClipboardFileOperation {
    fn drop_effect(self) -> u32 {
        match self {
            Self::Copy => hdrop::DROPEFFECT_COPY_VALUE,
            Self::Move => hdrop::DROPEFFECT_MOVE_VALUE,
        }
    }

    fn from_drop_effect(effect: u32) -> Self {
        match effect {
            hdrop::DROPEFFECT_MOVE_VALUE => Self::Move,
            _ => Self::Copy,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClipboardFileItems {
    pub operation: ClipboardFileOperation,
    pub paths: Vec<PathBuf>,
}

struct ClipboardGuard;

impl ClipboardGuard {
    fn open(owner_window: isize) -> ExplorerResult<Self> {
        // SAFETY: owner_window is either null or an HWND value owned by this process.
        let opened = unsafe { OpenClipboard(owner_window_from_isize(owner_window)) };
        if opened == 0 {
            return Err(clipboard_error("open clipboard", "OpenClipboard"));
        }

        Ok(Self)
    }
}

impl Drop for ClipboardGuard {
    fn drop(&mut self) {
        // SAFETY: ClipboardGuard is only created after OpenClipboard succeeds.
        unsafe {
            CloseClipboard();
        }
    }
}

pub fn set_clipboard_file_items(
    owner_window: isize,
    paths: &[PathBuf],
    operation: ClipboardFileOperation,
) -> ExplorerResult<()> {
    if paths.is_empty() {
        return Err(ExplorerError::invalid_input(
            "선택된 파일 또는 폴더가 없습니다.",
        ));
    }

    let hdrop_handle = hdrop::create_hdrop_handle(paths, FileDropUsage::Clipboard)?;
    let drop_effect_format = hdrop::preferred_drop_effect_format()?;
    let drop_effect = drop_effect_handle(operation)?;

    let _clipboard = ClipboardGuard::open(owner_window)?;
    // SAFETY: the clipboard is open for the current thread and will be closed by ClipboardGuard.
    if unsafe { EmptyClipboard() } == 0 {
        return Err(clipboard_error("empty clipboard", "EmptyClipboard"));
    }

    let hdrop = hdrop_handle.into_raw();
    // SAFETY: hdrop is a movable global memory handle. On success, clipboard ownership is
    // transferred to the system; on failure we free it below.
    if unsafe { SetClipboardData(hdrop::CF_HDROP_FORMAT as u32, hdrop) }.is_null() {
        let error = clipboard_error("set clipboard files", "SetClipboardData");
        hdrop::free_hglobal(hdrop);
        return Err(error);
    }

    let drop_effect = drop_effect.into_raw();
    // SAFETY: drop_effect is a movable global memory handle. On success, clipboard ownership is
    // transferred to the system; on failure we free it below.
    if unsafe { SetClipboardData(drop_effect_format, drop_effect) }.is_null() {
        let error = clipboard_error("set clipboard drop effect", "SetClipboardData");
        hdrop::free_hglobal(drop_effect);
        clear_clipboard_after_partial_set();
        return Err(error);
    }

    Ok(())
}

pub fn clipboard_file_items(owner_window: isize) -> ExplorerResult<Option<ClipboardFileItems>> {
    let _clipboard = ClipboardGuard::open(owner_window)?;
    // SAFETY: the clipboard is open and CF_HDROP_FORMAT is a plain clipboard format id.
    if unsafe { IsClipboardFormatAvailable(hdrop::CF_HDROP_FORMAT as u32) } == 0 {
        return Ok(None);
    }

    // SAFETY: the clipboard is open and CF_HDROP data remains owned by the clipboard.
    let hdrop = unsafe { GetClipboardData(hdrop::CF_HDROP_FORMAT as u32) };
    if hdrop.is_null() {
        return Err(clipboard_error("read clipboard files", "GetClipboardData"));
    }

    let paths = hdrop::read_hdrop_paths(hdrop as HDROP, FileDropUsage::Clipboard)?;
    if paths.is_empty() {
        return Ok(None);
    }

    Ok(Some(ClipboardFileItems {
        operation: clipboard_drop_effect(),
        paths,
    }))
}

fn drop_effect_handle(operation: ClipboardFileOperation) -> ExplorerResult<OwnedHglobal> {
    let handle = OwnedHglobal::allocate(size_of::<u32>(), "allocate clipboard drop effect")?;
    // SAFETY: handle is a movable global memory block allocated above.
    let data = unsafe { GlobalLock(handle.as_raw()) } as *mut u32;
    if data.is_null() {
        return Err(clipboard_error("lock clipboard drop effect", "GlobalLock"));
    }

    // SAFETY: data points to a writable u32-sized memory block.
    unsafe {
        std::ptr::write_unaligned(data, operation.drop_effect());
        GlobalUnlock(handle.as_raw());
    }

    Ok(handle)
}

fn clipboard_drop_effect() -> ClipboardFileOperation {
    let Ok(format) = hdrop::preferred_drop_effect_format() else {
        return ClipboardFileOperation::Copy;
    };
    // SAFETY: the clipboard is open and format is a registered clipboard format id.
    if unsafe { IsClipboardFormatAvailable(format) } == 0 {
        return ClipboardFileOperation::Copy;
    }

    // SAFETY: the clipboard is open and the returned handle remains owned by the clipboard.
    let handle = unsafe { GetClipboardData(format) };
    if handle.is_null() {
        return ClipboardFileOperation::Copy;
    }

    // SAFETY: handle is a non-null clipboard global memory handle for Preferred DropEffect.
    let byte_len = hdrop::hglobal_size(handle);
    if byte_len < size_of::<u32>() {
        return ClipboardFileOperation::Copy;
    }

    // SAFETY: handle is a clipboard global memory handle for Preferred DropEffect.
    let data = unsafe { GlobalLock(handle) } as *const u32;
    if data.is_null() {
        return ClipboardFileOperation::Copy;
    }

    // SAFETY: GlobalSize verified that Preferred DropEffect has enough bytes for a DWORD.
    let effect = unsafe { std::ptr::read_unaligned(data) };
    // SAFETY: handle was locked above.
    unsafe {
        GlobalUnlock(handle);
    }

    ClipboardFileOperation::from_drop_effect(effect)
}

fn clear_clipboard_after_partial_set() {
    // SAFETY: callers only invoke this while the clipboard is open for the current thread.
    unsafe {
        EmptyClipboard();
    }
}

fn owner_window_from_isize(owner_window: isize) -> HWND {
    if owner_window == 0 {
        null_mut()
    } else {
        owner_window as HWND
    }
}

fn clipboard_error(operation: &'static str, api: &'static str) -> ExplorerError {
    // SAFETY: GetLastError reads thread-local Windows error state and has no preconditions.
    ExplorerError::windows_api(operation, api, unsafe { GetLastError() }, None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drop_effect_uses_move_only_for_exact_move_value() {
        let cases = [
            (hdrop::DROPEFFECT_COPY_VALUE, ClipboardFileOperation::Copy),
            (hdrop::DROPEFFECT_MOVE_VALUE, ClipboardFileOperation::Move),
            (
                hdrop::DROPEFFECT_COPY_VALUE | hdrop::DROPEFFECT_MOVE_VALUE,
                ClipboardFileOperation::Copy,
            ),
            (0, ClipboardFileOperation::Copy),
            (
                hdrop::DROPEFFECT_MOVE_VALUE | 0x8000_0000,
                ClipboardFileOperation::Copy,
            ),
        ];

        for (effect, expected) in cases {
            assert_eq!(ClipboardFileOperation::from_drop_effect(effect), expected);
        }
    }

    #[test]
    fn clipboard_hdrop_preflight_uses_clipboard_user_messages() {
        let paths = (0..=4096)
            .map(|index| PathBuf::from(format!(r"C:\bulk\{index}.txt")))
            .collect::<Vec<_>>();

        let error = hdrop::validate_hdrop_paths(&paths, FileDropUsage::Clipboard)
            .expect_err("oversized clipboard item count must fail");

        assert_eq!(
            error.user_message(),
            "클립보드 파일 항목이 너무 많아 붙여넣을 수 없습니다."
        );
    }
}
