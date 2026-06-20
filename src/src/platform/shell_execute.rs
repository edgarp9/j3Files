use std::ffi::OsStr;
use std::mem::size_of;
use std::os::windows::ffi::OsStrExt;
use std::path::Path;
use std::ptr::{null, null_mut};

use windows_sys::core::HRESULT;
use windows_sys::Win32::Foundation::{
    GetLastError, SetLastError, ERROR_CANCELLED, ERROR_ELEVATION_REQUIRED, HWND, RPC_E_CHANGED_MODE,
};
use windows_sys::Win32::System::Com::{
    CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE,
};
use windows_sys::Win32::UI::Shell::{ShellExecuteExW, SEE_MASK_INVOKEIDLIST, SHELLEXECUTEINFOW};
use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

use crate::domain::{ExplorerError, ExplorerResult, ShellOperation};

pub fn shell_open_path(path: &Path) -> ExplorerResult<()> {
    shell_open_path_with_owner(0, path)
}

pub fn shell_open_path_with_owner(owner_window: isize, path: &Path) -> ExplorerResult<()> {
    shell_execute_with_owner(owner_window, path, ShellOperation::Open)
}

pub fn shell_open_with(path: &Path) -> ExplorerResult<()> {
    shell_open_with_owner(0, path)
}

pub fn shell_open_with_owner(owner_window: isize, path: &Path) -> ExplorerResult<()> {
    shell_execute_with_owner(owner_window, path, ShellOperation::OpenWith)
}

pub fn shell_show_properties(path: &Path) -> ExplorerResult<()> {
    shell_show_properties_with_owner(0, path)
}

pub fn shell_show_properties_with_owner(owner_window: isize, path: &Path) -> ExplorerResult<()> {
    shell_execute_with_owner(owner_window, path, ShellOperation::ShowProperties)
}

pub fn shell_execute(path: &Path, operation: ShellOperation) -> ExplorerResult<()> {
    shell_execute_with_owner(0, path, operation)
}

pub fn shell_execute_with_owner(
    owner_window: isize,
    path: &Path,
    operation: ShellOperation,
) -> ExplorerResult<()> {
    ShellExecuteRequest {
        path,
        action: ShellExecuteAction::from_operation(operation)?,
        owner: owner_window_from_isize(owner_window),
    }
    .execute()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ShellExecuteAction {
    operation: ShellOperation,
    verb: ShellExecuteVerb,
    mask: u32,
}

impl ShellExecuteAction {
    fn from_operation(operation: ShellOperation) -> ExplorerResult<Self> {
        let action = match operation {
            ShellOperation::Open => Self {
                operation,
                verb: ShellExecuteVerb::Default,
                mask: 0,
            },
            ShellOperation::OpenWith => Self {
                operation,
                verb: ShellExecuteVerb::Named("openas"),
                mask: 0,
            },
            ShellOperation::ShowProperties => Self {
                operation,
                verb: ShellExecuteVerb::Named("properties"),
                mask: SEE_MASK_INVOKEIDLIST,
            },
            _ => {
                return Err(ExplorerError::unsupported(
                    "shell execute",
                    format!("{operation} is not a ShellExecuteEx operation"),
                ));
            }
        };

        Ok(action)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ShellExecuteVerb {
    Default,
    Named(&'static str),
}

struct ShellExecuteRequest<'a> {
    path: &'a Path,
    action: ShellExecuteAction,
    owner: HWND,
}

impl ShellExecuteRequest<'_> {
    fn execute(&self) -> ExplorerResult<()> {
        let _apartment = ComApartment::initialize(self.action.operation, self.path)?;
        let file = os_to_wide_null(self.path.as_os_str());
        let verb = match self.action.verb {
            ShellExecuteVerb::Default => None,
            ShellExecuteVerb::Named(value) => Some(str_to_wide_null(value)),
        };
        let verb_ptr = verb.as_ref().map_or(null(), |value| value.as_ptr());
        let directory = shell_execute_directory(self.path, self.action.operation);
        let directory = directory.map(|path| os_to_wide_null(path.as_os_str()));
        let directory_ptr = directory.as_ref().map_or(null(), |value| value.as_ptr());

        // SAFETY: SHELLEXECUTEINFOW is a C POD struct. Zero initialization is the documented
        // baseline before setting cbSize and the fields used by ShellExecuteExW.
        let mut execute_info: SHELLEXECUTEINFOW = unsafe { std::mem::zeroed() };
        execute_info.cbSize = size_of::<SHELLEXECUTEINFOW>() as u32;
        execute_info.fMask = self.action.mask;
        execute_info.hwnd = self.owner;
        execute_info.lpVerb = verb_ptr;
        execute_info.lpFile = file.as_ptr();
        execute_info.lpParameters = null();
        execute_info.lpDirectory = directory_ptr;
        execute_info.nShow = SW_SHOWNORMAL;

        clear_last_error();
        // SAFETY: execute_info points to a valid initialized structure; string pointers reference
        // null-terminated UTF-16 buffers that live until the call returns.
        let succeeded = unsafe { ShellExecuteExW(&mut execute_info) };
        if succeeded == 0 {
            return Err(shell_execute_error(
                self.action.operation,
                self.path,
                &execute_info,
            ));
        }

        Ok(())
    }
}

struct ComApartment {
    should_uninitialize: bool,
}

impl ComApartment {
    fn initialize(operation: ShellOperation, path: &Path) -> ExplorerResult<Self> {
        let coinit = (COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE) as u32;
        // SAFETY: CoInitializeEx accepts a null reserved pointer and initializes COM for the
        // current thread. The matching CoUninitialize call is guarded by ComApartment::drop.
        let hresult = unsafe { CoInitializeEx(null(), coinit) };
        if hresult == RPC_E_CHANGED_MODE {
            return Err(shell_execute_hresult_error(
                operation,
                "CoInitializeEx",
                hresult,
                path,
            ));
        }
        if hresult < 0 {
            return Err(shell_execute_hresult_error(
                operation,
                "CoInitializeEx",
                hresult,
                path,
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

fn shell_execute_error(
    operation: ShellOperation,
    path: &Path,
    execute_info: &SHELLEXECUTEINFOW,
) -> ExplorerError {
    let code = shell_execute_error_code(execute_info);
    let cancelled = code == Some(ERROR_CANCELLED);
    let elevation_required = code == Some(ERROR_ELEVATION_REQUIRED);
    ExplorerError::shell_operation_failed_with_context(
        operation,
        "ShellExecuteExW",
        code,
        None,
        vec![path.to_path_buf()],
        cancelled,
        elevation_required,
    )
}

fn shell_execute_hresult_error(
    operation: ShellOperation,
    api: &'static str,
    hresult: HRESULT,
    path: &Path,
) -> ExplorerError {
    let code = windows_code_from_hresult(hresult);
    let cancelled = code == Some(ERROR_CANCELLED);
    let elevation_required = code == Some(ERROR_ELEVATION_REQUIRED);
    ExplorerError::shell_operation_failed_with_context(
        operation,
        api,
        code,
        Some(hresult),
        vec![path.to_path_buf()],
        cancelled,
        elevation_required,
    )
}

fn shell_execute_error_code(execute_info: &SHELLEXECUTEINFOW) -> Option<u32> {
    let code = last_error_code();
    if code != 0 {
        return Some(code);
    }

    let shell_error = execute_info.hInstApp as isize;
    if (1..=32).contains(&shell_error) {
        Some(shell_error as u32)
    } else {
        None
    }
}

fn shell_execute_directory(path: &Path, operation: ShellOperation) -> Option<&Path> {
    if operation == ShellOperation::Open {
        path.parent()
            .filter(|parent| !parent.as_os_str().is_empty())
    } else {
        None
    }
}

fn owner_window_from_isize(owner_window: isize) -> HWND {
    if owner_window == 0 {
        null_mut()
    } else {
        owner_window as HWND
    }
}

fn windows_code_from_hresult(hresult: HRESULT) -> Option<u32> {
    const HRESULT_FROM_WIN32_MASK: u32 = 0x8007_0000;

    let raw = hresult as u32;
    if raw & 0xffff_0000 == HRESULT_FROM_WIN32_MASK {
        Some(raw & 0x0000_ffff)
    } else {
        None
    }
}

fn str_to_wide_null(value: &str) -> Vec<u16> {
    os_to_wide_null(OsStr::new(value))
}

fn os_to_wide_null(value: &OsStr) -> Vec<u16> {
    value.encode_wide().chain(std::iter::once(0)).collect()
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
    fn open_with_maps_to_openas_verb() -> ExplorerResult<()> {
        let action = ShellExecuteAction::from_operation(ShellOperation::OpenWith)?;

        assert_eq!(action.verb, ShellExecuteVerb::Named("openas"));
        assert_eq!(action.mask, 0);

        Ok(())
    }

    #[test]
    fn properties_uses_context_menu_verb_mask() -> ExplorerResult<()> {
        let action = ShellExecuteAction::from_operation(ShellOperation::ShowProperties)?;

        assert_eq!(action.verb, ShellExecuteVerb::Named("properties"));
        assert_ne!(action.mask & SEE_MASK_INVOKEIDLIST, 0);

        Ok(())
    }

    #[test]
    fn open_uses_parent_directory_as_working_directory() {
        let path = Path::new(r"C:\work\tool.cmd");

        assert_eq!(
            shell_execute_directory(path, ShellOperation::Open),
            Some(Path::new(r"C:\work"))
        );
        assert_eq!(
            shell_execute_directory(path, ShellOperation::OpenWith),
            None
        );
    }

    #[test]
    fn zero_owner_maps_to_null_hwnd() {
        assert!(owner_window_from_isize(0).is_null());
    }

    #[test]
    fn shell_execute_error_falls_back_to_hinstapp_when_last_error_is_zero() {
        clear_last_error();
        // SAFETY: SHELLEXECUTEINFOW is a C POD struct used here only for error-code extraction.
        let mut execute_info: SHELLEXECUTEINFOW = unsafe { std::mem::zeroed() };
        execute_info.hInstApp = 31usize as _;

        assert_eq!(shell_execute_error_code(&execute_info), Some(31));
    }
}
