use std::ffi::c_void;
use std::mem::size_of;
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::ptr::{null, null_mut, NonNull};

use windows_sys::core::{GUID, HRESULT, PCSTR};
use windows_sys::Win32::Foundation::{
    GetLastError, ERROR_CANCELLED, HWND, POINT, RPC_E_CHANGED_MODE,
};
use windows_sys::Win32::System::Com::{
    CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE,
};
use windows_sys::Win32::UI::Shell::Common::ITEMIDLIST;
use windows_sys::Win32::UI::Shell::{
    ILCreateFromPathW, ILFree, SHBindToObject, SHBindToParent, CMF_EXPLORE, CMF_NORMAL,
    CMIC_MASK_PTINVOKE, CMINVOKECOMMANDINFOEX,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreatePopupMenu, DestroyMenu, PostMessageW, SetForegroundWindow, TrackPopupMenu, HMENU,
    SW_SHOWNORMAL, TPM_RETURNCMD, TPM_RIGHTBUTTON, WM_NULL,
};

use crate::domain::{ExplorerError, ExplorerResult, ShellOperation};

const IID_ISHELL_FOLDER: GUID = GUID::from_u128(0x000214e6_0000_0000_c000_000000000046);
const IID_ICONTEXT_MENU: GUID = GUID::from_u128(0x000214e4_0000_0000_c000_000000000046);
const CONTEXT_MENU_FIRST_ID: u32 = 1;
const CONTEXT_MENU_LAST_ID: u32 = 0x7fff;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShellContextMenuPoint {
    pub x: i32,
    pub y: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ShellContextMenuOutcome {
    pub command_invoked: bool,
    pub refresh_current_folder: bool,
}

pub fn shell_show_context_menu(
    owner_window: isize,
    targets: &[PathBuf],
    point: ShellContextMenuPoint,
) -> ExplorerResult<ShellContextMenuOutcome> {
    let owner = owner_window as HWND;
    ensure_context_menu_targets(owner, targets)?;
    let _apartment = ComApartment::initialize(targets)?;

    let pidls = target_pidls(targets)?;
    let (parent_folder, first_child) = bind_to_parent(&pidls[0], targets)?;
    let mut child_pidls: Vec<*const ITEMIDLIST> = Vec::with_capacity(pidls.len());
    child_pidls.push(first_child as *const ITEMIDLIST);
    for pidl in &pidls[1..] {
        let (_parent, child) = bind_to_parent(pidl, targets)?;
        child_pidls.push(child as *const ITEMIDLIST);
    }

    let context_menu = context_menu_from_parent(&parent_folder, owner, &child_pidls, targets)?;
    let popup_menu = PopupMenu::new(targets)?;
    query_context_menu(&context_menu, popup_menu.raw(), targets)?;

    let Some(command_id) = popup_menu.track(owner, point) else {
        return Ok(ShellContextMenuOutcome::default());
    };

    invoke_context_menu_command(&context_menu, owner, command_id, point, targets)?;
    Ok(ShellContextMenuOutcome {
        command_invoked: true,
        refresh_current_folder: true,
    })
}

pub fn shell_show_folder_background_context_menu(
    owner_window: isize,
    folder: &Path,
    point: ShellContextMenuPoint,
) -> ExplorerResult<ShellContextMenuOutcome> {
    let owner = owner_window as HWND;
    let targets = [folder.to_path_buf()];
    ensure_folder_background_context_menu_target(owner, folder, &targets)?;
    let _apartment = ComApartment::initialize(&targets)?;

    let folder_pidl = Pidl::from_path(folder, &targets)?;
    let folder = bind_to_folder(&folder_pidl, &targets)?;
    let context_menu = context_menu_for_folder_background(&folder, owner, &targets)?;
    let popup_menu = PopupMenu::new(&targets)?;
    query_context_menu(&context_menu, popup_menu.raw(), &targets)?;

    let Some(command_id) = popup_menu.track(owner, point) else {
        return Ok(ShellContextMenuOutcome::default());
    };

    invoke_context_menu_command(&context_menu, owner, command_id, point, &targets)?;
    Ok(ShellContextMenuOutcome {
        command_invoked: true,
        refresh_current_folder: true,
    })
}

fn ensure_context_menu_targets(owner: HWND, targets: &[PathBuf]) -> ExplorerResult<()> {
    if owner.is_null() {
        return Err(shell_failure(
            "context menu owner window",
            None,
            None,
            targets,
        ));
    }

    if targets.is_empty() {
        return Err(shell_failure("IContextMenu", None, None, targets));
    }

    let first_parent = targets[0].parent().map(Path::to_path_buf);
    if targets
        .iter()
        .any(|target| target.parent().map(Path::to_path_buf) != first_parent)
    {
        return Err(ExplorerError::invalid_input(
            "컨텍스트 메뉴 대상은 같은 폴더 안에 있어야 합니다.",
        ));
    }

    Ok(())
}

fn ensure_folder_background_context_menu_target(
    owner: HWND,
    folder: &Path,
    targets: &[PathBuf],
) -> ExplorerResult<()> {
    if owner.is_null() {
        return Err(shell_failure(
            "folder background context menu owner window",
            None,
            None,
            targets,
        ));
    }

    if folder.as_os_str().is_empty() {
        return Err(ExplorerError::invalid_input(
            "컨텍스트 메뉴를 표시할 현재 폴더가 없습니다.",
        ));
    }

    Ok(())
}

fn target_pidls(targets: &[PathBuf]) -> ExplorerResult<Vec<Pidl>> {
    let mut pidls = Vec::with_capacity(targets.len());
    for target in targets {
        pidls.push(Pidl::from_path(target, targets)?);
    }
    Ok(pidls)
}

fn bind_to_parent(pidl: &Pidl, targets: &[PathBuf]) -> ExplorerResult<(ComPtr, *mut ITEMIDLIST)> {
    let mut raw_parent = null_mut();
    let mut child: *mut ITEMIDLIST = null_mut();
    // SAFETY: pidl is a full PIDL owned by Pidl. raw_parent and child are valid out pointers.
    let hresult = unsafe {
        SHBindToParent(
            pidl.as_ptr(),
            &IID_ISHELL_FOLDER,
            &mut raw_parent,
            &mut child,
        )
    };
    check_hresult("SHBindToParent", hresult, targets)?;
    if child.is_null() {
        return Err(shell_failure(
            "SHBindToParent",
            None,
            Some(hresult),
            targets,
        ));
    }

    let parent = ComPtr::from_raw(raw_parent, "SHBindToParent", targets)?;
    Ok((parent, child))
}

fn bind_to_folder(pidl: &Pidl, targets: &[PathBuf]) -> ExplorerResult<ComPtr> {
    let mut raw_folder = null_mut();
    // SAFETY: pidl is a full PIDL owned by Pidl. raw_folder is a valid out pointer for the
    // requested IShellFolder interface.
    let hresult = unsafe {
        SHBindToObject(
            null_mut(),
            pidl.as_ptr(),
            null_mut(),
            &IID_ISHELL_FOLDER,
            &mut raw_folder,
        )
    };
    check_hresult("SHBindToObject(IShellFolder)", hresult, targets)?;
    ComPtr::from_raw(raw_folder, "SHBindToObject(IShellFolder)", targets)
}

fn context_menu_from_parent(
    parent: &ComPtr,
    owner: HWND,
    child_pidls: &[*const ITEMIDLIST],
    targets: &[PathBuf],
) -> ExplorerResult<ComPtr> {
    let count = u32::try_from(child_pidls.len())
        .map_err(|_| ExplorerError::state_conflict("컨텍스트 메뉴 대상이 너무 많습니다."))?;
    let mut raw_context_menu = null_mut();
    let vtable = parent.vtable::<IShellFolderVtbl>();
    // SAFETY: parent is an IShellFolder pointer returned by SHBindToParent. child_pidls point into
    // full PIDLs kept alive by the caller for the duration of this call.
    let hresult = unsafe {
        (vtable.get_ui_object_of)(
            parent.as_raw(),
            owner,
            count,
            child_pidls.as_ptr(),
            &IID_ICONTEXT_MENU,
            null_mut(),
            &mut raw_context_menu,
        )
    };
    check_hresult("IShellFolder::GetUIObjectOf", hresult, targets)?;
    ComPtr::from_raw(
        raw_context_menu,
        "IShellFolder::GetUIObjectOf(IContextMenu)",
        targets,
    )
}

fn context_menu_for_folder_background(
    folder: &ComPtr,
    owner: HWND,
    targets: &[PathBuf],
) -> ExplorerResult<ComPtr> {
    let mut raw_context_menu = null_mut();
    let vtable = folder.vtable::<IShellFolderVtbl>();
    // SAFETY: folder is an IShellFolder pointer for the current folder. CreateViewObject with
    // IID_IContextMenu returns the Shell folder background context menu for the owner window.
    let hresult = unsafe {
        (vtable.create_view_object)(
            folder.as_raw(),
            owner,
            &IID_ICONTEXT_MENU,
            &mut raw_context_menu,
        )
    };
    check_hresult(
        "IShellFolder::CreateViewObject(IContextMenu)",
        hresult,
        targets,
    )?;
    ComPtr::from_raw(
        raw_context_menu,
        "IShellFolder::CreateViewObject(IContextMenu)",
        targets,
    )
}

fn query_context_menu(
    context_menu: &ComPtr,
    menu: HMENU,
    targets: &[PathBuf],
) -> ExplorerResult<()> {
    let vtable = context_menu.vtable::<IContextMenuVtbl>();
    // SAFETY: context_menu is an IContextMenu pointer and menu is an owned empty popup menu.
    let hresult = unsafe {
        (vtable.query_context_menu)(
            context_menu.as_raw(),
            menu,
            0,
            CONTEXT_MENU_FIRST_ID,
            CONTEXT_MENU_LAST_ID,
            CMF_NORMAL | CMF_EXPLORE,
        )
    };
    check_hresult("IContextMenu::QueryContextMenu", hresult, targets)
}

fn invoke_context_menu_command(
    context_menu: &ComPtr,
    owner: HWND,
    command_id: u32,
    point: ShellContextMenuPoint,
    targets: &[PathBuf],
) -> ExplorerResult<()> {
    if !(CONTEXT_MENU_FIRST_ID..=CONTEXT_MENU_LAST_ID).contains(&command_id) {
        return Err(shell_failure("TrackPopupMenu", None, None, targets));
    }

    let command_offset = command_id - CONTEXT_MENU_FIRST_ID;
    let command_offset = u16::try_from(command_offset)
        .map_err(|_| ExplorerError::state_conflict("컨텍스트 메뉴 명령 식별자가 너무 큽니다."))?;
    let mut command = CMINVOKECOMMANDINFOEX {
        cbSize: size_of::<CMINVOKECOMMANDINFOEX>() as u32,
        fMask: CMIC_MASK_PTINVOKE,
        hwnd: owner,
        lpVerb: make_int_resource_a(command_offset),
        nShow: SW_SHOWNORMAL,
        ptInvoke: POINT {
            x: point.x,
            y: point.y,
        },
        ..Default::default()
    };

    let vtable = context_menu.vtable::<IContextMenuVtbl>();
    // SAFETY: command is initialized according to CMINVOKECOMMANDINFOEX. IContextMenu accepts the
    // base CMINVOKECOMMANDINFO layout, which is the prefix of this structure.
    let hresult = unsafe {
        (vtable.invoke_command)(
            context_menu.as_raw(),
            (&mut command as *mut CMINVOKECOMMANDINFOEX).cast::<c_void>(),
        )
    };
    check_hresult("IContextMenu::InvokeCommand", hresult, targets)
}

struct Pidl {
    ptr: NonNull<ITEMIDLIST>,
}

impl Pidl {
    fn from_path(path: &Path, targets: &[PathBuf]) -> ExplorerResult<Self> {
        let wide_path = path_to_wide_null(path);
        // SAFETY: wide_path is a null-terminated UTF-16 parsing path.
        let raw = unsafe { ILCreateFromPathW(wide_path.as_ptr()) };
        let ptr = NonNull::new(raw).ok_or_else(|| {
            let code = match last_error_code() {
                0 => None,
                code => Some(code),
            };
            shell_failure("ILCreateFromPathW", code, None, targets)
        })?;
        Ok(Self { ptr })
    }

    fn as_ptr(&self) -> *const ITEMIDLIST {
        self.ptr.as_ptr()
    }
}

impl Drop for Pidl {
    fn drop(&mut self) {
        // SAFETY: ptr was returned by ILCreateFromPathW and must be freed with ILFree.
        unsafe {
            ILFree(self.ptr.as_ptr());
        }
    }
}

struct PopupMenu(HMENU);

impl PopupMenu {
    fn new(targets: &[PathBuf]) -> ExplorerResult<Self> {
        // SAFETY: CreatePopupMenu has no preconditions and returns an owned menu handle.
        let menu = unsafe { CreatePopupMenu() };
        if menu.is_null() {
            return Err(shell_failure(
                "CreatePopupMenu",
                Some(last_error_code()),
                None,
                targets,
            ));
        }
        Ok(Self(menu))
    }

    fn raw(&self) -> HMENU {
        self.0
    }

    fn track(&self, owner: HWND, point: ShellContextMenuPoint) -> Option<u32> {
        // SAFETY: owner is the top-level application window for this popup menu.
        unsafe {
            SetForegroundWindow(owner);
        }

        // SAFETY: self.0 is an owned popup menu populated by IContextMenu. owner is the app
        // window that owns the popup. A null RECT permits normal menu positioning.
        let command = unsafe {
            TrackPopupMenu(
                self.0,
                TPM_RETURNCMD | TPM_RIGHTBUTTON,
                point.x,
                point.y,
                0,
                owner,
                null(),
            )
        };
        // SAFETY: posting WM_NULL to the owner completes the standard TrackPopupMenu foreground
        // handling sequence without transferring ownership or carrying pointers.
        unsafe {
            PostMessageW(owner, WM_NULL, 0, 0);
        }
        u32::try_from(command).ok().filter(|value| *value != 0)
    }
}

impl Drop for PopupMenu {
    fn drop(&mut self) {
        // SAFETY: self.0 is an owned popup menu created by CreatePopupMenu.
        unsafe {
            DestroyMenu(self.0);
        }
    }
}

struct ComApartment {
    should_uninitialize: bool,
}

impl ComApartment {
    fn initialize(targets: &[PathBuf]) -> ExplorerResult<Self> {
        let coinit = (COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE) as u32;
        // SAFETY: CoInitializeEx accepts a null reserved pointer and initializes COM for the
        // current thread. The matching CoUninitialize call is guarded by ComApartment::drop.
        let hresult = unsafe { CoInitializeEx(null(), coinit) };
        if hresult == RPC_E_CHANGED_MODE {
            return Err(shell_hresult_error("CoInitializeEx", hresult, targets));
        }
        check_hresult("CoInitializeEx", hresult, targets)?;
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
    fn from_raw(raw: *mut c_void, api: &'static str, targets: &[PathBuf]) -> ExplorerResult<Self> {
        let ptr = NonNull::new(raw).ok_or_else(|| shell_failure(api, None, None, targets))?;
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
struct IShellFolderVtbl {
    _base: IUnknownVtbl,
    _parse_display_name: usize,
    _enum_objects: usize,
    _bind_to_object: usize,
    _bind_to_storage: usize,
    _compare_ids: usize,
    create_view_object:
        unsafe extern "system" fn(*mut c_void, HWND, *const GUID, *mut *mut c_void) -> HRESULT,
    _get_attributes_of: usize,
    get_ui_object_of: unsafe extern "system" fn(
        *mut c_void,
        HWND,
        u32,
        *const *const ITEMIDLIST,
        *const GUID,
        *mut u32,
        *mut *mut c_void,
    ) -> HRESULT,
    _get_display_name_of: usize,
    _set_name_of: usize,
}

#[repr(C)]
struct IContextMenuVtbl {
    _base: IUnknownVtbl,
    query_context_menu:
        unsafe extern "system" fn(*mut c_void, HMENU, u32, u32, u32, u32) -> HRESULT,
    invoke_command: unsafe extern "system" fn(*mut c_void, *mut c_void) -> HRESULT,
    _get_command_string: usize,
}

fn check_hresult(api: &'static str, hresult: HRESULT, targets: &[PathBuf]) -> ExplorerResult<()> {
    if hresult < 0 {
        Err(shell_hresult_error(api, hresult, targets))
    } else {
        Ok(())
    }
}

fn shell_hresult_error(api: &'static str, hresult: HRESULT, targets: &[PathBuf]) -> ExplorerError {
    let code = windows_code_from_hresult(hresult);
    let cancelled = code == Some(ERROR_CANCELLED);
    ExplorerError::shell_operation_failed_with_context(
        ShellOperation::ShowContextMenu,
        api,
        code,
        Some(hresult),
        targets.to_vec(),
        cancelled,
        false,
    )
}

fn shell_failure(
    api: &'static str,
    code: Option<u32>,
    hresult: Option<HRESULT>,
    targets: &[PathBuf],
) -> ExplorerError {
    ExplorerError::shell_operation_failed_with_context(
        ShellOperation::ShowContextMenu,
        api,
        code,
        hresult,
        targets.to_vec(),
        false,
        false,
    )
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

fn make_int_resource_a(value: u16) -> PCSTR {
    value as usize as PCSTR
}

fn path_to_wide_null(path: &Path) -> Vec<u16> {
    path.as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

fn last_error_code() -> u32 {
    // SAFETY: GetLastError reads thread-local Windows error state and has no preconditions.
    unsafe { GetLastError() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_context_menu_targets_from_different_parents() {
        let targets = vec![
            PathBuf::from(r"C:\one\a.txt"),
            PathBuf::from(r"C:\two\b.txt"),
        ];

        let error = ensure_context_menu_targets(null_mut(), &targets)
            .expect_err("null owner should be rejected before parent validation");
        assert_eq!(
            error.user_message(),
            "컨텍스트 메뉴를 열거나 실행할 수 없습니다."
        );

        let owner = 1usize as HWND;
        let error = ensure_context_menu_targets(owner, &targets)
            .expect_err("mixed parent targets should be rejected");
        assert_eq!(
            error.user_message(),
            "컨텍스트 메뉴 대상은 같은 폴더 안에 있어야 합니다."
        );
    }

    #[test]
    fn command_offset_uses_make_int_resource_pointer() {
        assert_eq!(make_int_resource_a(42) as usize, 42);
    }

    #[test]
    fn non_win32_hresult_does_not_create_synthetic_win32_code() {
        assert_eq!(windows_code_from_hresult(RPC_E_CHANGED_MODE), None);
    }
}
