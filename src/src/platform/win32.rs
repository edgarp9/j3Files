use std::ffi::{c_void, OsStr, OsString};
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};
use std::ptr::{null, null_mut};
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, SystemTime};

use windows_sys::Win32::Foundation::{
    CloseHandle, DuplicateHandle, GetLastError, DUPLICATE_SAME_ACCESS, ERROR_FILE_NOT_FOUND,
    ERROR_INVALID_PARAMETER, ERROR_IO_INCOMPLETE, ERROR_IO_PENDING, ERROR_NOT_FOUND,
    ERROR_NO_MORE_FILES, ERROR_OPERATION_ABORTED, ERROR_PATH_NOT_FOUND, FALSE, FILETIME, HANDLE,
    INVALID_HANDLE_VALUE, RPC_E_CHANGED_MODE, TRUE, WAIT_FAILED, WAIT_OBJECT_0, WAIT_TIMEOUT,
};
use windows_sys::Win32::Storage::FileSystem::{
    CreateDirectoryW, CreateFileW, FindClose, FindExInfoBasic, FindExSearchNameMatch,
    FindFirstFileExW, FindFirstFileW, FindNextFileW, GetFileAttributesW, GetLogicalDriveStringsW,
    MoveFileExW, ReadDirectoryChangesW, FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_HIDDEN,
    FILE_ATTRIBUTE_READONLY, FILE_ATTRIBUTE_REPARSE_POINT, FILE_ATTRIBUTE_SYSTEM,
    FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OVERLAPPED, FILE_LIST_DIRECTORY,
    FILE_NOTIFY_CHANGE_ATTRIBUTES, FILE_NOTIFY_CHANGE_CREATION, FILE_NOTIFY_CHANGE_DIR_NAME,
    FILE_NOTIFY_CHANGE_FILE_NAME, FILE_NOTIFY_CHANGE_LAST_WRITE, FILE_NOTIFY_CHANGE_SECURITY,
    FILE_NOTIFY_CHANGE_SIZE, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE,
    FIND_FIRST_EX_LARGE_FETCH, INVALID_FILE_ATTRIBUTES, MOVEFILE_REPLACE_EXISTING,
    MOVEFILE_WRITE_THROUGH, OPEN_EXISTING, WIN32_FIND_DATAW,
};
use windows_sys::Win32::System::Com::{
    CoInitializeEx, CoTaskMemFree, CoUninitialize, COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE,
};
use windows_sys::Win32::System::Threading::{
    CreateEventW, GetCurrentProcess, GetCurrentThread, ResetEvent, SetEvent,
    WaitForMultipleObjects, WaitForSingleObject, INFINITE,
};
use windows_sys::Win32::System::IO::{
    CancelIoEx, CancelSynchronousIo, GetOverlappedResult, OVERLAPPED,
};
use windows_sys::Win32::UI::Shell::{
    FOLDERID_Desktop, FOLDERID_Documents, FOLDERID_Downloads, FOLDERID_Profile,
    SHGetKnownFolderPath, KF_FLAG_DEFAULT,
};

use crate::domain::{ExplorerError, ExplorerResult};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Win32FileAttributes {
    pub hidden: bool,
    pub system: bool,
    pub read_only: bool,
    pub directory: bool,
    pub reparse_point: bool,
}

impl Win32FileAttributes {
    fn from_raw(raw_attributes: u32) -> Self {
        Self {
            hidden: has_attribute(raw_attributes, FILE_ATTRIBUTE_HIDDEN),
            system: has_attribute(raw_attributes, FILE_ATTRIBUTE_SYSTEM),
            read_only: has_attribute(raw_attributes, FILE_ATTRIBUTE_READONLY),
            directory: has_attribute(raw_attributes, FILE_ATTRIBUTE_DIRECTORY),
            reparse_point: has_attribute(raw_attributes, FILE_ATTRIBUTE_REPARSE_POINT),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Win32DirectoryEntry {
    pub file_name: OsString,
    pub attributes: Win32FileAttributes,
    pub file_size: u64,
    pub last_write_time: Option<SystemTime>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DirectoryVisit {
    Continue,
    Stop,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DirectoryChangeKind {
    Added,
    Removed,
    Modified,
    RenamedOldName,
    RenamedNewName,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirectoryChange {
    pub file_name: OsString,
    pub kind: DirectoryChangeKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirectoryChangeBatch {
    pub changes: Vec<DirectoryChange>,
    pub overflowed: bool,
}

impl DirectoryChangeBatch {
    fn precise(changes: Vec<DirectoryChange>) -> Self {
        Self {
            changes,
            overflowed: false,
        }
    }

    fn overflowed() -> Self {
        Self {
            changes: Vec::new(),
            overflowed: true,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Win32KnownFolder {
    Desktop,
    Downloads,
    Documents,
    Profile,
}

#[derive(Debug)]
pub struct DirectoryChangeCancellation {
    event: OwnedHandle,
}

unsafe impl Send for DirectoryChangeCancellation {}
unsafe impl Sync for DirectoryChangeCancellation {}

impl DirectoryChangeCancellation {
    pub fn new() -> ExplorerResult<Self> {
        Ok(Self {
            event: OwnedHandle::manual_reset_event("create directory watch cancellation event")?,
        })
    }

    pub fn request_cancel(&self) -> ExplorerResult<()> {
        // SAFETY: event is a valid manual-reset event handle owned by this wrapper.
        let succeeded = unsafe { SetEvent(self.event.raw()) };
        if succeeded == 0 {
            return Err(ExplorerError::windows_api(
                "cancel directory watch",
                "SetEvent",
                last_error_code(),
                None,
            ));
        }

        Ok(())
    }

    pub fn is_cancel_requested(&self) -> ExplorerResult<bool> {
        wait_for_event_signal(
            self.event.raw(),
            0,
            "check directory watch cancellation",
            "WaitForSingleObject",
            None,
        )
    }

    fn raw(&self) -> HANDLE {
        self.event.raw()
    }
}

#[derive(Debug)]
pub struct SynchronousIoCancellation {
    thread: Mutex<Option<OwnedHandle>>,
}

unsafe impl Send for SynchronousIoCancellation {}
unsafe impl Sync for SynchronousIoCancellation {}

impl SynchronousIoCancellation {
    pub fn new() -> Self {
        Self {
            thread: Mutex::new(None),
        }
    }

    pub fn register_current_thread(
        &self,
    ) -> ExplorerResult<SynchronousIoCancellationRegistration<'_>> {
        let thread_handle = duplicate_current_thread_handle()?;
        let mut thread = self.thread_handle("register synchronous I/O cancellation")?;
        *thread = Some(thread_handle);
        Ok(SynchronousIoCancellationRegistration { cancellation: self })
    }

    pub fn request_cancel(&self) -> ExplorerResult<()> {
        let thread = self.thread_handle("cancel synchronous I/O")?;
        let Some(thread) = thread.as_ref() else {
            return Ok(());
        };

        // SAFETY: thread is a duplicated thread handle kept alive by this cancellation object.
        let cancelled = unsafe { CancelSynchronousIo(thread.raw()) };
        if cancelled == 0 {
            let code = last_error_code();
            if code != ERROR_NOT_FOUND && code != ERROR_OPERATION_ABORTED {
                return Err(ExplorerError::windows_api(
                    "cancel synchronous I/O",
                    "CancelSynchronousIo",
                    code,
                    None,
                ));
            }
        }

        Ok(())
    }

    fn thread_handle(
        &self,
        operation: &'static str,
    ) -> ExplorerResult<MutexGuard<'_, Option<OwnedHandle>>> {
        self.thread.lock().map_err(|_| {
            ExplorerError::state_conflict(format!("{operation} 상태를 사용할 수 없습니다."))
        })
    }

    fn unregister_current_thread(&self) {
        if let Ok(mut thread) = self.thread.lock() {
            thread.take();
        }
    }
}

impl Default for SynchronousIoCancellation {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug)]
pub struct SynchronousIoCancellationRegistration<'a> {
    cancellation: &'a SynchronousIoCancellation,
}

impl Drop for SynchronousIoCancellationRegistration<'_> {
    fn drop(&mut self) {
        self.cancellation.unregister_current_thread();
    }
}

pub fn file_attributes(path: &Path) -> ExplorerResult<Win32FileAttributes> {
    let wide_path = path_to_file_api_wide_null(path);
    // SAFETY: wide_path is a null-terminated UTF-16 buffer that remains alive for the call.
    let raw_attributes = unsafe { GetFileAttributesW(wide_path.as_ptr()) };
    if raw_attributes == INVALID_FILE_ATTRIBUTES {
        return Err(ExplorerError::windows_api(
            "read file attributes",
            "GetFileAttributesW",
            last_error_code(),
            Some(path.to_path_buf()),
        ));
    }

    Ok(Win32FileAttributes::from_raw(raw_attributes))
}

pub fn visit_directory_entries<F>(path: &Path, visitor: F) -> ExplorerResult<()>
where
    F: FnMut(Win32DirectoryEntry) -> ExplorerResult<DirectoryVisit>,
{
    visit_directory_entries_until(path, || false, visitor)
}

pub fn visit_directory_entries_until<S, F>(
    path: &Path,
    mut should_stop: S,
    mut visitor: F,
) -> ExplorerResult<()>
where
    S: FnMut() -> bool,
    F: FnMut(Win32DirectoryEntry) -> ExplorerResult<DirectoryVisit>,
{
    if should_stop() {
        return Ok(());
    }

    let search_pattern = directory_search_pattern(path);
    let wide_pattern = path_to_file_api_wide_null(&search_pattern);

    let mut find_data = WIN32_FIND_DATAW::default();
    // SAFETY: wide_pattern is a null-terminated UTF-16 search pattern and find_data is writable.
    let mut handle = unsafe {
        FindFirstFileExW(
            wide_pattern.as_ptr(),
            FindExInfoBasic,
            (&mut find_data as *mut WIN32_FIND_DATAW).cast::<c_void>(),
            FindExSearchNameMatch,
            null(),
            FIND_FIRST_EX_LARGE_FETCH,
        )
    };
    let mut find_first_api = "FindFirstFileExW";
    let mut find_first_error = if handle == INVALID_HANDLE_VALUE {
        Some(last_error_code())
    } else {
        None
    };
    if find_first_error == Some(ERROR_INVALID_PARAMETER) {
        // SAFETY: wide_pattern is a null-terminated UTF-16 search pattern and find_data is writable.
        handle = unsafe { FindFirstFileW(wide_pattern.as_ptr(), &mut find_data) };
        find_first_api = "FindFirstFileW";
        find_first_error = if handle == INVALID_HANDLE_VALUE {
            Some(last_error_code())
        } else {
            None
        };
    }
    if handle == INVALID_HANDLE_VALUE {
        let code = find_first_error.unwrap_or_else(last_error_code);
        if code == ERROR_FILE_NOT_FOUND {
            return Ok(());
        }
        if code == ERROR_OPERATION_ABORTED && should_stop() {
            return Ok(());
        }

        return Err(ExplorerError::windows_api(
            "read directory",
            find_first_api,
            code,
            Some(path.to_path_buf()),
        ));
    }

    let handle = FindHandle(handle);

    loop {
        if should_stop() {
            return Ok(());
        }

        let entry = directory_entry_from_find_data(&find_data);
        if !is_current_or_parent_directory(&entry.file_name)
            && visitor(entry)? == DirectoryVisit::Stop
        {
            return Ok(());
        }

        if should_stop() {
            return Ok(());
        }

        // SAFETY: handle is a valid search handle and find_data is writable for the next entry.
        let succeeded = unsafe { FindNextFileW(handle.0, &mut find_data) };
        if succeeded != 0 {
            continue;
        }

        let code = last_error_code();
        if code == ERROR_NO_MORE_FILES {
            break;
        }
        if code == ERROR_OPERATION_ABORTED && should_stop() {
            return Ok(());
        }

        return Err(ExplorerError::windows_api(
            "read directory entry",
            "FindNextFileW",
            code,
            Some(path.to_path_buf()),
        ));
    }

    Ok(())
}

pub fn directory_entry(path: &Path) -> ExplorerResult<Option<Win32DirectoryEntry>> {
    let wide_path = path_to_file_api_wide_null(path);

    let mut find_data = WIN32_FIND_DATAW::default();
    // SAFETY: wide_path is a null-terminated UTF-16 path and find_data is writable.
    let handle = unsafe { FindFirstFileW(wide_path.as_ptr(), &mut find_data) };
    if handle == INVALID_HANDLE_VALUE {
        let code = last_error_code();
        if code == ERROR_FILE_NOT_FOUND || code == ERROR_PATH_NOT_FOUND {
            return Ok(None);
        }

        return Err(ExplorerError::windows_api(
            "read directory entry",
            "FindFirstFileW",
            code,
            Some(path.to_path_buf()),
        ));
    }

    let _handle = FindHandle(handle);
    let entry = directory_entry_from_find_data(&find_data);
    if is_current_or_parent_directory(&entry.file_name) {
        Ok(None)
    } else {
        Ok(Some(entry))
    }
}

pub fn directory_entries(path: &Path) -> ExplorerResult<Vec<Win32DirectoryEntry>> {
    let mut entries = Vec::new();
    visit_directory_entries(path, |entry| {
        entries.push(entry);
        Ok(DirectoryVisit::Continue)
    })?;
    Ok(entries)
}

pub fn ensure_directory_listable(path: &Path) -> ExplorerResult<()> {
    let wide_path = path_to_file_api_wide_null(path);
    let share_mode = FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE;

    // SAFETY: wide_path is a null-terminated UTF-16 directory path. A null security attributes
    // pointer requests the default descriptor, and the template handle is unused.
    let handle = unsafe {
        CreateFileW(
            wide_path.as_ptr(),
            FILE_LIST_DIRECTORY,
            share_mode,
            null(),
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS,
            null_mut(),
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        let code = last_error_code();
        return Err(ExplorerError::windows_api(
            "read directory",
            "CreateFileW",
            code,
            Some(path.to_path_buf()),
        ));
    }

    let _handle = OwnedHandle(handle);
    Ok(())
}

pub fn create_directory(path: &Path) -> ExplorerResult<()> {
    let wide_path = path_to_file_api_wide_null(path);
    // SAFETY: wide_path is a null-terminated UTF-16 buffer, and a null security attributes
    // pointer requests the default security descriptor.
    let succeeded = unsafe { CreateDirectoryW(wide_path.as_ptr(), null()) };
    if succeeded == 0 {
        return Err(ExplorerError::windows_api(
            "create folder",
            "CreateDirectoryW",
            last_error_code(),
            Some(path.to_path_buf()),
        ));
    }

    Ok(())
}

pub fn watch_directory_changes<F>(
    path: &Path,
    cancellation: &DirectoryChangeCancellation,
    mut on_changed: F,
) -> ExplorerResult<()>
where
    F: FnMut(DirectoryChangeBatch) -> ExplorerResult<()>,
{
    const DIRECTORY_CHANGE_BUFFER_SIZE: usize = 64 * 1024;
    const DIRECTORY_CHANGE_FILTER: u32 = FILE_NOTIFY_CHANGE_FILE_NAME
        | FILE_NOTIFY_CHANGE_DIR_NAME
        | FILE_NOTIFY_CHANGE_ATTRIBUTES
        | FILE_NOTIFY_CHANGE_SIZE
        | FILE_NOTIFY_CHANGE_LAST_WRITE
        | FILE_NOTIFY_CHANGE_CREATION
        | FILE_NOTIFY_CHANGE_SECURITY;

    let directory = open_directory_watch_handle(path)?;
    let change_event = OwnedHandle::manual_reset_event("create directory watch change event")?;
    let mut buffer = vec![0_u8; DIRECTORY_CHANGE_BUFFER_SIZE];

    loop {
        if cancellation.is_cancel_requested()? {
            return Ok(());
        }

        reset_event(
            change_event.raw(),
            "reset directory watch change event",
            path,
        )?;
        let mut overlapped = OVERLAPPED {
            hEvent: change_event.raw(),
            ..Default::default()
        };
        let mut bytes_returned = 0_u32;

        // SAFETY: directory is opened for overlapped directory notifications, buffer is writable
        // for the supplied length, and overlapped remains alive until the operation completes or is
        // cancelled and drained before the next loop iteration.
        let started = unsafe {
            ReadDirectoryChangesW(
                directory.raw(),
                buffer.as_mut_ptr().cast::<c_void>(),
                buffer.len() as u32,
                FALSE,
                DIRECTORY_CHANGE_FILTER,
                &mut bytes_returned,
                &mut overlapped,
                None,
            )
        };
        if started == 0 {
            let code = last_error_code();
            if cancellation.is_cancel_requested()?
                || code == ERROR_OPERATION_ABORTED
                || code == ERROR_IO_PENDING
            {
                if code == ERROR_IO_PENDING {
                    // The request has been queued and must be waited on before `overlapped` is
                    // reused or dropped.
                } else {
                    return Ok(());
                }
            } else {
                return Err(ExplorerError::windows_api(
                    "watch directory changes",
                    "ReadDirectoryChangesW",
                    code,
                    Some(path.to_path_buf()),
                ));
            }

            if code != ERROR_IO_PENDING {
                return Ok(());
            }
        }

        match wait_for_directory_watch_change(&change_event, cancellation, path)? {
            DirectoryWatchWait::Cancelled => {
                cancel_pending_directory_watch(path, &directory, &change_event, &mut overlapped)?;
                return Ok(());
            }
            DirectoryWatchWait::Changed => {
                let mut transferred = 0_u32;
                // SAFETY: the change event was signaled for this overlapped operation, so the
                // result can be read without waiting.
                let completed = unsafe {
                    GetOverlappedResult(directory.raw(), &overlapped, &mut transferred, FALSE)
                };
                if completed == 0 {
                    let code = last_error_code();
                    if cancellation.is_cancel_requested()? || code == ERROR_OPERATION_ABORTED {
                        return Ok(());
                    }
                    return Err(ExplorerError::windows_api(
                        "watch directory changes",
                        "GetOverlappedResult",
                        code,
                        Some(path.to_path_buf()),
                    ));
                }

                on_changed(directory_change_batch_from_buffer(&buffer, transferred))?;
            }
        }
    }
}

pub fn replace_file(source: &Path, destination: &Path) -> ExplorerResult<()> {
    let wide_source = path_to_file_api_wide_null(source);
    let wide_destination = path_to_file_api_wide_null(destination);
    let flags = MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH;

    // SAFETY: both paths are null-terminated UTF-16 buffers that remain alive for the call.
    let succeeded = unsafe { MoveFileExW(wide_source.as_ptr(), wide_destination.as_ptr(), flags) };
    if succeeded == 0 {
        return Err(ExplorerError::windows_api(
            "replace file",
            "MoveFileExW",
            last_error_code(),
            Some(destination.to_path_buf()),
        ));
    }

    Ok(())
}

pub fn logical_drive_roots() -> ExplorerResult<Vec<PathBuf>> {
    // SAFETY: passing a zero length and null buffer asks Windows for the required buffer length.
    let required_len = unsafe { GetLogicalDriveStringsW(0, null_mut()) };
    if required_len == 0 {
        return Err(ExplorerError::windows_api(
            "read drive roots",
            "GetLogicalDriveStringsW",
            last_error_code(),
            None,
        ));
    }

    let mut buffer = vec![0_u16; required_len as usize + 1];
    // SAFETY: buffer is writable and its length is passed exactly as the API expects.
    let copied_len = unsafe { GetLogicalDriveStringsW(buffer.len() as u32, buffer.as_mut_ptr()) };
    if copied_len == 0 {
        return Err(ExplorerError::windows_api(
            "read drive roots",
            "GetLogicalDriveStringsW",
            last_error_code(),
            None,
        ));
    }

    Ok(parse_double_null_paths(&buffer))
}

pub fn known_folder_path(folder: Win32KnownFolder) -> ExplorerResult<PathBuf> {
    let _com = ComApartment::initialize("read known folder path")?;
    let folder_id = known_folder_id(folder);
    let mut raw_path = null_mut();

    // SAFETY: folder_id points to a valid known-folder GUID and raw_path is an out pointer that
    // Shell initializes with CoTaskMemAlloc on success.
    let hresult = unsafe {
        SHGetKnownFolderPath(
            &folder_id,
            KF_FLAG_DEFAULT as u32,
            null_mut(),
            &mut raw_path,
        )
    };
    if hresult < 0 {
        return Err(ExplorerError::windows_hresult(
            "read known folder path",
            "SHGetKnownFolderPath",
            hresult,
            None,
        ));
    }

    if raw_path.is_null() {
        return Err(ExplorerError::windows_api(
            "read known folder path",
            "SHGetKnownFolderPath",
            0,
            None,
        ));
    }

    let path = CoTaskMemWideString(raw_path).to_path_buf();
    if path.as_os_str().is_empty() {
        return Err(ExplorerError::invalid_input(
            "Windows 기본 폴더 경로가 비어 있습니다.",
        ));
    }

    Ok(path)
}

fn has_attribute(raw_attributes: u32, attribute: u32) -> bool {
    raw_attributes & attribute != 0
}

struct FindHandle(HANDLE);

impl Drop for FindHandle {
    fn drop(&mut self) {
        // SAFETY: FindHandle is only constructed from a successful Win32 find handle.
        unsafe {
            FindClose(self.0);
        }
    }
}

#[derive(Debug)]
struct OwnedHandle(HANDLE);

impl OwnedHandle {
    fn manual_reset_event(operation: &'static str) -> ExplorerResult<Self> {
        // SAFETY: null security attributes and null name request an unnamed event with the
        // default security descriptor.
        let handle = unsafe { CreateEventW(null(), TRUE, FALSE, null()) };
        if handle.is_null() {
            return Err(ExplorerError::windows_api(
                operation,
                "CreateEventW",
                last_error_code(),
                None,
            ));
        }

        Ok(Self(handle))
    }

    fn raw(&self) -> HANDLE {
        self.0
    }
}

impl Drop for OwnedHandle {
    fn drop(&mut self) {
        if !self.0.is_null() && self.0 != INVALID_HANDLE_VALUE {
            // SAFETY: OwnedHandle only wraps handles returned by CreateFileW/CreateEventW/DuplicateHandle.
            unsafe {
                CloseHandle(self.0);
            }
        }
    }
}

fn duplicate_current_thread_handle() -> ExplorerResult<OwnedHandle> {
    let mut thread_handle = null_mut();
    // SAFETY: current process/thread pseudo-handles are valid for DuplicateHandle and
    // thread_handle points to writable storage for the duplicated handle.
    let succeeded = unsafe {
        DuplicateHandle(
            GetCurrentProcess(),
            GetCurrentThread(),
            GetCurrentProcess(),
            &mut thread_handle,
            0,
            FALSE,
            DUPLICATE_SAME_ACCESS,
        )
    };
    if succeeded == 0 {
        return Err(ExplorerError::windows_api(
            "duplicate current thread handle",
            "DuplicateHandle",
            last_error_code(),
            None,
        ));
    }

    Ok(OwnedHandle(thread_handle))
}

enum DirectoryWatchWait {
    Changed,
    Cancelled,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DirectoryWatchCancelOutcome {
    CancelRequested,
    AlreadyAborted,
    NoPendingIo,
}

fn open_directory_watch_handle(path: &Path) -> ExplorerResult<OwnedHandle> {
    let wide_path = path_to_file_api_wide_null(path);
    let share_mode = FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE;
    let flags = FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OVERLAPPED;

    // SAFETY: wide_path is a null-terminated UTF-16 directory path. A null security attributes
    // pointer requests the default descriptor, and the template handle is unused.
    let handle = unsafe {
        CreateFileW(
            wide_path.as_ptr(),
            FILE_LIST_DIRECTORY,
            share_mode,
            null(),
            OPEN_EXISTING,
            flags,
            null_mut(),
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        return Err(ExplorerError::windows_api(
            "open directory watch",
            "CreateFileW",
            last_error_code(),
            Some(path.to_path_buf()),
        ));
    }

    Ok(OwnedHandle(handle))
}

fn reset_event(handle: HANDLE, operation: &'static str, path: &Path) -> ExplorerResult<()> {
    // SAFETY: handle is a valid manual-reset event handle.
    let succeeded = unsafe { ResetEvent(handle) };
    if succeeded == 0 {
        return Err(ExplorerError::windows_api(
            operation,
            "ResetEvent",
            last_error_code(),
            Some(path.to_path_buf()),
        ));
    }

    Ok(())
}

fn wait_for_directory_watch_change(
    change_event: &OwnedHandle,
    cancellation: &DirectoryChangeCancellation,
    path: &Path,
) -> ExplorerResult<DirectoryWatchWait> {
    let handles = [change_event.raw(), cancellation.raw()];
    // SAFETY: both handles are valid waitable event handles for the duration of the call.
    let result =
        unsafe { WaitForMultipleObjects(handles.len() as u32, handles.as_ptr(), FALSE, INFINITE) };

    if result == WAIT_OBJECT_0 {
        return Ok(DirectoryWatchWait::Changed);
    }
    if result == WAIT_OBJECT_0 + 1 {
        return Ok(DirectoryWatchWait::Cancelled);
    }
    if result == WAIT_FAILED {
        return Err(ExplorerError::windows_api(
            "wait for directory changes",
            "WaitForMultipleObjects",
            last_error_code(),
            Some(path.to_path_buf()),
        ));
    }

    Err(ExplorerError::state_conflict(
        "알 수 없는 디렉터리 변경 감시 대기 결과입니다.",
    ))
}

fn wait_for_event_signal(
    handle: HANDLE,
    timeout_ms: u32,
    operation: &'static str,
    api: &'static str,
    path: Option<PathBuf>,
) -> ExplorerResult<bool> {
    // SAFETY: handle is a valid waitable handle for the duration of the call.
    let result = unsafe { WaitForSingleObject(handle, timeout_ms) };
    if result == WAIT_OBJECT_0 {
        return Ok(true);
    }
    if result == WAIT_TIMEOUT {
        return Ok(false);
    }
    if result == WAIT_FAILED {
        return Err(ExplorerError::windows_api(
            operation,
            api,
            last_error_code(),
            path,
        ));
    }

    Err(ExplorerError::state_conflict(
        "알 수 없는 이벤트 대기 결과입니다.",
    ))
}

fn cancel_pending_directory_watch(
    path: &Path,
    directory: &OwnedHandle,
    change_event: &OwnedHandle,
    overlapped: &mut OVERLAPPED,
) -> ExplorerResult<()> {
    // SAFETY: overlapped belongs to the pending ReadDirectoryChangesW issued on directory.
    let cancelled = unsafe { CancelIoEx(directory.raw(), overlapped) };
    let cancel_outcome = if cancelled != 0 {
        DirectoryWatchCancelOutcome::CancelRequested
    } else {
        let code = last_error_code();
        directory_watch_cancel_outcome(code).map_err(|code| {
            ExplorerError::windows_api(
                "cancel directory watch",
                "CancelIoEx",
                code,
                Some(path.to_path_buf()),
            )
        })?
    };

    if cancel_outcome == DirectoryWatchCancelOutcome::NoPendingIo {
        return Ok(());
    }
    if cancel_outcome == DirectoryWatchCancelOutcome::AlreadyAborted
        && try_finish_cancelled_directory_watch(path, directory, overlapped)?
    {
        return Ok(());
    }

    let _ = wait_for_event_signal(
        change_event.raw(),
        INFINITE,
        "wait for cancelled directory watch",
        "WaitForSingleObject",
        Some(path.to_path_buf()),
    )?;

    if !try_finish_cancelled_directory_watch(path, directory, overlapped)? {
        return Err(ExplorerError::state_conflict(
            "취소된 디렉터리 변경 감시 작업이 완료되지 않았습니다.",
        ));
    }

    Ok(())
}

fn try_finish_cancelled_directory_watch(
    path: &Path,
    directory: &OwnedHandle,
    overlapped: &mut OVERLAPPED,
) -> ExplorerResult<bool> {
    let mut transferred = 0_u32;
    // SAFETY: overlapped belongs to the ReadDirectoryChangesW issued on directory and is only
    // inspected here to determine whether that operation has already completed.
    let completed =
        unsafe { GetOverlappedResult(directory.raw(), overlapped, &mut transferred, FALSE) };
    if completed == 0 {
        let code = last_error_code();
        if code == ERROR_IO_INCOMPLETE {
            return Ok(false);
        }
        if code != ERROR_OPERATION_ABORTED {
            return Err(ExplorerError::windows_api(
                "cancel directory watch",
                "GetOverlappedResult",
                code,
                Some(path.to_path_buf()),
            ));
        }
    }

    Ok(true)
}

fn directory_watch_cancel_outcome(error_code: u32) -> Result<DirectoryWatchCancelOutcome, u32> {
    match error_code {
        ERROR_NOT_FOUND => Ok(DirectoryWatchCancelOutcome::NoPendingIo),
        ERROR_OPERATION_ABORTED => Ok(DirectoryWatchCancelOutcome::AlreadyAborted),
        code => Err(code),
    }
}

struct ComApartment {
    should_uninitialize: bool,
}

impl ComApartment {
    fn initialize(operation: &'static str) -> ExplorerResult<Self> {
        let coinit = (COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE) as u32;
        // SAFETY: CoInitializeEx accepts a null reserved pointer and initializes COM for the
        // current thread. The matching CoUninitialize call is guarded by ComApartment::drop.
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

impl Drop for ComApartment {
    fn drop(&mut self) {
        if self.should_uninitialize {
            // SAFETY: this balances a successful CoInitializeEx call on the current thread.
            unsafe {
                CoUninitialize();
            }
        }
    }
}

struct CoTaskMemWideString(*mut u16);

impl CoTaskMemWideString {
    fn to_path_buf(&self) -> PathBuf {
        // SAFETY: SHGetKnownFolderPath returns a null-terminated UTF-16 string on success.
        let value = unsafe { os_string_from_pwstr(self.0) };
        PathBuf::from(value)
    }
}

impl Drop for CoTaskMemWideString {
    fn drop(&mut self) {
        // SAFETY: the pointer was allocated by SHGetKnownFolderPath and must be freed with
        // CoTaskMemFree exactly once.
        unsafe {
            CoTaskMemFree(self.0.cast());
        }
    }
}

fn known_folder_id(folder: Win32KnownFolder) -> windows_sys::core::GUID {
    match folder {
        Win32KnownFolder::Desktop => FOLDERID_Desktop,
        Win32KnownFolder::Downloads => FOLDERID_Downloads,
        Win32KnownFolder::Documents => FOLDERID_Documents,
        Win32KnownFolder::Profile => FOLDERID_Profile,
    }
}

fn directory_entry_from_find_data(find_data: &WIN32_FIND_DATAW) -> Win32DirectoryEntry {
    Win32DirectoryEntry {
        file_name: os_string_from_null_terminated_wide(&find_data.cFileName),
        attributes: Win32FileAttributes::from_raw(find_data.dwFileAttributes),
        file_size: file_size_from_find_data(find_data),
        last_write_time: filetime_to_system_time(find_data.ftLastWriteTime),
    }
}

fn directory_search_pattern(path: &Path) -> PathBuf {
    let mut pattern = PathBuf::from(path);
    pattern.push("*");
    pattern
}

fn directory_change_batch_from_buffer(buffer: &[u8], transferred: u32) -> DirectoryChangeBatch {
    const FILE_NOTIFY_INFORMATION_HEADER_SIZE: usize = 12;

    let Ok(transferred_len) = usize::try_from(transferred) else {
        return DirectoryChangeBatch::overflowed();
    };
    if transferred_len == 0 || transferred_len > buffer.len() {
        return DirectoryChangeBatch::overflowed();
    }

    let buffer = &buffer[..transferred_len];
    let mut changes = Vec::new();
    let mut offset = 0_usize;
    loop {
        let Some(next_entry_offset) = read_u32_le(buffer, offset) else {
            return DirectoryChangeBatch::overflowed();
        };
        let Some(action) = read_u32_le(buffer, offset + 4) else {
            return DirectoryChangeBatch::overflowed();
        };
        let Some(file_name_length) = read_u32_le(buffer, offset + 8) else {
            return DirectoryChangeBatch::overflowed();
        };
        let Ok(file_name_length) = usize::try_from(file_name_length) else {
            return DirectoryChangeBatch::overflowed();
        };
        if file_name_length % 2 != 0 {
            return DirectoryChangeBatch::overflowed();
        }

        let name_start = offset + FILE_NOTIFY_INFORMATION_HEADER_SIZE;
        let Some(name_end) = name_start.checked_add(file_name_length) else {
            return DirectoryChangeBatch::overflowed();
        };
        let Some(name_bytes) = buffer.get(name_start..name_end) else {
            return DirectoryChangeBatch::overflowed();
        };
        let file_name = os_string_from_wide_bytes(name_bytes);
        if !file_name.is_empty() {
            changes.push(DirectoryChange {
                file_name,
                kind: directory_change_kind_from_action(action),
            });
        }

        if next_entry_offset == 0 {
            break;
        }

        let Ok(next_entry_offset) = usize::try_from(next_entry_offset) else {
            return DirectoryChangeBatch::overflowed();
        };
        if next_entry_offset < FILE_NOTIFY_INFORMATION_HEADER_SIZE {
            return DirectoryChangeBatch::overflowed();
        }
        let Some(next_offset) = offset.checked_add(next_entry_offset) else {
            return DirectoryChangeBatch::overflowed();
        };
        if next_offset >= transferred_len {
            return DirectoryChangeBatch::overflowed();
        }
        offset = next_offset;
    }

    DirectoryChangeBatch::precise(changes)
}

fn read_u32_le(buffer: &[u8], offset: usize) -> Option<u32> {
    let bytes = buffer.get(offset..offset + 4)?;
    let bytes: [u8; 4] = bytes.try_into().ok()?;
    Some(u32::from_le_bytes(bytes))
}

fn os_string_from_wide_bytes(bytes: &[u8]) -> OsString {
    let mut wide = Vec::with_capacity(bytes.len() / 2);
    for unit in bytes.chunks_exact(2) {
        wide.push(u16::from_le_bytes([unit[0], unit[1]]));
    }
    OsString::from_wide(&wide)
}

fn directory_change_kind_from_action(action: u32) -> DirectoryChangeKind {
    const FILE_ACTION_ADDED_VALUE: u32 = 0x0000_0001;
    const FILE_ACTION_REMOVED_VALUE: u32 = 0x0000_0002;
    const FILE_ACTION_MODIFIED_VALUE: u32 = 0x0000_0003;
    const FILE_ACTION_RENAMED_OLD_NAME_VALUE: u32 = 0x0000_0004;
    const FILE_ACTION_RENAMED_NEW_NAME_VALUE: u32 = 0x0000_0005;

    match action {
        FILE_ACTION_ADDED_VALUE => DirectoryChangeKind::Added,
        FILE_ACTION_REMOVED_VALUE => DirectoryChangeKind::Removed,
        FILE_ACTION_MODIFIED_VALUE => DirectoryChangeKind::Modified,
        FILE_ACTION_RENAMED_OLD_NAME_VALUE => DirectoryChangeKind::RenamedOldName,
        FILE_ACTION_RENAMED_NEW_NAME_VALUE => DirectoryChangeKind::RenamedNewName,
        _ => DirectoryChangeKind::Other,
    }
}

fn file_size_from_find_data(find_data: &WIN32_FIND_DATAW) -> u64 {
    ((find_data.nFileSizeHigh as u64) << 32) | find_data.nFileSizeLow as u64
}

fn filetime_to_system_time(filetime: FILETIME) -> Option<SystemTime> {
    const UNIX_EPOCH_FILETIME_TICKS: u64 = 116_444_736_000_000_000;

    let ticks = ((filetime.dwHighDateTime as u64) << 32) | filetime.dwLowDateTime as u64;
    if ticks == 0 {
        return None;
    }

    if ticks >= UNIX_EPOCH_FILETIME_TICKS {
        let delta_ticks = ticks - UNIX_EPOCH_FILETIME_TICKS;
        SystemTime::UNIX_EPOCH.checked_add(duration_from_filetime_ticks(delta_ticks))
    } else {
        let delta_ticks = UNIX_EPOCH_FILETIME_TICKS - ticks;
        SystemTime::UNIX_EPOCH.checked_sub(duration_from_filetime_ticks(delta_ticks))
    }
}

fn duration_from_filetime_ticks(ticks: u64) -> Duration {
    const WINDOWS_TICKS_PER_SECOND: u64 = 10_000_000;
    const WINDOWS_TICK_NANOS: u32 = 100;

    Duration::new(
        ticks / WINDOWS_TICKS_PER_SECOND,
        ((ticks % WINDOWS_TICKS_PER_SECOND) as u32) * WINDOWS_TICK_NANOS,
    )
}

fn os_string_from_null_terminated_wide(buffer: &[u16]) -> OsString {
    let len = buffer
        .iter()
        .position(|value| *value == 0)
        .unwrap_or(buffer.len());
    OsString::from_wide(&buffer[..len])
}

unsafe fn os_string_from_pwstr(ptr: *const u16) -> OsString {
    let mut len = 0;
    while unsafe { *ptr.add(len) } != 0 {
        len += 1;
    }

    // SAFETY: len was computed by scanning the null-terminated string.
    OsString::from_wide(unsafe { std::slice::from_raw_parts(ptr, len) })
}

fn is_current_or_parent_directory(file_name: &OsStr) -> bool {
    file_name == OsStr::new(".") || file_name == OsStr::new("..")
}

fn path_to_file_api_wide_null(path: &Path) -> Vec<u16> {
    let mut raw_path = path.as_os_str().encode_wide().collect::<Vec<_>>();
    normalize_windows_separators(&mut raw_path);
    append_null(extended_length_path(raw_path))
}

fn append_null(mut value: Vec<u16>) -> Vec<u16> {
    value.push(0);
    value
}

fn normalize_windows_separators(value: &mut [u16]) {
    for unit in value {
        if *unit == b'/' as u16 {
            *unit = b'\\' as u16;
        }
    }
}

fn extended_length_path(raw_path: Vec<u16>) -> Vec<u16> {
    const EXTENDED_PREFIX: &[u16] = &[b'\\' as u16, b'\\' as u16, b'?' as u16, b'\\' as u16];
    const EXTENDED_UNC_PREFIX: &[u16] = &[
        b'\\' as u16,
        b'\\' as u16,
        b'?' as u16,
        b'\\' as u16,
        b'U' as u16,
        b'N' as u16,
        b'C' as u16,
        b'\\' as u16,
    ];

    if is_extended_or_device_path(&raw_path) {
        return raw_path;
    }

    if is_unc_path(&raw_path) {
        let mut value = Vec::with_capacity(EXTENDED_UNC_PREFIX.len() + raw_path.len() - 2);
        value.extend_from_slice(EXTENDED_UNC_PREFIX);
        value.extend_from_slice(&raw_path[2..]);
        return value;
    }

    if is_drive_absolute_path(&raw_path) {
        let mut value = Vec::with_capacity(EXTENDED_PREFIX.len() + raw_path.len());
        value.extend_from_slice(EXTENDED_PREFIX);
        value.extend_from_slice(&raw_path);
        return value;
    }

    raw_path
}

fn is_extended_or_device_path(value: &[u16]) -> bool {
    const EXTENDED_PREFIX: &[u16] = &[b'\\' as u16, b'\\' as u16, b'?' as u16, b'\\' as u16];
    const DEVICE_PREFIX: &[u16] = &[b'\\' as u16, b'\\' as u16, b'.' as u16, b'\\' as u16];
    starts_with_wide(value, EXTENDED_PREFIX) || starts_with_wide(value, DEVICE_PREFIX)
}

fn is_unc_path(value: &[u16]) -> bool {
    value.len() >= 2 && value[0] == b'\\' as u16 && value[1] == b'\\' as u16
}

fn is_drive_absolute_path(value: &[u16]) -> bool {
    value.len() >= 3
        && is_ascii_alpha(value[0])
        && value[1] == b':' as u16
        && value[2] == b'\\' as u16
}

fn is_ascii_alpha(value: u16) -> bool {
    (value >= b'A' as u16 && value <= b'Z' as u16) || (value >= b'a' as u16 && value <= b'z' as u16)
}

fn starts_with_wide(value: &[u16], prefix: &[u16]) -> bool {
    value.len() >= prefix.len() && &value[..prefix.len()] == prefix
}

fn parse_double_null_paths(buffer: &[u16]) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    let mut start = 0;

    for (index, value) in buffer.iter().enumerate() {
        if *value != 0 {
            continue;
        }

        if index == start {
            break;
        }

        let os_string = OsString::from_wide(&buffer[start..index]);
        paths.push(PathBuf::from(os_string));
        start = index + 1;
    }

    paths
}

fn last_error_code() -> u32 {
    // SAFETY: GetLastError reads thread-local Windows error state and has no preconditions.
    unsafe { GetLastError() }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::{mpsc, Arc};
    use std::thread;
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    #[test]
    fn converts_raw_file_attributes() {
        let attributes = Win32FileAttributes::from_raw(
            FILE_ATTRIBUTE_HIDDEN
                | FILE_ATTRIBUTE_SYSTEM
                | FILE_ATTRIBUTE_READONLY
                | FILE_ATTRIBUTE_DIRECTORY
                | FILE_ATTRIBUTE_REPARSE_POINT,
        );

        assert!(attributes.hidden);
        assert!(attributes.system);
        assert!(attributes.read_only);
        assert!(attributes.directory);
        assert!(attributes.reparse_point);
    }

    #[test]
    fn extracts_null_terminated_wide_file_name() {
        let mut buffer = [0_u16; 260];
        let value = OsStr::new("한글.txt").encode_wide().collect::<Vec<_>>();
        buffer[..value.len()].copy_from_slice(&value);

        assert_eq!(
            os_string_from_null_terminated_wide(&buffer),
            OsString::from("한글.txt")
        );
    }

    #[test]
    fn converts_unix_epoch_filetime() {
        const UNIX_EPOCH_FILETIME_TICKS: u64 = 116_444_736_000_000_000;
        let filetime = FILETIME {
            dwLowDateTime: UNIX_EPOCH_FILETIME_TICKS as u32,
            dwHighDateTime: (UNIX_EPOCH_FILETIME_TICKS >> 32) as u32,
        };

        assert_eq!(filetime_to_system_time(filetime), Some(UNIX_EPOCH));
    }

    #[test]
    fn parses_directory_change_buffer_records() {
        let buffer =
            directory_change_test_buffer(&[(1, "created.txt", true), (3, "updated.txt", false)]);
        let batch = directory_change_batch_from_buffer(&buffer, buffer.len() as u32);

        assert!(!batch.overflowed);
        assert_eq!(batch.changes.len(), 2);
        assert_eq!(batch.changes[0].file_name, OsString::from("created.txt"));
        assert_eq!(batch.changes[0].kind, DirectoryChangeKind::Added);
        assert_eq!(batch.changes[1].file_name, OsString::from("updated.txt"));
        assert_eq!(batch.changes[1].kind, DirectoryChangeKind::Modified);
    }

    #[test]
    fn malformed_directory_change_buffer_requires_overflow_refresh() {
        let mut buffer = Vec::new();
        buffer.extend_from_slice(&16_u32.to_le_bytes());
        buffer.extend_from_slice(&1_u32.to_le_bytes());
        buffer.extend_from_slice(&2_u32.to_le_bytes());
        buffer.extend_from_slice(&[b'a', 0]);

        let batch = directory_change_batch_from_buffer(&buffer, buffer.len() as u32);

        assert!(batch.overflowed);
        assert!(batch.changes.is_empty());
    }

    #[test]
    fn prefixes_absolute_drive_paths_for_file_apis() {
        let wide = path_to_file_api_wide_null(Path::new("C:\\Temp"));
        let prefix = [b'\\' as u16, b'\\' as u16, b'?' as u16, b'\\' as u16];

        assert!(starts_with_wide(&wide, &prefix));
        assert_eq!(wide.last().copied(), Some(0));
    }

    #[test]
    fn ensure_directory_listable_reports_deleted_directory(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let temp_dir = TempDirectory::new()?;
        let deleted_path = temp_dir.path.join("deleted");
        fs::create_dir(&deleted_path)?;
        fs::remove_dir(&deleted_path)?;

        assert!(
            ensure_directory_listable(&deleted_path).is_err(),
            "expected deleted directory to be reported as inaccessible"
        );

        Ok(())
    }

    #[test]
    fn synchronous_io_cancellation_allows_cancel_without_pending_io(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let cancellation = SynchronousIoCancellation::new();
        let _registration = cancellation.register_current_thread()?;

        cancellation.request_cancel()?;
        Ok(())
    }

    #[test]
    fn directory_watch_cancel_outcome_separates_cancel_races() {
        assert_eq!(
            directory_watch_cancel_outcome(ERROR_NOT_FOUND),
            Ok(DirectoryWatchCancelOutcome::NoPendingIo)
        );
        assert_eq!(
            directory_watch_cancel_outcome(ERROR_OPERATION_ABORTED),
            Ok(DirectoryWatchCancelOutcome::AlreadyAborted)
        );
    }

    #[test]
    fn directory_watch_reports_file_creation_and_cancels() -> Result<(), Box<dyn std::error::Error>>
    {
        let temp_dir = TempDirectory::new()?;
        let cancellation = Arc::new(DirectoryChangeCancellation::new()?);
        let worker_cancellation = Arc::clone(&cancellation);
        let watch_path = temp_dir.path.clone();
        let (changed_tx, changed_rx) = mpsc::channel();

        let handle = thread::spawn(move || {
            watch_directory_changes(&watch_path, &worker_cancellation, |changes| {
                let _ = changed_tx.send(changes);
                Ok(())
            })
        });

        let mut observed = false;
        for index in 0..20 {
            fs::write(
                temp_dir.path.join(format!("created-{index}.txt")),
                b"changed",
            )?;
            if let Ok(changes) = changed_rx.recv_timeout(Duration::from_millis(250)) {
                observed = changes.overflowed || !changes.changes.is_empty();
                break;
            }
        }

        cancellation.request_cancel()?;
        handle
            .join()
            .map_err(|_| ExplorerError::state_conflict("directory watch test worker panicked"))??;

        assert!(observed);
        Ok(())
    }

    fn directory_change_test_buffer(records: &[(u32, &str, bool)]) -> Vec<u8> {
        let mut buffer = Vec::new();
        for (action, name, has_next) in records {
            let name = OsStr::new(name).encode_wide().collect::<Vec<_>>();
            let record_len = 12 + name.len() * 2;
            let next_offset = if *has_next {
                align_to_u32(record_len)
            } else {
                0
            };

            buffer.extend_from_slice(&(next_offset as u32).to_le_bytes());
            buffer.extend_from_slice(&action.to_le_bytes());
            buffer.extend_from_slice(&((name.len() * 2) as u32).to_le_bytes());
            for unit in name {
                buffer.extend_from_slice(&unit.to_le_bytes());
            }
            if *has_next {
                buffer.resize(buffer.len() + next_offset - record_len, 0);
            }
        }
        buffer
    }

    fn align_to_u32(value: usize) -> usize {
        (value + 3) & !3
    }

    struct TempDirectory {
        path: PathBuf,
    }

    impl TempDirectory {
        fn new() -> Result<Self, Box<dyn std::error::Error>> {
            let unique = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
            let path = std::env::temp_dir().join(format!(
                "j3files-directory-watch-test-{}-{unique}",
                std::process::id()
            ));
            fs::create_dir(&path)?;
            Ok(Self { path })
        }
    }

    impl Drop for TempDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }
}
