use std::ffi::c_void;
use std::ptr::null_mut;

use windows_sys::Win32::Foundation::{HINSTANCE, HWND};
use windows_sys::Win32::UI::WindowsAndMessaging::{HICON, HMENU};

pub type RawWindowHandle = *mut c_void;
pub type MessageWord = usize;
pub type MessageLong = isize;
pub type MessageResult = isize;
pub type WindowProcedure = Option<
    unsafe extern "system" fn(RawWindowHandle, u32, MessageWord, MessageLong) -> MessageResult,
>;

#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WindowHandle(HWND);

impl WindowHandle {
    pub fn null() -> Self {
        Self(null_mut())
    }

    pub fn from_raw(raw: RawWindowHandle) -> Self {
        Self(raw)
    }

    pub fn from_isize(value: isize) -> Self {
        Self(value as HWND)
    }

    pub fn is_null(self) -> bool {
        self.0.is_null()
    }

    pub fn as_isize(self) -> isize {
        self.0 as isize
    }

    pub(super) fn from_sys(raw: HWND) -> Self {
        Self(raw)
    }

    pub(super) fn raw(self) -> HWND {
        self.0
    }
}

#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InstanceHandle(HINSTANCE);

impl InstanceHandle {
    pub(super) fn from_sys(raw: HINSTANCE) -> Self {
        Self(raw)
    }

    pub(super) fn raw(self) -> HINSTANCE {
        self.0
    }
}

#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MenuHandle(HMENU);

impl MenuHandle {
    pub fn null() -> Self {
        Self(null_mut())
    }

    pub fn is_null(self) -> bool {
        self.0.is_null()
    }

    pub(super) fn from_sys(raw: HMENU) -> Self {
        Self(raw)
    }

    pub(super) fn raw(self) -> HMENU {
        self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClientRect {
    pub width: i32,
    pub height: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClientPoint {
    pub x: i32,
    pub y: i32,
}

#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IconHandle(HICON);

impl IconHandle {
    pub(super) fn from_sys(raw: HICON) -> Self {
        Self(raw)
    }

    pub(super) fn raw(self) -> HICON {
        self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScreenPoint {
    pub x: i32,
    pub y: i32,
}
