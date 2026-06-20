use std::ffi::OsString;
use std::mem::size_of;
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};
use std::ptr::null_mut;

use windows_sys::Win32::Foundation::{GetLastError, GlobalFree, HGLOBAL, POINT};
use windows_sys::Win32::System::DataExchange::RegisterClipboardFormatW;
use windows_sys::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock, GHND};
use windows_sys::Win32::UI::Shell::{DragQueryFileW, CFSTR_PREFERREDDROPEFFECT, DROPFILES, HDROP};

use crate::domain::{ExplorerError, ExplorerResult};

pub(super) const CF_HDROP_FORMAT: u16 = 15;
pub(super) const DROPEFFECT_COPY_VALUE: u32 = 1;
pub(super) const DROPEFFECT_MOVE_VALUE: u32 = 2;

const DRAG_QUERY_FILE_COUNT: u32 = u32::MAX;
const MAX_CF_HDROP_FILE_ITEMS: u32 = 4_096;
const MAX_CF_HDROP_PATH_UNITS: u32 = 32_767;
const MAX_CF_HDROP_HGLOBAL_BYTES: usize = 64 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum FileDropUsage {
    Clipboard,
    DragSource,
    DropTarget,
}

impl FileDropUsage {
    fn empty_list_message(self) -> &'static str {
        match self {
            Self::Clipboard => "클립보드 파일 목록이 비어 있습니다.",
            Self::DragSource => "드래그할 파일 또는 폴더가 없습니다.",
            Self::DropTarget => "드롭 파일 목록이 비어 있습니다.",
        }
    }

    fn too_many_items_message(self) -> String {
        match self {
            Self::Clipboard => "클립보드 파일 항목이 너무 많아 붙여넣을 수 없습니다.".to_string(),
            Self::DragSource => format!(
                "파일 또는 폴더가 너무 많습니다. 한 번에 최대 {}개까지 드래그할 수 있습니다.",
                MAX_CF_HDROP_FILE_ITEMS
            ),
            Self::DropTarget => format!(
                "파일 또는 폴더가 너무 많습니다. 한 번에 최대 {}개까지 드롭할 수 있습니다.",
                MAX_CF_HDROP_FILE_ITEMS
            ),
        }
    }

    fn empty_path_message(self) -> &'static str {
        match self {
            Self::Clipboard => "클립보드 파일 경로가 비어 있습니다.",
            Self::DragSource => "드래그 파일 경로가 비어 있습니다.",
            Self::DropTarget => "드롭 파일 경로가 비어 있습니다.",
        }
    }

    fn nul_path_message(self) -> &'static str {
        match self {
            Self::Clipboard => "클립보드 파일 경로에 NUL 문자가 포함되어 있습니다.",
            Self::DragSource => "드래그 파일 경로에 NUL 문자가 포함되어 있습니다.",
            Self::DropTarget => "드롭 파일 경로에 NUL 문자가 포함되어 있습니다.",
        }
    }

    fn path_too_long_message(self) -> &'static str {
        match self {
            Self::Clipboard => "클립보드 파일 경로가 너무 길어 붙여넣을 수 없습니다.",
            Self::DragSource => "드래그 파일 경로가 너무 길어 처리할 수 없습니다.",
            Self::DropTarget => "드롭 파일 경로가 너무 길어 처리할 수 없습니다.",
        }
    }

    fn list_too_large_message(self) -> &'static str {
        match self {
            Self::Clipboard => "클립보드 파일 목록이 너무 큽니다.",
            Self::DragSource => "드래그 파일 목록이 너무 커서 처리할 수 없습니다.",
            Self::DropTarget => "드롭 파일 목록이 너무 커서 처리할 수 없습니다.",
        }
    }

    fn allocate_operation(self) -> &'static str {
        match self {
            Self::Clipboard => "allocate clipboard files",
            Self::DragSource => "allocate drag files",
            Self::DropTarget => "allocate dropped files",
        }
    }

    fn lock_operation(self) -> &'static str {
        match self {
            Self::Clipboard => "lock clipboard files",
            Self::DragSource => "lock drag files",
            Self::DropTarget => "lock dropped files",
        }
    }

    fn read_operation(self) -> &'static str {
        match self {
            Self::Clipboard => "read clipboard file path",
            Self::DragSource => "read drag file path",
            Self::DropTarget => "read dropped file path",
        }
    }
}

pub(super) struct OwnedHglobal(HGLOBAL);

impl OwnedHglobal {
    pub(super) fn allocate(byte_len: usize, operation: &'static str) -> ExplorerResult<Self> {
        // SAFETY: GHND requests a zero-initialized movable global memory block.
        let handle = unsafe { GlobalAlloc(GHND, byte_len) };
        if handle.is_null() {
            Err(windows_api_error(operation, "GlobalAlloc"))
        } else {
            Ok(Self(handle))
        }
    }

    pub(super) fn as_raw(&self) -> HGLOBAL {
        self.0
    }

    pub(super) fn into_raw(mut self) -> HGLOBAL {
        let handle = self.0;
        self.0 = null_mut();
        handle
    }
}

impl Drop for OwnedHglobal {
    fn drop(&mut self) {
        free_hglobal(self.0);
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PreparedFileDrop {
    hglobal_byte_len: usize,
    wide_paths: Vec<Vec<u16>>,
}

pub(super) fn create_hdrop_handle(
    paths: &[PathBuf],
    usage: FileDropUsage,
) -> ExplorerResult<OwnedHglobal> {
    let prepared = prepare_file_drop(paths, usage)?;
    let handle = OwnedHglobal::allocate(prepared.hglobal_byte_len, usage.allocate_operation())?;
    // SAFETY: handle is a movable global memory block allocated above.
    let data = unsafe { GlobalLock(handle.as_raw()) } as *mut u8;
    if data.is_null() {
        return Err(windows_api_error(usage.lock_operation(), "GlobalLock"));
    }

    let dropfiles = DROPFILES {
        pFiles: size_of::<DROPFILES>() as u32,
        pt: POINT { x: 0, y: 0 },
        fNC: 0,
        fWide: 1,
    };

    // SAFETY: data points to prepared.hglobal_byte_len writable bytes. DROPFILES is a Win32
    // header and may be unaligned inside HGLOBAL, so write_unaligned is used deliberately.
    unsafe {
        std::ptr::write_unaligned(data.cast::<DROPFILES>(), dropfiles);
        let mut cursor = data.add(size_of::<DROPFILES>()).cast::<u16>();
        for path in &prepared.wide_paths {
            std::ptr::copy_nonoverlapping(path.as_ptr(), cursor, path.len());
            cursor = cursor.add(path.len());
        }
        *cursor = 0;
        GlobalUnlock(handle.as_raw());
    }

    Ok(handle)
}

pub(super) fn validate_hdrop_paths(paths: &[PathBuf], usage: FileDropUsage) -> ExplorerResult<()> {
    prepare_file_drop(paths, usage).map(|_| ())
}

pub(super) fn read_hdrop_paths(hdrop: HDROP, usage: FileDropUsage) -> ExplorerResult<Vec<PathBuf>> {
    // SAFETY: hdrop is a valid HDROP handle from a clipboard or IDataObject STGMEDIUM.
    let count = unsafe { DragQueryFileW(hdrop, DRAG_QUERY_FILE_COUNT, null_mut(), 0) };
    let count = bounded_file_count(count, usage)?;
    let capacity = usize::try_from(count)
        .map_err(|_| ExplorerError::invalid_input(usage.too_many_items_message()))?;
    let mut paths = Vec::with_capacity(capacity);
    let mut total_units = 1_usize;

    for index in 0..count {
        // SAFETY: hdrop is valid and index is below the reported count.
        let len = unsafe { DragQueryFileW(hdrop, index, null_mut(), 0) };
        let buffer_len = hdrop_path_buffer_len(len, usage)?;
        total_units = total_units
            .checked_add(buffer_len)
            .ok_or_else(|| ExplorerError::invalid_input(usage.list_too_large_message()))?;
        file_drop_hglobal_byte_len(total_units, usage)?;

        let mut buffer = vec![0_u16; buffer_len];
        // SAFETY: buffer is writable and sized to include the terminating null.
        let copied = unsafe { DragQueryFileW(hdrop, index, buffer.as_mut_ptr(), len + 1) };
        if copied == 0 {
            return Err(windows_api_error(usage.read_operation(), "DragQueryFileW"));
        }

        let copied = usize::try_from(copied)
            .map_err(|_| ExplorerError::invalid_input(usage.path_too_long_message()))?;
        if copied >= buffer.len() {
            return Err(ExplorerError::invalid_input(usage.path_too_long_message()));
        }
        paths.push(PathBuf::from(OsString::from_wide(&buffer[..copied])));
    }

    Ok(paths)
}

pub(super) fn hdrop_path_buffer_len(len: u32, usage: FileDropUsage) -> ExplorerResult<usize> {
    if len == 0 {
        return Err(ExplorerError::invalid_input(usage.empty_path_message()));
    }
    if len > MAX_CF_HDROP_PATH_UNITS {
        return Err(ExplorerError::invalid_input(usage.path_too_long_message()));
    }

    usize::try_from(len)
        .ok()
        .and_then(|len| len.checked_add(1))
        .ok_or_else(|| ExplorerError::invalid_input(usage.path_too_long_message()))
}

pub(super) fn file_drop_hglobal_byte_len(
    total_path_units: usize,
    usage: FileDropUsage,
) -> ExplorerResult<usize> {
    let path_bytes = total_path_units
        .checked_mul(size_of::<u16>())
        .ok_or_else(|| ExplorerError::invalid_input(usage.list_too_large_message()))?;
    let byte_len = size_of::<DROPFILES>()
        .checked_add(path_bytes)
        .ok_or_else(|| ExplorerError::invalid_input(usage.list_too_large_message()))?;
    if byte_len > MAX_CF_HDROP_HGLOBAL_BYTES {
        return Err(ExplorerError::invalid_input(usage.list_too_large_message()));
    }
    Ok(byte_len)
}

pub(super) fn preferred_drop_effect_format() -> ExplorerResult<u32> {
    // SAFETY: CFSTR_PREFERREDDROPEFFECT is a process-static null-terminated UTF-16 string.
    let format = unsafe { RegisterClipboardFormatW(CFSTR_PREFERREDDROPEFFECT) };
    if format == 0 {
        Err(windows_api_error(
            "register preferred drop effect format",
            "RegisterClipboardFormatW",
        ))
    } else {
        Ok(format)
    }
}

pub(super) fn preferred_drop_effect_format_u16() -> ExplorerResult<u16> {
    let format = preferred_drop_effect_format()?;
    u16::try_from(format).map_err(|_| {
        ExplorerError::state_conflict("Preferred DropEffect 클립보드 형식 값이 너무 큽니다.")
    })
}

pub(super) fn free_hglobal(handle: HGLOBAL) {
    if !handle.is_null() {
        // SAFETY: handle is an HGLOBAL owned by this process and not transferred to Windows.
        unsafe {
            GlobalFree(handle);
        }
    }
}

pub(super) fn hglobal_size(handle: HGLOBAL) -> usize {
    // SAFETY: handle is an HGLOBAL supplied by the clipboard/OLE or allocated by this process.
    unsafe { GlobalSize(handle) }
}

fn prepare_file_drop(paths: &[PathBuf], usage: FileDropUsage) -> ExplorerResult<PreparedFileDrop> {
    if paths.is_empty() {
        return Err(ExplorerError::invalid_input(usage.empty_list_message()));
    }
    if paths.len() > MAX_CF_HDROP_FILE_ITEMS as usize {
        return Err(ExplorerError::invalid_input(usage.too_many_items_message()));
    }

    let mut total_units = 1_usize;
    let mut wide_paths = Vec::with_capacity(paths.len());
    for path in paths {
        let wide_path = wide_path_with_null(path.as_path(), usage)?;
        total_units = total_units
            .checked_add(wide_path.len())
            .ok_or_else(|| ExplorerError::invalid_input(usage.list_too_large_message()))?;
        wide_paths.push(wide_path);
    }

    Ok(PreparedFileDrop {
        hglobal_byte_len: file_drop_hglobal_byte_len(total_units, usage)?,
        wide_paths,
    })
}

fn wide_path_with_null(path: &Path, usage: FileDropUsage) -> ExplorerResult<Vec<u16>> {
    let mut units = Vec::new();
    for unit in path.as_os_str().encode_wide() {
        if unit == 0 {
            return Err(ExplorerError::invalid_input(usage.nul_path_message()));
        }
        units.push(unit);
        if units.len() > MAX_CF_HDROP_PATH_UNITS as usize {
            return Err(ExplorerError::invalid_input(usage.path_too_long_message()));
        }
    }
    if units.is_empty() {
        return Err(ExplorerError::invalid_input(usage.empty_path_message()));
    }
    units.push(0);
    Ok(units)
}

fn bounded_file_count(count: u32, usage: FileDropUsage) -> ExplorerResult<u32> {
    if count > MAX_CF_HDROP_FILE_ITEMS {
        Err(ExplorerError::invalid_input(usage.too_many_items_message()))
    } else {
        Ok(count)
    }
}

fn windows_api_error(operation: &'static str, api: &'static str) -> ExplorerError {
    // SAFETY: GetLastError reads thread-local Windows error state and has no preconditions.
    ExplorerError::windows_api(operation, api, unsafe { GetLastError() }, None)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn drag_path(index: u32) -> PathBuf {
        PathBuf::from(format!(r"C:\bulk\{index}.txt"))
    }

    #[test]
    fn hdrop_preflight_allows_maximum_file_count() -> ExplorerResult<()> {
        let paths = (0..MAX_CF_HDROP_FILE_ITEMS)
            .map(drag_path)
            .collect::<Vec<_>>();

        validate_hdrop_paths(&paths, FileDropUsage::DragSource)?;

        Ok(())
    }

    #[test]
    fn hdrop_preflight_rejects_count_above_limit_before_allocation() {
        let paths = (0..=MAX_CF_HDROP_FILE_ITEMS)
            .map(drag_path)
            .collect::<Vec<_>>();

        let error = validate_hdrop_paths(&paths, FileDropUsage::DragSource)
            .expect_err("oversized CF_HDROP item list must be rejected");

        assert_eq!(
            error.user_message(),
            "파일 또는 폴더가 너무 많습니다. 한 번에 최대 4096개까지 드래그할 수 있습니다."
        );
    }

    #[test]
    fn hdrop_path_buffer_len_allows_configured_limit() {
        assert!(matches!(
            hdrop_path_buffer_len(MAX_CF_HDROP_PATH_UNITS, FileDropUsage::DropTarget),
            Ok(len) if len == MAX_CF_HDROP_PATH_UNITS as usize + 1
        ));
    }

    #[test]
    fn hdrop_path_buffer_len_rejects_empty_path() {
        let error = hdrop_path_buffer_len(0, FileDropUsage::DropTarget)
            .expect_err("empty CF_HDROP path entries must be rejected");

        assert_eq!(error.user_message(), "드롭 파일 경로가 비어 있습니다.");
    }

    #[test]
    fn hdrop_preflight_allows_maximum_path_units() -> ExplorerResult<()> {
        let mut units = vec![b'C' as u16, b':' as u16, b'\\' as u16];
        units.extend(std::iter::repeat_n(
            b'a' as u16,
            MAX_CF_HDROP_PATH_UNITS as usize - units.len(),
        ));
        let path = PathBuf::from(OsString::from_wide(&units));

        validate_hdrop_paths(&[path], FileDropUsage::DragSource)?;

        Ok(())
    }

    #[test]
    fn hdrop_preflight_rejects_too_long_path_before_allocation() {
        let mut units = vec![b'C' as u16, b':' as u16, b'\\' as u16];
        units.extend(std::iter::repeat_n(
            b'a' as u16,
            MAX_CF_HDROP_PATH_UNITS as usize,
        ));
        let path = PathBuf::from(OsString::from_wide(&units));

        let error = validate_hdrop_paths(&[path], FileDropUsage::DragSource)
            .expect_err("too long path must be rejected");

        assert_eq!(
            error.user_message(),
            "드래그 파일 경로가 너무 길어 처리할 수 없습니다."
        );
    }

    #[test]
    fn hdrop_hglobal_byte_len_rejects_arithmetic_overflow() {
        let overflowing_units = usize::MAX / size_of::<u16>() + 1;

        let error = file_drop_hglobal_byte_len(overflowing_units, FileDropUsage::DragSource)
            .expect_err("overflowing CF_HDROP byte length must be rejected");

        assert_eq!(
            error.user_message(),
            "드래그 파일 목록이 너무 커서 처리할 수 없습니다."
        );
    }

    #[test]
    fn hdrop_hglobal_byte_len_enforces_memory_cap() -> ExplorerResult<()> {
        let max_units_under_cap =
            (MAX_CF_HDROP_HGLOBAL_BYTES - size_of::<DROPFILES>()) / size_of::<u16>();

        assert_eq!(
            file_drop_hglobal_byte_len(max_units_under_cap, FileDropUsage::DragSource)?,
            size_of::<DROPFILES>() + max_units_under_cap * size_of::<u16>()
        );

        let error = file_drop_hglobal_byte_len(max_units_under_cap + 1, FileDropUsage::DragSource)
            .expect_err("CF_HDROP byte length over the memory cap must be rejected");

        assert_eq!(
            error.user_message(),
            "드래그 파일 목록이 너무 커서 처리할 수 없습니다."
        );
        Ok(())
    }

    #[test]
    fn hdrop_handle_round_trips_unicode_paths() -> ExplorerResult<()> {
        let paths = vec![
            PathBuf::from(r"C:\드롭 테스트\a b.txt"),
            PathBuf::from(r"\\server\share\자료\#1.txt"),
            PathBuf::from(r"\\?\UNC\server\share\긴 경로\한글.txt"),
        ];
        let handle = create_hdrop_handle(&paths, FileDropUsage::DragSource)?;

        let parsed = read_hdrop_paths(handle.as_raw() as HDROP, FileDropUsage::DropTarget)?;

        assert_eq!(parsed, paths);
        Ok(())
    }
}
