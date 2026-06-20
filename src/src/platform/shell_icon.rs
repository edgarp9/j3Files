use std::mem::size_of;
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};

use windows_sys::Win32::Foundation::GetLastError;
use windows_sys::Win32::Storage::FileSystem::{
    FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_NORMAL, FILE_FLAGS_AND_ATTRIBUTES,
};
use windows_sys::Win32::UI::Controls::HIMAGELIST;
use windows_sys::Win32::UI::Shell::{
    SHGetFileInfoW, SHFILEINFOW, SHGFI_SMALLICON, SHGFI_SYSICONINDEX, SHGFI_USEFILEATTRIBUTES,
};

use crate::domain::{ExplorerError, ExplorerResult};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShellIconIndex {
    system_image_index: i32,
}

impl ShellIconIndex {
    pub fn system_image_index(self) -> i32 {
        self.system_image_index
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShellImageListHandle(HIMAGELIST);

impl ShellImageListHandle {
    pub(super) fn raw(self) -> HIMAGELIST {
        self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShellIconKind {
    File,
    Folder,
    Drive,
    NetworkShare,
    KnownFolder,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShellIconQuery {
    path: PathBuf,
    kind: ShellIconKind,
    use_file_attributes: bool,
}

impl ShellIconQuery {
    pub fn generic_file(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            kind: ShellIconKind::File,
            use_file_attributes: true,
        }
    }

    pub fn generic_folder(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            kind: ShellIconKind::Folder,
            use_file_attributes: true,
        }
    }

    pub fn drive(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            kind: ShellIconKind::Drive,
            use_file_attributes: false,
        }
    }

    pub fn network_share(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            kind: ShellIconKind::NetworkShare,
            use_file_attributes: true,
        }
    }

    pub fn known_folder(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            kind: ShellIconKind::KnownFolder,
            use_file_attributes: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShellIconLookup {
    pub icon: ShellIconIndex,
    pub image_list: ShellImageListHandle,
}

pub fn shell_file_icon(query: &ShellIconQuery) -> ExplorerResult<ShellIconLookup> {
    let wide_path = path_to_shell_wide_null(&query.path);
    let mut file_info = SHFILEINFOW::default();
    let flags = shell_icon_flags(query.use_file_attributes);

    // SAFETY: wide_path is a null-terminated UTF-16 Shell parsing name. file_info is writable
    // for the duration of the call.
    let raw_image_list = unsafe {
        SHGetFileInfoW(
            wide_path.as_ptr(),
            shell_file_attributes(query.kind),
            &mut file_info,
            size_of::<SHFILEINFOW>() as u32,
            flags,
        )
    };
    if raw_image_list == 0 || file_info.iIcon < 0 {
        return Err(ExplorerError::windows_api(
            "read shell file icon",
            "SHGetFileInfoW",
            last_error_code(),
            Some(query.path.clone()),
        ));
    }

    Ok(ShellIconLookup {
        icon: ShellIconIndex {
            system_image_index: file_info.iIcon,
        },
        image_list: ShellImageListHandle(raw_image_list as HIMAGELIST),
    })
}

fn shell_icon_flags(use_file_attributes: bool) -> u32 {
    let mut flags = SHGFI_SYSICONINDEX | SHGFI_SMALLICON;
    if use_file_attributes {
        flags |= SHGFI_USEFILEATTRIBUTES;
    }
    flags
}

fn shell_file_attributes(kind: ShellIconKind) -> FILE_FLAGS_AND_ATTRIBUTES {
    match kind {
        ShellIconKind::File => FILE_ATTRIBUTE_NORMAL,
        ShellIconKind::Folder
        | ShellIconKind::Drive
        | ShellIconKind::NetworkShare
        | ShellIconKind::KnownFolder => FILE_ATTRIBUTE_DIRECTORY,
    }
}

fn path_to_shell_wide_null(path: &Path) -> Vec<u16> {
    let mut raw_path = path.as_os_str().encode_wide().collect::<Vec<_>>();
    normalize_windows_separators(&mut raw_path);
    raw_path.push(0);
    raw_path
}

fn normalize_windows_separators(value: &mut [u16]) {
    for unit in value {
        if *unit == b'/' as u16 {
            *unit = b'\\' as u16;
        }
    }
}

fn last_error_code() -> u32 {
    // SAFETY: GetLastError reads thread-local Windows error state and has no preconditions.
    unsafe { GetLastError() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generic_file_query_uses_file_attributes() {
        let query = ShellIconQuery::generic_file(PathBuf::from("sample.txt"));

        assert_eq!(query.kind, ShellIconKind::File);
        assert!(query.use_file_attributes);
    }

    #[test]
    fn drive_query_uses_actual_shell_path() {
        let query = ShellIconQuery::drive(PathBuf::from(r"C:\"));

        assert_eq!(query.kind, ShellIconKind::Drive);
        assert!(!query.use_file_attributes);
    }

    #[test]
    fn shell_path_is_null_terminated_and_normalized() {
        let wide = path_to_shell_wide_null(Path::new("C:/Temp"));

        assert_eq!(wide.last().copied(), Some(0));
        assert!(wide.contains(&(b'\\' as u16)));
    }
}
