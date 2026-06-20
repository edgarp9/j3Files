use std::borrow::Cow;
use std::ffi::{c_void, OsStr};
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::ptr::{null, null_mut, NonNull};

use windows_sys::core::{BOOL, GUID, HRESULT, PCWSTR};
use windows_sys::Win32::Foundation::{
    ERROR_ACCESS_DENIED, ERROR_ALREADY_EXISTS, ERROR_CANCELLED, ERROR_ELEVATION_REQUIRED,
    ERROR_FILENAME_EXCED_RANGE, ERROR_FILE_NOT_FOUND, ERROR_PATH_NOT_FOUND,
    ERROR_SHARING_VIOLATION, HWND, RPC_E_CHANGED_MODE,
};
use windows_sys::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_ALL, COINIT_APARTMENTTHREADED,
    COINIT_DISABLE_OLE1DDE,
};
use windows_sys::Win32::UI::Shell::{
    FileOperation, SHCreateItemFromParsingName, COPYENGINE_E_ACCESSDENIED_READONLY,
    COPYENGINE_E_ACCESS_DENIED_DEST, COPYENGINE_E_ACCESS_DENIED_SRC,
    COPYENGINE_E_ALREADY_EXISTS_FOLDER, COPYENGINE_E_ALREADY_EXISTS_NORMAL,
    COPYENGINE_E_ALREADY_EXISTS_READONLY, COPYENGINE_E_ALREADY_EXISTS_SYSTEM,
    COPYENGINE_E_CANCELLED, COPYENGINE_E_CANT_REACH_SOURCE, COPYENGINE_E_NEWFILE_NAME_TOO_LONG,
    COPYENGINE_E_NEWFOLDER_NAME_TOO_LONG, COPYENGINE_E_PATH_NOT_FOUND_DEST,
    COPYENGINE_E_PATH_NOT_FOUND_SRC, COPYENGINE_E_REQUIRES_ELEVATION,
    COPYENGINE_E_SHARING_VIOLATION_DEST, COPYENGINE_E_SHARING_VIOLATION_SRC,
    COPYENGINE_E_USER_CANCELLED, FOFX_RECYCLEONDELETE, FOFX_SHOWELEVATIONPROMPT, FOF_ALLOWUNDO,
};

use crate::domain::{
    ExplorerError, ExplorerResult, FileNameErrorKind, RenameItemName, ShellOperation,
};

const IID_ISHELL_ITEM: GUID = GUID::from_u128(0x43826d1e_e718_42ee_bc55_a1e261c37bfe);
const IID_IFILE_OPERATION: GUID = GUID::from_u128(0x947aab5f_0a5c_4c13_b4d6_4bf7836fc9f8);

pub fn shell_copy_items(sources: &[PathBuf], destination: &Path) -> ExplorerResult<()> {
    shell_copy_items_with_owner(0, sources, destination)
}

pub fn shell_copy_items_with_owner<'a>(
    owner_window: isize,
    sources: impl Into<Cow<'a, [PathBuf]>>,
    destination: &Path,
) -> ExplorerResult<()> {
    let operation = ShellOperation::Copy;
    let sources = sources.into();
    ensure_non_empty(sources.as_ref(), operation)?;
    let source_count = sources.len();
    let targets = targets_with_destination(sources.into_owned(), destination);
    let file_operation = ShellFileOperation::new(operation, &targets, owner_window)?;
    file_operation.set_operation_flags(operation_flags(operation))?;

    let destination_item = shell_item_from_path(destination, operation, &targets)?;
    let source_items = shell_items_from_paths(&targets[..source_count], operation, &targets)?;
    for source_item in &source_items {
        file_operation.copy_item(source_item, &destination_item)?;
    }

    file_operation.perform()
}

pub fn shell_move_items(sources: &[PathBuf], destination: &Path) -> ExplorerResult<()> {
    shell_move_items_with_owner(0, sources, destination)
}

pub fn shell_move_items_with_owner<'a>(
    owner_window: isize,
    sources: impl Into<Cow<'a, [PathBuf]>>,
    destination: &Path,
) -> ExplorerResult<()> {
    let operation = ShellOperation::Move;
    let sources = sources.into();
    ensure_non_empty(sources.as_ref(), operation)?;
    let source_count = sources.len();
    let targets = targets_with_destination(sources.into_owned(), destination);
    let file_operation = ShellFileOperation::new(operation, &targets, owner_window)?;
    file_operation.set_operation_flags(operation_flags(operation))?;

    let destination_item = shell_item_from_path(destination, operation, &targets)?;
    let source_items = shell_items_from_paths(&targets[..source_count], operation, &targets)?;
    for source_item in &source_items {
        file_operation.move_item(source_item, &destination_item)?;
    }

    file_operation.perform()
}

pub fn shell_delete_to_recycle_bin(targets: &[PathBuf]) -> ExplorerResult<()> {
    shell_delete_to_recycle_bin_with_owner(0, targets)
}

pub fn shell_delete_to_recycle_bin_with_owner<'a>(
    owner_window: isize,
    targets: impl Into<Cow<'a, [PathBuf]>>,
) -> ExplorerResult<()> {
    let targets = targets.into();
    ensure_non_empty(targets.as_ref(), ShellOperation::DeleteToRecycleBin)?;
    delete_items(owner_window, targets, ShellOperation::DeleteToRecycleBin)
}

pub fn shell_delete_permanently(targets: &[PathBuf]) -> ExplorerResult<()> {
    shell_delete_permanently_with_owner(0, targets)
}

pub fn shell_delete_permanently_with_owner<'a>(
    owner_window: isize,
    targets: impl Into<Cow<'a, [PathBuf]>>,
) -> ExplorerResult<()> {
    let targets = targets.into();
    ensure_non_empty(targets.as_ref(), ShellOperation::DeletePermanently)?;
    delete_items(owner_window, targets, ShellOperation::DeletePermanently)
}

fn delete_items(
    owner_window: isize,
    targets: Cow<'_, [PathBuf]>,
    operation: ShellOperation,
) -> ExplorerResult<()> {
    let file_operation = ShellFileOperation::new(operation, targets.as_ref(), owner_window)?;
    file_operation.set_operation_flags(operation_flags(operation))?;

    let target_items = shell_items_from_paths(targets.as_ref(), operation, targets.as_ref())?;
    for target_item in &target_items {
        file_operation.delete_item(target_item)?;
    }

    file_operation.perform()
}

pub fn shell_rename_item(target: &Path, new_name: &OsStr) -> ExplorerResult<()> {
    shell_rename_item_with_owner(0, target, new_name)
}

pub fn shell_rename_item_with_owner(
    owner_window: isize,
    target: &Path,
    new_name: &OsStr,
) -> ExplorerResult<()> {
    let new_name = RenameItemName::new(new_name)?;

    let operation = ShellOperation::Rename;
    let targets = vec![target.to_path_buf()];
    let file_operation = ShellFileOperation::new(operation, &targets, owner_window)?;
    file_operation.set_operation_flags(operation_flags(operation))?;

    let target_item = shell_item_from_path(target, operation, &targets)?;
    let new_name_wide = os_str_to_wide_null(new_name.as_os_str())?;
    file_operation.rename_item(&target_item, new_name_wide.as_ptr())?;
    file_operation.perform()
}

struct ShellFileOperation<'a> {
    ptr: ComPtr,
    _apartment: ComApartment,
    operation: ShellOperation,
    targets: &'a [PathBuf],
}

impl<'a> ShellFileOperation<'a> {
    fn new(
        operation: ShellOperation,
        targets: &'a [PathBuf],
        owner_window: isize,
    ) -> ExplorerResult<Self> {
        let apartment = ComApartment::initialize(operation, targets)?;
        let mut raw_operation = null_mut();
        // SAFETY: FileOperation and IID_IFILE_OPERATION are valid COM identifiers, and
        // raw_operation is a writable out pointer for the created COM interface.
        let hresult = unsafe {
            CoCreateInstance(
                &FileOperation,
                null_mut(),
                CLSCTX_ALL,
                &IID_IFILE_OPERATION,
                &mut raw_operation,
            )
        };
        check_hresult(
            operation,
            "CoCreateInstance(FileOperation)",
            hresult,
            targets,
        )?;

        let file_operation = Self {
            ptr: ComPtr::from_raw(
                raw_operation,
                operation,
                "CoCreateInstance(FileOperation)",
                targets,
            )?,
            _apartment: apartment,
            operation,
            targets,
        };
        file_operation.set_owner_window(owner_window)?;
        Ok(file_operation)
    }

    fn set_operation_flags(&self, flags: u32) -> ExplorerResult<()> {
        let vtable = self.ptr.vtable::<IFileOperationVtbl>();
        // SAFETY: self.ptr is an IFileOperation pointer created by CoCreateInstance, and the
        // vtable entry is called with the same interface pointer.
        let hresult = unsafe { (vtable.set_operation_flags)(self.ptr.as_raw(), flags) };
        self.check("IFileOperation::SetOperationFlags", hresult)
    }

    fn set_owner_window(&self, owner_window: isize) -> ExplorerResult<()> {
        let Some(hwnd) = owner_window_from_isize(owner_window) else {
            return Ok(());
        };

        let vtable = self.ptr.vtable::<IFileOperationVtbl>();
        // SAFETY: self.ptr is an IFileOperation pointer and hwnd is the application window
        // handle supplied by the Win32 entry layer for Shell-owned progress and prompt UI.
        let hresult = unsafe { (vtable.set_owner_window)(self.ptr.as_raw(), hwnd) };
        self.check("IFileOperation::SetOwnerWindow", hresult)
    }

    fn copy_item(&self, source: &ComPtr, destination: &ComPtr) -> ExplorerResult<()> {
        let vtable = self.ptr.vtable::<IFileOperationVtbl>();
        // SAFETY: source and destination are IShellItem pointers created by Shell, and null
        // optional arguments request default naming and no per-item progress sink.
        let hresult = unsafe {
            (vtable.copy_item)(
                self.ptr.as_raw(),
                source.as_raw(),
                destination.as_raw(),
                null(),
                null_mut(),
            )
        };
        self.check("IFileOperation::CopyItem", hresult)
    }

    fn move_item(&self, source: &ComPtr, destination: &ComPtr) -> ExplorerResult<()> {
        let vtable = self.ptr.vtable::<IFileOperationVtbl>();
        // SAFETY: source and destination are IShellItem pointers created by Shell, and null
        // optional arguments request default naming and no per-item progress sink.
        let hresult = unsafe {
            (vtable.move_item)(
                self.ptr.as_raw(),
                source.as_raw(),
                destination.as_raw(),
                null(),
                null_mut(),
            )
        };
        self.check("IFileOperation::MoveItem", hresult)
    }

    fn delete_item(&self, target: &ComPtr) -> ExplorerResult<()> {
        let vtable = self.ptr.vtable::<IFileOperationVtbl>();
        // SAFETY: target is an IShellItem pointer created by Shell; null progress sink keeps
        // Shell's default progress and cancellation UI.
        let hresult =
            unsafe { (vtable.delete_item)(self.ptr.as_raw(), target.as_raw(), null_mut()) };
        self.check("IFileOperation::DeleteItem", hresult)
    }

    fn rename_item(&self, target: &ComPtr, new_name: PCWSTR) -> ExplorerResult<()> {
        let vtable = self.ptr.vtable::<IFileOperationVtbl>();
        // SAFETY: target is an IShellItem pointer created by Shell, new_name is a
        // null-terminated UTF-16 buffer that lives until PerformOperations returns.
        let hresult = unsafe {
            (vtable.rename_item)(self.ptr.as_raw(), target.as_raw(), new_name, null_mut())
        };
        self.check("IFileOperation::RenameItem", hresult)
    }

    fn perform(&self) -> ExplorerResult<()> {
        let vtable = self.ptr.vtable::<IFileOperationVtbl>();
        // SAFETY: self.ptr is an initialized IFileOperation and all queued Shell items remain
        // alive for the duration of this call.
        let hresult = unsafe { (vtable.perform_operations)(self.ptr.as_raw()) };
        self.check("IFileOperation::PerformOperations", hresult)?;

        let mut aborted = 0;
        // SAFETY: aborted is a valid BOOL out pointer.
        let hresult =
            unsafe { (vtable.get_any_operations_aborted)(self.ptr.as_raw(), &mut aborted) };
        self.check("IFileOperation::GetAnyOperationsAborted", hresult)?;
        if aborted != 0 {
            return Err(shell_cancelled_error(
                self.operation,
                "IFileOperation::GetAnyOperationsAborted",
                self.targets,
            ));
        }

        Ok(())
    }

    fn check(&self, api: &'static str, hresult: HRESULT) -> ExplorerResult<()> {
        check_hresult(self.operation, api, hresult, self.targets)
    }
}

struct ComApartment {
    should_uninitialize: bool,
}

impl ComApartment {
    fn initialize(operation: ShellOperation, targets: &[PathBuf]) -> ExplorerResult<Self> {
        let coinit = (COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE) as u32;
        // SAFETY: CoInitializeEx accepts a null reserved pointer and initializes COM for the
        // current thread. The matching CoUninitialize call is guarded by ComApartment::drop.
        let hresult = unsafe { CoInitializeEx(null(), coinit) };
        if hresult == RPC_E_CHANGED_MODE {
            return Err(shell_hresult_error(
                operation,
                "CoInitializeEx",
                hresult,
                targets,
            ));
        }
        check_hresult(operation, "CoInitializeEx", hresult, targets)?;
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

struct ComPtr {
    ptr: NonNull<c_void>,
}

impl ComPtr {
    fn from_raw(
        raw: *mut c_void,
        operation: ShellOperation,
        api: &'static str,
        targets: &[PathBuf],
    ) -> ExplorerResult<Self> {
        let ptr = NonNull::new(raw).ok_or_else(|| {
            ExplorerError::shell_operation_failed_with_context(
                operation,
                api,
                None,
                None,
                targets.to_vec(),
                false,
                false,
            )
        })?;
        Ok(Self { ptr })
    }

    fn as_raw(&self) -> *mut c_void {
        self.ptr.as_ptr()
    }

    fn vtable<T>(&self) -> &T {
        // SAFETY: COM interface pointers point at a vtable pointer as their first field.
        unsafe { &**(self.ptr.as_ptr() as *mut *mut T) }
    }
}

impl Drop for ComPtr {
    fn drop(&mut self) {
        let vtable = self.vtable::<IUnknownVtbl>();
        // SAFETY: self.ptr is a live COM interface pointer owned by this ComPtr.
        unsafe {
            (vtable.release)(self.ptr.as_ptr());
        }
    }
}

#[repr(C)]
struct IUnknownVtbl {
    _query_interface:
        unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> HRESULT,
    _add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
    release: unsafe extern "system" fn(*mut c_void) -> u32,
}

#[repr(C)]
struct IFileOperationVtbl {
    _base: IUnknownVtbl,
    _advise: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut u32) -> HRESULT,
    _unadvise: unsafe extern "system" fn(*mut c_void, u32) -> HRESULT,
    set_operation_flags: unsafe extern "system" fn(*mut c_void, u32) -> HRESULT,
    _set_progress_message: unsafe extern "system" fn(*mut c_void, PCWSTR) -> HRESULT,
    _set_progress_dialog: unsafe extern "system" fn(*mut c_void, *mut c_void) -> HRESULT,
    _set_properties: unsafe extern "system" fn(*mut c_void, *mut c_void) -> HRESULT,
    set_owner_window: unsafe extern "system" fn(*mut c_void, HWND) -> HRESULT,
    _apply_properties_to_item: unsafe extern "system" fn(*mut c_void, *mut c_void) -> HRESULT,
    _apply_properties_to_items: unsafe extern "system" fn(*mut c_void, *mut c_void) -> HRESULT,
    rename_item:
        unsafe extern "system" fn(*mut c_void, *mut c_void, PCWSTR, *mut c_void) -> HRESULT,
    _rename_items: unsafe extern "system" fn(*mut c_void, *mut c_void, PCWSTR) -> HRESULT,
    move_item: unsafe extern "system" fn(
        *mut c_void,
        *mut c_void,
        *mut c_void,
        PCWSTR,
        *mut c_void,
    ) -> HRESULT,
    _move_items: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void) -> HRESULT,
    copy_item: unsafe extern "system" fn(
        *mut c_void,
        *mut c_void,
        *mut c_void,
        PCWSTR,
        *mut c_void,
    ) -> HRESULT,
    _copy_items: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void) -> HRESULT,
    delete_item: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void) -> HRESULT,
    _delete_items: unsafe extern "system" fn(*mut c_void, *mut c_void) -> HRESULT,
    _new_item: unsafe extern "system" fn(
        *mut c_void,
        *mut c_void,
        u32,
        PCWSTR,
        PCWSTR,
        *mut c_void,
    ) -> HRESULT,
    perform_operations: unsafe extern "system" fn(*mut c_void) -> HRESULT,
    get_any_operations_aborted: unsafe extern "system" fn(*mut c_void, *mut BOOL) -> HRESULT,
}

fn shell_item_from_path(
    path: &Path,
    operation: ShellOperation,
    targets: &[PathBuf],
) -> ExplorerResult<ComPtr> {
    let wide_path = path_to_wide_null(path);
    let mut raw_item = null_mut();
    // SAFETY: wide_path is a null-terminated UTF-16 parsing name and raw_item is a valid out
    // pointer for the requested IShellItem interface.
    let hresult = unsafe {
        SHCreateItemFromParsingName(
            wide_path.as_ptr(),
            null_mut(),
            &IID_ISHELL_ITEM,
            &mut raw_item,
        )
    };
    check_hresult(operation, "SHCreateItemFromParsingName", hresult, targets)?;
    ComPtr::from_raw(raw_item, operation, "SHCreateItemFromParsingName", targets)
}

fn shell_items_from_paths(
    paths: &[PathBuf],
    operation: ShellOperation,
    targets: &[PathBuf],
) -> ExplorerResult<Vec<ComPtr>> {
    let mut items = Vec::with_capacity(paths.len());
    for path in paths {
        items.push(shell_item_from_path(path, operation, targets)?);
    }
    Ok(items)
}

fn ensure_non_empty(paths: &[PathBuf], operation: ShellOperation) -> ExplorerResult<()> {
    if paths.is_empty() {
        Err(ExplorerError::shell_operation_failed_with_context(
            operation,
            "IFileOperation",
            None,
            None,
            Vec::new(),
            false,
            false,
        ))
    } else {
        Ok(())
    }
}

fn targets_with_destination(sources: Vec<PathBuf>, destination: &Path) -> Vec<PathBuf> {
    let mut targets = sources;
    targets.push(destination.to_path_buf());
    targets
}

fn operation_flags(operation: ShellOperation) -> u32 {
    let mut flags = FOFX_SHOWELEVATIONPROMPT;
    if operation != ShellOperation::DeletePermanently {
        flags |= FOF_ALLOWUNDO;
    }
    if operation == ShellOperation::DeleteToRecycleBin {
        flags |= FOFX_RECYCLEONDELETE;
    }
    flags
}

fn owner_window_from_isize(owner_window: isize) -> Option<HWND> {
    if owner_window == 0 {
        None
    } else {
        Some(owner_window as HWND)
    }
}

fn check_hresult(
    operation: ShellOperation,
    api: &'static str,
    hresult: HRESULT,
    targets: &[PathBuf],
) -> ExplorerResult<()> {
    if hresult < 0 {
        Err(shell_hresult_error(operation, api, hresult, targets))
    } else {
        Ok(())
    }
}

fn shell_hresult_error(
    operation: ShellOperation,
    api: &'static str,
    hresult: HRESULT,
    targets: &[PathBuf],
) -> ExplorerError {
    let code = windows_code_from_hresult(hresult);
    let cancelled = code == Some(ERROR_CANCELLED) || is_copy_engine_cancelled_hresult(hresult);
    let elevation_required =
        code == Some(ERROR_ELEVATION_REQUIRED) || hresult == COPYENGINE_E_REQUIRES_ELEVATION;
    ExplorerError::shell_operation_failed_with_context(
        operation,
        api,
        code,
        Some(hresult),
        targets.to_vec(),
        cancelled,
        elevation_required,
    )
}

fn shell_cancelled_error(
    operation: ShellOperation,
    api: &'static str,
    targets: &[PathBuf],
) -> ExplorerError {
    ExplorerError::shell_operation_failed_with_context(
        operation,
        api,
        Some(ERROR_CANCELLED),
        Some(hresult_from_win32(ERROR_CANCELLED)),
        targets.to_vec(),
        true,
        false,
    )
}

fn windows_code_from_hresult(hresult: HRESULT) -> Option<u32> {
    const HRESULT_FROM_WIN32_MASK: u32 = 0x8007_0000;

    let raw = hresult as u32;
    if raw & 0xffff_0000 == HRESULT_FROM_WIN32_MASK {
        Some(raw & 0x0000_ffff)
    } else {
        copy_engine_win32_code(hresult)
    }
}

fn copy_engine_win32_code(hresult: HRESULT) -> Option<u32> {
    match hresult {
        COPYENGINE_E_USER_CANCELLED | COPYENGINE_E_CANCELLED => Some(ERROR_CANCELLED),
        COPYENGINE_E_REQUIRES_ELEVATION => Some(ERROR_ELEVATION_REQUIRED),
        COPYENGINE_E_ACCESSDENIED_READONLY
        | COPYENGINE_E_ACCESS_DENIED_DEST
        | COPYENGINE_E_ACCESS_DENIED_SRC => Some(ERROR_ACCESS_DENIED),
        COPYENGINE_E_ALREADY_EXISTS_FOLDER
        | COPYENGINE_E_ALREADY_EXISTS_NORMAL
        | COPYENGINE_E_ALREADY_EXISTS_READONLY
        | COPYENGINE_E_ALREADY_EXISTS_SYSTEM => Some(ERROR_ALREADY_EXISTS),
        COPYENGINE_E_PATH_NOT_FOUND_DEST | COPYENGINE_E_PATH_NOT_FOUND_SRC => {
            Some(ERROR_PATH_NOT_FOUND)
        }
        COPYENGINE_E_CANT_REACH_SOURCE => Some(ERROR_FILE_NOT_FOUND),
        COPYENGINE_E_NEWFILE_NAME_TOO_LONG | COPYENGINE_E_NEWFOLDER_NAME_TOO_LONG => {
            Some(ERROR_FILENAME_EXCED_RANGE)
        }
        COPYENGINE_E_SHARING_VIOLATION_DEST | COPYENGINE_E_SHARING_VIOLATION_SRC => {
            Some(ERROR_SHARING_VIOLATION)
        }
        _ => None,
    }
}

fn is_copy_engine_cancelled_hresult(hresult: HRESULT) -> bool {
    matches!(
        hresult,
        COPYENGINE_E_USER_CANCELLED | COPYENGINE_E_CANCELLED
    )
}

fn hresult_from_win32(code: u32) -> HRESULT {
    ((code & 0x0000_ffff) | 0x8007_0000) as HRESULT
}

fn path_to_wide_null(path: &Path) -> Vec<u16> {
    path.as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

fn os_str_to_wide_null(value: &OsStr) -> ExplorerResult<Vec<u16>> {
    let mut units = value.encode_wide().collect::<Vec<_>>();
    if units.contains(&0) {
        return Err(ExplorerError::invalid_file_name(
            value.to_os_string(),
            FileNameErrorKind::HasControlCharacter,
        ));
    }
    units.push(0);
    Ok(units)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;
    use std::os::windows::ffi::OsStringExt;

    #[test]
    fn extracts_win32_code_from_hresult() {
        assert_eq!(
            windows_code_from_hresult(hresult_from_win32(ERROR_CANCELLED)),
            Some(ERROR_CANCELLED)
        );
    }

    #[test]
    fn does_not_treat_plain_com_hresult_as_win32_code() {
        assert_eq!(windows_code_from_hresult(RPC_E_CHANGED_MODE), None);
    }

    #[test]
    fn rename_rejects_embedded_nul_before_shell_operation() {
        let new_name = OsString::from_wide(&[b'a' as u16, 0, b'b' as u16]);

        let error = shell_rename_item(Path::new(r"C:\source\a.txt"), new_name.as_os_str())
            .expect_err("embedded NUL must be rejected before IFileOperation::RenameItem");

        assert!(matches!(
            error,
            ExplorerError::InvalidFileName {
                reason: FileNameErrorKind::HasControlCharacter,
                ..
            }
        ));
    }

    #[test]
    fn maps_copy_engine_hresult_classifications() {
        assert_eq!(
            windows_code_from_hresult(COPYENGINE_E_USER_CANCELLED),
            Some(ERROR_CANCELLED)
        );
        assert_eq!(
            windows_code_from_hresult(COPYENGINE_E_REQUIRES_ELEVATION),
            Some(ERROR_ELEVATION_REQUIRED)
        );
        assert_eq!(
            windows_code_from_hresult(COPYENGINE_E_ACCESS_DENIED_DEST),
            Some(ERROR_ACCESS_DENIED)
        );
        assert_eq!(
            windows_code_from_hresult(COPYENGINE_E_ALREADY_EXISTS_NORMAL),
            Some(ERROR_ALREADY_EXISTS)
        );
        assert_eq!(
            windows_code_from_hresult(COPYENGINE_E_NEWFILE_NAME_TOO_LONG),
            Some(ERROR_FILENAME_EXCED_RANGE)
        );
        assert_eq!(
            windows_code_from_hresult(COPYENGINE_E_SHARING_VIOLATION_SRC),
            Some(ERROR_SHARING_VIOLATION)
        );
    }

    #[test]
    fn copy_engine_cancelled_hresult_is_user_cancelled() {
        let error = shell_hresult_error(
            ShellOperation::Copy,
            "IFileOperation::PerformOperations",
            COPYENGINE_E_CANCELLED,
            &[PathBuf::from(r"C:\source.txt")],
        );

        assert!(error.is_cancelled());
        assert!(!error.requires_elevation());
    }

    #[test]
    fn targets_with_destination_appends_destination_after_sources() {
        let sources = vec![
            PathBuf::from(r"C:\source\a.txt"),
            PathBuf::from(r"C:\source\b.txt"),
        ];

        let targets = targets_with_destination(sources, Path::new(r"D:\destination"));

        assert_eq!(
            targets,
            vec![
                PathBuf::from(r"C:\source\a.txt"),
                PathBuf::from(r"C:\source\b.txt"),
                PathBuf::from(r"D:\destination"),
            ]
        );
    }

    #[test]
    fn delete_operation_uses_recycle_bin_flag() {
        let flags = operation_flags(ShellOperation::DeleteToRecycleBin);

        assert_ne!(flags & FOFX_RECYCLEONDELETE, 0);
        assert_ne!(flags & FOF_ALLOWUNDO, 0);
    }

    #[test]
    fn permanent_delete_operation_does_not_use_recycle_or_undo_flags() {
        let flags = operation_flags(ShellOperation::DeletePermanently);

        assert_eq!(flags & FOFX_RECYCLEONDELETE, 0);
        assert_eq!(flags & FOF_ALLOWUNDO, 0);
    }
}
