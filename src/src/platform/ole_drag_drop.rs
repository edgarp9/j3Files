use std::cell::{Cell, RefCell};
use std::ffi::c_void;
use std::mem::{size_of, zeroed};
use std::path::PathBuf;
use std::ptr::{null, null_mut};
use std::rc::Rc;
use std::time::Instant;

use windows_sys::core::{GUID, HRESULT};
use windows_sys::Win32::Foundation::{
    DRAGDROP_S_CANCEL, DRAGDROP_S_DROP, DRAGDROP_S_USEDEFAULTCURSORS, DV_E_FORMATETC, E_INVALIDARG,
    E_NOINTERFACE, E_NOTIMPL, E_POINTER, POINTL, RPC_E_CHANGED_MODE, S_FALSE, S_OK,
};
use windows_sys::Win32::System::Com::{DVASPECT_CONTENT, FORMATETC, STGMEDIUM, TYMED_HGLOBAL};
use windows_sys::Win32::System::DataExchange::RegisterClipboardFormatW;
use windows_sys::Win32::System::Memory::{GlobalLock, GlobalUnlock};
use windows_sys::Win32::System::Ole::{
    DoDragDrop, OleInitialize, OleUninitialize, RegisterDragDrop, ReleaseStgMedium, RevokeDragDrop,
    DROPEFFECT_COPY, DROPEFFECT_MOVE, DROPEFFECT_NONE,
};
use windows_sys::Win32::UI::Shell::HDROP;
use windows_sys::Win32::UI::WindowsAndMessaging::PostMessageW;

use crate::domain::{ExplorerError, ExplorerResult, HoverExpandAction, HoverExpandState};

use super::hdrop::{self, FileDropUsage, OwnedHglobal};
use super::win32_ui::{self as ui, ScreenPoint, TreeViewItemHandle, WindowHandle};

const INTERNAL_DRAG_FORMAT_NAME: [u16; 22] = [
    'j' as u16, '3' as u16, 'F' as u16, 'i' as u16, 'l' as u16, 'e' as u16, 's' as u16, ' ' as u16,
    'I' as u16, 'n' as u16, 't' as u16, 'e' as u16, 'r' as u16, 'n' as u16, 'a' as u16, 'l' as u16,
    ' ' as u16, 'D' as u16, 'r' as u16, 'a' as u16, 'g' as u16, 0,
];
const MK_LBUTTON_VALUE: u32 = 0x0001;
const MK_SHIFT_VALUE: u32 = 0x0004;
const MK_CONTROL_VALUE: u32 = 0x0008;
const TREE_HOVER_EXPAND_DELAY_MS: u64 = 700;

const IID_IUNKNOWN: GUID = GUID::from_u128(0x00000000_0000_0000_c000_000000000046);
const IID_IDATA_OBJECT: GUID = GUID::from_u128(0x0000010e_0000_0000_c000_000000000046);
const IID_IDROP_SOURCE: GUID = GUID::from_u128(0x00000121_0000_0000_c000_000000000046);
const IID_IDROP_TARGET: GUID = GUID::from_u128(0x00000122_0000_0000_c000_000000000046);
const IID_IENUM_FORMAT_ETC: GUID = GUID::from_u128(0x00000103_0000_0000_c000_000000000046);
const DATADIR_GET_VALUE: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OleDropTargetKind {
    FileList,
    FolderTree,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct OleDropKeyState {
    pub control: bool,
    pub shift: bool,
}

impl OleDropKeyState {
    fn from_raw(raw: u32) -> Self {
        Self {
            control: raw & MK_CONTROL_VALUE != 0,
            shift: raw & MK_SHIFT_VALUE != 0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct OleDropEffects {
    pub copy: bool,
    pub move_: bool,
}

impl OleDropEffects {
    fn from_mask(mask: u32) -> Self {
        Self {
            copy: mask & DROPEFFECT_COPY != 0,
            move_: mask & DROPEFFECT_MOVE != 0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OleDragSourceOutcome {
    Cancelled,
    NoDrop,
    Copy,
    Move,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OleDropPreferredEffect {
    Copy,
    Move,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OleDropEffectHint {
    pub copy: bool,
    pub move_: bool,
    pub default: Option<OleDropPreferredEffect>,
}

impl OleDropEffectHint {
    pub const fn none() -> Self {
        Self {
            copy: false,
            move_: false,
            default: None,
        }
    }

    pub const fn copy_move(default: Option<OleDropPreferredEffect>) -> Self {
        Self {
            copy: true,
            move_: true,
            default,
        }
    }

    pub const fn copy_only() -> Self {
        Self {
            copy: true,
            move_: false,
            default: Some(OleDropPreferredEffect::Copy),
        }
    }

    pub const fn move_only() -> Self {
        Self {
            copy: false,
            move_: true,
            default: Some(OleDropPreferredEffect::Move),
        }
    }

    fn allows_any(self) -> bool {
        self.copy || self.move_
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct DropEffectSelection {
    key_state: OleDropKeyState,
    allowed_effects: OleDropEffects,
    hint: OleDropEffectHint,
}

impl DropEffectSelection {
    fn from_raw(key_state: u32, allowed_effects: u32, hint: OleDropEffectHint) -> Self {
        Self {
            key_state: OleDropKeyState::from_raw(key_state),
            allowed_effects: OleDropEffects::from_mask(allowed_effects),
            hint,
        }
    }

    fn effect_mask(self) -> u32 {
        match self.preferred_effect() {
            Some(OleDropPreferredEffect::Copy) => DROPEFFECT_COPY,
            Some(OleDropPreferredEffect::Move) => DROPEFFECT_MOVE,
            None => DROPEFFECT_NONE,
        }
    }

    fn preferred_effect(self) -> Option<OleDropPreferredEffect> {
        let copy_allowed = self.copy_allowed();
        let move_allowed = self.move_allowed();

        if self.key_state.control {
            if copy_allowed {
                Some(OleDropPreferredEffect::Copy)
            } else {
                None
            }
        } else if self.key_state.shift {
            if move_allowed {
                Some(OleDropPreferredEffect::Move)
            } else {
                None
            }
        } else if self.hint.default == Some(OleDropPreferredEffect::Copy) && copy_allowed {
            Some(OleDropPreferredEffect::Copy)
        } else if self.hint.default == Some(OleDropPreferredEffect::Move) && move_allowed {
            Some(OleDropPreferredEffect::Move)
        } else if copy_allowed {
            Some(OleDropPreferredEffect::Copy)
        } else if move_allowed {
            Some(OleDropPreferredEffect::Move)
        } else {
            None
        }
    }

    fn copy_allowed(self) -> bool {
        self.hint.copy && self.allowed_effects.copy
    }

    fn move_allowed(self) -> bool {
        self.hint.move_ && self.allowed_effects.move_
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OleDropFeedbackTimerConfig {
    pub timer_id: usize,
    pub interval_ms: u32,
    pub auto_scroll_edge_px: i32,
}

impl OleDropFeedbackTimerConfig {
    pub const fn new(timer_id: usize, interval_ms: u32, auto_scroll_edge_px: i32) -> Self {
        Self {
            timer_id,
            interval_ms,
            auto_scroll_edge_px,
        }
    }
}

#[derive(Debug, Clone)]
pub struct OleDropFeedback {
    inner: Rc<RefCell<OleDropFeedbackState>>,
}

impl Default for OleDropFeedback {
    fn default() -> Self {
        Self {
            inner: Rc::new(RefCell::new(OleDropFeedbackState::default())),
        }
    }
}

impl OleDropFeedback {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set_file_list_hints(
        &self,
        external_empty: OleDropEffectHint,
        internal_empty: OleDropEffectHint,
        internal_items: Vec<OleDropEffectHint>,
    ) {
        let mut state = self.inner.borrow_mut();
        state.file_list_external_empty = external_empty;
        state.file_list_internal_empty = internal_empty;
        if !internal_items.is_empty() || !state.file_list_internal_items.is_empty() {
            state.file_list_internal_items = internal_items;
        }
    }

    pub fn set_folder_tree_hints(
        &self,
        external_item: OleDropEffectHint,
        internal_items: Vec<OleDropEffectHint>,
    ) {
        let mut state = self.inner.borrow_mut();
        state.folder_tree_external_item = external_item;
        if !internal_items.is_empty() || !state.folder_tree_internal_items.is_empty() {
            state.folder_tree_internal_items = internal_items;
        }
    }

    fn file_list_external_empty_hint(&self) -> OleDropEffectHint {
        self.inner.borrow().file_list_external_empty
    }

    fn file_list_internal_empty_hint(&self) -> OleDropEffectHint {
        self.inner.borrow().file_list_internal_empty
    }

    fn file_list_internal_item_hint(&self, index: usize) -> OleDropEffectHint {
        self.inner
            .borrow()
            .file_list_internal_items
            .get(index)
            .copied()
            .unwrap_or_else(OleDropEffectHint::none)
    }

    fn folder_tree_external_item_hint(&self) -> OleDropEffectHint {
        self.inner.borrow().folder_tree_external_item
    }

    fn folder_tree_internal_item_hint(&self, index: usize) -> OleDropEffectHint {
        self.inner
            .borrow()
            .folder_tree_internal_items
            .get(index)
            .copied()
            .unwrap_or_else(OleDropEffectHint::none)
    }
}

#[derive(Debug, Clone)]
struct OleDropFeedbackState {
    file_list_external_empty: OleDropEffectHint,
    file_list_internal_empty: OleDropEffectHint,
    file_list_internal_items: Vec<OleDropEffectHint>,
    folder_tree_external_item: OleDropEffectHint,
    folder_tree_internal_items: Vec<OleDropEffectHint>,
}

impl Default for OleDropFeedbackState {
    fn default() -> Self {
        Self {
            file_list_external_empty: OleDropEffectHint::copy_move(None),
            file_list_internal_empty: OleDropEffectHint::none(),
            file_list_internal_items: Vec::new(),
            folder_tree_external_item: OleDropEffectHint::copy_move(None),
            folder_tree_internal_items: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OleDropDataKind {
    ExternalPaths,
    InternalDrag,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OleDropData {
    ExternalPaths(Vec<PathBuf>),
    InternalDrag { drag_id: u64 },
}

impl OleDropData {
    pub fn is_internal(&self) -> bool {
        matches!(self, Self::InternalDrag { .. })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OleDropEvent {
    pub target: OleDropTargetKind,
    pub point: ScreenPoint,
    pub key_state: OleDropKeyState,
    pub allowed_effects: OleDropEffects,
    pub preferred_effect: Option<OleDropPreferredEffect>,
    pub data: OleDropData,
}

#[derive(Debug, Clone, Default)]
pub struct OleDropEventQueue {
    inner: Rc<RefCell<Vec<OleDropEvent>>>,
}

impl OleDropEventQueue {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn drain(&self) -> Vec<OleDropEvent> {
        self.inner.borrow_mut().drain(..).collect()
    }

    fn push(&self, event: OleDropEvent) -> bool {
        match self.inner.try_borrow_mut() {
            Ok(mut events) => {
                events.push(event);
                true
            }
            Err(_) => false,
        }
    }
}

pub struct OleDropTargetRegistration {
    hwnd: WindowHandle,
    target: *mut c_void,
    _apartment: OleApartment,
}

impl OleDropTargetRegistration {
    pub fn tick_drag_feedback(&self) {
        let Some(target) = (unsafe { (self.target as *mut DropTarget).as_ref() }) else {
            return;
        };
        tick_drag_feedback(target);
    }
}

impl Drop for OleDropTargetRegistration {
    fn drop(&mut self) {
        if let Some(target) = unsafe { (self.target as *mut DropTarget).as_ref() } {
            clear_drag_feedback_state(target);
        }
        // SAFETY: hwnd was successfully registered by RegisterDragDrop for this registration.
        unsafe {
            RevokeDragDrop(hwnd_from_window(self.hwnd));
            drop_target_release(self.target);
        }
    }
}

pub fn register_file_drop_target(
    hwnd: WindowHandle,
    owner: WindowHandle,
    notify_message: u32,
    target_kind: OleDropTargetKind,
    queue: OleDropEventQueue,
    feedback: OleDropFeedback,
    timer_config: OleDropFeedbackTimerConfig,
) -> ExplorerResult<OleDropTargetRegistration> {
    let apartment = OleApartment::initialize("initialize OLE drop target")?;
    let internal_format = internal_drag_format()?;
    let target = DropTarget::into_raw(DropTarget {
        vtable: &DROP_TARGET_VTABLE,
        ref_count: Cell::new(1),
        hwnd,
        target_kind,
        owner,
        notify_message,
        queue,
        feedback,
        internal_format,
        active_data_kind: Cell::new(None),
        active_allowed_effects: Cell::new(DROPEFFECT_NONE),
        active_preferred_effect: Cell::new(None),
        hover_tree_item: Cell::new(None),
        hover_tree_state: Cell::new(HoverExpandState::default()),
        active_point: Cell::new(None),
        active_key_state: Cell::new(0),
        drag_feedback_origin: Cell::new(None),
        drag_feedback_timer_active: Cell::new(false),
        drag_feedback_timer_id: timer_config.timer_id,
        drag_feedback_timer_interval_ms: timer_config.interval_ms,
        auto_scroll_edge_px: Cell::new(timer_config.auto_scroll_edge_px.max(1)),
    });

    // SAFETY: hwnd is a live child window and target is a valid IDropTarget COM object.
    let hresult = unsafe { RegisterDragDrop(hwnd_from_window(hwnd), target) };
    if hresult < 0 {
        // SAFETY: target is still owned by this function because registration failed.
        unsafe {
            drop_target_release(target);
        }
        return Err(ExplorerError::windows_hresult(
            "register drag drop target",
            "RegisterDragDrop",
            hresult,
            None,
        ));
    }

    Ok(OleDropTargetRegistration {
        hwnd,
        target,
        _apartment: apartment,
    })
}

pub fn start_internal_file_drag(drag_id: u64) -> ExplorerResult<OleDragSourceOutcome> {
    start_file_drag_source(drag_id, &[])
}

pub fn start_shell_file_drag(
    drag_id: u64,
    paths: &[PathBuf],
) -> ExplorerResult<OleDragSourceOutcome> {
    validate_shell_file_drag_paths(paths)?;

    start_file_drag_source(drag_id, paths)
}

pub fn validate_shell_file_drag_paths(paths: &[PathBuf]) -> ExplorerResult<()> {
    hdrop::validate_hdrop_paths(paths, FileDropUsage::DragSource)
}

fn start_file_drag_source(drag_id: u64, paths: &[PathBuf]) -> ExplorerResult<OleDragSourceOutcome> {
    let _apartment = OleApartment::initialize("initialize OLE drag source")?;
    let internal_format = internal_drag_format()?;
    let data_object = OwnedComReference::new(
        InternalDragDataObject::into_raw(InternalDragDataObject {
            vtable: &INTERNAL_DRAG_DATA_OBJECT_VTABLE,
            ref_count: Cell::new(1),
            internal_format,
            drag_id,
            paths: paths.to_vec(),
        }),
        internal_drag_data_object_release,
    );
    let drop_source_ptr = DropSource::into_raw(DropSource {
        vtable: &DROP_SOURCE_VTABLE,
        ref_count: Cell::new(1),
        cancelled_by_escape: Cell::new(false),
    });
    let drop_source = OwnedComReference::new(drop_source_ptr, drop_source_release);

    let mut effect = DROPEFFECT_NONE;
    // SAFETY: data_object and drop_source are valid COM objects kept alive for the call.
    let hresult = unsafe {
        DoDragDrop(
            data_object.as_raw(),
            drop_source.as_raw(),
            DROPEFFECT_COPY | DROPEFFECT_MOVE,
            &mut effect,
        )
    };

    let cancelled_by_escape = drop_source_cancelled_by_escape(drop_source_ptr);
    let outcome = drag_source_outcome(hresult, effect, cancelled_by_escape)?;
    Ok(outcome)
}

fn drag_source_outcome(
    hresult: HRESULT,
    effect: u32,
    cancelled_by_escape: bool,
) -> ExplorerResult<OleDragSourceOutcome> {
    if hresult == DRAGDROP_S_CANCEL {
        return Ok(if cancelled_by_escape {
            OleDragSourceOutcome::Cancelled
        } else {
            OleDragSourceOutcome::NoDrop
        });
    }
    if hresult == DRAGDROP_S_DROP || hresult == S_OK {
        return Ok(drag_source_outcome_from_effect(effect));
    }
    if hresult < 0 {
        return Err(ExplorerError::windows_hresult(
            "perform file drag",
            "DoDragDrop",
            hresult,
            None,
        ));
    }

    Ok(drag_source_outcome_from_effect(effect))
}

fn drag_source_outcome_from_effect(effect: u32) -> OleDragSourceOutcome {
    if effect & DROPEFFECT_MOVE != 0 {
        OleDragSourceOutcome::Move
    } else if effect & DROPEFFECT_COPY != 0 {
        OleDragSourceOutcome::Copy
    } else {
        OleDragSourceOutcome::NoDrop
    }
}

fn drop_source_cancelled_by_escape(drop_source: *mut c_void) -> bool {
    let Some(drop_source) = (unsafe { (drop_source as *mut DropSource).as_ref() }) else {
        return false;
    };
    drop_source.cancelled_by_escape.get()
}

struct OleApartment {
    should_uninitialize: bool,
}

impl OleApartment {
    fn initialize(operation: &'static str) -> ExplorerResult<Self> {
        // SAFETY: OleInitialize accepts a null reserved pointer and initializes OLE on this thread.
        let hresult = unsafe { OleInitialize(null()) };
        if hresult == RPC_E_CHANGED_MODE || hresult < 0 {
            return Err(ExplorerError::windows_hresult(
                operation,
                "OleInitialize",
                hresult,
                None,
            ));
        }

        Ok(Self {
            should_uninitialize: true,
        })
    }
}

impl Drop for OleApartment {
    fn drop(&mut self) {
        if self.should_uninitialize {
            // SAFETY: this balances a successful OleInitialize call on the current thread.
            unsafe {
                OleUninitialize();
            }
        }
    }
}

// Owns one COM reference and releases it with the matching vtable Release function.
struct OwnedComReference {
    ptr: *mut c_void,
    release: unsafe extern "system" fn(*mut c_void) -> u32,
}

impl OwnedComReference {
    fn new(ptr: *mut c_void, release: unsafe extern "system" fn(*mut c_void) -> u32) -> Self {
        Self { ptr, release }
    }

    fn as_raw(&self) -> *mut c_void {
        self.ptr
    }
}

impl Drop for OwnedComReference {
    fn drop(&mut self) {
        if !self.ptr.is_null() {
            // SAFETY: ptr is an owned COM object reference paired with this release function.
            unsafe {
                (self.release)(self.ptr);
            }
        }
    }
}

#[repr(C)]
struct DropTarget {
    vtable: *const IDropTargetVtbl,
    ref_count: Cell<u32>,
    hwnd: WindowHandle,
    target_kind: OleDropTargetKind,
    owner: WindowHandle,
    notify_message: u32,
    queue: OleDropEventQueue,
    feedback: OleDropFeedback,
    internal_format: u16,
    active_data_kind: Cell<Option<OleDropDataKind>>,
    active_allowed_effects: Cell<u32>,
    active_preferred_effect: Cell<Option<OleDropPreferredEffect>>,
    hover_tree_item: Cell<Option<TreeViewItemHandle>>,
    hover_tree_state: Cell<HoverExpandState>,
    active_point: Cell<Option<ScreenPoint>>,
    active_key_state: Cell<u32>,
    drag_feedback_origin: Cell<Option<Instant>>,
    drag_feedback_timer_active: Cell<bool>,
    drag_feedback_timer_id: usize,
    drag_feedback_timer_interval_ms: u32,
    auto_scroll_edge_px: Cell<i32>,
}

impl DropTarget {
    fn into_raw(self) -> *mut c_void {
        Box::into_raw(Box::new(self)).cast()
    }
}

#[repr(C)]
struct DropSource {
    vtable: *const IDropSourceVtbl,
    ref_count: Cell<u32>,
    cancelled_by_escape: Cell<bool>,
}

impl DropSource {
    fn into_raw(self) -> *mut c_void {
        Box::into_raw(Box::new(self)).cast()
    }
}

#[repr(C)]
struct InternalDragDataObject {
    vtable: *const IDataObjectVtbl,
    ref_count: Cell<u32>,
    internal_format: u16,
    drag_id: u64,
    paths: Vec<PathBuf>,
}

impl InternalDragDataObject {
    fn into_raw(self) -> *mut c_void {
        Box::into_raw(Box::new(self)).cast()
    }

    fn format_etc_entries(&self) -> Vec<FORMATETC> {
        drag_data_format_entries(self.internal_format, !self.paths.is_empty())
    }
}

#[repr(C)]
struct FormatEtcEnumerator {
    vtable: *const IEnumFormatEtcVtbl,
    ref_count: Cell<u32>,
    formats: Vec<FORMATETC>,
    index: Cell<usize>,
}

impl FormatEtcEnumerator {
    fn into_raw(formats: Vec<FORMATETC>, index: usize) -> *mut c_void {
        Box::into_raw(Box::new(Self {
            vtable: &FORMAT_ETC_ENUMERATOR_VTABLE,
            ref_count: Cell::new(1),
            formats,
            index: Cell::new(index),
        }))
        .cast()
    }
}

#[repr(C)]
struct IUnknownVtbl {
    query_interface:
        unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> HRESULT,
    add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
    release: unsafe extern "system" fn(*mut c_void) -> u32,
}

#[repr(C)]
struct IDropTargetVtbl {
    base: IUnknownVtbl,
    drag_enter:
        unsafe extern "system" fn(*mut c_void, *mut c_void, u32, POINTL, *mut u32) -> HRESULT,
    drag_over: unsafe extern "system" fn(*mut c_void, u32, POINTL, *mut u32) -> HRESULT,
    drag_leave: unsafe extern "system" fn(*mut c_void) -> HRESULT,
    drop: unsafe extern "system" fn(*mut c_void, *mut c_void, u32, POINTL, *mut u32) -> HRESULT,
}

#[repr(C)]
struct IDropSourceVtbl {
    base: IUnknownVtbl,
    query_continue_drag: unsafe extern "system" fn(*mut c_void, i32, u32) -> HRESULT,
    give_feedback: unsafe extern "system" fn(*mut c_void, u32) -> HRESULT,
}

#[repr(C)]
struct IDataObjectVtbl {
    base: IUnknownVtbl,
    get_data: unsafe extern "system" fn(*mut c_void, *mut FORMATETC, *mut STGMEDIUM) -> HRESULT,
    get_data_here:
        unsafe extern "system" fn(*mut c_void, *mut FORMATETC, *mut STGMEDIUM) -> HRESULT,
    query_get_data: unsafe extern "system" fn(*mut c_void, *mut FORMATETC) -> HRESULT,
    get_canonical_format_etc:
        unsafe extern "system" fn(*mut c_void, *mut FORMATETC, *mut FORMATETC) -> HRESULT,
    set_data:
        unsafe extern "system" fn(*mut c_void, *mut FORMATETC, *mut STGMEDIUM, i32) -> HRESULT,
    enum_format_etc: unsafe extern "system" fn(*mut c_void, u32, *mut *mut c_void) -> HRESULT,
    d_advise: unsafe extern "system" fn(
        *mut c_void,
        *mut FORMATETC,
        u32,
        *mut c_void,
        *mut u32,
    ) -> HRESULT,
    d_unadvise: unsafe extern "system" fn(*mut c_void, u32) -> HRESULT,
    enum_d_advise: unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> HRESULT,
}

#[repr(C)]
struct IEnumFormatEtcVtbl {
    base: IUnknownVtbl,
    next: unsafe extern "system" fn(*mut c_void, u32, *mut FORMATETC, *mut u32) -> HRESULT,
    skip: unsafe extern "system" fn(*mut c_void, u32) -> HRESULT,
    reset: unsafe extern "system" fn(*mut c_void) -> HRESULT,
    clone: unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> HRESULT,
}

static DROP_TARGET_VTABLE: IDropTargetVtbl = IDropTargetVtbl {
    base: IUnknownVtbl {
        query_interface: drop_target_query_interface,
        add_ref: drop_target_add_ref,
        release: drop_target_release,
    },
    drag_enter: drop_target_drag_enter,
    drag_over: drop_target_drag_over,
    drag_leave: drop_target_drag_leave,
    drop: drop_target_drop,
};

static DROP_SOURCE_VTABLE: IDropSourceVtbl = IDropSourceVtbl {
    base: IUnknownVtbl {
        query_interface: drop_source_query_interface,
        add_ref: drop_source_add_ref,
        release: drop_source_release,
    },
    query_continue_drag: drop_source_query_continue_drag,
    give_feedback: drop_source_give_feedback,
};

static INTERNAL_DRAG_DATA_OBJECT_VTABLE: IDataObjectVtbl = IDataObjectVtbl {
    base: IUnknownVtbl {
        query_interface: internal_drag_data_object_query_interface,
        add_ref: internal_drag_data_object_add_ref,
        release: internal_drag_data_object_release,
    },
    get_data: internal_drag_data_object_get_data,
    get_data_here: internal_drag_data_object_get_data_here,
    query_get_data: internal_drag_data_object_query_get_data,
    get_canonical_format_etc: internal_drag_data_object_get_canonical_format_etc,
    set_data: internal_drag_data_object_set_data,
    enum_format_etc: internal_drag_data_object_enum_format_etc,
    d_advise: internal_drag_data_object_d_advise,
    d_unadvise: internal_drag_data_object_d_unadvise,
    enum_d_advise: internal_drag_data_object_enum_d_advise,
};

static FORMAT_ETC_ENUMERATOR_VTABLE: IEnumFormatEtcVtbl = IEnumFormatEtcVtbl {
    base: IUnknownVtbl {
        query_interface: format_etc_enumerator_query_interface,
        add_ref: format_etc_enumerator_add_ref,
        release: format_etc_enumerator_release,
    },
    next: format_etc_enumerator_next,
    skip: format_etc_enumerator_skip,
    reset: format_etc_enumerator_reset,
    clone: format_etc_enumerator_clone,
};

unsafe extern "system" fn drop_target_query_interface(
    this: *mut c_void,
    iid: *const GUID,
    out: *mut *mut c_void,
) -> HRESULT {
    query_interface(this, iid, out, &IID_IDROP_TARGET, drop_target_add_ref)
}

unsafe extern "system" fn drop_target_add_ref(this: *mut c_void) -> u32 {
    let Some(target) = (unsafe { (this as *mut DropTarget).as_ref() }) else {
        return 0;
    };
    let next = target.ref_count.get().saturating_add(1);
    target.ref_count.set(next);
    next
}

unsafe extern "system" fn drop_target_release(this: *mut c_void) -> u32 {
    let Some(target) = (unsafe { (this as *mut DropTarget).as_ref() }) else {
        return 0;
    };
    let next = target.ref_count.get().saturating_sub(1);
    target.ref_count.set(next);
    if next == 0 {
        // SAFETY: the final COM reference owns the allocation produced by Box::into_raw.
        unsafe {
            drop(Box::from_raw(this as *mut DropTarget));
        }
    }
    next
}

unsafe extern "system" fn drop_target_drag_enter(
    this: *mut c_void,
    data_object: *mut c_void,
    key_state: u32,
    point: POINTL,
    effect: *mut u32,
) -> HRESULT {
    let Some(target) = (unsafe { (this as *mut DropTarget).as_ref() }) else {
        return E_POINTER;
    };
    clear_drag_feedback_state(target);
    let data_kind = drop_data_kind(data_object, target.internal_format);
    target.active_data_kind.set(data_kind);
    target
        .active_preferred_effect
        .set(preferred_effect_for_data_object(data_object, data_kind));
    let allowed = effect_mask(effect);
    target.active_allowed_effects.set(allowed);
    set_drag_effect_for_target(target, data_kind, allowed, key_state, point, effect);
    S_OK
}

unsafe extern "system" fn drop_target_drag_over(
    this: *mut c_void,
    key_state: u32,
    point: POINTL,
    effect: *mut u32,
) -> HRESULT {
    let Some(target) = (unsafe { (this as *mut DropTarget).as_ref() }) else {
        return E_POINTER;
    };
    set_drag_effect_for_target(
        target,
        target.active_data_kind.get(),
        target.active_allowed_effects.get(),
        key_state,
        point,
        effect,
    );
    S_OK
}

unsafe extern "system" fn drop_target_drag_leave(this: *mut c_void) -> HRESULT {
    if let Some(target) = unsafe { (this as *mut DropTarget).as_ref() } {
        target.active_data_kind.set(None);
        target.active_allowed_effects.set(DROPEFFECT_NONE);
        target.active_preferred_effect.set(None);
        clear_drag_feedback_state(target);
    }
    S_OK
}

unsafe extern "system" fn drop_target_drop(
    this: *mut c_void,
    data_object: *mut c_void,
    key_state: u32,
    point: POINTL,
    effect: *mut u32,
) -> HRESULT {
    let Some(target) = (unsafe { (this as *mut DropTarget).as_ref() }) else {
        set_no_effect(effect);
        return E_POINTER;
    };

    let active_allowed = target.active_allowed_effects.get();
    let allowed_mask = if active_allowed == DROPEFFECT_NONE {
        effect_mask(effect)
    } else {
        active_allowed
    };
    let data_kind = target
        .active_data_kind
        .get()
        .or_else(|| drop_data_kind(data_object, target.internal_format));
    let preferred_effect = target
        .active_preferred_effect
        .get()
        .or_else(|| preferred_effect_for_data_object(data_object, data_kind));
    target.active_preferred_effect.set(preferred_effect);
    if set_drag_effect_for_target(target, data_kind, allowed_mask, key_state, point, effect)
        == DROPEFFECT_NONE
    {
        target.active_data_kind.set(None);
        target.active_allowed_effects.set(DROPEFFECT_NONE);
        target.active_preferred_effect.set(None);
        clear_drag_feedback_state(target);
        return S_OK;
    }

    match read_drop_data(data_object, target.internal_format) {
        Ok(Some(data)) => {
            let event = OleDropEvent {
                target: target.target_kind,
                point: ScreenPoint {
                    x: point.x,
                    y: point.y,
                },
                key_state: OleDropKeyState::from_raw(key_state),
                allowed_effects: OleDropEffects::from_mask(allowed_mask),
                preferred_effect,
                data,
            };
            if post_drop_event_message(target.owner, target.notify_message)
                && target.queue.push(event)
            {
                set_drag_effect_for_target(
                    target,
                    data_kind,
                    allowed_mask,
                    key_state,
                    point,
                    effect,
                );
            } else {
                set_no_effect(effect);
            }
        }
        Ok(None) => {
            set_no_effect(effect);
        }
        Err(error) => {
            eprintln!("failed to read dropped files: {error}");
            set_no_effect(effect);
        }
    }

    target.active_data_kind.set(None);
    target.active_allowed_effects.set(DROPEFFECT_NONE);
    target.active_preferred_effect.set(None);
    clear_drag_feedback_state(target);
    S_OK
}

unsafe extern "system" fn drop_source_query_interface(
    this: *mut c_void,
    iid: *const GUID,
    out: *mut *mut c_void,
) -> HRESULT {
    query_interface(this, iid, out, &IID_IDROP_SOURCE, drop_source_add_ref)
}

unsafe extern "system" fn drop_source_add_ref(this: *mut c_void) -> u32 {
    let Some(source) = (unsafe { (this as *mut DropSource).as_ref() }) else {
        return 0;
    };
    let next = source.ref_count.get().saturating_add(1);
    source.ref_count.set(next);
    next
}

unsafe extern "system" fn drop_source_release(this: *mut c_void) -> u32 {
    let Some(source) = (unsafe { (this as *mut DropSource).as_ref() }) else {
        return 0;
    };
    let next = source.ref_count.get().saturating_sub(1);
    source.ref_count.set(next);
    if next == 0 {
        // SAFETY: the final COM reference owns the allocation produced by Box::into_raw.
        unsafe {
            drop(Box::from_raw(this as *mut DropSource));
        }
    }
    next
}

unsafe extern "system" fn drop_source_query_continue_drag(
    this: *mut c_void,
    escape_pressed: i32,
    key_state: u32,
) -> HRESULT {
    if escape_pressed != 0 {
        if let Some(source) = unsafe { (this as *mut DropSource).as_ref() } {
            source.cancelled_by_escape.set(true);
        }
        DRAGDROP_S_CANCEL
    } else if key_state & MK_LBUTTON_VALUE == 0 {
        DRAGDROP_S_DROP
    } else {
        S_OK
    }
}

unsafe extern "system" fn drop_source_give_feedback(_this: *mut c_void, _effect: u32) -> HRESULT {
    DRAGDROP_S_USEDEFAULTCURSORS
}

unsafe extern "system" fn internal_drag_data_object_query_interface(
    this: *mut c_void,
    iid: *const GUID,
    out: *mut *mut c_void,
) -> HRESULT {
    query_interface(
        this,
        iid,
        out,
        &IID_IDATA_OBJECT,
        internal_drag_data_object_add_ref,
    )
}

unsafe extern "system" fn internal_drag_data_object_add_ref(this: *mut c_void) -> u32 {
    let Some(data_object) = (unsafe { (this as *mut InternalDragDataObject).as_ref() }) else {
        return 0;
    };
    let next = data_object.ref_count.get().saturating_add(1);
    data_object.ref_count.set(next);
    next
}

unsafe extern "system" fn internal_drag_data_object_release(this: *mut c_void) -> u32 {
    let Some(data_object) = (unsafe { (this as *mut InternalDragDataObject).as_ref() }) else {
        return 0;
    };
    let next = data_object.ref_count.get().saturating_sub(1);
    data_object.ref_count.set(next);
    if next == 0 {
        // SAFETY: the final COM reference owns the allocation produced by Box::into_raw.
        unsafe {
            drop(Box::from_raw(this as *mut InternalDragDataObject));
        }
    }
    next
}

unsafe extern "system" fn internal_drag_data_object_get_data(
    this: *mut c_void,
    format: *mut FORMATETC,
    medium: *mut STGMEDIUM,
) -> HRESULT {
    let Some(data_object) = (unsafe { (this as *mut InternalDragDataObject).as_ref() }) else {
        return E_POINTER;
    };
    if medium.is_null() {
        return E_POINTER;
    }

    let value = if format_matches(format, data_object.internal_format) {
        internal_drag_id_medium(data_object.drag_id)
    } else if !data_object.paths.is_empty() && format_matches(format, hdrop::CF_HDROP_FORMAT) {
        file_drop_medium(&data_object.paths)
    } else {
        return DV_E_FORMATETC;
    };

    match value {
        Ok(value) => {
            // SAFETY: medium is a valid out pointer supplied by the caller.
            unsafe {
                *medium = value;
            }
            S_OK
        }
        Err(error) => {
            eprintln!("failed to allocate internal drag data: {error}");
            E_INVALIDARG
        }
    }
}

unsafe extern "system" fn internal_drag_data_object_get_data_here(
    _this: *mut c_void,
    _format: *mut FORMATETC,
    _medium: *mut STGMEDIUM,
) -> HRESULT {
    E_NOTIMPL
}

unsafe extern "system" fn internal_drag_data_object_query_get_data(
    this: *mut c_void,
    format: *mut FORMATETC,
) -> HRESULT {
    let Some(data_object) = (unsafe { (this as *mut InternalDragDataObject).as_ref() }) else {
        return E_POINTER;
    };
    if format_matches(format, data_object.internal_format)
        || (!data_object.paths.is_empty() && format_matches(format, hdrop::CF_HDROP_FORMAT))
    {
        S_OK
    } else {
        DV_E_FORMATETC
    }
}

unsafe extern "system" fn internal_drag_data_object_get_canonical_format_etc(
    _this: *mut c_void,
    _format_in: *mut FORMATETC,
    format_out: *mut FORMATETC,
) -> HRESULT {
    if !format_out.is_null() {
        // SAFETY: format_out is a caller-supplied out pointer.
        unsafe {
            (*format_out).ptd = null_mut();
        }
    }
    E_NOTIMPL
}

unsafe extern "system" fn internal_drag_data_object_set_data(
    _this: *mut c_void,
    _format: *mut FORMATETC,
    _medium: *mut STGMEDIUM,
    _release: i32,
) -> HRESULT {
    E_NOTIMPL
}

unsafe extern "system" fn internal_drag_data_object_enum_format_etc(
    this: *mut c_void,
    direction: u32,
    enum_format: *mut *mut c_void,
) -> HRESULT {
    if enum_format.is_null() {
        return E_POINTER;
    }
    // SAFETY: enum_format is a caller-supplied out pointer checked above.
    unsafe {
        *enum_format = null_mut();
    }
    if direction != DATADIR_GET_VALUE {
        return E_NOTIMPL;
    }

    let Some(data_object) = (unsafe { (this as *mut InternalDragDataObject).as_ref() }) else {
        return E_POINTER;
    };
    let formats = data_object.format_etc_entries();
    // SAFETY: enum_format is a valid out pointer and receives a new IEnumFORMATETC reference.
    unsafe {
        *enum_format = FormatEtcEnumerator::into_raw(formats, 0);
    }
    S_OK
}

unsafe extern "system" fn internal_drag_data_object_d_advise(
    _this: *mut c_void,
    _format: *mut FORMATETC,
    _advf: u32,
    _sink: *mut c_void,
    _connection: *mut u32,
) -> HRESULT {
    windows_sys::Win32::Foundation::OLE_E_ADVISENOTSUPPORTED
}

unsafe extern "system" fn internal_drag_data_object_d_unadvise(
    _this: *mut c_void,
    _connection: u32,
) -> HRESULT {
    windows_sys::Win32::Foundation::OLE_E_ADVISENOTSUPPORTED
}

unsafe extern "system" fn internal_drag_data_object_enum_d_advise(
    _this: *mut c_void,
    _enum_advise: *mut *mut c_void,
) -> HRESULT {
    windows_sys::Win32::Foundation::OLE_E_ADVISENOTSUPPORTED
}

unsafe extern "system" fn format_etc_enumerator_query_interface(
    this: *mut c_void,
    iid: *const GUID,
    out: *mut *mut c_void,
) -> HRESULT {
    query_interface(
        this,
        iid,
        out,
        &IID_IENUM_FORMAT_ETC,
        format_etc_enumerator_add_ref,
    )
}

unsafe extern "system" fn format_etc_enumerator_add_ref(this: *mut c_void) -> u32 {
    let Some(enumerator) = (unsafe { (this as *mut FormatEtcEnumerator).as_ref() }) else {
        return 0;
    };
    let next = enumerator.ref_count.get().saturating_add(1);
    enumerator.ref_count.set(next);
    next
}

unsafe extern "system" fn format_etc_enumerator_release(this: *mut c_void) -> u32 {
    let Some(enumerator) = (unsafe { (this as *mut FormatEtcEnumerator).as_ref() }) else {
        return 0;
    };
    let next = enumerator.ref_count.get().saturating_sub(1);
    enumerator.ref_count.set(next);
    if next == 0 {
        // SAFETY: the final COM reference owns the allocation produced by Box::into_raw.
        unsafe {
            drop(Box::from_raw(this as *mut FormatEtcEnumerator));
        }
    }
    next
}

unsafe extern "system" fn format_etc_enumerator_next(
    this: *mut c_void,
    count: u32,
    out_formats: *mut FORMATETC,
    fetched_count: *mut u32,
) -> HRESULT {
    if out_formats.is_null() || (count > 1 && fetched_count.is_null()) {
        return E_POINTER;
    }
    if !fetched_count.is_null() {
        // SAFETY: fetched_count is a caller-supplied out pointer checked above.
        unsafe {
            *fetched_count = 0;
        }
    }

    let Some(enumerator) = (unsafe { (this as *mut FormatEtcEnumerator).as_ref() }) else {
        return E_POINTER;
    };

    let requested = usize::try_from(count).unwrap_or(usize::MAX);
    let start = enumerator.index.get();
    let available = enumerator.formats.len().saturating_sub(start);
    let fetched = requested.min(available);

    for offset in 0..fetched {
        // SAFETY: out_formats points to at least count FORMATETC slots by COM contract.
        unsafe {
            *out_formats.add(offset) = clone_format_etc(&enumerator.formats[start + offset]);
        }
    }

    enumerator.index.set(start + fetched);
    if !fetched_count.is_null() {
        let fetched = u32::try_from(fetched).unwrap_or(u32::MAX);
        // SAFETY: fetched_count is a caller-supplied out pointer checked above.
        unsafe {
            *fetched_count = fetched;
        }
    }

    if fetched == requested {
        S_OK
    } else {
        S_FALSE
    }
}

unsafe extern "system" fn format_etc_enumerator_skip(this: *mut c_void, count: u32) -> HRESULT {
    let Some(enumerator) = (unsafe { (this as *mut FormatEtcEnumerator).as_ref() }) else {
        return E_POINTER;
    };
    let requested = usize::try_from(count).unwrap_or(usize::MAX);
    let start = enumerator.index.get();
    let next = start
        .saturating_add(requested)
        .min(enumerator.formats.len());
    enumerator.index.set(next);
    if next.saturating_sub(start) == requested {
        S_OK
    } else {
        S_FALSE
    }
}

unsafe extern "system" fn format_etc_enumerator_reset(this: *mut c_void) -> HRESULT {
    let Some(enumerator) = (unsafe { (this as *mut FormatEtcEnumerator).as_ref() }) else {
        return E_POINTER;
    };
    enumerator.index.set(0);
    S_OK
}

unsafe extern "system" fn format_etc_enumerator_clone(
    this: *mut c_void,
    out: *mut *mut c_void,
) -> HRESULT {
    if out.is_null() {
        return E_POINTER;
    }
    // SAFETY: out is a caller-supplied out pointer checked above.
    unsafe {
        *out = null_mut();
    }

    let Some(enumerator) = (unsafe { (this as *mut FormatEtcEnumerator).as_ref() }) else {
        return E_POINTER;
    };
    let formats = enumerator
        .formats
        .iter()
        .map(clone_format_etc)
        .collect::<Vec<_>>();
    // SAFETY: out is valid and receives a new IEnumFORMATETC reference.
    unsafe {
        *out = FormatEtcEnumerator::into_raw(formats, enumerator.index.get());
    }
    S_OK
}

unsafe fn query_interface(
    this: *mut c_void,
    iid: *const GUID,
    out: *mut *mut c_void,
    interface_iid: &GUID,
    add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
) -> HRESULT {
    if out.is_null() {
        return E_POINTER;
    }
    // SAFETY: out is a valid pointer checked above.
    unsafe {
        *out = null_mut();
    }
    if this.is_null() || iid.is_null() {
        return E_NOINTERFACE;
    }

    // SAFETY: iid is non-null and points to the requested interface id.
    let requested = unsafe { *iid };
    if guid_eq(&requested, &IID_IUNKNOWN) || guid_eq(&requested, interface_iid) {
        // SAFETY: out is valid and this is the requested COM identity pointer.
        unsafe {
            *out = this;
            add_ref(this);
        }
        S_OK
    } else {
        E_NOINTERFACE
    }
}

fn guid_eq(left: &GUID, right: &GUID) -> bool {
    left.data1 == right.data1
        && left.data2 == right.data2
        && left.data3 == right.data3
        && left.data4 == right.data4
}

fn set_drag_effect_for_target(
    target: &DropTarget,
    data_kind: Option<OleDropDataKind>,
    allowed: u32,
    key_state: u32,
    point: POINTL,
    effect: *mut u32,
) -> u32 {
    if effect.is_null() {
        return DROPEFFECT_NONE;
    }

    let screen_point = ScreenPoint {
        x: point.x,
        y: point.y,
    };
    target.active_point.set(Some(screen_point));
    target.active_key_state.set(key_state);
    if data_kind.is_some() && allowed != DROPEFFECT_NONE {
        ensure_drag_feedback_timer(target);
    } else {
        clear_drag_feedback_state(target);
    }

    let hint = target_effect_hint(target, data_kind, point);
    let selected = selected_effect(key_state, allowed, hint);
    // SAFETY: effect is a valid in/out pointer from OLE when non-null.
    unsafe {
        *effect = selected;
    }

    if selected != DROPEFFECT_NONE {
        update_tree_hover_expand(target, screen_point, hint);
    } else {
        reset_tree_hover(target);
    }

    selected
}

fn target_effect_hint(
    target: &DropTarget,
    data_kind: Option<OleDropDataKind>,
    point: POINTL,
) -> OleDropEffectHint {
    let Some(data_kind) = data_kind else {
        return OleDropEffectHint::none();
    };
    let point = ScreenPoint {
        x: point.x,
        y: point.y,
    };

    match target.target_kind {
        OleDropTargetKind::FileList => hint_with_external_preferred(
            file_list_effect_hint(target, data_kind, point),
            data_kind,
            target.active_preferred_effect.get(),
        ),
        OleDropTargetKind::FolderTree => hint_with_external_preferred(
            folder_tree_effect_hint(target, data_kind, point),
            data_kind,
            target.active_preferred_effect.get(),
        ),
    }
}

fn hint_with_external_preferred(
    hint: OleDropEffectHint,
    data_kind: OleDropDataKind,
    preferred_effect: Option<OleDropPreferredEffect>,
) -> OleDropEffectHint {
    if data_kind == OleDropDataKind::ExternalPaths
        && hint.allows_any()
        && preferred_effect.is_some()
    {
        OleDropEffectHint {
            default: preferred_effect,
            ..hint
        }
    } else {
        hint
    }
}

fn file_list_effect_hint(
    target: &DropTarget,
    data_kind: OleDropDataKind,
    point: ScreenPoint,
) -> OleDropEffectHint {
    match ui::list_view_item_at_screen_point(target.hwnd, point) {
        Ok(Some(index)) if data_kind == OleDropDataKind::InternalDrag => {
            target.feedback.file_list_internal_item_hint(index)
        }
        Ok(None) if data_kind == OleDropDataKind::InternalDrag => {
            target.feedback.file_list_internal_empty_hint()
        }
        Ok(None) if data_kind == OleDropDataKind::ExternalPaths => {
            target.feedback.file_list_external_empty_hint()
        }
        Ok(_) => OleDropEffectHint::none(),
        Err(error) => {
            eprintln!("failed to resolve file list drop target: {error}");
            OleDropEffectHint::none()
        }
    }
}

fn folder_tree_effect_hint(
    target: &DropTarget,
    data_kind: OleDropDataKind,
    point: ScreenPoint,
) -> OleDropEffectHint {
    let item = match ui::tree_view_item_at_screen_point(target.hwnd, point) {
        Ok(Some(item)) => item,
        Ok(None) => return OleDropEffectHint::none(),
        Err(error) => {
            eprintln!("failed to resolve folder tree drop target: {error}");
            return OleDropEffectHint::none();
        }
    };

    match data_kind {
        OleDropDataKind::ExternalPaths => target.feedback.folder_tree_external_item_hint(),
        OleDropDataKind::InternalDrag => match ui::tree_view_item_value(target.hwnd, item) {
            Ok(Some(value)) => target.feedback.folder_tree_internal_item_hint(value.get()),
            Ok(None) => OleDropEffectHint::none(),
            Err(error) => {
                eprintln!("failed to read folder tree drop target value: {error}");
                OleDropEffectHint::none()
            }
        },
    }
}

fn tick_drag_feedback(target: &DropTarget) {
    let Some(data_kind) = target.active_data_kind.get() else {
        clear_drag_feedback_state(target);
        return;
    };
    let Some(point) = target.active_point.get() else {
        clear_drag_feedback_state(target);
        return;
    };

    update_edge_auto_scroll(target, point);

    let pointl = POINTL {
        x: point.x,
        y: point.y,
    };
    let hint = target_effect_hint(target, Some(data_kind), pointl);
    let selected = selected_effect(
        target.active_key_state.get(),
        target.active_allowed_effects.get(),
        hint,
    );
    if selected != DROPEFFECT_NONE {
        update_tree_hover_expand(target, point, hint);
    } else {
        reset_tree_hover(target);
    }
}

fn update_tree_hover_expand(target: &DropTarget, point: ScreenPoint, hint: OleDropEffectHint) {
    if target.target_kind != OleDropTargetKind::FolderTree || !hint.allows_any() {
        reset_tree_hover(target);
        return;
    }

    let item = match ui::tree_view_item_at_screen_point(target.hwnd, point) {
        Ok(Some(item)) => item,
        Ok(None) => {
            reset_tree_hover(target);
            return;
        }
        Err(error) => {
            eprintln!("failed to resolve folder tree hover target: {error}");
            reset_tree_hover(target);
            return;
        }
    };
    let value = match ui::tree_view_item_value(target.hwnd, item) {
        Ok(Some(value)) => value,
        Ok(None) => {
            reset_tree_hover(target);
            return;
        }
        Err(error) => {
            eprintln!("failed to read folder tree hover target value: {error}");
            reset_tree_hover(target);
            return;
        }
    };

    let now_ms = drag_feedback_elapsed_ms(target);
    let mut state = target.hover_tree_state.get();
    let action = state.update(Some(value.get()), now_ms, TREE_HOVER_EXPAND_DELAY_MS);
    target.hover_tree_state.set(state);
    target.hover_tree_item.set(Some(item));

    if matches!(action, HoverExpandAction::Expand { .. }) {
        if let Err(error) = ui::expand_tree_view_item(target.hwnd, item) {
            eprintln!("failed to expand folder tree hover target: {error}");
        }
    }
}

fn update_edge_auto_scroll(target: &DropTarget, point: ScreenPoint) {
    match ui::vertical_auto_scroll_direction(target.hwnd, point, target.auto_scroll_edge_px.get()) {
        Ok(Some(direction)) => ui::scroll_window_vertically(target.hwnd, direction),
        Ok(None) => {}
        Err(error) => eprintln!("failed to resolve drag auto-scroll edge: {error}"),
    }
}

fn ensure_drag_feedback_timer(target: &DropTarget) {
    if target.drag_feedback_timer_active.get() {
        return;
    }

    match ui::set_window_timer(
        target.owner,
        target.drag_feedback_timer_id,
        target.drag_feedback_timer_interval_ms,
    ) {
        Ok(()) => target.drag_feedback_timer_active.set(true),
        Err(error) => eprintln!("failed to start drag feedback timer: {error}"),
    }
}

fn stop_drag_feedback_timer(target: &DropTarget) {
    if !target.drag_feedback_timer_active.get() {
        return;
    }

    match ui::kill_window_timer(target.owner, target.drag_feedback_timer_id) {
        Ok(()) => target.drag_feedback_timer_active.set(false),
        Err(error) => {
            eprintln!("failed to stop drag feedback timer: {error}");
            target.drag_feedback_timer_active.set(false);
        }
    }
}

fn drag_feedback_elapsed_ms(target: &DropTarget) -> u64 {
    let now = Instant::now();
    let Some(origin) = target.drag_feedback_origin.get() else {
        target.drag_feedback_origin.set(Some(now));
        return 0;
    };

    let millis = origin.elapsed().as_millis();
    if millis > u128::from(u64::MAX) {
        u64::MAX
    } else {
        millis as u64
    }
}

fn clear_drag_feedback_state(target: &DropTarget) {
    target.active_point.set(None);
    target.active_key_state.set(0);
    target.drag_feedback_origin.set(None);
    reset_tree_hover(target);
    stop_drag_feedback_timer(target);
}

fn reset_tree_hover(target: &DropTarget) {
    target.hover_tree_item.set(None);
    target.hover_tree_state.set(HoverExpandState::default());
}

fn set_no_effect(effect: *mut u32) {
    if !effect.is_null() {
        // SAFETY: effect is a valid in/out pointer from OLE when non-null.
        unsafe {
            *effect = DROPEFFECT_NONE;
        }
    }
}

fn effect_mask(effect: *mut u32) -> u32 {
    if effect.is_null() {
        DROPEFFECT_NONE
    } else {
        // SAFETY: effect is a valid in/out pointer from OLE when non-null.
        unsafe { *effect }
    }
}

fn selected_effect(key_state: u32, allowed: u32, hint: OleDropEffectHint) -> u32 {
    DropEffectSelection::from_raw(key_state, allowed, hint).effect_mask()
}

fn drop_data_kind(data_object: *mut c_void, internal_format: u16) -> Option<OleDropDataKind> {
    if data_object_supports_format(data_object, internal_format) {
        Some(OleDropDataKind::InternalDrag)
    } else if data_object_supports_format(data_object, hdrop::CF_HDROP_FORMAT) {
        Some(OleDropDataKind::ExternalPaths)
    } else {
        None
    }
}

fn preferred_effect_for_data_object(
    data_object: *mut c_void,
    data_kind: Option<OleDropDataKind>,
) -> Option<OleDropPreferredEffect> {
    if data_kind != Some(OleDropDataKind::ExternalPaths) {
        return None;
    }

    match read_preferred_drop_effect(data_object) {
        Ok(effect) => effect,
        Err(error) => {
            eprintln!("failed to read preferred drop effect: {error}");
            None
        }
    }
}

fn read_preferred_drop_effect(
    data_object: *mut c_void,
) -> ExplorerResult<Option<OleDropPreferredEffect>> {
    let preferred_format = hdrop::preferred_drop_effect_format_u16()?;
    if !data_object_supports_format(data_object, preferred_format) {
        return Ok(None);
    }

    let mut medium = get_data(data_object, preferred_format, "read preferred drop effect")?;
    let result = read_drop_effect_medium(&medium.medium);
    medium.release();
    result
}

fn read_drop_data(
    data_object: *mut c_void,
    internal_format: u16,
) -> ExplorerResult<Option<OleDropData>> {
    if data_object.is_null() {
        return Ok(None);
    }

    if let Some(drag_id) = read_internal_drag_id(data_object, internal_format)? {
        return Ok(Some(OleDropData::InternalDrag { drag_id }));
    }

    read_hdrop_data_object(data_object).map(|paths| paths.map(OleDropData::ExternalPaths))
}

fn read_internal_drag_id(
    data_object: *mut c_void,
    internal_format: u16,
) -> ExplorerResult<Option<u64>> {
    if !data_object_supports_format(data_object, internal_format) {
        return Ok(None);
    }

    let mut medium = get_data(data_object, internal_format, "read internal drag data")?;
    let result = read_drag_id_medium(&medium.medium);
    medium.release();
    result.map(Some)
}

fn read_hdrop_data_object(data_object: *mut c_void) -> ExplorerResult<Option<Vec<PathBuf>>> {
    if !data_object_supports_format(data_object, hdrop::CF_HDROP_FORMAT) {
        return Ok(None);
    }

    let mut medium = get_data(
        data_object,
        hdrop::CF_HDROP_FORMAT,
        "read dropped file list",
    )?;
    let result = read_hdrop_medium(&medium.medium);
    medium.release();
    result.map(Some)
}

fn data_object_supports_format(data_object: *mut c_void, format: u16) -> bool {
    let Some(vtable) = data_object_vtable(data_object) else {
        return false;
    };
    let mut format_etc = hglobal_format_etc(format);
    // SAFETY: data_object is a live IDataObject pointer and format_etc is valid for the call.
    unsafe { (vtable.query_get_data)(data_object, &mut format_etc) == S_OK }
}

fn get_data(
    data_object: *mut c_void,
    format: u16,
    operation: &'static str,
) -> ExplorerResult<StgMediumGuard> {
    let Some(vtable) = data_object_vtable(data_object) else {
        return Err(ExplorerError::invalid_input(
            "드롭 데이터 형식이 올바르지 않습니다.",
        ));
    };
    let mut format_etc = hglobal_format_etc(format);
    let mut medium = unsafe { zeroed::<STGMEDIUM>() };
    // SAFETY: data_object is a live IDataObject pointer and medium is a valid out pointer.
    let hresult = unsafe { (vtable.get_data)(data_object, &mut format_etc, &mut medium) };
    if hresult < 0 {
        Err(ExplorerError::windows_hresult(
            operation,
            "IDataObject::GetData",
            hresult,
            None,
        ))
    } else {
        Ok(StgMediumGuard {
            medium,
            released: false,
        })
    }
}

fn data_object_vtable(data_object: *mut c_void) -> Option<&'static IDataObjectVtbl> {
    if data_object.is_null() {
        return None;
    }

    // SAFETY: COM interface pointers have a vtable pointer as their first field.
    let vtable = unsafe { *(data_object as *mut *const IDataObjectVtbl) };
    if vtable.is_null() {
        None
    } else {
        // SAFETY: vtable belongs to the live COM object.
        unsafe { vtable.as_ref() }
    }
}

struct StgMediumGuard {
    medium: STGMEDIUM,
    released: bool,
}

impl StgMediumGuard {
    fn release(&mut self) {
        if !self.released {
            // SAFETY: medium was returned by IDataObject::GetData and is released exactly once.
            unsafe {
                ReleaseStgMedium(&mut self.medium);
            }
            self.released = true;
        }
    }
}

impl Drop for StgMediumGuard {
    fn drop(&mut self) {
        self.release();
    }
}

fn read_drag_id_medium(medium: &STGMEDIUM) -> ExplorerResult<u64> {
    if medium.tymed != TYMED_HGLOBAL as u32 {
        return Err(ExplorerError::invalid_input(
            "내부 드래그 데이터 형식이 올바르지 않습니다.",
        ));
    }

    // SAFETY: hGlobal is the active STGMEDIUM union field when tymed is TYMED_HGLOBAL.
    let handle = unsafe { medium.u.hGlobal };
    if handle.is_null() {
        return Err(ExplorerError::invalid_input(
            "내부 드래그 데이터가 비어 있습니다.",
        ));
    }
    if hdrop::hglobal_size(handle) < size_of::<u64>() {
        return Err(ExplorerError::invalid_input(
            "내부 드래그 데이터가 너무 짧습니다.",
        ));
    }

    // SAFETY: handle is a movable global memory block from STGMEDIUM.
    let data = unsafe { GlobalLock(handle) } as *const u64;
    if data.is_null() {
        return Err(ExplorerError::windows_api(
            "lock internal drag data",
            "GlobalLock",
            last_error_code(),
            None,
        ));
    }

    // SAFETY: GlobalSize verified that the HGLOBAL has enough bytes for u64.
    let value = unsafe { std::ptr::read_unaligned(data) };
    // SAFETY: handle was locked above.
    unsafe {
        GlobalUnlock(handle);
    }
    Ok(value)
}

fn read_hdrop_medium(medium: &STGMEDIUM) -> ExplorerResult<Vec<PathBuf>> {
    if medium.tymed != TYMED_HGLOBAL as u32 {
        return Err(ExplorerError::invalid_input(
            "드롭 파일 데이터 형식이 올바르지 않습니다.",
        ));
    }
    // SAFETY: hGlobal is the active STGMEDIUM union field when tymed is TYMED_HGLOBAL.
    let handle = unsafe { medium.u.hGlobal };
    if handle.is_null() {
        return Err(ExplorerError::invalid_input(
            "드롭 파일 데이터가 비어 있습니다.",
        ));
    }
    hdrop::read_hdrop_paths(handle as HDROP, FileDropUsage::DropTarget)
}

fn read_drop_effect_medium(medium: &STGMEDIUM) -> ExplorerResult<Option<OleDropPreferredEffect>> {
    if medium.tymed != TYMED_HGLOBAL as u32 {
        return Err(ExplorerError::invalid_input(
            "Preferred DropEffect 데이터 형식이 올바르지 않습니다.",
        ));
    }
    // SAFETY: hGlobal is the active STGMEDIUM union field when tymed is TYMED_HGLOBAL.
    let handle = unsafe { medium.u.hGlobal };
    if handle.is_null() {
        return Ok(None);
    }
    if hdrop::hglobal_size(handle) < size_of::<u32>() {
        return Ok(None);
    }

    // SAFETY: handle is a movable global memory block from STGMEDIUM.
    let data = unsafe { GlobalLock(handle) } as *const u32;
    if data.is_null() {
        return Err(ExplorerError::windows_api(
            "lock preferred drop effect",
            "GlobalLock",
            last_error_code(),
            None,
        ));
    }

    // SAFETY: GlobalSize verified that the HGLOBAL has enough bytes for u32.
    let effect = unsafe { std::ptr::read_unaligned(data) };
    // SAFETY: handle was locked above.
    unsafe {
        GlobalUnlock(handle);
    }
    Ok(preferred_effect_from_raw(effect))
}

fn preferred_effect_from_raw(effect: u32) -> Option<OleDropPreferredEffect> {
    match effect {
        hdrop::DROPEFFECT_MOVE_VALUE => Some(OleDropPreferredEffect::Move),
        hdrop::DROPEFFECT_COPY_VALUE => Some(OleDropPreferredEffect::Copy),
        _ => None,
    }
}

fn internal_drag_id_medium(drag_id: u64) -> ExplorerResult<STGMEDIUM> {
    let handle = OwnedHglobal::allocate(size_of::<u64>(), "allocate internal drag data")?;
    // SAFETY: handle is a movable global memory block allocated above.
    let data = unsafe { GlobalLock(handle.as_raw()) } as *mut u64;
    if data.is_null() {
        return Err(ExplorerError::windows_api(
            "lock internal drag data",
            "GlobalLock",
            last_error_code(),
            None,
        ));
    }
    // SAFETY: data points to writable memory with size_of::<u64>() bytes.
    unsafe {
        std::ptr::write_unaligned(data, drag_id);
        GlobalUnlock(handle.as_raw());
    }

    hglobal_medium(handle)
}

fn file_drop_medium(paths: &[PathBuf]) -> ExplorerResult<STGMEDIUM> {
    hglobal_medium(hdrop::create_hdrop_handle(
        paths,
        FileDropUsage::DragSource,
    )?)
}

fn hglobal_medium(handle: OwnedHglobal) -> ExplorerResult<STGMEDIUM> {
    let mut medium = unsafe { zeroed::<STGMEDIUM>() };
    medium.tymed = TYMED_HGLOBAL as u32;
    medium.pUnkForRelease = null_mut();
    medium.u.hGlobal = handle.into_raw();
    Ok(medium)
}

fn hglobal_format_etc(format: u16) -> FORMATETC {
    FORMATETC {
        cfFormat: format,
        ptd: null_mut(),
        dwAspect: DVASPECT_CONTENT,
        lindex: -1,
        tymed: TYMED_HGLOBAL as u32,
    }
}

fn drag_data_format_entries(internal_format: u16, include_shell_files: bool) -> Vec<FORMATETC> {
    let mut formats = Vec::new();
    if include_shell_files {
        formats.push(hglobal_format_etc(hdrop::CF_HDROP_FORMAT));
    }
    formats.push(hglobal_format_etc(internal_format));
    formats
}

fn clone_format_etc(format: &FORMATETC) -> FORMATETC {
    FORMATETC {
        cfFormat: format.cfFormat,
        ptd: null_mut(),
        dwAspect: format.dwAspect,
        lindex: format.lindex,
        tymed: format.tymed,
    }
}

fn format_matches(format: *mut FORMATETC, expected_format: u16) -> bool {
    let Some(format) = (unsafe { format.as_ref() }) else {
        return false;
    };
    format.cfFormat == expected_format
        && format.dwAspect == DVASPECT_CONTENT
        && format.tymed & TYMED_HGLOBAL as u32 != 0
}

fn internal_drag_format() -> ExplorerResult<u16> {
    // SAFETY: INTERNAL_DRAG_FORMAT_NAME is null-terminated and static.
    let format = unsafe { RegisterClipboardFormatW(INTERNAL_DRAG_FORMAT_NAME.as_ptr()) };
    if format == 0 {
        return Err(ExplorerError::windows_api(
            "register internal drag format",
            "RegisterClipboardFormatW",
            last_error_code(),
            None,
        ));
    }

    u16::try_from(format)
        .map_err(|_| ExplorerError::state_conflict("내부 드래그 클립보드 형식 값이 너무 큽니다."))
}

fn post_drop_event_message(owner: WindowHandle, message: u32) -> bool {
    if owner.is_null() {
        return false;
    }

    // SAFETY: owner is the application window and the queued event owns its data in queue.
    let posted = unsafe { PostMessageW(hwnd_from_window(owner), message, 0, 0) };
    if posted == 0 {
        eprintln!(
            "failed to post OLE drop event message: {}",
            last_error_code()
        );
        false
    } else {
        true
    }
}

fn hwnd_from_window(hwnd: WindowHandle) -> windows_sys::Win32::Foundation::HWND {
    hwnd.as_isize() as windows_sys::Win32::Foundation::HWND
}

fn last_error_code() -> u32 {
    // SAFETY: GetLastError reads thread-local Windows error state and has no preconditions.
    unsafe { windows_sys::Win32::Foundation::GetLastError() }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;
    use std::os::windows::ffi::OsStringExt;

    #[test]
    fn selected_effect_prefers_explicit_modifiers() {
        let both = DROPEFFECT_COPY | DROPEFFECT_MOVE;
        let hint = OleDropEffectHint::copy_move(Some(OleDropPreferredEffect::Move));

        assert_eq!(
            selected_effect(MK_CONTROL_VALUE, both, hint),
            DROPEFFECT_COPY
        );
        assert_eq!(selected_effect(MK_SHIFT_VALUE, both, hint), DROPEFFECT_MOVE);
        assert_eq!(
            selected_effect(MK_CONTROL_VALUE | MK_SHIFT_VALUE, both, hint),
            DROPEFFECT_COPY
        );
    }

    #[test]
    fn selected_effect_uses_available_default() {
        assert_eq!(
            selected_effect(0, DROPEFFECT_COPY, OleDropEffectHint::copy_only()),
            DROPEFFECT_COPY
        );
        assert_eq!(
            selected_effect(0, DROPEFFECT_MOVE, OleDropEffectHint::move_only()),
            DROPEFFECT_MOVE
        );
        assert_eq!(
            selected_effect(0, DROPEFFECT_NONE, OleDropEffectHint::copy_move(None)),
            DROPEFFECT_NONE
        );
    }

    #[test]
    fn selected_effect_respects_target_hint() {
        let both = DROPEFFECT_COPY | DROPEFFECT_MOVE;

        assert_eq!(
            selected_effect(MK_SHIFT_VALUE, both, OleDropEffectHint::copy_only()),
            DROPEFFECT_NONE
        );
        assert_eq!(
            selected_effect(
                0,
                both,
                OleDropEffectHint::copy_move(Some(OleDropPreferredEffect::Move))
            ),
            DROPEFFECT_MOVE
        );
        assert_eq!(
            selected_effect(0, both, OleDropEffectHint::none()),
            DROPEFFECT_NONE
        );
    }

    #[test]
    fn file_list_feedback_keeps_internal_empty_area_separate_from_external_empty_area() {
        let feedback = OleDropFeedback::new();
        feedback.set_file_list_hints(
            OleDropEffectHint::copy_only(),
            OleDropEffectHint::move_only(),
            Vec::new(),
        );

        assert_eq!(
            feedback.file_list_external_empty_hint(),
            OleDropEffectHint::copy_only()
        );
        assert_eq!(
            feedback.file_list_internal_empty_hint(),
            OleDropEffectHint::move_only()
        );
    }

    #[test]
    fn external_preferred_effect_updates_default_hint() {
        let hint = OleDropEffectHint::copy_move(None);

        assert_eq!(
            hint_with_external_preferred(
                hint,
                OleDropDataKind::ExternalPaths,
                Some(OleDropPreferredEffect::Move)
            )
            .default,
            Some(OleDropPreferredEffect::Move)
        );
        assert_eq!(
            hint_with_external_preferred(
                hint,
                OleDropDataKind::InternalDrag,
                Some(OleDropPreferredEffect::Move)
            )
            .default,
            None
        );
    }

    #[test]
    fn preferred_effect_from_raw_accepts_copy_and_move_flags() {
        assert_eq!(
            preferred_effect_from_raw(hdrop::DROPEFFECT_MOVE_VALUE),
            Some(OleDropPreferredEffect::Move)
        );
        assert_eq!(
            preferred_effect_from_raw(hdrop::DROPEFFECT_COPY_VALUE),
            Some(OleDropPreferredEffect::Copy)
        );
        assert_eq!(preferred_effect_from_raw(DROPEFFECT_NONE), None);
    }

    #[test]
    fn preferred_effect_from_raw_ignores_ambiguous_or_unknown_flags() {
        assert_eq!(
            preferred_effect_from_raw(hdrop::DROPEFFECT_COPY_VALUE | hdrop::DROPEFFECT_MOVE_VALUE),
            None
        );
        assert_eq!(
            preferred_effect_from_raw(hdrop::DROPEFFECT_MOVE_VALUE | 0x8000_0000),
            None
        );
    }

    #[test]
    fn hdrop_path_buffer_len_allows_configured_limit() {
        assert!(matches!(
            hdrop::hdrop_path_buffer_len(32_767, FileDropUsage::DropTarget),
            Ok(len) if len == 32_768
        ));
    }

    #[test]
    fn drag_source_outcome_distinguishes_cancel_no_drop_and_effects() -> ExplorerResult<()> {
        assert_eq!(
            drag_source_outcome(DRAGDROP_S_CANCEL, DROPEFFECT_NONE, true)?,
            OleDragSourceOutcome::Cancelled
        );
        assert_eq!(
            drag_source_outcome(DRAGDROP_S_CANCEL, DROPEFFECT_NONE, false)?,
            OleDragSourceOutcome::NoDrop
        );
        assert_eq!(
            drag_source_outcome(DRAGDROP_S_DROP, DROPEFFECT_COPY, false)?,
            OleDragSourceOutcome::Copy
        );
        assert_eq!(
            drag_source_outcome(DRAGDROP_S_DROP, DROPEFFECT_MOVE, false)?,
            OleDragSourceOutcome::Move
        );

        Ok(())
    }

    #[test]
    fn drag_data_formats_include_internal_and_shell_formats_without_forcing_default() {
        let formats = drag_data_format_entries(42, true)
            .into_iter()
            .map(|format| format.cfFormat)
            .collect::<Vec<_>>();

        assert_eq!(formats, vec![hdrop::CF_HDROP_FORMAT, 42]);
    }

    #[test]
    fn shell_drag_hdrop_round_trips_unicode_paths() -> ExplorerResult<()> {
        let paths = vec![
            PathBuf::from(r"C:\드롭 테스트\a b.txt"),
            PathBuf::from(r"\\server\share\자료\#1.txt"),
            PathBuf::from(r"\\?\UNC\server\share\긴 경로\한글.txt"),
        ];
        let handle = hdrop::create_hdrop_handle(&paths, FileDropUsage::DragSource)?;

        let parsed = hdrop::read_hdrop_paths(handle.as_raw() as HDROP, FileDropUsage::DropTarget)?;

        assert_eq!(parsed, paths);
        Ok(())
    }

    #[test]
    fn shell_drag_hdrop_preflight_allows_maximum_file_count() -> ExplorerResult<()> {
        let paths = (0..4096)
            .map(|index| PathBuf::from(format!(r"C:\bulk\{index}.txt")))
            .collect::<Vec<_>>();

        validate_shell_file_drag_paths(&paths)?;

        Ok(())
    }

    #[test]
    fn shell_drag_hdrop_rejects_too_many_paths_before_shell_drag() {
        let paths = (0..=4096)
            .map(|index| PathBuf::from(format!(r"C:\bulk\{index}.txt")))
            .collect::<Vec<_>>();

        let error = validate_shell_file_drag_paths(&paths)
            .expect_err("oversized CF_HDROP item list must be rejected before drag starts");

        assert_eq!(
            error.user_message(),
            "파일 또는 폴더가 너무 많습니다. 한 번에 최대 4096개까지 드래그할 수 있습니다."
        );
    }

    #[test]
    fn shell_drag_hdrop_preflight_allows_maximum_path_units() -> ExplorerResult<()> {
        let mut units = vec![b'C' as u16, b':' as u16, b'\\' as u16];
        units.extend(std::iter::repeat_n(b'a' as u16, 32_767_usize - units.len()));
        let path = PathBuf::from(OsString::from_wide(&units));

        validate_shell_file_drag_paths(&[path])?;

        Ok(())
    }

    #[test]
    fn shell_drag_hdrop_rejects_too_long_path_before_allocation() {
        let mut units = vec![b'C' as u16, b':' as u16, b'\\' as u16];
        units.extend(std::iter::repeat_n(b'a' as u16, 32_767));
        let path = PathBuf::from(OsString::from_wide(&units));

        let error =
            validate_shell_file_drag_paths(&[path]).expect_err("too long path must be rejected");

        assert_eq!(
            error.user_message(),
            "드래그 파일 경로가 너무 길어 처리할 수 없습니다."
        );
    }

    #[test]
    fn drop_event_post_fails_for_null_owner() {
        assert!(!post_drop_event_message(WindowHandle::null(), 0));
    }
}
