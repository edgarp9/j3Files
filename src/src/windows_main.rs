use std::collections::{HashMap, HashSet, VecDeque};
use std::ffi::{OsStr, OsString};
use std::fs;
use std::os::windows::ffi::OsStrExt;
use std::path::{Component, Path, PathBuf};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
use std::thread::{Builder as ThreadBuilder, JoinHandle};
use std::time::{SystemTime, UNIX_EPOCH};

use j3files::app::explorer::StartupSessionRestore;
use j3files::app::{
    ContextMenuPosition, ExplorerApp, ItemActivation, SearchFileSystemGateway,
    SearchFileSystemOutcome, SearchOutcome, SearchRequest, ShellDeleteGateway, ShellRenameGateway,
    ShellTransferGateway,
};
use j3files::domain::{
    default_external_drop_operation, drag_source_refresh_locations,
    file_transfer_refresh_locations, snapshot_drag_source_locations, sort_file_items,
    source_parent_locations, unique_drag_sources, validate_move_drop, AppearanceFont,
    AppearanceTheme, BookmarkAddOutcome, DisplayOptions, DragSourceCompletion,
    DropAllowedOperations, DropModifierKeys, DropOperation, DropSourceKind, ExplorerError,
    ExplorerResult, FileItem, FolderTreeItem, FolderTreeItemKind, KnownFolderKind,
    NavigationLocation, PreparedNavigationPath, RenameItemName, SearchCriteria, SearchDiagnostic,
    SearchProgress, SearchRunId, SearchScope, SearchState, SortDirection, SortKey, SortState,
    TabId, TabState,
};
use j3files::infra::{
    startup_plan_from_args, startup_plan_from_configured_folder, NativeFileSystemGateway,
    NativeUserSettingsStore, ShellIconCache, ShellIconLoadCompletion, ShellIconLoadTask,
    WindowsShellGateway,
};
use j3files::platform::{self, win32_ui as ui, ClipboardFileOperation};

#[path = "main_window_commands.rs"]
mod main_window_commands;
#[path = "main_window_workers.rs"]
mod main_window_workers;

#[cfg(test)]
use main_window_workers::{
    cancel_search_workers, detach_cancelled_search_workers, join_file_operation_worker,
    join_listing_worker, reap_finished_listing_workers, reap_finished_search_workers,
    replace_pending_search_worker, retire_listing_worker, ActiveFileOperationWorker,
    ActiveListingWorker, ActiveSearchWorker, SearchProgressMessage, WorkerMessageStore,
};
use main_window_workers::{
    DeleteFileOperation, FileOperationCompleteMessage, FileOperationRequest,
    FileOperationWorkerOutcome, FileWatchChangeMessage, ListingCompleteMessage, ListingRequest,
    PendingSearchWorker, SearchCompleteMessage, SharedSearchCancellation, UiSearchProgressReporter,
    WorkerController,
};

const WINDOW_CLASS_NAME: &str = "J3FilesMainWindow";
const PROGRAM_NAME: &str = "j3Files";
const PROGRAM_VERSION: &str = env!("CARGO_PKG_VERSION");
const PROJECT_URL: &str = "https://github.com/edgarp9";
const WINDOW_TITLE: &str = PROGRAM_NAME;
const ABOUT_TEXT_FILE_NAME: &str = "about.txt";
const DEFAULT_ABOUT_TEXT: &str = include_str!("../about.txt");

fn distribution_text(file_name: &str, fallback: &str) -> String {
    distribution_text_path(file_name)
        .and_then(|path| fs::read_to_string(path).ok())
        .filter(|text| !text.trim().is_empty())
        .unwrap_or_else(|| fallback.to_owned())
}

fn distribution_text_path(file_name: &str) -> Option<PathBuf> {
    let mut path = std::env::current_exe().ok()?;
    path.set_file_name(file_name);
    Some(path)
}

const APP_ICON_RESOURCE_ID: u16 = 101;
const NAV_BACK_ICON_RESOURCE_ID: u16 = 102;
const NAV_FORWARD_ICON_RESOURCE_ID: u16 = 103;
const NAV_UP_ICON_RESOURCE_ID: u16 = 104;
const NAV_REFRESH_ICON_RESOURCE_ID: u16 = 105;
const NAV_GO_ICON_RESOURCE_ID: u16 = 106;
const SEARCH_CANCEL_ICON_RESOURCE_ID: u16 = 107;

const ID_NAV_BACK: u16 = 1005;
const ID_NAV_FORWARD: u16 = 1006;
const ID_NAV_UP: u16 = 1001;
const ID_REFRESH: u16 = 1002;
const ID_GO: u16 = 1003;
const ID_EXIT: u16 = 1004;
const ID_FILE_NEW_FOLDER: u16 = 1007;
const ID_FILE_OPEN: u16 = 1008;
const ID_FILE_OPEN_WITH: u16 = 1009;
const ID_FILE_PROPERTIES: u16 = 1010;
const ID_FILE_COPY: u16 = 1011;
const ID_FILE_CUT: u16 = 1012;
const ID_FILE_PASTE: u16 = 1013;
const ID_FILE_UNDO: u16 = 1014;
const ID_FILE_DELETE: u16 = 1015;
const ID_FILE_DELETE_PERMANENTLY: u16 = 1016;
const ID_FILE_RENAME: u16 = 1017;
const ID_FILE_SELECT_ALL: u16 = 1018;
const ID_TAB_CONTROL: u16 = 1100;
const ID_ADDRESS: u16 = 1101;
const ID_FILE_LIST: u16 = 1102;
const ID_FOLDER_TREE: u16 = 1103;
const ID_ADDRESS_FOCUS: u16 = 1104;
const ID_KNOWN_HOME: u16 = 1201;
const ID_KNOWN_DESKTOP: u16 = 1202;
const ID_KNOWN_DOWNLOADS: u16 = 1203;
const ID_KNOWN_DOCUMENTS: u16 = 1204;
const ID_DRIVE_BASE: u16 = 1300;
const ID_TAB_NEW: u16 = 1401;
const ID_TAB_CLOSE: u16 = 1402;
const ID_TAB_REOPEN: u16 = 1403;
const ID_TAB_MOVE_LEFT: u16 = 1404;
const ID_TAB_MOVE_RIGHT: u16 = 1405;
const ID_TAB_OPEN_SELECTED_FOLDER: u16 = 1406;
const ID_TAB_RESTORE_ON_STARTUP: u16 = 1407;
const ID_TAB_NEXT: u16 = 1408;
const ID_TAB_SET_STARTUP_FOLDER: u16 = 1409;
const ID_TAB_CLEAR_STARTUP_FOLDER: u16 = 1410;
const ID_BOOKMARK_ADD_CURRENT: u16 = 1501;
const ID_BOOKMARK_ADD_SELECTED_FOLDER: u16 = 1502;
const ID_BOOKMARK_REMOVE_CURRENT: u16 = 1503;
const ID_BOOKMARK_OPEN_TREE_ITEM: u16 = 1504;
const ID_BOOKMARK_REMOVE_TREE_ITEM: u16 = 1505;
const ID_BOOKMARK_BASE: u16 = 1600;
const ID_VIEW_SHOW_HIDDEN: u16 = 1801;
const ID_VIEW_SHOW_SYSTEM: u16 = 1802;
const ID_SORT_NAME: u16 = 1811;
const ID_SORT_SIZE: u16 = 1812;
const ID_SORT_UPDATED: u16 = 1813;
const ID_SORT_KIND: u16 = 1814;
const ID_SORT_ASCENDING: u16 = 1821;
const ID_SORT_DESCENDING: u16 = 1822;
const ID_THEME_LIGHT: u16 = 1831;
const ID_THEME_CLASSIC_DARK: u16 = 1832;
const ID_THEME_SEPIA_TEAL: u16 = 1833;
const ID_THEME_GRAPHITE: u16 = 1834;
const ID_THEME_FOREST: u16 = 1835;
const ID_THEME_STEEL_BLUE: u16 = 1836;
const ID_VIEW_FONT: u16 = 1841;
const ID_VIEW_FONT_RESET: u16 = 1842;
const ID_SEARCH_QUERY_LABEL: u16 = 1901;
const ID_SEARCH_QUERY: u16 = 1902;
const ID_SEARCH_FIND: u16 = 1903;
const ID_SEARCH_SUBFOLDERS: u16 = 1904;
const ID_SEARCH_CANCEL: u16 = 1905;
const ID_SEARCH_FOCUS: u16 = 1906;
const ID_SEARCH_CLOSE: u16 = 1907;
const ID_SEARCH_INCLUDE_SUBFOLDERS: u16 = 1908;
const ID_FILE_OPERATION_STATUS: u16 = 1909;
const ID_ABOUT: u16 = 2001;
const MAX_DRIVE_MENU_ITEMS: usize = 64;
const MAX_BOOKMARK_MENU_ITEMS: usize = 128;
const DEFAULT_NEW_FOLDER_NAME: &str = "New Folder";
const FILE_OPERATION_IN_PROGRESS_MESSAGE: &str = "파일 작업 진행 중...";
const FILE_OPERATION_SHUTDOWN_PENDING_MESSAGE: &str = "파일 작업이 끝나면 창을 닫습니다...";
const MESSAGE_SEARCH_PROGRESS: u32 = ui::MESSAGE_APP + 1;
const MESSAGE_SEARCH_COMPLETE: u32 = ui::MESSAGE_APP + 2;
const MESSAGE_LISTING_COMPLETE: u32 = ui::MESSAGE_APP + 3;
const MESSAGE_FILE_OPERATION_COMPLETE: u32 = ui::MESSAGE_APP + 4;
const MESSAGE_OLE_DROP_EVENT: u32 = ui::MESSAGE_APP + 5;
const MESSAGE_FILE_WATCH_CHANGED: u32 = ui::MESSAGE_APP + 6;
const MESSAGE_FOLDER_TREE_CHILDREN_COMPLETE: u32 = ui::MESSAGE_APP + 7;
const MESSAGE_ICON_LOAD_COMPLETE: u32 = ui::MESSAGE_APP + 8;
const MESSAGE_CLOSE: u32 = 0x0010;
const ID_SEARCH_COMPLETION_TIMER: usize = 1;
const ID_TREE_DRAG_FEEDBACK_TIMER: usize = 2;
const ID_LIST_DRAG_FEEDBACK_TIMER: usize = 3;
const ID_FILE_WATCH_REFRESH_TIMER: usize = 4;
const ID_USER_SETTINGS_SAVE_TIMER: usize = 5;
const ID_DEFERRED_STARTUP_TIMER: usize = 6;
const SEARCH_COMPLETION_POLL_MS: u32 = 250;
const DRAG_FEEDBACK_POLL_MS: u32 = 100;
const FILE_WATCH_REFRESH_DEBOUNCE_MS: u32 = 200;
const MAX_INCREMENTAL_FILE_WATCH_CHANGES: usize = 64;
const MAX_FILE_WATCH_CHILD_INDEX_CACHE_KEYS: usize = MAX_INCREMENTAL_FILE_WATCH_CHANGES * 4;
const MAX_EXACT_TRANSFER_COMPLETION_EXISTENCE_CHECKS: usize = 1024;
const MAX_CONCURRENT_SEARCH_WORKERS: usize = 4;
const MAX_CONCURRENT_FOLDER_TREE_CHILD_WORKERS: usize = 4;
const MAX_CONCURRENT_ICON_LOAD_WORKERS: usize = 2;
const MAX_LOGGED_SEARCH_DIAGNOSTICS: usize = SearchDiagnostic::MAX_RECORDED_DETAILS + 1;
const USER_SETTINGS_SAVE_DEBOUNCE_MS: u32 = 500;
const DEFERRED_STARTUP_DELAY_MS: u32 = 1;
const DRAG_AUTO_SCROLL_EDGE: i32 = 24;

const DEFAULT_WINDOW_WIDTH: i32 = 1000;
const DEFAULT_WINDOW_HEIGHT: i32 = 700;
const MIN_WINDOW_WIDTH: i32 = 720;
const MIN_WINDOW_HEIGHT: i32 = 460;
const MARGIN: i32 = 8;
const SPACING: i32 = 6;
const TAB_HEIGHT: i32 = 28;
const TOOLBAR_HEIGHT: i32 = 28;
const SEARCH_ROW_HEIGHT: i32 = 24;
const BUTTON_WIDTH: i32 = 34;
const GO_BUTTON_WIDTH: i32 = 34;
const SEARCH_FIND_BUTTON_WIDTH: i32 = 58;
const SEARCH_SUBFOLDER_CHECKBOX_WIDTH: i32 = 58;
const SEARCH_CANCEL_BUTTON_WIDTH: i32 = 34;
const SEARCH_LABEL_WIDTH: i32 = 68;
const FILE_OPERATION_STATUS_HEIGHT: i32 = 22;
const DEFAULT_FOLDER_TREE_WIDTH: i32 = 240;
const MIN_FOLDER_TREE_WIDTH: i32 = 120;
const MIN_RIGHT_PANE_WIDTH: i32 = 360;
const FOLDER_TREE_SPLITTER_WIDTH: i32 = 8;
const NAVIGATION_ICON_SIZE: i32 = 20;
const LIST_NAME_COLUMN_INDEX: usize = 0;
const LIST_TYPE_COLUMN_INDEX: usize = 1;
const LIST_SIZE_COLUMN_INDEX: usize = 2;
const LIST_UPDATED_COLUMN_INDEX: usize = 3;
const LIST_COLUMN_COUNT: usize = 4;
const LIST_NAME_COLUMN_WIDTH: i32 = 360;
const LIST_SIZE_COLUMN_WIDTH: i32 = 120;
const LIST_UPDATED_COLUMN_WIDTH: i32 = 180;
const LIST_TYPE_COLUMN_WIDTH: i32 = 160;

type ExplorerModel = ExplorerApp<NativeFileSystemGateway, WindowsShellGateway>;
type IconLoadCompletion = ShellIconLoadCompletion;
type IconLoadTask = ShellIconLoadTask;

#[derive(Debug, Clone)]
struct FolderTreeNodeState {
    handle: Option<ui::TreeViewItemHandle>,
    parent: Option<usize>,
    kind: FolderTreeItemKind,
    location: NavigationLocation,
    prepared_location_path: PreparedNavigationPath,
    children_loaded: bool,
    children_loading_generation: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FolderTreeChildrenRequestKind {
    LoadChildren,
    RefreshChildPresence,
    RefreshLoadedChildren,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct FolderTreeChildrenRequest {
    generation: u64,
    parent_index: usize,
    location: NavigationLocation,
    display_options: DisplayOptions,
    kind: FolderTreeChildrenRequestKind,
    selection_sync: bool,
}

enum FolderTreeChildrenWorkerResult {
    ChildPresence(bool),
    Children(Vec<FolderTreeItem>),
}

struct FolderTreeChildrenCompleteMessage {
    request: FolderTreeChildrenRequest,
    result: ExplorerResult<FolderTreeChildrenWorkerResult>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PendingFolderTreeChildWorker {
    request: FolderTreeChildrenRequest,
    loading_generation_on_spawn_error: Option<u64>,
}

#[derive(Clone, Default)]
struct FolderTreeChildrenMessageStore {
    inner: Arc<Mutex<FolderTreeChildrenMessageStoreInner>>,
}

#[derive(Default)]
struct FolderTreeChildrenMessageStoreInner {
    next_token: i64,
    complete_messages: Vec<(i64, FolderTreeChildrenCompleteMessage)>,
}

impl FolderTreeChildrenMessageStore {
    fn insert_complete(
        &self,
        message: FolderTreeChildrenCompleteMessage,
    ) -> Option<ui::MessageLong> {
        let Ok(mut inner) = self.inner.lock() else {
            eprintln!("failed to lock folder tree children message store");
            return None;
        };

        if inner.next_token <= 0 {
            inner.next_token = 1;
        }
        if inner.next_token > isize::MAX as i64 {
            eprintln!("folder tree children message token space exhausted");
            return None;
        }

        let token = inner.next_token;
        inner.next_token += 1;
        inner.complete_messages.push((token, message));
        Some(token as ui::MessageLong)
    }

    fn take_complete(&self, lparam: ui::MessageLong) -> Option<FolderTreeChildrenCompleteMessage> {
        let token = lparam as i64;
        if token <= 0 {
            return None;
        }

        let Ok(mut inner) = self.inner.lock() else {
            eprintln!("failed to lock folder tree children message store");
            return None;
        };
        let index = inner
            .complete_messages
            .iter()
            .position(|(message_token, _)| *message_token == token)?;
        Some(inner.complete_messages.remove(index).1)
    }
}

#[derive(Clone, Default)]
struct IconLoadMessageStore {
    inner: Arc<Mutex<IconLoadMessageStoreInner>>,
}

#[derive(Default)]
struct IconLoadMessageStoreInner {
    next_token: i64,
    complete_messages: Vec<(i64, IconLoadCompletion)>,
}

impl IconLoadMessageStore {
    fn insert_complete(&self, completion: IconLoadCompletion) -> Option<ui::MessageLong> {
        let Ok(mut inner) = self.inner.lock() else {
            eprintln!("failed to lock icon load message store");
            return None;
        };

        if inner.next_token <= 0 {
            inner.next_token = 1;
        }
        if inner.next_token > isize::MAX as i64 {
            eprintln!("icon load message token space exhausted");
            return None;
        }
        let token = inner.next_token;
        inner.next_token += 1;
        inner.complete_messages.push((token, completion));
        Some(token as ui::MessageLong)
    }

    fn take_complete(&self, lparam: ui::MessageLong) -> Option<IconLoadCompletion> {
        let token = lparam as i64;
        if token <= 0 {
            return None;
        }

        let Ok(mut inner) = self.inner.lock() else {
            eprintln!("failed to lock icon load message store");
            return None;
        };
        let index = inner
            .complete_messages
            .iter()
            .position(|(message_token, _)| *message_token == token)?;
        Some(inner.complete_messages.remove(index).1)
    }

    fn clear(&self) {
        let Ok(mut inner) = self.inner.lock() else {
            eprintln!("failed to lock icon load message store");
            return;
        };
        inner.complete_messages.clear();
    }

    fn post_complete(&self, hwnd_value: isize, completion: IconLoadCompletion) {
        let Some(token) = self.insert_complete(completion) else {
            return;
        };

        let hwnd = ui::WindowHandle::from_isize(hwnd_value);
        if let Err(error) = ui::post_window_message(hwnd, MESSAGE_ICON_LOAD_COMPLETE, 0, token) {
            eprintln!("failed to post icon load completion: {error}");
            self.take_complete(token);
        }
    }
}

struct ActiveFolderTreeChildWorker {
    request: FolderTreeChildrenRequest,
    cancel_requested: Arc<AtomicBool>,
    completion_message_abandoned: Arc<AtomicBool>,
    io_cancellation: Arc<platform::SynchronousIoCancellation>,
    handle: JoinHandle<()>,
}

impl ActiveFolderTreeChildWorker {
    fn request_cancel(&self) {
        self.cancel_requested.store(true, Ordering::Relaxed);
        if let Err(error) = self.io_cancellation.request_cancel() {
            eprintln!("failed to cancel folder tree child worker synchronous I/O: {error}");
        }
    }

    fn is_cancel_requested(&self) -> bool {
        self.cancel_requested.load(Ordering::Relaxed)
    }

    fn is_completion_message_abandoned(&self) -> bool {
        self.completion_message_abandoned.load(Ordering::Relaxed)
    }

    fn is_finished(&self) -> bool {
        self.handle.is_finished()
    }
}

#[derive(Debug, Clone)]
struct CurrentListingRows {
    tab_id: TabId,
    location: NavigationLocation,
    display_options: DisplayOptions,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CurrentSearchRowsKind {
    Results,
    Cancelled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct CurrentSearchRows {
    tab_id: TabId,
    kind: CurrentSearchRowsKind,
    item_count: usize,
    display_options: DisplayOptions,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct CurrentSearchItems {
    tab_id: TabId,
    kind: CurrentSearchRowsKind,
}

#[derive(Debug, Default)]
enum CurrentItems {
    #[default]
    Empty,
    Listing(Vec<FileItem>),
    Search(CurrentSearchItems),
}

impl CurrentItems {
    fn listing(items: Vec<FileItem>) -> Self {
        Self::Listing(items)
    }

    fn search(rows: CurrentSearchRows) -> Self {
        Self::Search(CurrentSearchItems {
            tab_id: rows.tab_id,
            kind: rows.kind,
        })
    }

    fn clear(&mut self) {
        *self = Self::Empty;
    }

    fn as_slice<'a>(&'a self, active_tab: &'a TabState) -> &'a [FileItem] {
        match (self, &active_tab.search) {
            (Self::Listing(items), _) => items,
            (
                Self::Search(CurrentSearchItems {
                    tab_id,
                    kind: CurrentSearchRowsKind::Results,
                }),
                SearchState::Results { items, .. },
            ) if *tab_id == active_tab.id => items,
            (
                Self::Search(CurrentSearchItems {
                    tab_id,
                    kind: CurrentSearchRowsKind::Cancelled,
                }),
                SearchState::Cancelled { partial_items, .. },
            ) if *tab_id == active_tab.id => partial_items,
            _ => &[],
        }
    }

    fn as_listing_mut(&mut self) -> Option<&mut Vec<FileItem>> {
        match self {
            Self::Listing(items) => Some(items),
            Self::Empty | Self::Search(_) => None,
        }
    }
}

#[derive(Debug, Default)]
struct FileItemCellTextCache {
    cells: [Option<Vec<u16>>; LIST_COLUMN_COUNT],
}

type FileItemCellTextCaches = HashMap<usize, FileItemCellTextCache>;

impl FileItemCellTextCache {
    fn text_for(&mut self, item: &FileItem, column_index: usize) -> &[u16] {
        let Some(cell) = self.cells.get_mut(column_index) else {
            return &EMPTY_DISPLAY_CELL_TEXT;
        };
        cell.get_or_insert_with(|| file_item_cell_text(item, column_index))
    }
}

static EMPTY_DISPLAY_CELL_TEXT: [u16; 1] = [0];

fn reset_file_item_cell_text_caches(caches: &mut FileItemCellTextCaches, _row_count: usize) {
    caches.clear();
}

fn cached_file_item_cell_text<'a>(
    caches: &'a mut FileItemCellTextCaches,
    row_index: usize,
    item: &FileItem,
    column_index: usize,
) -> &'a [u16] {
    caches
        .entry(row_index)
        .or_default()
        .text_for(item, column_index)
}

#[derive(Debug, Default)]
struct PendingFileWatchRefresh {
    requires_full_refresh: bool,
    changed_names: Vec<OsString>,
}

impl PendingFileWatchRefresh {
    fn merge(&mut self, batch: platform::DirectoryChangeBatch) {
        if self.requires_full_refresh {
            return;
        }
        if batch.overflowed || batch.changes.is_empty() {
            self.require_full_refresh();
            return;
        }

        for change in batch.changes {
            if self
                .changed_names
                .iter()
                .any(|name| name == &change.file_name)
            {
                continue;
            }
            if self.changed_names.len() >= MAX_INCREMENTAL_FILE_WATCH_CHANGES {
                self.require_full_refresh();
                return;
            }
            self.changed_names.push(change.file_name);
        }
    }

    fn require_full_refresh(&mut self) {
        self.requires_full_refresh = true;
        self.changed_names.clear();
    }

    fn is_empty(&self) -> bool {
        !self.requires_full_refresh && self.changed_names.is_empty()
    }

    fn clear(&mut self) {
        self.requires_full_refresh = false;
        self.changed_names.clear();
    }
}

#[derive(Debug, Clone, Copy)]
struct PendingListingViewportRestore {
    generation: u64,
}

#[derive(Debug, Clone)]
struct InternalDragState {
    drag_id: u64,
    origin: InternalDragOrigin,
    sources: Vec<NavigationLocation>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InternalDragOrigin {
    FileList,
    FolderTree,
}

#[derive(Debug, Clone)]
struct ResolvedDropSources {
    sources: Vec<NavigationLocation>,
    source_kind: DropSourceKind,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct PendingDropProcessing {
    handled_internal_drag: bool,
}

#[derive(Debug, Clone)]
enum UndoFileOperation {
    Rename {
        current: NavigationLocation,
        original_name: OsString,
    },
    Copy {
        copied: Vec<NavigationLocation>,
    },
    Move {
        moved: Vec<(NavigationLocation, NavigationLocation)>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TransferTargetExistence {
    Exists,
    Missing,
    Unknown,
}

impl TransferTargetExistence {
    fn from_path(path: &Path) -> Self {
        Self::from_try_exists(path.try_exists())
    }

    fn from_try_exists(result: std::io::Result<bool>) -> Self {
        match result {
            Ok(true) => Self::Exists,
            Ok(false) => Self::Missing,
            Err(_) => Self::Unknown,
        }
    }
}

#[derive(Debug, Clone)]
struct ExpectedTransferLocation {
    location: NavigationLocation,
    existed_before: TransferTargetExistence,
}

#[derive(Debug, Clone, Copy)]
enum CompletedTransfer<'a> {
    Copy {
        expected: &'a [ExpectedTransferLocation],
    },
    Move {
        expected: &'a [ExpectedTransferLocation],
    },
    Incomplete,
}

impl<'a> CompletedTransfer<'a> {
    fn expected(self) -> Option<&'a [ExpectedTransferLocation]> {
        match self {
            Self::Copy { expected } | Self::Move { expected } => Some(expected),
            Self::Incomplete => None,
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct NavigationIcons {
    back: ui::IconHandle,
    forward: ui::IconHandle,
    up: ui::IconHandle,
    refresh: ui::IconHandle,
    go: ui::IconHandle,
    search_cancel: ui::IconHandle,
}

impl NavigationIcons {
    fn load(instance: ui::InstanceHandle, scale: ui::UiScale) -> ExplorerResult<Self> {
        let size = scale.px(NAVIGATION_ICON_SIZE);
        Ok(Self {
            back: ui::load_shared_icon_resource(instance, NAV_BACK_ICON_RESOURCE_ID, size)?,
            forward: ui::load_shared_icon_resource(instance, NAV_FORWARD_ICON_RESOURCE_ID, size)?,
            up: ui::load_shared_icon_resource(instance, NAV_UP_ICON_RESOURCE_ID, size)?,
            refresh: ui::load_shared_icon_resource(instance, NAV_REFRESH_ICON_RESOURCE_ID, size)?,
            go: ui::load_shared_icon_resource(instance, NAV_GO_ICON_RESOURCE_ID, size)?,
            search_cancel: ui::load_shared_icon_resource(
                instance,
                SEARCH_CANCEL_ICON_RESOURCE_ID,
                size,
            )?,
        })
    }

    fn for_command(self, id: u16) -> Option<ui::IconHandle> {
        match id {
            ID_NAV_BACK => Some(self.back),
            ID_NAV_FORWARD => Some(self.forward),
            ID_NAV_UP => Some(self.up),
            ID_REFRESH => Some(self.refresh),
            ID_GO => Some(self.go),
            ID_SEARCH_CANCEL => Some(self.search_cancel),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ListingLoadingPresentation {
    StatusRow,
    PreserveCurrentRows,
}

pub(crate) fn main() {
    if let Err(error) = run() {
        if should_show_user_error_dialog(&error) {
            ui::show_error_message(
                ui::WindowHandle::null(),
                WINDOW_TITLE,
                &error.user_message(),
            );
            eprintln!("{}", error.user_message());
        }
        eprintln!("detail: {error}");
        std::process::exit(1);
    }
}

fn should_show_user_error_dialog(error: &ExplorerError) -> bool {
    !error.is_cancelled()
}

fn show_settings_load_warning(error: &ExplorerError, save_allowed: bool) {
    let mut message = format!(
        "저장된 사용자 설정을 읽을 수 없어 기본값으로 시작합니다.\n\n{}",
        error.user_message()
    );
    if !save_allowed {
        message.push_str(
            "\n\n기존 설정 파일을 보호하기 위해 이번 실행에서 사용자 설정은 자동 저장하지 않습니다.",
        );
    }
    ui::show_error_message(ui::WindowHandle::null(), WINDOW_TITLE, &message);
    eprintln!("{message}");
    eprintln!("detail: {error}");
}

fn log_dpi_awareness_outcome(outcome: &ui::DpiAwarenessOutcome) {
    if let Some(step) = outcome.applied_step() {
        if !outcome.failures().is_empty() {
            eprintln!(
                "DPI awareness fallback selected: {} {}",
                step.api(),
                step.mode()
            );
            log_dpi_awareness_failures(outcome.failures());
        }
        return;
    }

    eprintln!("DPI awareness could not be configured; continuing with Windows default scaling.");
    log_dpi_awareness_failures(outcome.failures());
}

fn log_dpi_awareness_failures(failures: &[ui::DpiAwarenessFailure]) {
    for failure in failures {
        let step = failure.step();
        match failure.reason() {
            ui::DpiAwarenessFailureReason::Unavailable => eprintln!(
                "DPI awareness attempt unavailable: {} {}",
                step.api(),
                step.mode()
            ),
            ui::DpiAwarenessFailureReason::Win32(code) => eprintln!(
                "DPI awareness attempt failed: {} {} Win32={}",
                step.api(),
                step.mode(),
                code
            ),
            ui::DpiAwarenessFailureReason::Hresult(hresult) => eprintln!(
                "DPI awareness attempt failed: {} {} HRESULT=0x{:08x}",
                step.api(),
                step.mode(),
                hresult as u32
            ),
        }
    }
}

fn run() -> ExplorerResult<()> {
    let dpi_awareness = ui::configure_process_dpi_awareness();
    log_dpi_awareness_outcome(&dpi_awareness);
    ui::initialize_common_controls()?;
    let initial_scale = ui::system_dpi_metrics().ui_scale();

    let file_system = NativeFileSystemGateway::new();
    let shell = WindowsShellGateway::new();
    let shell_window = shell.clone();
    let mut startup_plan = startup_plan_from_args(std::env::args_os().skip(1))?;
    let settings_store = NativeUserSettingsStore::new()?;
    let settings_load = settings_store.load_user_settings_with_recovery();
    if let Some(warning) = &settings_load.warning {
        show_settings_load_warning(warning, settings_load.save_allowed);
    }
    let settings_save_enabled = settings_load.save_allowed;
    let mut settings = settings_load.settings;
    if !startup_plan.has_explicit_path() {
        if let Some(startup_folder) = settings.startup_folder.clone() {
            startup_plan = startup_plan_from_configured_folder(startup_folder)?;
        }
    }
    if startup_plan.has_explicit_path() {
        settings.restore_tabs_on_startup = false;
    }
    let (start_locations, selected_item) = startup_plan.into_parts();
    let (mut app, pending_startup_restore) =
        ExplorerApp::new_at_accessible_start_deferring_startup_session(
            start_locations,
            file_system,
            shell,
            settings,
        )?;
    if let Some(selected_item) = selected_item {
        app.active_tab_mut()?.select_only(selected_item);
    }
    let instance = ui::module_handle()?;
    ui::register_window_class(
        instance,
        WINDOW_CLASS_NAME,
        Some(window_proc),
        Some(APP_ICON_RESOURCE_ID),
    )?;

    let window_create_ownership = MainWindowCreateOwnership::new();
    let window = Box::new(MainWindow::new(
        app,
        pending_startup_restore,
        settings_store,
        settings_save_enabled,
        shell_window,
        instance,
        window_create_ownership.clone(),
    )?);
    let window_ptr = Box::into_raw(window);
    let hwnd = match ui::create_main_window(
        instance,
        WINDOW_CLASS_NAME,
        WINDOW_TITLE,
        window_ptr.cast(),
        initial_scale.size(DEFAULT_WINDOW_WIDTH, DEFAULT_WINDOW_HEIGHT),
    ) {
        Ok(hwnd) => hwnd,
        Err(error) => {
            if !window_create_ownership.is_window_proc_owner() {
                // SAFETY: WM_NCCREATE did not attach MainWindow to the window, so the caller
                // still owns the Box converted by Box::into_raw above.
                unsafe {
                    drop(Box::from_raw(window_ptr));
                }
            }
            return Err(error);
        }
    };

    ui::show_window(hwnd);
    ui::set_window_timer(hwnd, ID_DEFERRED_STARTUP_TIMER, DEFERRED_STARTUP_DELAY_MS)?;
    let accelerators = create_shortcut_table()?;
    let control_key_commands = create_control_key_commands();
    ui::message_loop(hwnd, Some(&accelerators), &control_key_commands).map(|_| ())
}

fn create_shortcut_table() -> ExplorerResult<ui::AcceleratorTable> {
    ui::create_accelerator_table(&[
        ui::Accelerator::new(ui::KEY_LEFT, ID_NAV_BACK).alt(),
        ui::Accelerator::new(ui::KEY_RIGHT, ID_NAV_FORWARD).alt(),
        ui::Accelerator::new(ui::KEY_UP, ID_NAV_UP).alt(),
        ui::Accelerator::new(ui::KEY_F5, ID_REFRESH),
        ui::Accelerator::new(b'L' as u16, ID_ADDRESS_FOCUS).control(),
        ui::Accelerator::new(b'D' as u16, ID_ADDRESS_FOCUS).alt(),
        ui::Accelerator::new(b'F' as u16, ID_SEARCH_FOCUS).control(),
        ui::Accelerator::new(ui::KEY_F3, ID_SEARCH_FOCUS),
        ui::Accelerator::new(b'C' as u16, ID_FILE_COPY).control(),
        ui::Accelerator::new(b'X' as u16, ID_FILE_CUT).control(),
        ui::Accelerator::new(b'V' as u16, ID_FILE_PASTE).control(),
        ui::Accelerator::new(b'Z' as u16, ID_FILE_UNDO).control(),
        ui::Accelerator::new(ui::KEY_DELETE, ID_FILE_DELETE_PERMANENTLY).shift(),
        ui::Accelerator::new(ui::KEY_DELETE, ID_FILE_DELETE),
        ui::Accelerator::new(ui::KEY_F2, ID_FILE_RENAME),
        ui::Accelerator::new(b'A' as u16, ID_FILE_SELECT_ALL).control(),
        ui::Accelerator::new(b'T' as u16, ID_TAB_NEW).control(),
        ui::Accelerator::new(b'W' as u16, ID_TAB_CLOSE).control(),
        ui::Accelerator::new(ui::KEY_TAB, ID_TAB_NEXT).control(),
    ])
}

fn create_control_key_commands() -> [ui::ControlKeyCommand; 1] {
    [ui::ControlKeyCommand::new(
        ID_SEARCH_QUERY,
        ui::KEY_ESCAPE,
        ID_SEARCH_CLOSE,
    )]
}

struct MainWindow {
    app: ExplorerModel,
    settings_store: NativeUserSettingsStore,
    settings_save_enabled: bool,
    pending_user_settings_save: bool,
    user_settings_save_timer_active: bool,
    deferred_shell_startup_complete: bool,
    pending_startup_restore: Option<StartupSessionRestore>,
    icon_cache: Option<ShellIconCache>,
    shell_gateway: WindowsShellGateway,
    instance: ui::InstanceHandle,
    dpi_metrics: ui::DpiMetrics,
    ui_scale: ui::UiScale,
    folder_tree_width_px: i32,
    theme_resources: ui::ThemeResources,
    font_resource: ui::FontResource,
    navigation_icons: NavigationIcons,
    hwnd: ui::WindowHandle,
    menu_bar: ui::MenuHandle,
    tree_view: ui::WindowHandle,
    tab_control: ui::WindowHandle,
    back_button: ui::WindowHandle,
    forward_button: ui::WindowHandle,
    up_button: ui::WindowHandle,
    refresh_button: ui::WindowHandle,
    new_tab_button: ui::WindowHandle,
    address_edit: ui::WindowHandle,
    go_button: ui::WindowHandle,
    search_query_label: ui::WindowHandle,
    search_query_edit: ui::WindowHandle,
    search_find_button: ui::WindowHandle,
    search_subfolders_checkbox: ui::WindowHandle,
    search_cancel_button: ui::WindowHandle,
    file_operation_status_label: ui::WindowHandle,
    list_view: ui::WindowHandle,
    drop_event_queue: platform::OleDropEventQueue,
    drop_feedback: platform::OleDropFeedback,
    list_drop_target: Option<platform::OleDropTargetRegistration>,
    tree_drop_target: Option<platform::OleDropTargetRegistration>,
    active_internal_drag: Option<InternalDragState>,
    next_internal_drag_id: u64,
    current_items: CurrentItems,
    current_item_cell_text_caches: FileItemCellTextCaches,
    current_item_rows_synced_to_list_view: bool,
    list_view_status_row: Option<String>,
    current_listing_child_indices: Option<HashMap<Vec<u16>, usize>>,
    current_listing_rows: Option<CurrentListingRows>,
    current_search_rows: Option<CurrentSearchRows>,
    pending_listing_viewport_restore: Option<PendingListingViewportRestore>,
    pending_file_watch_refresh: PendingFileWatchRefresh,
    drive_menu_locations: Vec<NavigationLocation>,
    folder_tree_nodes: Vec<FolderTreeNodeState>,
    folder_tree_child_indices_by_parent: Vec<Vec<usize>>,
    folder_tree_selection_suppressed: bool,
    next_folder_tree_child_generation: u64,
    folder_tree_child_workers: Vec<ActiveFolderTreeChildWorker>,
    pending_folder_tree_child_workers: VecDeque<PendingFolderTreeChildWorker>,
    folder_tree_child_messages: FolderTreeChildrenMessageStore,
    icon_load_workers: Vec<JoinHandle<()>>,
    pending_icon_load_tasks: VecDeque<IconLoadTask>,
    icon_load_messages: IconLoadMessageStore,
    icon_load_shutdown_requested: Arc<AtomicBool>,
    search_controls_requested: bool,
    shutdown_after_file_operation: bool,
    workers: WorkerController,
    undo_file_operation: Option<UndoFileOperation>,
    size_move_dpi: SizeMoveDpiState,
    folder_tree_resize_drag: Option<FolderTreeResizeDrag>,
    create_ownership: MainWindowCreateOwnership,
}

#[derive(Clone)]
struct MainWindowCreateOwnership {
    window_proc_owns_state: Arc<AtomicBool>,
}

impl MainWindowCreateOwnership {
    fn new() -> Self {
        Self {
            window_proc_owns_state: Arc::new(AtomicBool::new(false)),
        }
    }

    fn mark_window_proc_owner(&self) {
        self.window_proc_owns_state.store(true, Ordering::Release);
    }

    fn is_window_proc_owner(&self) -> bool {
        self.window_proc_owns_state.load(Ordering::Acquire)
    }
}

#[derive(Default)]
struct SizeMoveDpiState {
    in_size_move: bool,
    dpi_refresh_pending: bool,
}

impl SizeMoveDpiState {
    fn enter(&mut self) {
        self.in_size_move = true;
        self.dpi_refresh_pending = false;
    }

    fn defer_dpi_refresh(&mut self) {
        if self.in_size_move {
            self.dpi_refresh_pending = true;
        }
    }

    fn should_defer_dpi_refresh(&self) -> bool {
        self.in_size_move
    }

    fn should_defer_layout(&self) -> bool {
        self.in_size_move && self.dpi_refresh_pending
    }

    fn exit(&mut self) -> SizeMoveDpiExit {
        let exit = SizeMoveDpiExit {
            dpi_refresh_pending: self.dpi_refresh_pending,
        };
        self.in_size_move = false;
        self.dpi_refresh_pending = false;
        exit
    }
}

struct SizeMoveDpiExit {
    dpi_refresh_pending: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FolderTreeResizeDrag {
    tree_width_delta: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct HorizontalPaneLayout {
    tree_width: i32,
    splitter_x: i32,
    splitter_width: i32,
    right_x: i32,
    right_width: i32,
}

impl HorizontalPaneLayout {
    fn contains_splitter_x(self, x: i32) -> bool {
        self.splitter_width > 0 && x >= self.splitter_x && x < self.splitter_x + self.splitter_width
    }
}

struct MainWindowWorkerBoundary<'a> {
    hwnd: ui::WindowHandle,
    workers: &'a WorkerController,
}

impl<'a> MainWindowWorkerBoundary<'a> {
    fn new(hwnd: ui::WindowHandle, workers: &'a WorkerController) -> Self {
        Self { hwnd, workers }
    }

    fn spawn_search_worker(
        &self,
        request: SearchRequest,
        cancel_requested: Arc<AtomicBool>,
        io_cancellation: Arc<platform::SynchronousIoCancellation>,
    ) -> ExplorerResult<JoinHandle<()>> {
        let hwnd_value = self.hwnd.as_isize();
        let shutdown_requested = self.workers.search_shutdown_requested();
        let worker_messages = self.workers.messages.clone();
        spawn_background_worker(
            "j3files-search-worker",
            "start search worker thread",
            move || {
                let tab_id = request.tab_id;
                let run_id = request.run_id;
                let _io_cancellation_registration = {
                    let registration = io_cancellation.register_current_thread();
                    match registration {
                        Ok(registration) => Some(registration),
                        Err(error) => {
                            eprintln!(
                                "search worker for tab {:?}, run {:?} cannot cancel synchronous I/O: {error}",
                                tab_id, run_id
                            );
                            None
                        }
                    }
                };
                let file_system = NativeFileSystemGateway::new();
                let cancellation = SharedSearchCancellation {
                    requested: cancel_requested,
                };
                let progress_reporter = UiSearchProgressReporter {
                    hwnd_value,
                    tab_id: request.tab_id,
                    run_id: request.run_id,
                    shutdown_requested: Arc::clone(&shutdown_requested),
                    worker_messages: worker_messages.clone(),
                };
                let result = file_system
                    .search_items(
                        &request.root,
                        &request.criteria,
                        request.display_options,
                        request.sort,
                        &cancellation,
                        &progress_reporter,
                    )
                    .map(|outcome| SearchOutcome::from_request(request, outcome));

                let message = SearchCompleteMessage {
                    tab_id,
                    run_id,
                    result,
                };
                if !shutdown_requested.load(Ordering::Relaxed) {
                    worker_messages.post_search_complete(hwnd_value, message);
                }
            },
        )
    }

    fn spawn_file_watch_worker(
        &self,
        generation: u64,
        location: NavigationLocation,
        cancellation: Arc<platform::DirectoryChangeCancellation>,
    ) -> ExplorerResult<JoinHandle<()>> {
        let hwnd_value = self.hwnd.as_isize();
        let worker_messages = self.workers.messages.clone();
        spawn_background_worker(
            "j3files-file-watch-worker",
            "start file watch worker thread",
            move || {
                let result = platform::watch_directory_changes(
                    location.as_path(),
                    &cancellation,
                    |changes| {
                        worker_messages.post_file_watch_changed(
                            hwnd_value,
                            FileWatchChangeMessage {
                                generation,
                                changes,
                            },
                        );
                        Ok(())
                    },
                );

                let cancelled = cancellation.is_cancel_requested().unwrap_or(false);
                if let Err(error) = result {
                    if !cancelled {
                        eprintln!(
                            "file watch worker for generation {generation} at {:?} stopped: {error}",
                            location.as_path()
                        );
                    }
                }
            },
        )
    }

    fn spawn_listing_worker(
        &self,
        request: ListingRequest,
        cancel_requested: Arc<AtomicBool>,
        io_cancellation: Arc<platform::SynchronousIoCancellation>,
    ) -> ExplorerResult<JoinHandle<()>> {
        let hwnd_value = self.hwnd.as_isize();
        let shutdown_requested = self.workers.listing_shutdown_requested();
        let worker_messages = self.workers.messages.clone();
        spawn_background_worker(
            "j3files-listing-worker",
            "start listing worker thread",
            move || {
                let mut result: ExplorerResult<Vec<FileItem>> = Ok(Vec::new());
                if !cancel_requested.load(Ordering::Relaxed)
                    && !shutdown_requested.load(Ordering::Relaxed)
                {
                    let _io_cancellation_registration = {
                        let registration = io_cancellation.register_current_thread();
                        match registration {
                            Ok(registration) => Some(registration),
                            Err(error) => {
                                eprintln!(
                                    "listing worker for tab {:?}, generation {} cannot cancel synchronous I/O: {error}",
                                    request.tab_id, request.generation
                                );
                                None
                            }
                        }
                    };
                    let file_system = NativeFileSystemGateway::new();
                    let cancellation = SharedSearchCancellation {
                        requested: Arc::clone(&cancel_requested),
                    };
                    result = file_system.list_items_with_cancellation(
                        &request.location,
                        request.display_options,
                        request.sort,
                        &cancellation,
                    );
                }

                if shutdown_requested.load(Ordering::Relaxed) {
                    return;
                }
                if cancel_requested.load(Ordering::Relaxed) {
                    result = Ok(Vec::new());
                }

                worker_messages
                    .post_listing_complete(hwnd_value, ListingCompleteMessage { request, result });
            },
        )
    }

    fn spawn_folder_tree_children_worker(
        &self,
        request: FolderTreeChildrenRequest,
        cancel_requested: Arc<AtomicBool>,
        completion_message_abandoned: Arc<AtomicBool>,
        io_cancellation: Arc<platform::SynchronousIoCancellation>,
        worker_messages: FolderTreeChildrenMessageStore,
    ) -> ExplorerResult<JoinHandle<()>> {
        let hwnd_value = self.hwnd.as_isize();
        spawn_background_worker(
            "j3files-folder-tree-worker",
            "start folder tree worker thread",
            move || {
                if cancel_requested.load(Ordering::Relaxed) {
                    return;
                }

                let _io_cancellation_registration = {
                    let registration = io_cancellation.register_current_thread();
                    match registration {
                        Ok(registration) => Some(registration),
                        Err(error) => {
                            eprintln!(
                                "folder tree child worker for parent {}, generation {} cannot cancel synchronous I/O: {error}",
                                request.parent_index, request.generation
                            );
                            None
                        }
                    }
                };
                let file_system = NativeFileSystemGateway::new();
                let cancellation = SharedSearchCancellation {
                    requested: Arc::clone(&cancel_requested),
                };
                let result = match request.kind {
                    FolderTreeChildrenRequestKind::LoadChildren
                    | FolderTreeChildrenRequestKind::RefreshLoadedChildren => file_system
                        .list_folder_tree_children_with_cancellation(
                            &request.location,
                            request.display_options,
                            &cancellation,
                        )
                        .map(FolderTreeChildrenWorkerResult::Children),
                    FolderTreeChildrenRequestKind::RefreshChildPresence => file_system
                        .has_child_folders_with_cancellation(
                            &request.location,
                            request.display_options,
                            &cancellation,
                        )
                        .map(FolderTreeChildrenWorkerResult::ChildPresence),
                };

                if cancel_requested.load(Ordering::Relaxed) {
                    return;
                }

                let Some(token) = worker_messages
                    .insert_complete(FolderTreeChildrenCompleteMessage { request, result })
                else {
                    return;
                };

                let hwnd = ui::WindowHandle::from_isize(hwnd_value);
                if let Err(error) =
                    ui::post_window_message(hwnd, MESSAGE_FOLDER_TREE_CHILDREN_COMPLETE, 0, token)
                {
                    eprintln!("failed to post folder tree children completion: {error}");
                    completion_message_abandoned.store(true, Ordering::Relaxed);
                    worker_messages.take_complete(token);
                }
            },
        )
    }
}

impl MainWindow {
    fn new(
        app: ExplorerModel,
        pending_startup_restore: Option<StartupSessionRestore>,
        settings_store: NativeUserSettingsStore,
        settings_save_enabled: bool,
        shell_gateway: WindowsShellGateway,
        instance: ui::InstanceHandle,
        create_ownership: MainWindowCreateOwnership,
    ) -> ExplorerResult<Self> {
        let theme_resources = ui::ThemeResources::new(app.appearance_theme())?;
        let dpi_metrics = ui::system_dpi_metrics();
        let ui_scale = dpi_metrics.ui_scale();
        let font_resource = ui::FontResource::new(app.appearance_font(), dpi_metrics)?;
        let navigation_icons = NavigationIcons::load(instance, ui_scale)?;
        Ok(Self {
            app,
            settings_store,
            settings_save_enabled,
            pending_user_settings_save: false,
            user_settings_save_timer_active: false,
            deferred_shell_startup_complete: false,
            pending_startup_restore,
            icon_cache: None,
            shell_gateway,
            instance,
            dpi_metrics,
            ui_scale,
            folder_tree_width_px: ui_scale.px(DEFAULT_FOLDER_TREE_WIDTH),
            theme_resources,
            font_resource,
            navigation_icons,
            hwnd: ui::WindowHandle::null(),
            menu_bar: ui::MenuHandle::null(),
            tree_view: ui::WindowHandle::null(),
            tab_control: ui::WindowHandle::null(),
            back_button: ui::WindowHandle::null(),
            forward_button: ui::WindowHandle::null(),
            up_button: ui::WindowHandle::null(),
            refresh_button: ui::WindowHandle::null(),
            new_tab_button: ui::WindowHandle::null(),
            address_edit: ui::WindowHandle::null(),
            go_button: ui::WindowHandle::null(),
            search_query_label: ui::WindowHandle::null(),
            search_query_edit: ui::WindowHandle::null(),
            search_find_button: ui::WindowHandle::null(),
            search_subfolders_checkbox: ui::WindowHandle::null(),
            search_cancel_button: ui::WindowHandle::null(),
            file_operation_status_label: ui::WindowHandle::null(),
            list_view: ui::WindowHandle::null(),
            drop_event_queue: platform::OleDropEventQueue::new(),
            drop_feedback: platform::OleDropFeedback::new(),
            list_drop_target: None,
            tree_drop_target: None,
            active_internal_drag: None,
            next_internal_drag_id: 1,
            current_items: CurrentItems::default(),
            current_item_cell_text_caches: FileItemCellTextCaches::new(),
            current_item_rows_synced_to_list_view: false,
            list_view_status_row: None,
            current_listing_child_indices: None,
            current_listing_rows: None,
            current_search_rows: None,
            pending_listing_viewport_restore: None,
            pending_file_watch_refresh: PendingFileWatchRefresh::default(),
            drive_menu_locations: Vec::new(),
            folder_tree_nodes: Vec::new(),
            folder_tree_child_indices_by_parent: Vec::new(),
            folder_tree_selection_suppressed: false,
            next_folder_tree_child_generation: 1,
            folder_tree_child_workers: Vec::new(),
            pending_folder_tree_child_workers: VecDeque::new(),
            folder_tree_child_messages: FolderTreeChildrenMessageStore::default(),
            icon_load_workers: Vec::new(),
            pending_icon_load_tasks: VecDeque::new(),
            icon_load_messages: IconLoadMessageStore::default(),
            icon_load_shutdown_requested: Arc::new(AtomicBool::new(false)),
            search_controls_requested: false,
            shutdown_after_file_operation: false,
            workers: WorkerController::new(),
            undo_file_operation: None,
            size_move_dpi: SizeMoveDpiState::default(),
            folder_tree_resize_drag: None,
            create_ownership,
        })
    }

    fn mark_window_proc_owner(&self) {
        self.create_ownership.mark_window_proc_owner();
    }

    fn worker_boundary(&self) -> MainWindowWorkerBoundary<'_> {
        MainWindowWorkerBoundary::new(self.hwnd, &self.workers)
    }

    fn register_drop_targets(&mut self) -> ExplorerResult<()> {
        let auto_scroll_edge = self.ui_scale.px(DRAG_AUTO_SCROLL_EDGE);
        self.tree_drop_target = Some(platform::register_file_drop_target(
            self.tree_view,
            self.hwnd,
            MESSAGE_OLE_DROP_EVENT,
            platform::OleDropTargetKind::FolderTree,
            self.drop_event_queue.clone(),
            self.drop_feedback.clone(),
            platform::OleDropFeedbackTimerConfig::new(
                ID_TREE_DRAG_FEEDBACK_TIMER,
                DRAG_FEEDBACK_POLL_MS,
                auto_scroll_edge,
            ),
        )?);
        self.list_drop_target = Some(platform::register_file_drop_target(
            self.list_view,
            self.hwnd,
            MESSAGE_OLE_DROP_EVENT,
            platform::OleDropTargetKind::FileList,
            self.drop_event_queue.clone(),
            self.drop_feedback.clone(),
            platform::OleDropFeedbackTimerConfig::new(
                ID_LIST_DRAG_FEEDBACK_TIMER,
                DRAG_FEEDBACK_POLL_MS,
                auto_scroll_edge,
            ),
        )?);
        Ok(())
    }

    fn unregister_drop_targets(&mut self) {
        self.list_drop_target = None;
        self.tree_drop_target = None;
    }

    fn refresh_drop_feedback(&self) {
        let file_operation_idle =
            !self.workers.has_file_operation_worker() && !self.shutdown_after_file_operation;
        let external_hint = if self.app.active_tab().is_ok() && file_operation_idle {
            platform::OleDropEffectHint::copy_move(None)
        } else {
            platform::OleDropEffectHint::none()
        };
        let internal_drop_operation = if file_operation_idle {
            self.active_internal_drag
                .as_ref()
                .map(|drag| DropSourceKind::internal_operation_resolver(&drag.sources))
        } else {
            None
        };
        let file_list_internal_hints = if let (Some(operation_for_destination), Ok(active_tab)) =
            (internal_drop_operation.as_ref(), self.app.active_tab())
        {
            if matches!(&active_tab.search, SearchState::Idle) {
                let items = self.current_items.as_slice(active_tab);
                collect_non_empty_drop_effect_hints(items.len(), items.iter(), |item| {
                    if item.is_folder() {
                        self.internal_drop_effect_hint(operation_for_destination, &item.location)
                    } else {
                        platform::OleDropEffectHint::none()
                    }
                })
            } else {
                Vec::new()
            }
        } else {
            Vec::new()
        };
        let file_list_internal_empty_hint =
            if let (Some(active_drag), Some(operation_for_destination), Ok(active_tab)) = (
                self.active_internal_drag.as_ref(),
                internal_drop_operation.as_ref(),
                self.app.active_tab(),
            ) {
                if active_drag.origin == InternalDragOrigin::FolderTree
                    && matches!(&active_tab.search, SearchState::Idle)
                {
                    self.internal_drop_effect_hint(
                        operation_for_destination,
                        active_tab.current_location(),
                    )
                } else {
                    platform::OleDropEffectHint::none()
                }
            } else {
                platform::OleDropEffectHint::none()
            };
        self.drop_feedback.set_file_list_hints(
            external_hint,
            file_list_internal_empty_hint,
            file_list_internal_hints,
        );

        let folder_tree_internal_hints = if let Some(operation_for_destination) =
            internal_drop_operation.as_ref()
        {
            collect_non_empty_drop_effect_hints(
                self.folder_tree_nodes.len(),
                self.folder_tree_nodes.iter(),
                |node| self.internal_drop_effect_hint(operation_for_destination, &node.location),
            )
        } else {
            Vec::new()
        };
        self.drop_feedback
            .set_folder_tree_hints(external_hint, folder_tree_internal_hints);
    }

    fn internal_drop_effect_hint<F>(
        &self,
        operation_for_destination: &F,
        destination: &NavigationLocation,
    ) -> platform::OleDropEffectHint
    where
        F: Fn(&NavigationLocation, DropModifierKeys) -> ExplorerResult<DropOperation>,
    {
        let Ok(default_operation) =
            operation_for_destination(destination, DropModifierKeys::default())
        else {
            return platform::OleDropEffectHint::none();
        };

        match default_operation {
            DropOperation::Copy => {
                platform::OleDropEffectHint::copy_move(Some(platform::OleDropPreferredEffect::Copy))
            }
            DropOperation::Move => {
                platform::OleDropEffectHint::copy_move(Some(platform::OleDropPreferredEffect::Move))
            }
        }
    }

    fn internal_file_list_empty_drop_destination(
        &self,
        active_drag: &InternalDragState,
    ) -> ExplorerResult<Option<NavigationLocation>> {
        let active_tab = self.app.active_tab()?;
        Ok(internal_empty_file_list_drop_destination(
            active_drag.origin,
            &active_tab.search,
            active_tab.current_location(),
        ))
    }

    fn on_create(&mut self, hwnd: ui::WindowHandle) -> ExplorerResult<()> {
        self.hwnd = hwnd;
        self.sync_window_title()?;
        self.apply_dpi_metrics(ui::dpi_metrics_for_window(hwnd), false)?;
        self.shell_gateway.set_owner_window(hwnd.as_isize());
        self.create_menu()?;

        self.tree_view = ui::create_tree_view(hwnd, self.instance, ID_FOLDER_TREE)?;
        self.tab_control = ui::create_tab_control(hwnd, self.instance, ID_TAB_CONTROL)?;
        self.back_button = ui::create_icon_button(hwnd, self.instance, ID_NAV_BACK, "Back")?;
        self.forward_button =
            ui::create_icon_button(hwnd, self.instance, ID_NAV_FORWARD, "Forward")?;
        self.up_button = ui::create_icon_button(hwnd, self.instance, ID_NAV_UP, "Up")?;
        self.refresh_button = ui::create_icon_button(hwnd, self.instance, ID_REFRESH, "Refresh")?;
        self.new_tab_button = ui::create_button(hwnd, self.instance, ID_TAB_NEW, "+")?;
        self.address_edit = ui::create_address_edit(hwnd, self.instance, ID_ADDRESS)?;
        self.go_button = ui::create_icon_button(hwnd, self.instance, ID_GO, "Go")?;
        self.search_query_label =
            ui::create_label(hwnd, self.instance, ID_SEARCH_QUERY_LABEL, "Name")?;
        self.search_query_edit = ui::create_address_edit(hwnd, self.instance, ID_SEARCH_QUERY)?;
        self.search_find_button = ui::create_button(hwnd, self.instance, ID_SEARCH_FIND, "find")?;
        self.search_subfolders_checkbox =
            ui::create_checkbox(hwnd, self.instance, ID_SEARCH_SUBFOLDERS, "sub")?;
        self.search_cancel_button =
            ui::create_icon_button(hwnd, self.instance, ID_SEARCH_CANCEL, "Cancel Search")?;
        self.file_operation_status_label =
            ui::create_label(hwnd, self.instance, ID_FILE_OPERATION_STATUS, "")?;
        self.list_view = ui::create_report_list_view(hwnd, self.instance, ID_FILE_LIST)?;
        self.register_drop_targets()?;

        ui::set_list_view_columns(
            self.list_view,
            &[
                ui::ListViewColumn {
                    title: "Name",
                    width: self.ui_scale.px(LIST_NAME_COLUMN_WIDTH),
                    align: ui::ColumnAlign::Left,
                },
                ui::ListViewColumn {
                    title: "Type",
                    width: self.ui_scale.px(LIST_TYPE_COLUMN_WIDTH),
                    align: ui::ColumnAlign::Left,
                },
                ui::ListViewColumn {
                    title: "Size",
                    width: self.ui_scale.px(LIST_SIZE_COLUMN_WIDTH),
                    align: ui::ColumnAlign::Right,
                },
                ui::ListViewColumn {
                    title: "Updated",
                    width: self.ui_scale.px(LIST_UPDATED_COLUMN_WIDTH),
                    align: ui::ColumnAlign::Left,
                },
            ],
        )?;

        self.apply_theme();
        self.apply_font();
        self.layout()?;
        self.refresh_view()
    }

    fn on_deferred_startup_timer(&mut self) -> ExplorerResult<()> {
        ui::kill_window_timer(self.hwnd, ID_DEFERRED_STARTUP_TIMER)?;
        self.initialize_deferred_shell_startup()
    }

    fn initialize_deferred_shell_startup(&mut self) -> ExplorerResult<()> {
        if self.deferred_shell_startup_complete {
            return Ok(());
        }

        self.deferred_shell_startup_complete = true;
        let restored_startup_session = self.apply_pending_startup_restore()?;
        if restored_startup_session {
            self.refresh_view()?;
        }
        self.initialize_icon_cache()?;
        self.create_menu()?;
        self.rebuild_folder_tree()
    }

    fn apply_pending_startup_restore(&mut self) -> ExplorerResult<bool> {
        let Some(restore) = self.pending_startup_restore.take() else {
            return Ok(false);
        };

        self.app.apply_deferred_startup_restore(restore)?;
        Ok(true)
    }

    fn initialize_icon_cache(&mut self) -> ExplorerResult<()> {
        if self.icon_cache.is_some() {
            return Ok(());
        }

        let icon_cache = ShellIconCache::new()?;
        if !self.list_view.is_null() {
            ui::set_list_view_small_image_list(self.list_view, icon_cache.system_image_list())?;
        }
        self.icon_cache = Some(icon_cache);
        self.refresh_current_item_icons()
    }

    fn apply_dpi_metrics(
        &mut self,
        metrics: ui::DpiMetrics,
        update_existing_controls: bool,
    ) -> ExplorerResult<()> {
        if metrics == self.dpi_metrics && update_existing_controls {
            return Ok(());
        }

        let previous_metrics = self.dpi_metrics;
        let next_ui_scale = metrics.ui_scale();
        let next_font_resource = ui::FontResource::new(self.app.appearance_font(), metrics)?;
        let next_navigation_icons = NavigationIcons::load(self.instance, next_ui_scale)?;

        self.dpi_metrics = metrics;
        self.ui_scale = next_ui_scale;
        self.folder_tree_width_px =
            scale_px_between_dpi(self.folder_tree_width_px, previous_metrics, metrics);
        let old_font_resource = std::mem::replace(&mut self.font_resource, next_font_resource);
        self.navigation_icons = next_navigation_icons;

        if update_existing_controls {
            self.apply_font();
            self.layout()?;
        }

        drop(old_font_resource);
        Ok(())
    }

    fn on_dpi_changed(&mut self, metrics: ui::DpiMetrics) -> ExplorerResult<()> {
        if self.size_move_dpi.should_defer_dpi_refresh() {
            self.size_move_dpi.defer_dpi_refresh();
            return Ok(());
        }

        if metrics == self.dpi_metrics {
            return Ok(());
        }

        self.apply_dpi_metrics(metrics, true)
    }

    fn enter_size_move(&mut self) {
        self.size_move_dpi.enter();
    }

    fn exit_size_move(&mut self, hwnd: ui::WindowHandle) -> ExplorerResult<()> {
        let exit = self.size_move_dpi.exit();
        if exit.dpi_refresh_pending {
            let previous_metrics = self.dpi_metrics;
            self.apply_dpi_metrics(ui::dpi_metrics_for_window(hwnd), true)?;
            if self.dpi_metrics == previous_metrics {
                self.layout()?;
            }
        }

        Ok(())
    }

    fn on_size(&mut self) -> ExplorerResult<()> {
        if self.size_move_dpi.should_defer_layout() {
            return Ok(());
        }

        self.layout()
    }

    fn on_set_cursor(&self) -> ExplorerResult<bool> {
        if self.folder_tree_resize_drag.is_some() {
            ui::set_horizontal_resize_cursor();
            return Ok(true);
        }

        let point = ui::screen_to_client_point(self.hwnd, ui::cursor_position()?)?;
        if self.folder_tree_splitter_contains(point)? {
            ui::set_horizontal_resize_cursor();
            return Ok(true);
        }

        Ok(false)
    }

    fn on_mouse_move(&mut self, lparam: ui::MessageLong) -> ExplorerResult<bool> {
        let point = ui::client_point_from_message_lparam(lparam);
        if self.folder_tree_resize_drag.is_some() {
            self.update_folder_tree_resize(point)?;
            ui::set_horizontal_resize_cursor();
            return Ok(true);
        }

        if self.folder_tree_splitter_contains(point)? {
            ui::set_horizontal_resize_cursor();
            return Ok(true);
        }

        Ok(false)
    }

    fn on_left_button_down(&mut self, lparam: ui::MessageLong) -> ExplorerResult<bool> {
        let point = ui::client_point_from_message_lparam(lparam);
        if !self.folder_tree_splitter_contains(point)? {
            return Ok(false);
        }

        let client = ui::client_rect(self.hwnd)?;
        let pane = self.horizontal_pane_layout(client.width, self.folder_tree_width_px);
        self.folder_tree_resize_drag = Some(FolderTreeResizeDrag {
            tree_width_delta: pane.tree_width - point.x,
        });
        ui::set_mouse_capture(self.hwnd);
        ui::set_horizontal_resize_cursor();
        Ok(true)
    }

    fn on_left_button_up(&mut self) -> bool {
        if self.folder_tree_resize_drag.is_none() {
            return false;
        }

        self.folder_tree_resize_drag = None;
        if ui::window_has_mouse_capture(self.hwnd) {
            ui::release_mouse_capture();
        }
        true
    }

    fn on_capture_changed(&mut self) {
        self.folder_tree_resize_drag = None;
    }

    fn update_folder_tree_resize(&mut self, point: ui::ClientPoint) -> ExplorerResult<()> {
        let Some(drag) = self.folder_tree_resize_drag else {
            return Ok(());
        };
        let client = ui::client_rect(self.hwnd)?;
        let desired_tree_width = point.x + drag.tree_width_delta;
        let pane = self.horizontal_pane_layout(client.width, desired_tree_width);
        if pane.tree_width != self.folder_tree_width_px {
            self.folder_tree_width_px = pane.tree_width;
            self.layout()?;
        }

        Ok(())
    }

    fn folder_tree_splitter_contains(&self, point: ui::ClientPoint) -> ExplorerResult<bool> {
        let client = ui::client_rect(self.hwnd)?;
        if point.y < 0 || point.y >= client.height {
            return Ok(false);
        }

        let pane = self.horizontal_pane_layout(client.width, self.folder_tree_width_px);
        Ok(pane.contains_splitter_x(point.x))
    }

    fn horizontal_pane_layout(
        &self,
        client_width: i32,
        desired_tree_width: i32,
    ) -> HorizontalPaneLayout {
        build_horizontal_pane_layout(
            client_width,
            self.ui_scale.px(MARGIN),
            self.ui_scale.px(FOLDER_TREE_SPLITTER_WIDTH),
            desired_tree_width,
            self.ui_scale.px(MIN_FOLDER_TREE_WIDTH),
            self.ui_scale.px(MIN_RIGHT_PANE_WIDTH),
        )
    }

    fn apply_minimum_tracking_size(&self, lparam: ui::MessageLong) -> bool {
        let (width, height) = self.ui_scale.size(MIN_WINDOW_WIDTH, MIN_WINDOW_HEIGHT);
        ui::set_minimum_tracking_size(lparam, width, height)
    }

    fn create_menu(&mut self) -> ExplorerResult<()> {
        let menu = ui::OwnedMenu::menu_bar()?;

        let file_menu = ui::OwnedMenu::popup()?;
        ui::append_menu_item(file_menu.handle(), ID_FILE_NEW_FOLDER, "New Folder")?;
        ui::append_menu_separator(file_menu.handle())?;
        ui::append_menu_item(file_menu.handle(), ID_FILE_OPEN, "Open")?;
        ui::append_menu_item(file_menu.handle(), ID_FILE_OPEN_WITH, "Open With...")?;
        ui::append_menu_separator(file_menu.handle())?;
        ui::append_menu_item(file_menu.handle(), ID_FILE_RENAME, "Rename")?;
        ui::append_menu_item(file_menu.handle(), ID_FILE_DELETE, "Move to Recycle Bin")?;
        ui::append_menu_item(
            file_menu.handle(),
            ID_FILE_DELETE_PERMANENTLY,
            "Delete Permanently",
        )?;
        ui::append_menu_separator(file_menu.handle())?;
        ui::append_menu_item(file_menu.handle(), ID_FILE_PROPERTIES, "Properties")?;
        ui::append_menu_separator(file_menu.handle())?;
        ui::append_menu_item(file_menu.handle(), ID_EXIT, "Exit")?;
        ui::append_owned_menu_popup(menu.handle(), file_menu, "File")?;

        let edit_menu = ui::OwnedMenu::popup()?;
        ui::append_menu_item(edit_menu.handle(), ID_FILE_UNDO, "Undo")?;
        ui::append_menu_separator(edit_menu.handle())?;
        ui::append_menu_item(edit_menu.handle(), ID_FILE_CUT, "Cut")?;
        ui::append_menu_item(edit_menu.handle(), ID_FILE_COPY, "Copy")?;
        ui::append_menu_item(edit_menu.handle(), ID_FILE_PASTE, "Paste")?;
        ui::append_menu_separator(edit_menu.handle())?;
        ui::append_menu_item(edit_menu.handle(), ID_FILE_SELECT_ALL, "Select All")?;
        ui::append_owned_menu_popup(menu.handle(), edit_menu, "Edit")?;

        let view_menu = ui::OwnedMenu::popup()?;
        ui::append_menu_item(view_menu.handle(), ID_REFRESH, "Refresh")?;
        ui::append_menu_separator(view_menu.handle())?;
        let sort_menu = ui::OwnedMenu::popup()?;
        let active_sort = self.app.active_tab()?.sort;
        ui::append_checked_menu_item(
            sort_menu.handle(),
            ID_SORT_NAME,
            "Name",
            active_sort.key == SortKey::Name,
        )?;
        ui::append_checked_menu_item(
            sort_menu.handle(),
            ID_SORT_SIZE,
            "Size",
            active_sort.key == SortKey::Size,
        )?;
        ui::append_checked_menu_item(
            sort_menu.handle(),
            ID_SORT_UPDATED,
            "Updated",
            active_sort.key == SortKey::UpdatedAt,
        )?;
        ui::append_checked_menu_item(
            sort_menu.handle(),
            ID_SORT_KIND,
            "Type",
            active_sort.key == SortKey::Kind,
        )?;
        ui::append_menu_separator(sort_menu.handle())?;
        ui::append_checked_menu_item(
            sort_menu.handle(),
            ID_SORT_ASCENDING,
            "Ascending",
            active_sort.direction == SortDirection::Ascending,
        )?;
        ui::append_checked_menu_item(
            sort_menu.handle(),
            ID_SORT_DESCENDING,
            "Descending",
            active_sort.direction == SortDirection::Descending,
        )?;
        ui::append_owned_menu_popup(view_menu.handle(), sort_menu, "Sort By")?;
        ui::append_menu_separator(view_menu.handle())?;
        let display_options = self.app.display_options();
        ui::append_checked_menu_item(
            view_menu.handle(),
            ID_VIEW_SHOW_HIDDEN,
            "Show Hidden Files",
            display_options.show_hidden,
        )?;
        ui::append_checked_menu_item(
            view_menu.handle(),
            ID_VIEW_SHOW_SYSTEM,
            "Show System Files",
            display_options.show_system,
        )?;
        ui::append_menu_separator(view_menu.handle())?;

        let appearance_menu = ui::OwnedMenu::popup()?;
        let theme_menu = ui::OwnedMenu::popup()?;
        for theme in AppearanceTheme::options() {
            ui::append_checked_menu_item(
                theme_menu.handle(),
                command_for_appearance_theme(*theme),
                theme.display_name(),
                *theme == self.app.appearance_theme(),
            )?;
        }
        ui::append_owned_menu_popup(appearance_menu.handle(), theme_menu, "Theme")?;
        ui::append_menu_item(
            appearance_menu.handle(),
            ID_VIEW_FONT,
            &appearance_font_menu_label(self.app.appearance_font()),
        )?;
        ui::append_menu_item(appearance_menu.handle(), ID_VIEW_FONT_RESET, "Reset Font")?;
        ui::append_owned_menu_popup(view_menu.handle(), appearance_menu, "Appearance")?;
        ui::append_owned_menu_popup(menu.handle(), view_menu, "View")?;

        let go_menu = ui::OwnedMenu::popup()?;
        ui::append_menu_item(go_menu.handle(), ID_NAV_BACK, "Back")?;
        ui::append_menu_item(go_menu.handle(), ID_NAV_FORWARD, "Forward")?;
        ui::append_menu_item(go_menu.handle(), ID_NAV_UP, "Up One Level")?;
        ui::append_menu_separator(go_menu.handle())?;
        ui::append_menu_item(go_menu.handle(), ID_KNOWN_HOME, "Home")?;
        ui::append_menu_item(go_menu.handle(), ID_KNOWN_DESKTOP, "Desktop")?;
        ui::append_menu_item(go_menu.handle(), ID_KNOWN_DOWNLOADS, "Downloads")?;
        ui::append_menu_item(go_menu.handle(), ID_KNOWN_DOCUMENTS, "Documents")?;
        ui::append_menu_separator(go_menu.handle())?;
        let drive_menu = ui::OwnedMenu::popup()?;
        let drive_menu_locations = self.append_drive_menu_items(drive_menu.handle())?;
        ui::append_owned_menu_popup(go_menu.handle(), drive_menu, "Drives")?;
        ui::append_owned_menu_popup(menu.handle(), go_menu, "Go")?;

        let bookmark_menu = ui::OwnedMenu::popup()?;
        ui::append_menu_item(
            bookmark_menu.handle(),
            ID_BOOKMARK_ADD_CURRENT,
            "Add Current Location",
        )?;
        ui::append_menu_item(
            bookmark_menu.handle(),
            ID_BOOKMARK_ADD_SELECTED_FOLDER,
            "Add Selected Folder",
        )?;
        ui::append_menu_separator(bookmark_menu.handle())?;
        ui::append_menu_item(
            bookmark_menu.handle(),
            ID_BOOKMARK_REMOVE_CURRENT,
            "Remove Bookmark for Current Location",
        )?;
        if !self.app.state().bookmarks.items().is_empty() {
            ui::append_menu_separator(bookmark_menu.handle())?;
            for (index, item) in self
                .app
                .state()
                .bookmarks
                .items()
                .iter()
                .take(MAX_BOOKMARK_MENU_ITEMS)
                .enumerate()
            {
                let offset = u16::try_from(index).map_err(|_| {
                    ExplorerError::state_conflict("북마크 메뉴 항목이 너무 많습니다.")
                })?;
                let id = ID_BOOKMARK_BASE.checked_add(offset).ok_or_else(|| {
                    ExplorerError::state_conflict("북마크 메뉴 항목이 너무 많습니다.")
                })?;
                let label = display_os(item.display_name.as_os_str());
                ui::append_menu_item(bookmark_menu.handle(), id, &label)?;
            }
        }
        ui::append_owned_menu_popup(menu.handle(), bookmark_menu, "Bookmarks")?;

        let tab_menu = ui::OwnedMenu::popup()?;
        self.populate_tab_menu(tab_menu.handle())?;
        ui::append_owned_menu_popup(menu.handle(), tab_menu, "Tabs")?;

        let search_menu = ui::OwnedMenu::popup()?;
        ui::append_menu_item(search_menu.handle(), ID_SEARCH_FIND, "Find...")?;
        let include_subfolders = ui::is_button_checked(self.search_subfolders_checkbox);
        ui::append_checked_menu_item(
            search_menu.handle(),
            ID_SEARCH_INCLUDE_SUBFOLDERS,
            "Include Subfolders",
            include_subfolders,
        )?;
        ui::append_menu_separator(search_menu.handle())?;
        ui::append_menu_item(search_menu.handle(), ID_SEARCH_CANCEL, "Cancel Search")?;
        ui::append_menu_item(search_menu.handle(), ID_SEARCH_CLOSE, "Close Search")?;
        ui::append_owned_menu_popup(menu.handle(), search_menu, "Search")?;

        let about_menu = ui::OwnedMenu::popup()?;
        ui::append_menu_item(about_menu.handle(), ID_ABOUT, "About j3Files")?;
        ui::append_owned_menu_popup(menu.handle(), about_menu, "About")?;

        let old_menu = self.menu_bar;
        let new_menu = menu.handle();
        ui::set_window_menu(self.hwnd, new_menu)?;
        self.menu_bar = menu.release();
        self.drive_menu_locations = drive_menu_locations;
        if !old_menu.is_null() {
            ui::destroy_menu(old_menu)?;
        }
        ui::draw_menu_bar(self.hwnd)
    }

    fn populate_tab_menu(&self, tab_menu: ui::MenuHandle) -> ExplorerResult<()> {
        ui::append_menu_item(tab_menu, ID_TAB_NEW, "New Tab")?;
        ui::append_menu_item(
            tab_menu,
            ID_TAB_OPEN_SELECTED_FOLDER,
            "Open Selected Folder in New Tab",
        )?;
        ui::append_menu_item(tab_menu, ID_TAB_CLOSE, "Close Tab")?;
        ui::append_menu_item(tab_menu, ID_TAB_NEXT, "Next Tab")?;
        ui::append_menu_item(tab_menu, ID_TAB_REOPEN, "Reopen Closed Tab")?;
        ui::append_menu_separator(tab_menu)?;
        ui::append_menu_item(tab_menu, ID_TAB_MOVE_LEFT, "Move Tab Left")?;
        ui::append_menu_item(tab_menu, ID_TAB_MOVE_RIGHT, "Move Tab Right")?;
        ui::append_menu_separator(tab_menu)?;

        let startup_menu = ui::OwnedMenu::popup()?;
        let active_location = self.app.active_tab()?.current_location();
        let current_is_startup_folder = self
            .app
            .startup_folder()
            .is_some_and(|startup_folder| startup_folder == active_location);
        ui::append_checked_menu_item(
            startup_menu.handle(),
            ID_TAB_SET_STARTUP_FOLDER,
            "Use Current Folder on Startup",
            current_is_startup_folder,
        )?;
        ui::append_menu_item(
            startup_menu.handle(),
            ID_TAB_CLEAR_STARTUP_FOLDER,
            "Clear Startup Folder",
        )?;
        ui::append_checked_menu_item(
            startup_menu.handle(),
            ID_TAB_RESTORE_ON_STARTUP,
            "Restore Previous Tabs on Startup",
            self.app.state().restore_tabs_on_startup,
        )?;
        ui::append_owned_menu_popup(tab_menu, startup_menu, "Startup")
    }

    fn append_drive_menu_items(
        &self,
        menu: ui::MenuHandle,
    ) -> ExplorerResult<Vec<NavigationLocation>> {
        let mut drive_menu_locations = Vec::new();
        if !self.deferred_shell_startup_complete {
            return Ok(drive_menu_locations);
        }

        for location in self.app.drive_roots()? {
            if drive_menu_locations.len() >= MAX_DRIVE_MENU_ITEMS {
                break;
            }

            let offset = u16::try_from(drive_menu_locations.len()).map_err(|_| {
                ExplorerError::state_conflict("드라이브 메뉴 항목이 너무 많습니다.")
            })?;
            let id = ID_DRIVE_BASE.checked_add(offset).ok_or_else(|| {
                ExplorerError::state_conflict("드라이브 메뉴 항목이 너무 많습니다.")
            })?;
            let label = display_os(location.as_path().as_os_str());
            ui::append_menu_item(menu, id, &label)?;
            drive_menu_locations.push(location);
        }

        Ok(drive_menu_locations)
    }

    fn tab_height(&self) -> i32 {
        self.row_height(TAB_HEIGHT)
    }

    fn toolbar_height(&self) -> i32 {
        self.row_height(TOOLBAR_HEIGHT)
    }

    fn search_row_height(&self) -> i32 {
        self.row_height(SEARCH_ROW_HEIGHT)
    }

    fn square_button_width(&self) -> i32 {
        self.ui_scale.px(BUTTON_WIDTH).max(self.toolbar_height())
    }

    fn text_button_width(&self, base: i32, ems: i32) -> i32 {
        let point_size = i32::from(self.app.appearance_font().point_size());
        self.ui_scale
            .px(base)
            .max(self.ui_scale.px(point_size * ems + 20))
    }

    fn row_height(&self, base: i32) -> i32 {
        let point_size = i32::from(self.app.appearance_font().point_size());
        self.ui_scale
            .px(base)
            .max(self.ui_scale.px(point_size + 18))
    }

    fn layout(&self) -> ExplorerResult<()> {
        if self.address_edit.is_null() {
            return Ok(());
        }

        let client = ui::client_rect(self.hwnd)?;
        let tab_height = self.tab_height();
        let toolbar_height = self.toolbar_height();
        let show_search_controls = self.should_show_search_controls()?;
        let search_row_height = if show_search_controls {
            self.search_row_height()
        } else {
            0
        };
        let file_operation_status_height = self.row_height(FILE_OPERATION_STATUS_HEIGHT);
        let button_width = self.square_button_width();
        let go_button_width = self.ui_scale.px(GO_BUTTON_WIDTH).max(button_width);
        let search_cancel_button_width = self
            .ui_scale
            .px(SEARCH_CANCEL_BUTTON_WIDTH)
            .max(button_width);
        let search_find_button_width = self.text_button_width(SEARCH_FIND_BUTTON_WIDTH, 4);
        let search_subfolder_checkbox_width =
            self.text_button_width(SEARCH_SUBFOLDER_CHECKBOX_WIDTH, 3);
        let search_label_width = self.text_button_width(SEARCH_LABEL_WIDTH, 4);
        let margin = self.ui_scale.px(MARGIN);
        let spacing = self.ui_scale.px(SPACING);
        let pane = self.horizontal_pane_layout(client.width, self.folder_tree_width_px);
        let tree_width = pane.tree_width;
        let right_x = pane.right_x;
        let right_width = pane.right_width;
        let tab_y = margin;
        let y = tab_y + tab_height + spacing;
        let back_x = right_x;
        let forward_x = back_x + button_width + spacing;
        let up_x = forward_x + button_width + spacing;
        let refresh_x = up_x + button_width + spacing;
        let new_tab_x = refresh_x + button_width + spacing;
        let address_x = new_tab_x + button_width + spacing;
        let go_x_limit = right_x + right_width - go_button_width;
        let address_width = (go_x_limit - spacing - address_x).max(0);
        let go_x = address_x + address_width + spacing;
        let search_y = y + toolbar_height + spacing;
        let list_top = if show_search_controls {
            search_y + search_row_height + margin
        } else {
            y + toolbar_height + margin
        };
        let list_height =
            (client.height - list_top - spacing - file_operation_status_height - margin).max(0);
        let file_operation_status_y = list_top + list_height + spacing;
        let search_cancel_x = (right_x + right_width - search_cancel_button_width).max(right_x);
        let search_subfolders_x =
            (search_cancel_x - spacing - search_subfolder_checkbox_width).max(right_x);
        let search_find_x = (search_subfolders_x - spacing - search_find_button_width).max(right_x);
        let search_query_x = right_x + search_label_width + spacing;
        let search_query_width = (search_find_x - spacing - search_query_x).max(0);

        ui::move_window(
            self.tree_view,
            margin,
            margin,
            tree_width,
            (client.height - margin * 2).max(0),
        )?;
        ui::move_window(self.tab_control, right_x, tab_y, right_width, tab_height)?;
        ui::move_window(self.back_button, back_x, y, button_width, toolbar_height)?;
        ui::move_window(
            self.forward_button,
            forward_x,
            y,
            button_width,
            toolbar_height,
        )?;
        ui::move_window(self.up_button, up_x, y, button_width, toolbar_height)?;
        ui::move_window(
            self.refresh_button,
            refresh_x,
            y,
            button_width,
            toolbar_height,
        )?;
        ui::move_window(
            self.new_tab_button,
            new_tab_x,
            y,
            button_width,
            toolbar_height,
        )?;
        ui::move_window(
            self.address_edit,
            address_x,
            y,
            address_width,
            toolbar_height,
        )?;
        ui::move_window(self.go_button, go_x, y, go_button_width, toolbar_height)?;
        self.set_search_controls_visible(show_search_controls);
        ui::move_window(
            self.search_query_label,
            right_x,
            search_y,
            search_label_width,
            search_row_height,
        )?;
        ui::move_window(
            self.search_query_edit,
            search_query_x,
            search_y,
            search_query_width,
            search_row_height,
        )?;
        ui::move_window(
            self.search_find_button,
            search_find_x,
            search_y,
            search_find_button_width,
            search_row_height,
        )?;
        ui::move_window(
            self.search_subfolders_checkbox,
            search_subfolders_x,
            search_y,
            search_subfolder_checkbox_width,
            search_row_height,
        )?;
        ui::move_window(
            self.search_cancel_button,
            search_cancel_x,
            search_y,
            search_cancel_button_width,
            search_row_height,
        )?;

        ui::move_window(self.list_view, right_x, list_top, right_width, list_height)?;
        ui::move_window(
            self.file_operation_status_label,
            right_x,
            file_operation_status_y,
            right_width,
            file_operation_status_height,
        )?;

        ui::set_list_view_column_width(
            self.list_view,
            LIST_NAME_COLUMN_INDEX,
            self.ui_scale.px(LIST_NAME_COLUMN_WIDTH),
        )?;
        ui::set_list_view_column_width(
            self.list_view,
            LIST_TYPE_COLUMN_INDEX,
            self.ui_scale.px(LIST_TYPE_COLUMN_WIDTH),
        )?;
        ui::set_list_view_column_width(
            self.list_view,
            LIST_SIZE_COLUMN_INDEX,
            self.ui_scale.px(LIST_SIZE_COLUMN_WIDTH),
        )?;
        ui::set_list_view_column_width(
            self.list_view,
            LIST_UPDATED_COLUMN_INDEX,
            self.ui_scale.px(LIST_UPDATED_COLUMN_WIDTH),
        )
    }

    fn show_or_start_search(&mut self) -> ExplorerResult<()> {
        if self.should_show_search_controls()? {
            self.start_search()
        } else {
            self.show_search_controls()
        }
    }

    fn show_search_controls(&mut self) -> ExplorerResult<()> {
        self.search_controls_requested = true;
        self.sync_search_controls()?;
        self.layout()?;
        ui::focus_window(self.search_query_edit);
        ui::select_all_edit_text(self.search_query_edit);
        Ok(())
    }

    fn toggle_search_subfolders(&mut self) -> ExplorerResult<()> {
        let include_subfolders = !ui::is_button_checked(self.search_subfolders_checkbox);
        self.search_controls_requested = true;
        self.set_search_controls_visible(true);
        ui::set_button_checked(self.search_subfolders_checkbox, include_subfolders);
        self.layout()?;
        self.create_menu()
    }

    fn sync_search_subfolders_menu(&mut self) -> ExplorerResult<()> {
        self.search_controls_requested = true;
        self.create_menu()
    }

    fn focus_address_bar(&self) -> ExplorerResult<()> {
        ui::focus_window(self.address_edit);
        ui::select_all_edit_text(self.address_edit);
        Ok(())
    }

    fn show_about_dialog(&self) -> ExplorerResult<()> {
        let about_text = distribution_text(ABOUT_TEXT_FILE_NAME, DEFAULT_ABOUT_TEXT);
        ui::show_about_dialog(
            self.hwnd,
            PROGRAM_NAME,
            PROGRAM_VERSION,
            PROJECT_URL,
            &about_text,
        )
    }

    fn should_show_search_controls(&self) -> ExplorerResult<bool> {
        Ok(self.search_controls_requested || self.active_search_criteria()?.is_some())
    }

    fn active_search_criteria(&self) -> ExplorerResult<Option<SearchCriteria>> {
        let criteria = match &self.app.active_tab()?.search {
            SearchState::Idle => None,
            SearchState::Running { criteria, .. }
            | SearchState::Results { criteria, .. }
            | SearchState::Cancelled { criteria, .. } => Some(criteria.clone()),
        };
        Ok(criteria)
    }

    fn sync_search_controls(&self) -> ExplorerResult<()> {
        if self.search_query_edit.is_null() {
            return Ok(());
        }

        let show_controls = self.should_show_search_controls()?;
        self.set_search_controls_visible(show_controls);
        if let Some(criteria) = self.active_search_criteria()? {
            ui::set_window_text(self.search_query_edit, OsStr::new(&criteria.query))?;
            ui::set_button_checked(
                self.search_subfolders_checkbox,
                criteria.scope == SearchScope::IncludeSubfolders,
            );
        }
        Ok(())
    }

    fn set_search_controls_visible(&self, visible: bool) {
        ui::set_window_visible(self.search_query_label, visible);
        ui::set_window_visible(self.search_query_edit, visible);
        ui::set_window_visible(self.search_find_button, visible);
        ui::set_window_visible(self.search_subfolders_checkbox, visible);
        ui::set_window_visible(self.search_cancel_button, visible);
    }

    fn navigate_to_address_if_changed(&mut self) -> ExplorerResult<()> {
        let raw_path = ui::window_text(self.address_edit)?;
        if raw_path.as_os_str().is_empty()
            || raw_path.as_os_str()
                == self
                    .app
                    .active_tab()?
                    .current_location()
                    .as_path()
                    .as_os_str()
        {
            return Ok(());
        }

        let tab_id = self.app.state().active_tab_id;
        self.app.navigate_active_path(PathBuf::from(raw_path))?;
        self.finish_successful_navigation(tab_id)
    }

    fn navigate_to_known_folder(&mut self, kind: KnownFolderKind) -> ExplorerResult<()> {
        let location = self.app.known_folder(kind)?;
        self.navigate_to_location(location)
    }

    fn add_current_location_bookmark(&mut self) -> ExplorerResult<()> {
        let outcome = self.app.add_active_location_bookmark(None)?;
        self.create_menu()?;
        self.rebuild_folder_tree()?;
        if matches!(outcome, BookmarkAddOutcome::Added(_)) {
            self.schedule_user_settings_save()?;
        }
        Ok(())
    }

    fn add_selected_folder_bookmark(&mut self) -> ExplorerResult<()> {
        let Some(item) = self.selected_item() else {
            return Ok(());
        };

        let outcome = self.app.add_selected_folder_bookmark(&item, None)?;
        self.create_menu()?;
        self.rebuild_folder_tree()?;
        if matches!(outcome, BookmarkAddOutcome::Added(_)) {
            self.schedule_user_settings_save()?;
        }
        Ok(())
    }

    fn remove_current_location_bookmark(&mut self) -> ExplorerResult<()> {
        let location = self.app.active_tab()?.current_location().clone();
        self.remove_bookmark_for_location(&location)
    }

    fn remove_bookmark_for_location(
        &mut self,
        location: &NavigationLocation,
    ) -> ExplorerResult<()> {
        let Some(index) = self.app.bookmark_index_for_location(location) else {
            return Ok(());
        };

        self.app.delete_bookmark(index)?;
        self.create_menu()?;
        self.rebuild_folder_tree()?;
        self.schedule_user_settings_save()
    }

    fn toggle_show_hidden_files(&mut self) -> ExplorerResult<()> {
        self.remember_current_selection()?;
        let show_hidden = !self.app.display_options().show_hidden;
        self.app.set_show_hidden(show_hidden);
        self.refresh_listing_options()?;
        self.schedule_user_settings_save()
    }

    fn toggle_show_system_files(&mut self) -> ExplorerResult<()> {
        self.remember_current_selection()?;
        let show_system = !self.app.display_options().show_system;
        self.app.set_show_system(show_system);
        self.refresh_listing_options()?;
        self.schedule_user_settings_save()
    }

    fn set_appearance_theme(&mut self, theme: AppearanceTheme) -> ExplorerResult<()> {
        if self.app.appearance_theme() == theme {
            self.create_menu()?;
            return Ok(());
        }

        let next_theme_resources = ui::ThemeResources::new(theme)?;
        self.app.set_appearance_theme(theme);
        self.theme_resources = next_theme_resources;
        self.apply_theme();
        self.create_menu()?;
        self.schedule_user_settings_save()
    }

    fn choose_appearance_font(&mut self) -> ExplorerResult<()> {
        let Some(font) = ui::choose_font(self.hwnd, self.app.appearance_font())? else {
            return Ok(());
        };
        self.set_appearance_font(font)
    }

    fn reset_appearance_font(&mut self) -> ExplorerResult<()> {
        self.set_appearance_font(AppearanceFont::default())
    }

    fn set_appearance_font(&mut self, font: AppearanceFont) -> ExplorerResult<()> {
        if self.app.appearance_font() == &font {
            self.create_menu()?;
            return Ok(());
        }

        let next_font_resource = ui::FontResource::new(&font, self.dpi_metrics)?;
        self.app.set_appearance_font(font);
        let old_font_resource = std::mem::replace(&mut self.font_resource, next_font_resource);
        self.apply_font();
        self.layout()?;
        self.create_menu()?;
        let result = self.schedule_user_settings_save();
        drop(old_font_resource);
        result
    }

    fn apply_theme(&self) {
        let theme = self.app.appearance_theme();
        ui::apply_window_theme(self.hwnd, theme);
        ui::apply_control_theme(
            theme,
            &[
                self.tree_view,
                self.tab_control,
                self.back_button,
                self.forward_button,
                self.up_button,
                self.refresh_button,
                self.new_tab_button,
                self.address_edit,
                self.go_button,
                self.search_query_label,
                self.search_query_edit,
                self.search_find_button,
                self.search_subfolders_checkbox,
                self.search_cancel_button,
                self.file_operation_status_label,
                self.list_view,
            ],
        );
        ui::apply_tree_view_theme(self.tree_view, theme);
        ui::apply_list_view_theme(self.list_view, theme);
    }

    fn apply_font(&self) {
        ui::apply_font(
            &self.font_resource,
            &[
                self.tree_view,
                self.tab_control,
                self.back_button,
                self.forward_button,
                self.up_button,
                self.refresh_button,
                self.new_tab_button,
                self.address_edit,
                self.go_button,
                self.search_query_label,
                self.search_query_edit,
                self.search_find_button,
                self.search_subfolders_checkbox,
                self.search_cancel_button,
                self.file_operation_status_label,
                self.list_view,
            ],
        );
    }

    fn toggle_restore_tabs_on_startup(&mut self) -> ExplorerResult<()> {
        let restore_tabs_on_startup = !self.app.state().restore_tabs_on_startup;
        if restore_tabs_on_startup {
            self.app.set_startup_folder(None);
        }
        self.app
            .set_restore_tabs_on_startup(restore_tabs_on_startup);
        self.create_menu()?;
        self.schedule_user_settings_save()
    }

    fn set_current_folder_as_startup_folder(&mut self) -> ExplorerResult<()> {
        let startup_folder = self.app.active_tab()?.current_location().clone();
        self.app.set_startup_folder(Some(startup_folder));
        self.app.set_restore_tabs_on_startup(false);
        self.create_menu()?;
        self.schedule_user_settings_save()
    }

    fn clear_startup_folder(&mut self) -> ExplorerResult<()> {
        self.app.set_startup_folder(None);
        self.create_menu()?;
        self.schedule_user_settings_save()
    }

    fn set_active_sort_key(&mut self, key: SortKey) -> ExplorerResult<()> {
        self.remember_current_selection()?;
        self.app.set_active_sort_key(key)?;
        self.refresh_active_listing_sort()
    }

    fn set_active_sort_direction(&mut self, direction: SortDirection) -> ExplorerResult<()> {
        self.remember_current_selection()?;
        self.app.set_active_sort_direction(direction)?;
        self.refresh_active_listing_sort()
    }

    fn sort_by_list_column(&mut self, column_index: usize) -> ExplorerResult<()> {
        let Some(key) = list_column_sort_key(column_index) else {
            return Ok(());
        };

        let current_sort = self.app.active_tab()?.sort;
        let next_sort = list_column_click_sort_state(current_sort, key);
        self.remember_current_selection()?;
        self.app.set_active_sort_key(next_sort.key)?;
        self.app.set_active_sort_direction(next_sort.direction)?;
        self.refresh_active_listing_sort()
    }

    fn refresh_active_listing_sort(&mut self) -> ExplorerResult<()> {
        self.create_menu()?;
        if self.resort_current_listing_rows()? {
            return Ok(());
        }
        self.refresh_view()
    }

    fn refresh_listing_options(&mut self) -> ExplorerResult<()> {
        self.create_menu()?;
        self.rebuild_folder_tree()?;
        self.refresh_view()
    }

    fn rebuild_folder_tree(&mut self) -> ExplorerResult<()> {
        if self.tree_view.is_null() {
            return Ok(());
        }

        ui::with_window_redraw_suspended(self.tree_view, || {
            self.suppress_folder_tree_selection_while(|window| {
                window.cancel_all_folder_tree_child_workers();
                window.reap_finished_folder_tree_child_workers();
                ui::clear_tree_view(window.tree_view)?;
                window.folder_tree_nodes.clear();
                window.folder_tree_child_indices_by_parent.clear();

                for item in window.app.folder_tree_roots()? {
                    window.insert_folder_tree_root(item)?;
                }

                window.sync_folder_tree_selection()
            })
        })?;
        self.refresh_drop_feedback();
        Ok(())
    }

    fn refresh_folder_tree_after_locations_changed(
        &mut self,
        locations: &[NavigationLocation],
    ) -> ExplorerResult<()> {
        if self.tree_view.is_null() {
            return Ok(());
        }

        let locations = unique_navigation_location_paths_by_path(locations);
        ui::with_window_redraw_suspended(self.tree_view, || {
            self.suppress_folder_tree_selection_while(|window| {
                for location in &locations {
                    window.refresh_folder_tree_nodes_for_location(location)?;
                }
                window.sync_folder_tree_selection()
            })
        })?;
        self.refresh_drop_feedback();
        Ok(())
    }

    fn refresh_folder_tree_nodes_for_location(
        &mut self,
        location_path: &PreparedNavigationPath,
    ) -> ExplorerResult<()> {
        let node_indices =
            live_folder_tree_node_indices_at_location(&self.folder_tree_nodes, location_path);
        for node_index in node_indices {
            if self
                .folder_tree_nodes
                .get(node_index)
                .and_then(|node| node.handle)
                .is_some()
            {
                self.refresh_folder_tree_node_children(node_index)?;
            }
        }
        Ok(())
    }

    fn refresh_folder_tree_node_children(&mut self, parent_index: usize) -> ExplorerResult<()> {
        let Some(parent_node) = self.folder_tree_nodes.get(parent_index) else {
            return Ok(());
        };
        if parent_node.handle.is_none() {
            return Ok(());
        }

        let children_loaded = parent_node.children_loaded;
        let request_kind = if children_loaded {
            FolderTreeChildrenRequestKind::RefreshLoadedChildren
        } else {
            FolderTreeChildrenRequestKind::RefreshChildPresence
        };
        let pending_cancelled_generation =
            self.cancel_folder_tree_child_workers_for_parent(parent_index);
        self.start_folder_tree_child_worker_for_node(
            parent_index,
            request_kind,
            false,
            pending_cancelled_generation,
        )?;
        Ok(())
    }

    fn invalidate_folder_tree_subtree(&mut self, node_index: usize) {
        self.cancel_folder_tree_child_workers_for_parent(node_index);
        let child_indices = take_folder_tree_child_indices(
            &mut self.folder_tree_child_indices_by_parent,
            node_index,
        );
        for child_index in child_indices {
            self.invalidate_folder_tree_subtree(child_index);
        }

        if let Some(node) = self.folder_tree_nodes.get_mut(node_index) {
            node.handle = None;
            node.children_loaded = false;
            node.children_loading_generation = None;
        }
    }

    fn insert_folder_tree_root(
        &mut self,
        item: FolderTreeItem,
    ) -> ExplorerResult<ui::TreeViewItemHandle> {
        let reusable_index = self.next_reusable_folder_tree_node_index();
        self.insert_folder_tree_item(None, None, item, reusable_index)
    }

    fn insert_folder_tree_child_with_reusable_index(
        &mut self,
        parent: ui::TreeViewItemHandle,
        parent_value: ui::TreeViewItemValue,
        item: FolderTreeItem,
        reusable_index: Option<usize>,
    ) -> ExplorerResult<ui::TreeViewItemHandle> {
        self.insert_folder_tree_item(Some(parent), Some(parent_value.get()), item, reusable_index)
    }

    fn next_reusable_folder_tree_node_index(&self) -> Option<usize> {
        self.folder_tree_nodes
            .iter()
            .position(|node| node.handle.is_none())
    }

    fn insert_folder_tree_item(
        &mut self,
        parent: Option<ui::TreeViewItemHandle>,
        parent_index: Option<usize>,
        item: FolderTreeItem,
        reusable_index: Option<usize>,
    ) -> ExplorerResult<ui::TreeViewItemHandle> {
        let text = display_os(item.display_name());
        let kind = item.kind();
        let has_children = item.has_children();
        let location = item.navigation_target();
        let prepared_location_path = location.prepared_path();
        let next_node = FolderTreeNodeState {
            handle: None,
            parent: parent_index,
            kind,
            location,
            prepared_location_path,
            children_loaded: false,
            children_loading_generation: None,
        };
        let replaced_node = if let Some(index) = reusable_index {
            debug_assert!(self
                .folder_tree_nodes
                .get(index)
                .is_some_and(|node| node.handle.is_none()));
            Some((
                index,
                std::mem::replace(&mut self.folder_tree_nodes[index], next_node),
            ))
        } else {
            self.folder_tree_nodes.push(next_node);
            self.folder_tree_child_indices_by_parent.push(Vec::new());
            None
        };
        let value_index = reusable_index.unwrap_or_else(|| self.folder_tree_nodes.len() - 1);

        let tree_item = ui::TreeViewItem {
            text: &text,
            value: Some(ui::TreeViewItemValue::new(value_index)),
            has_children,
        };
        let inserted = match parent {
            Some(parent) => ui::insert_tree_view_child_item(self.tree_view, parent, tree_item),
            None => ui::insert_tree_view_root_item(self.tree_view, tree_item),
        };

        match inserted {
            Ok(handle) => {
                if let Some(node) = self.folder_tree_nodes.get_mut(value_index) {
                    node.handle = Some(handle);
                }
                if let Some(child_indices) = self
                    .folder_tree_child_indices_by_parent
                    .get_mut(value_index)
                {
                    child_indices.clear();
                }
                if let Some(parent_index) = parent_index {
                    if let Some(child_indices) = self
                        .folder_tree_child_indices_by_parent
                        .get_mut(parent_index)
                    {
                        child_indices.push(value_index);
                    }
                }
                Ok(handle)
            }
            Err(error) => {
                if let Some((index, node)) = replaced_node {
                    self.folder_tree_nodes[index] = node;
                } else {
                    self.folder_tree_nodes.pop();
                    self.folder_tree_child_indices_by_parent.pop();
                }
                Err(error)
            }
        }
    }

    fn on_folder_tree_selection_changed(&mut self) -> ExplorerResult<()> {
        if self.folder_tree_selection_suppressed {
            return Ok(());
        }

        let Some(value) = ui::selected_tree_view_item_value(self.tree_view)? else {
            return Ok(());
        };
        let Some(location) = self.folder_tree_node_location(value) else {
            return Ok(());
        };

        let (tab_id, same_location, search_idle) = {
            let active_tab = self.app.active_tab()?;
            (
                active_tab.id,
                active_tab.current_location().as_path() == location.as_path(),
                matches!(&active_tab.search, SearchState::Idle),
            )
        };
        if same_location && search_idle {
            return Ok(());
        }
        if same_location {
            self.app.clear_active_search()?;
            return self.finish_successful_navigation(tab_id);
        }

        let location = location.clone();
        self.navigate_to_location(location)
    }

    fn on_folder_tree_item_expanding(&mut self, lparam: ui::MessageLong) -> ExplorerResult<()> {
        // SAFETY: the notification code identifies a TreeView item-expanding payload.
        let Some(notification) = (unsafe { ui::tree_view_expand_notification(lparam) }) else {
            return Ok(());
        };
        if notification.action != ui::TreeViewExpandAction::Expand {
            return Ok(());
        }
        let Some(value) = notification.value else {
            return Ok(());
        };

        self.load_folder_tree_children(notification.item, value)
    }

    fn load_folder_tree_children(
        &mut self,
        parent: ui::TreeViewItemHandle,
        value: ui::TreeViewItemValue,
    ) -> ExplorerResult<()> {
        self.load_folder_tree_children_for_node(value.get(), parent, false)
            .map(|_| ())
    }

    fn load_folder_tree_children_for_node(
        &mut self,
        parent_index: usize,
        _parent: ui::TreeViewItemHandle,
        selection_sync: bool,
    ) -> ExplorerResult<bool> {
        self.reap_finished_folder_tree_child_workers();

        let Some(node) = self.folder_tree_nodes.get(parent_index) else {
            return Ok(false);
        };
        if node.children_loaded {
            return Ok(true);
        }
        if node.children_loading_generation.is_some() {
            return Ok(false);
        }

        self.start_folder_tree_child_worker_for_node(
            parent_index,
            FolderTreeChildrenRequestKind::LoadChildren,
            selection_sync,
            None,
        )
    }

    fn start_folder_tree_child_worker_for_node(
        &mut self,
        parent_index: usize,
        request_kind: FolderTreeChildrenRequestKind,
        selection_sync: bool,
        loading_generation_on_spawn_error: Option<u64>,
    ) -> ExplorerResult<bool> {
        self.reap_finished_folder_tree_child_workers();

        let Some(node) = self.folder_tree_nodes.get(parent_index) else {
            return Ok(false);
        };
        if node.handle.is_none() {
            return Ok(false);
        }
        let location = node.location.clone();
        let request = FolderTreeChildrenRequest {
            generation: self.next_folder_tree_child_generation(),
            parent_index,
            location,
            display_options: self.app.state().display_options,
            kind: request_kind,
            selection_sync,
        };
        if let Some(node) = self.folder_tree_nodes.get_mut(parent_index) {
            node.children_loading_generation = Some(request.generation);
        }

        let pending_worker = PendingFolderTreeChildWorker {
            request,
            loading_generation_on_spawn_error,
        };
        if self.folder_tree_child_workers.len() >= MAX_CONCURRENT_FOLDER_TREE_CHILD_WORKERS
            || !self.pending_folder_tree_child_workers.is_empty()
        {
            enqueue_pending_folder_tree_child_worker(
                &mut self.pending_folder_tree_child_workers,
                pending_worker,
            );
            return Ok(false);
        }

        self.spawn_folder_tree_child_worker_request(pending_worker)?;
        Ok(false)
    }

    fn spawn_folder_tree_child_worker_request(
        &mut self,
        pending_worker: PendingFolderTreeChildWorker,
    ) -> ExplorerResult<()> {
        let request = pending_worker.request;
        let cancel_requested = Arc::new(AtomicBool::new(false));
        let completion_message_abandoned = Arc::new(AtomicBool::new(false));
        let io_cancellation = Arc::new(platform::SynchronousIoCancellation::new());
        let worker_messages = self.folder_tree_child_messages.clone();
        let handle = match self.worker_boundary().spawn_folder_tree_children_worker(
            request.clone(),
            Arc::clone(&cancel_requested),
            Arc::clone(&completion_message_abandoned),
            Arc::clone(&io_cancellation),
            worker_messages,
        ) {
            Ok(handle) => handle,
            Err(error) => {
                let loading_generation_on_spawn_error =
                    recoverable_folder_tree_child_loading_generation_on_spawn_error(
                        &self.folder_tree_child_workers,
                        request.parent_index,
                        pending_worker.loading_generation_on_spawn_error,
                    );
                if let Some(node) = self.folder_tree_nodes.get_mut(request.parent_index) {
                    if node.children_loading_generation == Some(request.generation) {
                        node.children_loading_generation = loading_generation_on_spawn_error;
                    }
                }
                return Err(error);
            }
        };

        self.folder_tree_child_workers
            .push(ActiveFolderTreeChildWorker {
                request,
                cancel_requested,
                completion_message_abandoned,
                io_cancellation,
                handle,
            });
        Ok(())
    }

    fn start_pending_folder_tree_child_workers(&mut self) {
        while self.folder_tree_child_workers.len() < MAX_CONCURRENT_FOLDER_TREE_CHILD_WORKERS {
            let Some(pending_worker) = self.pending_folder_tree_child_workers.pop_front() else {
                break;
            };
            if !self.pending_folder_tree_child_worker_is_current(&pending_worker.request) {
                self.clear_pending_folder_tree_child_loading(&pending_worker.request);
                continue;
            }

            if let Err(error) = self.spawn_folder_tree_child_worker_request(pending_worker) {
                eprintln!("failed to start queued folder tree child worker: {error}");
            }
        }
    }

    fn pending_folder_tree_child_worker_is_current(
        &self,
        request: &FolderTreeChildrenRequest,
    ) -> bool {
        let Some(node) = self.folder_tree_nodes.get(request.parent_index) else {
            return false;
        };
        if node.handle.is_none()
            || node.children_loading_generation != Some(request.generation)
            || !node.location.has_same_path(request.location.as_path())
            || request.display_options != self.app.state().display_options
        {
            return false;
        }

        match request.kind {
            FolderTreeChildrenRequestKind::LoadChildren
            | FolderTreeChildrenRequestKind::RefreshChildPresence => !node.children_loaded,
            FolderTreeChildrenRequestKind::RefreshLoadedChildren => node.children_loaded,
        }
    }

    fn clear_pending_folder_tree_child_loading(&mut self, request: &FolderTreeChildrenRequest) {
        let Some(node) = self.folder_tree_nodes.get_mut(request.parent_index) else {
            return;
        };
        if node.children_loading_generation == Some(request.generation) {
            node.children_loading_generation = None;
        }
    }

    fn try_load_folder_tree_children_for_sync(
        &mut self,
        parent_index: usize,
        parent: ui::TreeViewItemHandle,
    ) -> ExplorerResult<bool> {
        self.load_folder_tree_children_for_node(parent_index, parent, true)
    }

    fn insert_loaded_folder_tree_children(
        &mut self,
        parent_index: usize,
        parent: ui::TreeViewItemHandle,
        children: Vec<FolderTreeItem>,
    ) -> ExplorerResult<bool> {
        let has_children = !children.is_empty();
        let parent_value = ui::TreeViewItemValue::new(parent_index);
        let mut reusable_indices = reusable_folder_tree_node_indices(&self.folder_tree_nodes);
        for child in children {
            self.insert_folder_tree_child_with_reusable_index(
                parent,
                parent_value,
                child,
                reusable_indices.pop(),
            )?;
        }

        if let Some(node) = self.folder_tree_nodes.get_mut(parent_index) {
            node.children_loaded = true;
            node.children_loading_generation = None;
        }
        ui::set_tree_view_item_has_children(self.tree_view, parent, has_children)?;

        self.refresh_drop_feedback();
        Ok(true)
    }

    fn replace_loaded_folder_tree_children(
        &mut self,
        parent_index: usize,
        parent: ui::TreeViewItemHandle,
        children: Vec<FolderTreeItem>,
    ) -> ExplorerResult<bool> {
        let child_indices = take_folder_tree_child_indices(
            &mut self.folder_tree_child_indices_by_parent,
            parent_index,
        );
        for child_index in child_indices {
            if let Some(child) = self
                .folder_tree_nodes
                .get(child_index)
                .and_then(|node| node.handle)
            {
                ui::delete_tree_view_item(self.tree_view, child)?;
            }
            self.invalidate_folder_tree_subtree(child_index);
        }

        self.insert_loaded_folder_tree_children(parent_index, parent, children)
    }

    fn next_folder_tree_child_generation(&mut self) -> u64 {
        let generation = self.next_folder_tree_child_generation;
        self.next_folder_tree_child_generation =
            self.next_folder_tree_child_generation.saturating_add(1);
        generation
    }

    fn cancel_folder_tree_child_workers_for_parent(&mut self, parent_index: usize) -> Option<u64> {
        request_cancel_for_folder_tree_child_workers(
            &self.folder_tree_child_workers,
            &mut self.pending_folder_tree_child_workers,
            parent_index,
        )
    }

    fn cancel_all_folder_tree_child_workers(&mut self) {
        for worker in &self.folder_tree_child_workers {
            worker.request_cancel();
        }
        self.pending_folder_tree_child_workers.clear();
        for node in &mut self.folder_tree_nodes {
            node.children_loading_generation = None;
        }
    }

    fn reap_finished_folder_tree_child_workers(&mut self) {
        let mut index = 0;
        while index < self.folder_tree_child_workers.len() {
            if self.folder_tree_child_workers[index].is_finished() {
                let worker = self.folder_tree_child_workers.swap_remove(index);
                clear_finished_recoverable_folder_tree_child_loading(
                    &mut self.folder_tree_nodes,
                    &self.folder_tree_child_workers,
                    &worker,
                );
                Self::join_folder_tree_child_worker(worker);
            } else {
                index += 1;
            }
        }
        self.start_pending_folder_tree_child_workers();
    }

    fn finish_folder_tree_child_worker_for_generation(&mut self, generation: u64) {
        let Some(index) = self
            .folder_tree_child_workers
            .iter()
            .position(|worker| worker.request.generation == generation)
        else {
            return;
        };

        if self.folder_tree_child_workers[index].is_finished() {
            let worker = self.folder_tree_child_workers.swap_remove(index);
            Self::join_folder_tree_child_worker(worker);
            self.start_pending_folder_tree_child_workers();
        }
    }

    fn join_folder_tree_child_worker(worker: ActiveFolderTreeChildWorker) {
        if worker.handle.join().is_err() {
            eprintln!("folder tree child worker panicked");
        }
    }

    fn on_folder_tree_children_complete(&mut self, lparam: ui::MessageLong) -> ExplorerResult<()> {
        let Some(message) = self.folder_tree_child_messages.take_complete(lparam) else {
            return Ok(());
        };

        self.handle_folder_tree_children_complete(message)
    }

    fn on_icon_load_complete(&mut self, lparam: ui::MessageLong) -> ExplorerResult<()> {
        let Some(completion) = self.icon_load_messages.take_complete(lparam) else {
            return Ok(());
        };

        let Some(icon_cache) = self.icon_cache.as_mut() else {
            return Ok(());
        };

        let changed = completion(icon_cache);
        reap_finished_icon_load_workers(&mut self.icon_load_workers);
        self.start_pending_icon_load_workers();
        if changed {
            self.refresh_current_item_icons()?;
        }
        Ok(())
    }

    fn handle_folder_tree_children_complete(
        &mut self,
        message: FolderTreeChildrenCompleteMessage,
    ) -> ExplorerResult<()> {
        self.finish_folder_tree_child_worker_for_generation(message.request.generation);

        let Some(parent) = self.take_current_folder_tree_child_loading(&message.request) else {
            return Ok(());
        };

        match message.result {
            Ok(FolderTreeChildrenWorkerResult::Children(children)) => {
                let should_sync_selection =
                    self.folder_tree_children_request_matches_active_location(&message.request);
                match message.request.kind {
                    FolderTreeChildrenRequestKind::LoadChildren => {
                        self.insert_loaded_folder_tree_children(
                            message.request.parent_index,
                            parent,
                            children,
                        )?;
                    }
                    FolderTreeChildrenRequestKind::RefreshLoadedChildren => {
                        self.replace_loaded_folder_tree_children(
                            message.request.parent_index,
                            parent,
                            children,
                        )?;
                    }
                    FolderTreeChildrenRequestKind::RefreshChildPresence => {
                        eprintln!("folder tree child presence request returned child items");
                        return Ok(());
                    }
                }
                if should_sync_selection {
                    self.sync_folder_tree_selection()?;
                }
                Ok(())
            }
            Ok(FolderTreeChildrenWorkerResult::ChildPresence(has_children)) => {
                if message.request.kind != FolderTreeChildrenRequestKind::RefreshChildPresence {
                    eprintln!("folder tree child load request returned child presence");
                    return Ok(());
                }

                ui::set_tree_view_item_has_children(self.tree_view, parent, has_children)?;
                self.refresh_drop_feedback();
                if has_children
                    && self.folder_tree_children_request_matches_active_location(&message.request)
                {
                    self.sync_folder_tree_selection()?;
                }
                Ok(())
            }
            Err(error) => {
                if message.request.selection_sync
                    || matches!(
                        message.request.kind,
                        FolderTreeChildrenRequestKind::RefreshChildPresence
                            | FolderTreeChildrenRequestKind::RefreshLoadedChildren
                    )
                {
                    eprintln!("detail: {error}");
                    Ok(())
                } else {
                    Err(error)
                }
            }
        }
    }

    fn folder_tree_children_request_matches_active_location(
        &self,
        request: &FolderTreeChildrenRequest,
    ) -> bool {
        let Ok(active_tab) = self.app.active_tab() else {
            return false;
        };
        let active_path = active_tab.current_location().as_path();
        request.location.has_same_path(active_path) || request.location.contains_path(active_path)
    }

    fn take_current_folder_tree_child_loading(
        &mut self,
        request: &FolderTreeChildrenRequest,
    ) -> Option<ui::TreeViewItemHandle> {
        let current_display_options = self.app.state().display_options;
        let node = self.folder_tree_nodes.get_mut(request.parent_index)?;
        if node.children_loading_generation != Some(request.generation)
            || !node.location.has_same_path(request.location.as_path())
        {
            return None;
        }

        node.children_loading_generation = None;
        if request.display_options != current_display_options {
            return None;
        }

        match request.kind {
            FolderTreeChildrenRequestKind::LoadChildren
            | FolderTreeChildrenRequestKind::RefreshChildPresence => {
                if node.children_loaded {
                    return None;
                }
            }
            FolderTreeChildrenRequestKind::RefreshLoadedChildren => {
                if !node.children_loaded {
                    return None;
                }
            }
        }

        node.handle
    }

    fn sync_folder_tree_selection(&mut self) -> ExplorerResult<()> {
        if self.tree_view.is_null() {
            return Ok(());
        }

        let current_location = self.app.active_tab()?.current_location().clone();

        self.suppress_folder_tree_selection_while(|window| {
            let selected_item = window.expand_folder_tree_to_location(&current_location)?;
            ui::set_tree_view_selected_item(window.tree_view, selected_item)
        })
    }

    fn expand_folder_tree_to_location(
        &mut self,
        target_location: &NavigationLocation,
    ) -> ExplorerResult<Option<ui::TreeViewItemHandle>> {
        let target_path = target_location.prepared_path();
        let Some(mut node_index) = self.best_folder_tree_root_index(&target_path) else {
            return Ok(None);
        };

        loop {
            let Some(node) = self.folder_tree_nodes.get(node_index) else {
                return Ok(None);
            };
            let Some(handle) = node.handle else {
                return Ok(None);
            };
            let is_exact_match = node.prepared_location_path.has_same_path(&target_path);
            let contains_target = node.prepared_location_path.contains_path(&target_path);
            if is_exact_match {
                self.expand_folder_tree_node_for_sync(node_index, handle)?;
                return Ok(Some(handle));
            }
            if !contains_target {
                return Ok(None);
            }

            if !self.try_load_folder_tree_children_for_sync(node_index, handle)? {
                return Ok(None);
            }
            let Some(child_index) = self.best_folder_tree_child_index(node_index, &target_path)
            else {
                return Ok(None);
            };

            ui::expand_tree_view_item(self.tree_view, handle)?;
            node_index = child_index;
        }
    }

    fn expand_folder_tree_node_for_sync(
        &mut self,
        node_index: usize,
        handle: ui::TreeViewItemHandle,
    ) -> ExplorerResult<()> {
        if self.try_load_folder_tree_children_for_sync(node_index, handle)?
            && self.folder_tree_node_has_loaded_children(node_index)
        {
            ui::expand_tree_view_item(self.tree_view, handle)?;
        }
        Ok(())
    }

    fn best_folder_tree_root_index(&self, target_path: &PreparedNavigationPath) -> Option<usize> {
        PreparedNavigationPath::best_containing_path_index(
            target_path,
            self.folder_tree_nodes
                .iter()
                .enumerate()
                .filter_map(|(index, node)| {
                    (node.handle.is_some() && node.parent.is_none())
                        .then_some((index, &node.prepared_location_path))
                }),
        )
    }

    fn best_folder_tree_child_index(
        &self,
        parent_index: usize,
        target_path: &PreparedNavigationPath,
    ) -> Option<usize> {
        let child_indices = self.folder_tree_child_indices_by_parent.get(parent_index)?;
        PreparedNavigationPath::best_containing_path_index(
            target_path,
            child_indices.iter().filter_map(|&index| {
                self.folder_tree_nodes.get(index).and_then(|node| {
                    (node.handle.is_some() && node.parent == Some(parent_index))
                        .then_some((index, &node.prepared_location_path))
                })
            }),
        )
    }

    fn folder_tree_node_has_loaded_children(&self, parent_index: usize) -> bool {
        let Some(child_indices) = self.folder_tree_child_indices_by_parent.get(parent_index) else {
            return false;
        };
        child_indices.iter().any(|&child_index| {
            self.folder_tree_nodes
                .get(child_index)
                .map(|node| node.handle.is_some() && node.parent == Some(parent_index))
                .unwrap_or(false)
        })
    }

    fn folder_tree_node_location(
        &self,
        value: ui::TreeViewItemValue,
    ) -> Option<&NavigationLocation> {
        self.folder_tree_nodes
            .get(value.get())
            .map(|node| &node.location)
    }

    fn suppress_folder_tree_selection_while<T>(
        &mut self,
        action: impl FnOnce(&mut Self) -> ExplorerResult<T>,
    ) -> ExplorerResult<T> {
        let previous = self.folder_tree_selection_suppressed;
        self.folder_tree_selection_suppressed = true;
        let result = action(self);
        self.folder_tree_selection_suppressed = previous;
        result
    }

    fn start_search(&mut self) -> ExplorerResult<()> {
        self.search_controls_requested = true;
        let active_tab_id = self.app.state().active_tab_id;
        self.cancel_search_for_tab(active_tab_id)?;
        let criteria = self.search_criteria_from_controls()?;
        if criteria.query.is_empty() {
            return self.refresh_view();
        }
        let request = self.app.start_search_in_active(criteria)?;
        let cancel_requested = Arc::new(AtomicBool::new(false));
        self.start_or_queue_search_worker(request, cancel_requested)?;
        self.refresh_view()
    }

    fn cancel_active_search(&mut self) -> ExplorerResult<()> {
        let active_tab_id = self.app.state().active_tab_id;
        let search_is_running =
            matches!(&self.app.active_tab()?.search, SearchState::Running { .. });
        if search_is_running {
            self.cancel_search_for_tab(active_tab_id)?;
        } else {
            if !matches!(&self.app.active_tab()?.search, SearchState::Idle) {
                self.app.clear_active_search()?;
            }
            self.search_controls_requested = false;
        }
        self.refresh_view()
    }

    fn close_search_controls(&mut self) -> ExplorerResult<()> {
        let active_tab_id = self.app.state().active_tab_id;
        self.cancel_search_for_tab(active_tab_id)?;
        if !matches!(&self.app.active_tab()?.search, SearchState::Idle) {
            self.app.clear_active_search()?;
        }
        self.search_controls_requested = false;
        self.refresh_view()
    }

    fn cancel_search_for_tab(&mut self, tab_id: TabId) -> ExplorerResult<()> {
        self.workers.reap_finished_search_workers();
        self.workers.cancel_running_search_workers_for_tab(tab_id);
        let cancelled_pending = self.workers.cancel_pending_search_workers_for_tab(tab_id);
        self.app.request_search_cancel(tab_id)?;
        self.finish_cancelled_pending_search_workers(cancelled_pending)?;
        Ok(())
    }

    fn finish_cancelled_pending_search_workers(
        &mut self,
        pending_workers: Vec<PendingSearchWorker>,
    ) -> ExplorerResult<()> {
        for pending_worker in pending_workers {
            if !self.is_current_running_search(pending_worker.tab_id(), pending_worker.run_id()) {
                continue;
            }

            let outcome = SearchOutcome::from_request(
                pending_worker.request,
                SearchFileSystemOutcome {
                    cancelled: true,
                    ..SearchFileSystemOutcome::default()
                },
            );
            let _ = self.app.finish_search(outcome)?;
        }
        Ok(())
    }

    fn start_or_queue_search_worker(
        &mut self,
        request: SearchRequest,
        cancel_requested: Arc<AtomicBool>,
    ) -> ExplorerResult<()> {
        self.workers.reap_finished_search_workers();
        self.workers.detach_cancelled_search_workers();
        if self
            .workers
            .has_running_search_worker_for_tab(request.tab_id)
            || self.workers.search_workers.len() >= MAX_CONCURRENT_SEARCH_WORKERS
            || !self.workers.pending_search_workers.is_empty()
        {
            self.workers
                .replace_pending_search_worker(PendingSearchWorker {
                    request,
                    cancel_requested,
                });
            self.start_pending_search_workers()?;
            return Ok(());
        }

        self.start_search_worker(request, cancel_requested)
    }

    fn start_search_worker(
        &mut self,
        request: SearchRequest,
        cancel_requested: Arc<AtomicBool>,
    ) -> ExplorerResult<()> {
        self.ensure_search_completion_timer();
        let tab_id = request.tab_id;
        let run_id = request.run_id;
        let io_cancellation = Arc::new(platform::SynchronousIoCancellation::new());
        let handle = match self.worker_boundary().spawn_search_worker(
            request,
            Arc::clone(&cancel_requested),
            Arc::clone(&io_cancellation),
        ) {
            Ok(handle) => handle,
            Err(error) => {
                if let Err(fail_error) = self.app.fail_search(tab_id, run_id) {
                    eprintln!(
                        "failed to mark search as failed after worker start failure: {fail_error}"
                    );
                }
                if let Err(timer_error) = self.stop_search_completion_timer_if_idle() {
                    eprintln!(
                        "failed to stop search completion timer after worker start failure: {timer_error}"
                    );
                    self.workers.search_completion_timer_active = false;
                }
                return Err(error);
            }
        };
        self.workers
            .push_search_worker(tab_id, run_id, cancel_requested, io_cancellation, handle);
        Ok(())
    }

    fn start_pending_search_workers(&mut self) -> ExplorerResult<()> {
        self.workers.reap_finished_search_workers();
        self.workers.detach_cancelled_search_workers();
        while self.workers.search_workers.len() < MAX_CONCURRENT_SEARCH_WORKERS {
            let Some(pending_worker) = self.take_startable_pending_search_worker() else {
                break;
            };

            if pending_worker.cancel_requested.load(Ordering::Relaxed)
                || !self.is_current_running_search(pending_worker.tab_id(), pending_worker.run_id())
            {
                continue;
            }

            self.start_search_worker(pending_worker.request, pending_worker.cancel_requested)?;
        }

        Ok(())
    }

    fn take_startable_pending_search_worker(&mut self) -> Option<PendingSearchWorker> {
        let tab_id = self
            .workers
            .pending_search_workers
            .iter()
            .find(|pending_worker| {
                !self
                    .workers
                    .search_workers
                    .iter()
                    .any(|worker| worker.tab_id == pending_worker.tab_id())
            })?
            .tab_id();
        self.workers.take_pending_search_worker_for_tab(tab_id)
    }

    fn ensure_search_completion_timer(&mut self) {
        if self.workers.search_completion_timer_active || self.hwnd.is_null() {
            return;
        }

        match ui::set_window_timer(
            self.hwnd,
            ID_SEARCH_COMPLETION_TIMER,
            SEARCH_COMPLETION_POLL_MS,
        ) {
            Ok(()) => {
                self.workers.search_completion_timer_active = true;
            }
            Err(error) => {
                eprintln!("failed to start search completion timer: {error}");
            }
        }
    }

    fn stop_search_completion_timer(&mut self) -> ExplorerResult<()> {
        if !self.workers.search_completion_timer_active {
            return Ok(());
        }

        ui::kill_window_timer(self.hwnd, ID_SEARCH_COMPLETION_TIMER)?;
        self.workers.search_completion_timer_active = false;
        Ok(())
    }

    fn stop_search_completion_timer_if_idle(&mut self) -> ExplorerResult<()> {
        if !self.workers.search_completion_timer_active
            || self.workers.has_active_search_work()
            || self.workers.messages.has_pending_complete()
            || (self.shutdown_after_file_operation && self.workers.has_file_operation_worker())
        {
            return Ok(());
        }

        self.stop_search_completion_timer()
    }

    fn is_current_running_search(&self, tab_id: TabId, run_id: SearchRunId) -> bool {
        self.app.state().tabs.iter().any(|tab| {
            tab.id == tab_id
                && matches!(&tab.search, SearchState::Running { run_id: active_run_id, .. } if *active_run_id == run_id)
        })
    }

    fn search_criteria_from_controls(&self) -> ExplorerResult<SearchCriteria> {
        let query = display_os(ui::window_text(self.search_query_edit)?.as_os_str());
        let scope = if ui::is_button_checked(self.search_subfolders_checkbox) {
            SearchScope::IncludeSubfolders
        } else {
            SearchScope::CurrentFolder
        };

        Ok(SearchCriteria { query, scope })
    }

    fn open_new_tab_from_active(&mut self) -> ExplorerResult<()> {
        let location = self.app.active_tab()?.current_location().clone();
        self.app.open_tab(location)?;
        self.search_controls_requested = false;
        self.create_menu()?;
        self.refresh_view()
    }

    fn open_selected_folder_in_new_tab(&mut self) -> ExplorerResult<()> {
        let Some(item) = self.selected_item() else {
            return Ok(());
        };

        self.app.open_folder_in_new_tab(&item)?;
        self.search_controls_requested = false;
        self.create_menu()?;
        self.refresh_view()
    }

    fn activate_selected_item(&mut self) -> ExplorerResult<()> {
        let Some(item) = self.selected_item() else {
            return Ok(());
        };

        self.activate_item(item)
    }

    fn open_selected_item_with_picker(&mut self) -> ExplorerResult<()> {
        let Some(item) = self.selected_item() else {
            return Ok(());
        };

        self.app.open_item_with_picker(&item)
    }

    fn show_selected_item_properties(&self) -> ExplorerResult<()> {
        let Some(item) = self.selected_item() else {
            return Ok(());
        };

        self.app.show_properties(&item.location)
    }

    fn create_new_folder(&mut self) -> ExplorerResult<()> {
        self.app
            .create_folder_in_active(OsStr::new(DEFAULT_NEW_FOLDER_NAME), true)?;
        self.undo_file_operation = None;
        self.refresh_after_file_operation()
    }

    fn copy_selected_items_to_clipboard(&mut self) -> ExplorerResult<()> {
        self.set_selected_items_clipboard(ClipboardFileOperation::Copy)
    }

    fn cut_selected_items_to_clipboard(&mut self) -> ExplorerResult<()> {
        self.set_selected_items_clipboard(ClipboardFileOperation::Move)
    }

    fn set_selected_items_clipboard(
        &mut self,
        operation: ClipboardFileOperation,
    ) -> ExplorerResult<()> {
        let targets = self.selected_locations();
        if targets.is_empty() {
            return Ok(());
        }

        let paths = targets
            .iter()
            .map(|location| location.as_path().to_path_buf())
            .collect::<Vec<_>>();
        platform::set_clipboard_file_items(self.hwnd.as_isize(), &paths, operation)?;
        self.app.active_tab_mut()?.selected_items = targets;
        Ok(())
    }

    fn paste_clipboard_items(&mut self) -> ExplorerResult<()> {
        let Some(clipboard) = platform::clipboard_file_items(self.hwnd.as_isize())? else {
            return Ok(());
        };
        let sources = clipboard
            .paths
            .into_iter()
            .map(NavigationLocation::from_path)
            .collect::<ExplorerResult<Vec<_>>>()?;
        if sources.is_empty() {
            return Ok(());
        }

        let active_tab = self.app.active_tab()?;
        let tab_id = active_tab.id;
        let location = active_tab.current_location().clone();
        self.start_file_operation_worker(FileOperationRequest::Transfer {
            tab_id,
            location: location.clone(),
            operation: drop_operation_from_clipboard(clipboard.operation),
            sources,
            destination: location,
            select_completed_items: true,
        })
    }

    fn begin_list_view_drag(&mut self, lparam: ui::MessageLong) -> ExplorerResult<()> {
        if !matches!(&self.app.active_tab()?.search, SearchState::Idle) {
            return Ok(());
        }
        if self.workers.has_file_operation_worker() || self.shutdown_after_file_operation {
            self.set_file_operation_status(FILE_OPERATION_IN_PROGRESS_MESSAGE)?;
            return Ok(());
        }

        // SAFETY: callers pass the LVN_BEGINDRAG notification payload from the ListView.
        let Some(index) = (unsafe { ui::list_view_drag_index(lparam) }) else {
            return Ok(());
        };
        let current_items = self.current_item_slice()?;
        if index >= current_items.len() {
            return Ok(());
        }
        let selected_indices = ui::selected_list_view_indices(self.list_view);
        let sources = snapshot_drag_source_locations(current_items, &selected_indices, index);
        if sources.is_empty() {
            return Ok(());
        }

        if !selected_indices.contains(&index) {
            ui::set_list_view_selected_index(self.list_view, Some(index))?;
        }

        self.app.active_tab_mut()?.selected_items = sources.clone();
        self.start_file_drag(sources, InternalDragOrigin::FileList)
    }

    fn begin_folder_tree_drag(&mut self, lparam: ui::MessageLong) -> ExplorerResult<()> {
        if self.workers.has_file_operation_worker() || self.shutdown_after_file_operation {
            self.set_file_operation_status(FILE_OPERATION_IN_PROGRESS_MESSAGE)?;
            return Ok(());
        }

        // SAFETY: callers pass the TVN_BEGINDRAGW notification payload from the TreeView.
        let Some(notification) = (unsafe { ui::tree_view_drag_notification(lparam) }) else {
            return Ok(());
        };
        let Some(value) = notification.value else {
            return Ok(());
        };
        let Some(node) = self.folder_tree_nodes.get(value.get()) else {
            return Ok(());
        };
        let sources = unique_drag_sources(vec![node.location.clone()]);
        if sources.is_empty() {
            return Ok(());
        }

        self.suppress_folder_tree_selection_while(|window| {
            ui::set_tree_view_selected_item(window.tree_view, Some(notification.item))
        })?;

        self.start_file_drag(sources, InternalDragOrigin::FolderTree)
    }

    fn start_file_drag(
        &mut self,
        sources: Vec<NavigationLocation>,
        origin: InternalDragOrigin,
    ) -> ExplorerResult<()> {
        if self.workers.has_file_operation_worker() || self.shutdown_after_file_operation {
            self.set_file_operation_status(FILE_OPERATION_IN_PROGRESS_MESSAGE)?;
            return Ok(());
        }

        let paths = sources
            .iter()
            .map(|location| location.as_path().to_path_buf())
            .collect::<Vec<_>>();
        platform::validate_shell_file_drag_paths(&paths)?;

        let drag_id = self.allocate_internal_drag_id();
        self.active_internal_drag = Some(InternalDragState {
            drag_id,
            origin,
            sources,
        });
        self.refresh_drop_feedback();
        let drag_result = platform::start_shell_file_drag(drag_id, &paths);
        let pending_drop_result = self.process_pending_drop_events();
        let active_drag = self.active_internal_drag.take();
        self.refresh_drop_feedback();

        let pending_drop = pending_drop_result?;
        let drag_outcome = drag_result?;
        if let (false, Some(active_drag)) = (pending_drop.handled_internal_drag, active_drag) {
            self.finish_shell_drag_source(&active_drag.sources, drag_outcome)?;
        }

        Ok(())
    }

    fn allocate_internal_drag_id(&mut self) -> u64 {
        let drag_id = self.next_internal_drag_id;
        self.next_internal_drag_id = if drag_id == u64::MAX { 1 } else { drag_id + 1 };
        drag_id
    }

    fn on_ole_drop_event_message(&mut self) -> ExplorerResult<()> {
        self.process_pending_drop_events().map(|_| ())
    }

    fn process_pending_drop_events(&mut self) -> ExplorerResult<PendingDropProcessing> {
        let mut first_error = None;
        let mut outcome = PendingDropProcessing::default();
        for event in self.drop_event_queue.drain() {
            if event.data.is_internal() {
                outcome.handled_internal_drag = true;
            }
            if let Err(error) = self.handle_ole_drop_event(event) {
                if first_error.is_none() {
                    first_error = Some(error);
                }
            }
        }

        if let Some(error) = first_error {
            Err(error)
        } else {
            Ok(outcome)
        }
    }

    fn handle_ole_drop_event(&mut self, event: platform::OleDropEvent) -> ExplorerResult<()> {
        let Some(resolved_sources) = self.resolve_drop_sources(&event)? else {
            return Ok(());
        };
        let Some(destination) = self.resolve_drop_destination(&event)? else {
            return Ok(());
        };

        self.start_drop_file_operation(resolved_sources, destination, &event)
    }

    fn resolve_drop_sources(
        &self,
        event: &platform::OleDropEvent,
    ) -> ExplorerResult<Option<ResolvedDropSources>> {
        match &event.data {
            platform::OleDropData::ExternalPaths(paths) => {
                let sources = unique_drag_sources(
                    paths
                        .iter()
                        .cloned()
                        .map(NavigationLocation::from_path)
                        .collect::<ExplorerResult<Vec<_>>>()?,
                );
                if sources.is_empty() {
                    return Ok(None);
                }
                Ok(Some(ResolvedDropSources {
                    sources,
                    source_kind: DropSourceKind::External {
                        default_operation: external_drop_default_operation(
                            event.allowed_effects,
                            event.preferred_effect,
                        ),
                    },
                }))
            }
            platform::OleDropData::InternalDrag { drag_id } => {
                let Some(active_drag) = self.active_internal_drag.as_ref() else {
                    return Ok(None);
                };
                if active_drag.drag_id != *drag_id || active_drag.sources.is_empty() {
                    return Ok(None);
                }
                Ok(Some(ResolvedDropSources {
                    sources: active_drag.sources.clone(),
                    source_kind: DropSourceKind::Internal,
                }))
            }
        }
    }

    fn resolve_drop_destination(
        &self,
        event: &platform::OleDropEvent,
    ) -> ExplorerResult<Option<NavigationLocation>> {
        match event.target {
            platform::OleDropTargetKind::FileList => self.file_list_drop_destination(event),
            platform::OleDropTargetKind::FolderTree => self.folder_tree_drop_destination(event),
        }
    }

    fn file_list_drop_destination(
        &self,
        event: &platform::OleDropEvent,
    ) -> ExplorerResult<Option<NavigationLocation>> {
        let external_drop = !event.data.is_internal();
        if !matches!(&self.app.active_tab()?.search, SearchState::Idle) {
            if external_drop {
                return Ok(Some(self.app.active_tab()?.current_location().clone()));
            }
            return Ok(None);
        }

        let hit_index = ui::list_view_item_at_screen_point(self.list_view, event.point)?;
        match (event.data.is_internal(), hit_index) {
            (true, Some(index)) => {
                let Some(item) = self.current_item_slice()?.get(index) else {
                    return Ok(None);
                };
                Ok(item.is_folder().then(|| item.location.clone()))
            }
            (true, None) => {
                let Some(active_drag) = self.active_internal_drag.as_ref() else {
                    return Ok(None);
                };
                self.internal_file_list_empty_drop_destination(active_drag)
            }
            (false, None) => Ok(Some(self.app.active_tab()?.current_location().clone())),
            _ => Ok(None),
        }
    }

    fn folder_tree_drop_destination(
        &self,
        event: &platform::OleDropEvent,
    ) -> ExplorerResult<Option<NavigationLocation>> {
        let Some(item) = ui::tree_view_item_at_screen_point(self.tree_view, event.point)? else {
            return Ok(None);
        };
        let Some(value) = ui::tree_view_item_value(self.tree_view, item)? else {
            return Ok(None);
        };
        Ok(self
            .folder_tree_nodes
            .get(value.get())
            .map(|node| node.location.clone()))
    }

    fn start_drop_file_operation(
        &mut self,
        resolved_sources: ResolvedDropSources,
        destination: NavigationLocation,
        event: &platform::OleDropEvent,
    ) -> ExplorerResult<()> {
        if self.workers.has_file_operation_worker() || self.shutdown_after_file_operation {
            self.refresh_drop_feedback();
            self.set_file_operation_status(FILE_OPERATION_IN_PROGRESS_MESSAGE)?;
            return Ok(());
        }

        let modifiers = DropModifierKeys {
            control: event.key_state.control,
            shift: event.key_state.shift,
        };
        let plan = self.app.prepare_file_drop(
            &resolved_sources.sources,
            &destination,
            resolved_sources.source_kind,
            modifiers,
        )?;
        let active_tab = self.app.active_tab()?;
        let tab_id = active_tab.id;
        let location = active_tab.current_location().clone();
        let select_completed_items = destination.has_same_path(location.as_path());

        self.start_file_operation_worker(FileOperationRequest::Transfer {
            tab_id,
            location,
            operation: plan.operation,
            sources: resolved_sources.sources,
            destination,
            select_completed_items,
        })
    }

    fn finish_shell_drag_source(
        &mut self,
        sources: &[NavigationLocation],
        outcome: platform::OleDragSourceOutcome,
    ) -> ExplorerResult<()> {
        let completion = drag_source_completion_from_ole(outcome);
        let affected_folders = drag_source_refresh_locations(sources, completion)?;
        if affected_folders.is_empty() {
            return Ok(());
        }

        self.invalidate_location_icon_cache_entries();
        self.refresh_folder_tree_after_locations_changed(&affected_folders)?;
        if self.active_location_in(&affected_folders)? {
            self.app.active_tab_mut()?.selected_items.clear();
            self.workers.retire_active_listing_worker();
            self.workers.reap_retired_listing_workers();
            self.refresh_view_preserving_current_rows()?;
        }
        Ok(())
    }

    fn delete_selected_items_to_recycle_bin(&mut self) -> ExplorerResult<()> {
        let targets = self.selected_locations();
        if targets.is_empty() {
            return Ok(());
        }

        let active_tab = self.app.active_tab()?;
        let tab_id = active_tab.id;
        let location = active_tab.current_location().clone();
        self.start_file_operation_worker(FileOperationRequest::Delete {
            tab_id,
            location,
            operation: DeleteFileOperation::ToRecycleBin,
            targets,
        })
    }

    fn delete_selected_items_permanently(&mut self) -> ExplorerResult<()> {
        let targets = self.selected_locations();
        if targets.is_empty() {
            return Ok(());
        }

        let active_tab = self.app.active_tab()?;
        let tab_id = active_tab.id;
        let location = active_tab.current_location().clone();
        self.start_file_operation_worker(FileOperationRequest::Delete {
            tab_id,
            location,
            operation: DeleteFileOperation::Permanently,
            targets,
        })
    }

    fn start_file_operation_worker(&mut self, request: FileOperationRequest) -> ExplorerResult<()> {
        self.recover_pending_complete_messages_after_timer_failure();
        self.workers.ensure_file_operation_worker_idle()?;

        let generation = self.workers.next_file_operation_generation();
        let tab_id = request.tab_id();
        let location = request.location().clone();
        let handle = self.spawn_file_operation_worker(generation, request, tab_id, location)?;
        self.undo_file_operation = None;
        self.workers
            .start_file_operation_worker(generation, tab_id, handle);
        self.refresh_drop_feedback();
        self.set_file_operation_status(FILE_OPERATION_IN_PROGRESS_MESSAGE)
    }

    fn spawn_file_operation_worker(
        &self,
        generation: u64,
        request: FileOperationRequest,
        tab_id: TabId,
        location: NavigationLocation,
    ) -> ExplorerResult<JoinHandle<()>> {
        let hwnd_value = self.hwnd.as_isize();
        let owner_window = self.hwnd.as_isize();
        let worker_messages = self.workers.messages.clone();
        spawn_background_worker(
            "j3files-file-operation-worker",
            "start file operation worker thread",
            move || {
                let shell_gateway = WindowsShellGateway::new();
                shell_gateway.set_owner_window(owner_window);
                let result = run_file_operation(&shell_gateway, request);
                worker_messages.post_file_operation_complete(
                    hwnd_value,
                    FileOperationCompleteMessage {
                        generation,
                        tab_id,
                        location,
                        result,
                    },
                );
            },
        )
    }

    fn begin_rename_selected_item(&mut self) -> ExplorerResult<()> {
        let Some(index) = ui::selected_list_view_indices(self.list_view)
            .into_iter()
            .next()
        else {
            return Ok(());
        };
        if index >= self.current_item_count()? {
            return Ok(());
        }

        ui::edit_list_view_label(self.list_view, index)
    }

    fn select_all_items(&mut self) -> ExplorerResult<()> {
        let item_count = self.current_item_count()?;
        ui::set_list_view_all_items_selected(self.list_view, item_count)?;

        let selected_items = self
            .current_item_slice()?
            .iter()
            .map(|item| item.location.clone())
            .collect();
        self.app.active_tab_mut()?.selected_items = selected_items;
        Ok(())
    }

    fn undo_last_file_operation(&mut self) -> ExplorerResult<()> {
        let Some(operation) = self.undo_file_operation.clone() else {
            return Ok(());
        };

        if self.workers.has_file_operation_worker() || self.shutdown_after_file_operation {
            self.set_file_operation_status(FILE_OPERATION_IN_PROGRESS_MESSAGE)?;
            return Ok(());
        }

        let active_tab = self.app.active_tab()?;
        let tab_id = active_tab.id;
        let location = active_tab.current_location().clone();

        match operation {
            UndoFileOperation::Rename {
                current,
                original_name,
            } => self.start_file_operation_worker(FileOperationRequest::Rename {
                tab_id,
                location,
                target: current,
                new_name: original_name,
                undo_original_name: None,
            }),
            UndoFileOperation::Copy { copied } => {
                self.start_file_operation_worker(FileOperationRequest::Delete {
                    tab_id,
                    location,
                    operation: DeleteFileOperation::ToRecycleBin,
                    targets: copied,
                })
            }
            UndoFileOperation::Move { moved } => {
                self.start_file_operation_worker(FileOperationRequest::UndoMove {
                    tab_id,
                    location,
                    moved,
                })
            }
        }
    }

    fn on_list_view_label_edit(&mut self, lparam: ui::MessageLong) -> ExplorerResult<()> {
        // SAFETY: the notification code identifies a ListView end-label-edit payload.
        let Some(edit) = (unsafe { ui::list_view_label_edit(lparam) }) else {
            return Ok(());
        };
        let Some(new_name) = edit.text else {
            return Ok(());
        };
        let Some(item) = self.current_item_slice()?.get(edit.index).cloned() else {
            return Ok(());
        };
        if new_name == item.display_name {
            return Ok(());
        }

        if self.workers.has_file_operation_worker() || self.shutdown_after_file_operation {
            self.set_file_operation_status(FILE_OPERATION_IN_PROGRESS_MESSAGE)?;
            return Ok(());
        }

        let active_tab = self.app.active_tab()?;
        let tab_id = active_tab.id;
        let location = active_tab.current_location().clone();
        self.start_file_operation_worker(FileOperationRequest::Rename {
            tab_id,
            location,
            target: item.location,
            new_name,
            undo_original_name: Some(item.display_name),
        })
    }

    fn on_list_view_get_display_info(&mut self, lparam: ui::MessageLong) -> ExplorerResult<()> {
        // SAFETY: the notification code identifies a virtual ListView display-info payload.
        let Some(request) = (unsafe { ui::list_view_display_request(lparam) }) else {
            return Ok(());
        };

        if let Some(status) = &self.list_view_status_row {
            if request.row_index == 0 && request.needs_text {
                let text = status_list_view_cell_text(status, request.column_index);
                // SAFETY: lparam is the current LVN_GETDISPINFOW payload.
                unsafe {
                    ui::set_list_view_display_text(lparam, &text);
                }
            }
            return Ok(());
        }

        let active_tab = self.app.active_tab()?;
        let Some(item) = self
            .current_items
            .as_slice(active_tab)
            .get(request.row_index)
        else {
            return Ok(());
        };

        if request.needs_text {
            let text = cached_file_item_cell_text(
                &mut self.current_item_cell_text_caches,
                request.row_index,
                item,
                request.column_index,
            );
            // SAFETY: lparam is the current LVN_GETDISPINFOW payload.
            unsafe {
                ui::set_list_view_display_text(lparam, text);
            }
        }

        if request.needs_image && request.column_index == LIST_NAME_COLUMN_INDEX {
            if let Some(icon_cache) = self.icon_cache.as_mut() {
                let icon = icon_cache.cached_or_default_icon_for_item(item);
                let hwnd_value = self.hwnd.as_isize();
                let messages = self.icon_load_messages.clone();
                let shutdown_requested = Arc::clone(&self.icon_load_shutdown_requested);
                let icon_load_workers = &mut self.icon_load_workers;
                let pending_icon_load_tasks = &mut self.pending_icon_load_tasks;
                if let Err(error) = icon_cache.request_icon_load_for_item(item, |task| {
                    start_or_queue_icon_load_task(
                        icon_load_workers,
                        pending_icon_load_tasks,
                        hwnd_value,
                        messages,
                        shutdown_requested,
                        task,
                    )
                }) {
                    eprintln!("failed to start shell icon load: {error}");
                }
                // SAFETY: lparam is the current LVN_GETDISPINFOW payload.
                unsafe {
                    ui::set_list_view_display_image(lparam, icon.system_image_index());
                }
            }
        }

        Ok(())
    }

    fn close_active_tab(&mut self) -> ExplorerResult<()> {
        if self.app.state().tabs.len() == 1 {
            return Ok(());
        }

        self.cancel_search_for_tab(self.app.state().active_tab_id)?;
        self.app.clear_active_search()?;
        self.app.close_active_tab()?;
        self.search_controls_requested = false;
        self.create_menu()?;
        self.refresh_view()
    }

    fn switch_to_next_tab(&mut self) -> ExplorerResult<()> {
        let tab_count = self.app.state().tabs.len();
        if tab_count <= 1 {
            return Ok(());
        }

        let index = self.app.active_tab_index()?;
        self.app.switch_to_tab_index((index + 1) % tab_count)?;
        self.search_controls_requested = false;
        self.create_menu()?;
        self.refresh_view()
    }

    fn reopen_closed_tab(&mut self) -> ExplorerResult<()> {
        self.app.reopen_last_closed_tab()?;
        self.search_controls_requested = false;
        self.create_menu()?;
        self.refresh_view()
    }

    fn move_active_tab_left(&mut self) -> ExplorerResult<()> {
        let index = self.app.active_tab_index()?;
        if index == 0 {
            return Ok(());
        }

        let tab_id = self.app.state().active_tab_id;
        self.app.move_tab(tab_id, index - 1)?;
        self.sync_tabs()
    }

    fn move_active_tab_right(&mut self) -> ExplorerResult<()> {
        let index = self.app.active_tab_index()?;
        let next_index = index + 1;
        if next_index >= self.app.state().tabs.len() {
            return Ok(());
        }

        let tab_id = self.app.state().active_tab_id;
        self.app.move_tab(tab_id, next_index)?;
        self.sync_tabs()
    }

    fn navigate_to_drive_menu_item(&mut self, id: u16) -> ExplorerResult<()> {
        let index = usize::from(id - ID_DRIVE_BASE);
        let Some(location) = self.drive_menu_locations.get(index).cloned() else {
            return Ok(());
        };

        self.navigate_to_location(location)
    }

    fn navigate_to_bookmark_menu_item(&mut self, id: u16) -> ExplorerResult<()> {
        let index = usize::from(id - ID_BOOKMARK_BASE);
        if index >= self.app.state().bookmarks.items().len() {
            return Ok(());
        }

        let tab_id = self.app.state().active_tab_id;
        self.app.navigate_active_to_bookmark(index)?;
        self.finish_successful_navigation(tab_id)
    }

    fn navigate_to_location(&mut self, location: NavigationLocation) -> ExplorerResult<()> {
        let tab_id = self.app.state().active_tab_id;
        self.app.navigate_active(location)?;
        self.finish_successful_navigation(tab_id)
    }

    fn go_back(&mut self) -> ExplorerResult<()> {
        if self.app.active_tab()?.back_history().is_empty() {
            return Ok(());
        }

        let tab_id = self.app.state().active_tab_id;
        self.app.go_back()?;
        self.finish_successful_navigation(tab_id)
    }

    fn go_forward(&mut self) -> ExplorerResult<()> {
        if self.app.active_tab()?.forward_history().is_empty() {
            return Ok(());
        }

        let tab_id = self.app.state().active_tab_id;
        self.app.go_forward()?;
        self.finish_successful_navigation(tab_id)
    }

    fn go_up(&mut self) -> ExplorerResult<()> {
        let tab_id = self.app.state().active_tab_id;
        let before = self
            .app
            .active_tab()?
            .current_location()
            .as_path()
            .to_path_buf();
        self.app.go_up()?;
        if self.app.active_tab()?.current_location().as_path() != before.as_path() {
            self.cancel_search_for_tab(tab_id)?;
            self.search_controls_requested = false;
        }
        self.refresh_view()
    }

    fn finish_successful_navigation(&mut self, tab_id: TabId) -> ExplorerResult<()> {
        self.cancel_search_for_tab(tab_id)?;
        self.search_controls_requested = false;
        self.refresh_view()
    }

    fn ensure_file_watch_for_active_location(&mut self) -> ExplorerResult<()> {
        let location = self.app.active_tab()?.current_location().clone();
        self.workers.reap_retired_file_watch_workers();
        if self.workers.active_file_watch_matches(&location) {
            return Ok(());
        }

        self.stop_file_watch_refresh_timer()?;
        self.pending_file_watch_refresh.clear();
        if !self.workers.retire_active_file_watch_worker() {
            return Ok(());
        }
        self.workers.reap_retired_file_watch_workers();

        let generation = self.workers.next_file_watch_generation();
        let cancellation = Arc::new(platform::DirectoryChangeCancellation::new()?);
        let handle = self.worker_boundary().spawn_file_watch_worker(
            generation,
            location.clone(),
            Arc::clone(&cancellation),
        )?;
        self.workers
            .start_file_watch_worker(generation, location, cancellation, handle);
        Ok(())
    }

    fn listing_request_for_active_tab(&mut self) -> ExplorerResult<ListingRequest> {
        let active_tab = self.app.active_tab()?;
        let tab_id = active_tab.id;
        let location = active_tab.current_location().clone();
        let sort = active_tab.sort;
        let display_options = self.app.state().display_options;
        if let Some(request) = self.workers.active_uncancelled_listing_request() {
            if request.has_same_listing_source(tab_id, &location, display_options) {
                return Ok(request.clone());
            }
        }

        let generation = self.workers.next_listing_generation();
        Ok(ListingRequest {
            generation,
            tab_id,
            location,
            display_options,
            sort,
        })
    }

    fn start_listing_worker_if_needed(&mut self, request: ListingRequest) -> ExplorerResult<()> {
        self.workers.reap_retired_listing_workers();
        if self.workers.active_listing_matches_source(&request)
            && self
                .workers
                .active_uncancelled_listing_request()
                .is_some_and(|active_request| {
                    listing_request_matches_source_and_sort(active_request, &request)
                })
        {
            return Ok(());
        }

        if !self.workers.retire_active_listing_worker() {
            self.workers.replace_pending_listing_request(request);
            return Ok(());
        }
        self.workers.reap_retired_listing_workers();

        let cancel_requested = Arc::new(AtomicBool::new(false));
        let io_cancellation = Arc::new(platform::SynchronousIoCancellation::new());
        let handle = self.worker_boundary().spawn_listing_worker(
            request.clone(),
            Arc::clone(&cancel_requested),
            Arc::clone(&io_cancellation),
        )?;
        self.workers.clear_pending_listing_request();
        self.workers
            .start_listing_worker(request, cancel_requested, io_cancellation, handle);
        Ok(())
    }

    fn start_pending_listing_worker_if_idle(&mut self) -> ExplorerResult<()> {
        if self.workers.has_active_listing_worker() {
            return Ok(());
        }

        let Some(request) = self.workers.take_pending_listing_request() else {
            return Ok(());
        };
        if !self.is_current_idle_listing_request(&request)? {
            return Ok(());
        }

        self.start_listing_worker_if_needed(request)
    }

    fn refresh_view(&mut self) -> ExplorerResult<()> {
        self.refresh_view_with_loading_presentation(ListingLoadingPresentation::StatusRow)
    }

    fn refresh_view_preserving_current_rows(&mut self) -> ExplorerResult<()> {
        self.refresh_view_with_loading_presentation(ListingLoadingPresentation::PreserveCurrentRows)
    }

    fn refresh_running_search_status(&mut self) -> ExplorerResult<()> {
        let active_tab = self.app.active_tab()?;
        let SearchState::Running {
            progress,
            cancel_requested,
            ..
        } = &active_tab.search
        else {
            return self.refresh_view();
        };

        let text = search_running_text(*progress, *cancel_requested);
        self.set_status_row(text)
    }

    fn refresh_view_with_loading_presentation(
        &mut self,
        loading_presentation: ListingLoadingPresentation,
    ) -> ExplorerResult<()> {
        self.sync_tabs()?;
        self.sync_address()?;
        self.sync_search_controls()?;
        self.layout()?;
        self.sync_folder_tree_selection()?;
        self.ensure_file_watch_for_active_location()?;
        if let Some(icon_cache) = self.icon_cache.as_mut() {
            icon_cache.invalidate_location(self.app.active_tab()?.current_location());
        }

        enum RefreshViewAction {
            StartListing,
            Status(String),
            CurrentSearchRows,
            SearchItems(CurrentSearchRows),
        }

        let active_tab = self.app.active_tab()?;
        let active_tab_id = active_tab.id;
        let display_options = self.app.state().display_options;
        let action = match &active_tab.search {
            SearchState::Idle => RefreshViewAction::StartListing,
            SearchState::Running {
                progress,
                cancel_requested,
                ..
            } => RefreshViewAction::Status(search_running_text(*progress, *cancel_requested)),
            SearchState::Results {
                items,
                diagnostics,
                progress,
                ..
            } => {
                if items.is_empty() {
                    RefreshViewAction::Status(search_finished_empty_text(
                        *progress,
                        diagnostics.len(),
                    ))
                } else {
                    let rows = CurrentSearchRows {
                        tab_id: active_tab_id,
                        kind: CurrentSearchRowsKind::Results,
                        item_count: items.len(),
                        display_options,
                    };
                    if current_search_rows_match(
                        self.current_search_rows.as_ref(),
                        self.current_items.as_slice(active_tab).len(),
                        &rows,
                    ) {
                        RefreshViewAction::CurrentSearchRows
                    } else {
                        RefreshViewAction::SearchItems(rows)
                    }
                }
            }
            SearchState::Cancelled {
                partial_items,
                diagnostics,
                progress,
                ..
            } => {
                if partial_items.is_empty() {
                    RefreshViewAction::Status(search_cancelled_empty_text(
                        *progress,
                        diagnostics.len(),
                    ))
                } else {
                    let rows = CurrentSearchRows {
                        tab_id: active_tab_id,
                        kind: CurrentSearchRowsKind::Cancelled,
                        item_count: partial_items.len(),
                        display_options,
                    };
                    if current_search_rows_match(
                        self.current_search_rows.as_ref(),
                        self.current_items.as_slice(active_tab).len(),
                        &rows,
                    ) {
                        RefreshViewAction::CurrentSearchRows
                    } else {
                        RefreshViewAction::SearchItems(rows)
                    }
                }
            }
        };

        match action {
            RefreshViewAction::StartListing => {
                let request = self.listing_request_for_active_tab()?;
                let preserve_current_rows = loading_presentation
                    == ListingLoadingPresentation::PreserveCurrentRows
                    && self.current_listing_rows_match_active_tab()?;
                self.start_listing_worker_if_needed(request.clone())?;
                if preserve_current_rows {
                    self.remember_listing_viewport_restore(&request);
                    Ok(())
                } else {
                    self.set_status_row("Loading folder...".to_string())
                }
            }
            RefreshViewAction::Status(text) => self.set_status_row(text),
            RefreshViewAction::CurrentSearchRows => {
                self.current_listing_rows = None;
                self.current_listing_child_indices = None;
                self.pending_listing_viewport_restore = None;
                self.sync_current_item_rows(None)?;
                self.refresh_drop_feedback();
                Ok(())
            }
            RefreshViewAction::SearchItems(rows) => {
                self.current_listing_rows = None;
                self.current_listing_child_indices = None;
                self.pending_listing_viewport_restore = None;
                self.set_search_item_rows(rows)
            }
        }
    }

    fn set_search_item_rows(&mut self, rows: CurrentSearchRows) -> ExplorerResult<()> {
        self.current_search_rows = Some(rows);
        self.current_items = CurrentItems::search(rows);
        reset_file_item_cell_text_caches(&mut self.current_item_cell_text_caches, rows.item_count);
        self.list_view_status_row = None;
        self.current_listing_child_indices = None;
        self.current_item_rows_synced_to_list_view = false;
        self.sync_current_item_rows(None)?;
        self.refresh_drop_feedback();
        Ok(())
    }

    fn set_listing_item_rows(
        &mut self,
        request: &ListingRequest,
        items: Vec<FileItem>,
        viewport_restore: Option<ui::ListViewViewport>,
    ) -> ExplorerResult<()> {
        self.current_listing_rows = Some(CurrentListingRows {
            tab_id: request.tab_id,
            location: request.location.clone(),
            display_options: request.display_options,
        });
        self.current_search_rows = None;
        self.list_view_status_row = None;
        self.current_listing_child_indices = None;
        let row_count = items.len();
        self.current_items = CurrentItems::listing(items);
        reset_file_item_cell_text_caches(&mut self.current_item_cell_text_caches, row_count);
        self.current_item_rows_synced_to_list_view = false;
        self.sync_current_item_rows(viewport_restore)?;
        self.refresh_drop_feedback();
        Ok(())
    }

    fn sync_current_item_rows(
        &mut self,
        viewport_restore: Option<ui::ListViewViewport>,
    ) -> ExplorerResult<()> {
        if !self.current_item_rows_synced_to_list_view {
            let active_tab = self.app.active_tab()?;
            let items = self.current_items.as_slice(active_tab);
            ui::set_list_view_virtual_row_count(self.list_view, items.len())?;
            self.current_item_rows_synced_to_list_view = true;
        }
        let active_tab = self.app.active_tab()?;
        let selected_index = selected_list_item_index(
            self.current_items.as_slice(active_tab),
            &active_tab.selected_items,
        );
        ui::set_list_view_selected_index(self.list_view, selected_index)?;
        if let Some(viewport) = viewport_restore {
            ui::restore_list_view_viewport(self.list_view, viewport);
        }
        Ok(())
    }

    fn resort_current_listing_rows(&mut self) -> ExplorerResult<bool> {
        if !self.current_listing_rows_match_active_tab()? {
            return Ok(false);
        }

        let active_sort = self.app.active_tab()?.sort;
        self.sort_current_items_preserving_rows(active_sort);
        self.sync_current_item_rows(None)?;
        Ok(true)
    }

    fn sort_current_items_preserving_rows(&mut self, active_sort: SortState) {
        let row_count = {
            let Some(current_items) = self.current_items.as_listing_mut() else {
                reset_file_item_cell_text_caches(&mut self.current_item_cell_text_caches, 0);
                self.current_item_rows_synced_to_list_view = false;
                self.current_listing_child_indices = None;
                return;
            };

            sort_file_items(current_items, active_sort);
            current_items.len()
        };
        reset_file_item_cell_text_caches(&mut self.current_item_cell_text_caches, row_count);
        self.current_item_rows_synced_to_list_view = false;
        self.current_listing_child_indices = None;
    }

    fn reset_current_item_cell_text_caches(&mut self) -> ExplorerResult<()> {
        let row_count = self.current_item_count()?;
        reset_file_item_cell_text_caches(&mut self.current_item_cell_text_caches, row_count);
        Ok(())
    }

    fn current_file_watch_existing_child_indices(
        &mut self,
        changed_names: &[OsString],
        active_sort: SortState,
    ) -> Option<Vec<Option<usize>>> {
        let CurrentItems::Listing(items) = &self.current_items else {
            return None;
        };
        let child_indices = self
            .current_listing_child_indices
            .get_or_insert_with(HashMap::new);
        file_watch_existing_child_indices_from_items_with_cache(
            items,
            child_indices,
            changed_names,
            active_sort,
        )
    }

    fn update_current_listing_child_indices_after_file_watch(
        &mut self,
        row_replacements: &[(usize, FileItem, FileItem)],
        row_removals: &[(usize, FileItem)],
    ) {
        let Some(child_indices) = self.current_listing_child_indices.as_mut() else {
            return;
        };
        if !update_file_watch_child_index_map_after_changes(
            child_indices,
            row_replacements,
            row_removals,
        ) {
            self.current_listing_child_indices = None;
        }
    }

    fn update_current_listing_child_indices_after_file_watch_insertions(
        &mut self,
        row_insertions: Vec<(usize, Vec<u16>)>,
    ) {
        let Some(child_indices) = self.current_listing_child_indices.as_mut() else {
            return;
        };
        if !update_file_watch_child_index_map_after_insertions(child_indices, row_insertions) {
            self.current_listing_child_indices = None;
        }
    }

    fn current_listing_rows_match_active_tab(&self) -> ExplorerResult<bool> {
        let Some(rows) = &self.current_listing_rows else {
            return Ok(false);
        };
        let active_tab = self.app.active_tab()?;
        Ok(rows.tab_id == active_tab.id
            && rows
                .location
                .has_same_path(active_tab.current_location().as_path())
            && rows.display_options == self.app.state().display_options
            && matches!(&active_tab.search, SearchState::Idle))
    }

    fn remember_listing_viewport_restore(&mut self, request: &ListingRequest) {
        self.pending_listing_viewport_restore = Some(PendingListingViewportRestore {
            generation: request.generation,
        });
    }

    fn take_listing_viewport_restore(&mut self, generation: u64) -> bool {
        if self
            .pending_listing_viewport_restore
            .as_ref()
            .is_none_or(|pending| pending.generation != generation)
        {
            return false;
        }

        self.pending_listing_viewport_restore.take();
        true
    }

    fn set_status_row(&mut self, text: String) -> ExplorerResult<()> {
        self.pending_listing_viewport_restore = None;
        self.list_view_status_row = Some(text);
        ui::set_list_view_virtual_row_count(self.list_view, 1)?;
        self.current_items.clear();
        reset_file_item_cell_text_caches(&mut self.current_item_cell_text_caches, 0);
        self.current_item_rows_synced_to_list_view = true;
        self.current_listing_child_indices = None;
        self.current_listing_rows = None;
        self.current_search_rows = None;
        self.refresh_drop_feedback();
        Ok(())
    }

    fn current_item_slice(&self) -> ExplorerResult<&[FileItem]> {
        let active_tab = self.app.active_tab()?;
        Ok(self.current_items.as_slice(active_tab))
    }

    fn current_item_count(&self) -> ExplorerResult<usize> {
        Ok(self.current_item_slice()?.len())
    }

    fn invalidate_location_icon_cache_entries(&mut self) {
        if let Some(icon_cache) = self.icon_cache.as_mut() {
            icon_cache.invalidate_location_entries();
        }
        self.current_item_rows_synced_to_list_view = false;
    }

    fn refresh_current_item_icons(&mut self) -> ExplorerResult<()> {
        if self.list_view_status_row.is_some() {
            return Ok(());
        }

        self.current_item_rows_synced_to_list_view = false;
        self.sync_current_item_rows(None)
    }

    fn start_pending_icon_load_workers(&mut self) {
        start_pending_icon_load_tasks(
            &mut self.icon_load_workers,
            &mut self.pending_icon_load_tasks,
            self.hwnd.as_isize(),
            self.icon_load_messages.clone(),
            Arc::clone(&self.icon_load_shutdown_requested),
        );
    }

    fn set_file_operation_status(&self, text: &str) -> ExplorerResult<()> {
        if self.file_operation_status_label.is_null() {
            return Ok(());
        }

        ui::set_window_text(self.file_operation_status_label, OsStr::new(text))
    }

    fn clear_file_operation_status(&self) -> ExplorerResult<()> {
        self.set_file_operation_status("")
    }

    fn refresh_active_view(&mut self) -> ExplorerResult<()> {
        self.invalidate_location_icon_cache_entries();
        self.rebuild_folder_tree()?;
        self.refresh_view()
    }

    fn refresh_after_file_operation(&mut self) -> ExplorerResult<()> {
        let location = self.app.active_tab()?.current_location().clone();
        self.refresh_after_file_operation_locations(std::slice::from_ref(&location))
    }

    fn refresh_after_file_operation_locations(
        &mut self,
        affected_folders: &[NavigationLocation],
    ) -> ExplorerResult<()> {
        self.invalidate_location_icon_cache_entries();
        self.refresh_folder_tree_after_locations_changed(affected_folders)?;
        self.workers.retire_active_listing_worker();
        self.workers.reap_retired_listing_workers();
        self.refresh_view_preserving_current_rows()
    }

    fn on_search_progress(&mut self, lparam: ui::MessageLong) -> ExplorerResult<()> {
        let Some(message) = self.workers.messages.take_search_progress(lparam) else {
            return Ok(());
        };

        let active_tab_id = self.app.state().active_tab_id;
        let accepted =
            self.app
                .update_search_progress(message.tab_id, message.run_id, message.progress)?;
        if accepted && message.tab_id == active_tab_id {
            self.refresh_running_search_status()?;
        }

        Ok(())
    }

    fn on_search_complete(&mut self, lparam: ui::MessageLong) -> ExplorerResult<()> {
        let Some(message) = self.workers.messages.take_search_complete(lparam) else {
            return Ok(());
        };

        self.handle_search_complete(message)
    }

    fn on_search_completion_timer(&mut self) -> ExplorerResult<()> {
        let mut first_error = None;

        while let Some(message) = self.workers.messages.take_next_search_complete() {
            if let Err(error) = self.handle_search_complete(message) {
                if first_error.is_none() {
                    first_error = Some(error);
                }
            }
        }

        while let Some(message) = self.workers.messages.take_next_listing_complete() {
            if let Err(error) = self.handle_listing_complete(message) {
                if first_error.is_none() {
                    first_error = Some(error);
                }
            }
        }

        while let Some(message) = self.workers.messages.take_next_file_operation_complete() {
            if let Err(error) = self.handle_file_operation_complete(message) {
                if first_error.is_none() {
                    first_error = Some(error);
                }
            }
        }

        if self.shutdown_after_file_operation {
            self.workers
                .reap_finished_file_operation_worker_for_shutdown();
            while let Some(message) = self.workers.messages.take_next_file_operation_complete() {
                if let Err(error) = self.handle_file_operation_complete(message) {
                    if first_error.is_none() {
                        first_error = Some(error);
                    }
                }
            }
        }

        self.finish_deferred_shutdown_if_file_operation_idle();

        self.stop_search_completion_timer_if_idle()?;

        if let Some(error) = first_error {
            Err(error)
        } else {
            Ok(())
        }
    }

    fn recover_pending_complete_messages_after_timer_failure(&mut self) {
        if !self.workers.messages.take_completion_recovery_request() {
            return;
        }

        if let Err(error) = self.on_search_completion_timer() {
            self.recover_after_error(&error);
        }
    }

    fn handle_search_complete(&mut self, message: SearchCompleteMessage) -> ExplorerResult<()> {
        let tab_id = message.tab_id;
        let run_id = message.run_id;
        self.workers.remove_search_worker(tab_id, run_id);
        self.start_pending_search_workers()?;
        if !self.is_current_running_search(tab_id, run_id) {
            self.stop_search_completion_timer_if_idle()?;
            return Ok(());
        }

        let active_tab_id = self.app.state().active_tab_id;
        match message.result {
            Ok(outcome) => {
                log_search_diagnostics(&outcome);
                let tab_id = outcome.tab_id;
                let accepted = self.app.finish_search(outcome)?;
                if accepted && tab_id == active_tab_id {
                    self.refresh_view()?;
                }
                self.stop_search_completion_timer_if_idle()?;
                Ok(())
            }
            Err(error) => {
                self.app.fail_search(tab_id, run_id)?;
                if tab_id == active_tab_id {
                    self.refresh_view()?;
                }
                self.stop_search_completion_timer_if_idle()?;
                Err(error)
            }
        }
    }

    fn on_listing_complete(&mut self, lparam: ui::MessageLong) -> ExplorerResult<()> {
        let Some(message) = self.workers.messages.take_listing_complete(lparam) else {
            return Ok(());
        };

        self.handle_listing_complete(message)
    }

    fn handle_listing_complete(&mut self, message: ListingCompleteMessage) -> ExplorerResult<()> {
        self.workers.reap_retired_listing_workers();
        self.workers
            .finish_listing_worker_for_generation(message.request.generation);
        let should_restore_viewport =
            self.take_listing_viewport_restore(message.request.generation);

        if !self.is_current_idle_listing_request(&message.request)? {
            self.start_pending_listing_worker_if_idle()?;
            return Ok(());
        }

        let active_sort = self.app.active_tab()?.sort;
        let viewport_restore = should_restore_viewport
            .then(|| ui::list_view_viewport(self.list_view))
            .flatten();
        match message.result {
            Ok(mut items) => {
                if message.request.sort != active_sort {
                    sort_file_items(&mut items, active_sort);
                }
                self.set_listing_item_rows(&message.request, items, viewport_restore)
            }
            Err(error) => Err(error),
        }
    }

    fn is_current_idle_listing_request(&self, request: &ListingRequest) -> ExplorerResult<bool> {
        let active_tab = self.app.active_tab()?;
        Ok(self
            .workers
            .is_current_listing_generation(request.generation)
            && request.tab_id == active_tab.id
            && request
                .location
                .has_same_path(active_tab.current_location().as_path())
            && request.display_options == self.app.state().display_options
            && matches!(&active_tab.search, SearchState::Idle))
    }

    fn on_file_watch_changed(&mut self, lparam: ui::MessageLong) -> ExplorerResult<()> {
        let mut should_schedule_refresh = false;
        if let Some(message) = self.workers.messages.take_file_watch_changed(lparam) {
            should_schedule_refresh |= self.merge_file_watch_changed(message);
        }
        should_schedule_refresh |= self.merge_pending_file_watch_changed_messages();

        if should_schedule_refresh {
            self.schedule_file_watch_refresh()?;
        }

        Ok(())
    }

    fn merge_pending_file_watch_changed_messages(&mut self) -> bool {
        let mut merged = false;
        while let Some(message) = self.workers.messages.take_next_file_watch_changed() {
            merged |= self.merge_file_watch_changed(message);
        }
        merged
    }

    fn merge_file_watch_changed(&mut self, message: FileWatchChangeMessage) -> bool {
        if !self
            .workers
            .is_current_file_watch_generation(message.generation)
        {
            return false;
        }

        self.pending_file_watch_refresh.merge(message.changes);
        true
    }

    fn schedule_file_watch_refresh(&mut self) -> ExplorerResult<()> {
        ui::set_window_timer(
            self.hwnd,
            ID_FILE_WATCH_REFRESH_TIMER,
            FILE_WATCH_REFRESH_DEBOUNCE_MS,
        )?;
        self.workers.file_watch_refresh_timer_active = true;
        Ok(())
    }

    fn stop_file_watch_refresh_timer(&mut self) -> ExplorerResult<()> {
        if !self.workers.file_watch_refresh_timer_active {
            return Ok(());
        }

        ui::kill_window_timer(self.hwnd, ID_FILE_WATCH_REFRESH_TIMER)?;
        self.workers.file_watch_refresh_timer_active = false;
        Ok(())
    }

    fn on_file_watch_refresh_timer(&mut self) -> ExplorerResult<()> {
        if !self.workers.file_watch_refresh_timer_active
            && self.workers.messages.has_pending_file_watch_changed()
        {
            self.workers.file_watch_refresh_timer_active = true;
        }
        self.stop_file_watch_refresh_timer()?;
        self.merge_pending_file_watch_changed_messages();
        if !self.workers.has_active_file_watch_worker() {
            return Ok(());
        }

        let pending_refresh = std::mem::take(&mut self.pending_file_watch_refresh);
        if pending_refresh.is_empty() {
            return Ok(());
        }

        let location = self.app.active_tab()?.current_location().clone();
        self.refresh_after_external_file_change(&location, pending_refresh)
    }

    fn refresh_after_external_file_change(
        &mut self,
        location: &NavigationLocation,
        pending_refresh: PendingFileWatchRefresh,
    ) -> ExplorerResult<()> {
        if !pending_refresh.requires_full_refresh {
            if let Some(folder_tree_changed) = self.apply_incremental_file_watch_changes(
                location,
                pending_refresh.changed_names.as_slice(),
            )? {
                if folder_tree_changed {
                    self.refresh_folder_tree_after_locations_changed(std::slice::from_ref(
                        location,
                    ))?;
                }
                return Ok(());
            }
        }

        self.refresh_after_external_file_change_full(location)
    }

    fn refresh_after_external_file_change_full(
        &mut self,
        location: &NavigationLocation,
    ) -> ExplorerResult<()> {
        self.invalidate_location_icon_cache_entries();
        self.refresh_folder_tree_after_locations_changed(std::slice::from_ref(location))?;
        self.workers.retire_active_listing_worker();
        self.workers.reap_retired_listing_workers();
        self.refresh_view_preserving_current_rows()
    }

    fn apply_incremental_file_watch_changes(
        &mut self,
        location: &NavigationLocation,
        changed_names: &[OsString],
    ) -> ExplorerResult<Option<bool>> {
        if changed_names.is_empty() {
            return Ok(Some(false));
        }
        if changed_names
            .iter()
            .any(|name| !is_direct_child_file_name(name.as_os_str()))
        {
            return Ok(None);
        }
        if self.workers.active_listing_request().is_some()
            || !self.current_listing_rows_match_active_tab()?
        {
            return Ok(None);
        }

        let active_sort = self.app.active_tab()?.sort;
        let display_options = self.app.state().display_options;
        let file_system = NativeFileSystemGateway::new();
        let mut changed = false;
        let mut folder_tree_changed = false;
        let mut row_replacements: Vec<(usize, FileItem, FileItem)> = Vec::new();
        let mut row_reordered_replacements: Vec<(usize, FileItem, FileItem)> = Vec::new();
        let mut row_removals: Vec<(usize, FileItem)> = Vec::new();
        let mut row_insertions: Vec<FileItem> = Vec::new();

        let Some(existing_indices) =
            self.current_file_watch_existing_child_indices(changed_names, active_sort)
        else {
            return Ok(None);
        };
        let visible_items = match file_system.items_for_existing_children(location, changed_names) {
            Ok(items) => items,
            Err(_) => return Ok(None),
        };
        if visible_items.len() != changed_names.len() {
            return Ok(None);
        }

        {
            let CurrentItems::Listing(items) = &self.current_items else {
                return Ok(None);
            };

            for (existing_index, visible_item) in existing_indices.into_iter().zip(visible_items) {
                let existing_item = match existing_index {
                    Some(index) => match items.get(index) {
                        Some(item) => Some(item),
                        None => return Ok(None),
                    },
                    None => None,
                };
                let visible_item = visible_item.filter(|item| display_options.allows(item));

                match (existing_index, existing_item, visible_item) {
                    (Some(index), Some(existing), Some(item)) if existing != &item => {
                        folder_tree_changed |= replacement_may_change_folder_tree(existing, &item);
                        if replacement_requires_resort(existing, &item, active_sort.key) {
                            row_reordered_replacements.push((index, existing.clone(), item));
                        } else {
                            row_replacements.push((index, existing.clone(), item));
                        }
                        changed = true;
                    }
                    (Some(_), Some(_), Some(_)) => {}
                    (Some(index), Some(existing), None) => {
                        folder_tree_changed |= existing.is_folder();
                        row_removals.push((index, existing.clone()));
                        changed = true;
                    }
                    (None, None, Some(item)) => {
                        folder_tree_changed |= item.is_folder();
                        row_insertions.push(item);
                        changed = true;
                    }
                    _ => {}
                }
            }
        }

        if changed {
            row_removals.sort_unstable_by_key(|(index, _)| *index);
            let had_row_removals = !row_removals.is_empty();
            let mut row_order_removals = row_removals.clone();
            row_order_removals.extend(
                row_reordered_replacements
                    .iter()
                    .map(|(index, existing, _)| (*index, existing.clone())),
            );
            row_order_removals.sort_unstable_by_key(|(index, _)| *index);

            if let Some(icon_cache) = self.icon_cache.as_mut() {
                for (_, existing, item) in row_replacements
                    .iter()
                    .chain(row_reordered_replacements.iter())
                {
                    icon_cache.invalidate_item_replacement_if_needed(existing, item);
                }
                for (_, item) in &row_removals {
                    icon_cache.invalidate_item_presence_change_if_needed(item);
                }
                for item in &row_insertions {
                    icon_cache.invalidate_item_presence_change_if_needed(item);
                }
            }

            self.update_current_listing_child_indices_after_file_watch(
                &row_replacements,
                &row_order_removals,
            );

            let row_insertion_indices = {
                let Some(items) = self.current_items.as_listing_mut() else {
                    return Ok(None);
                };

                for (index, _, item) in row_replacements {
                    items[index] = item;
                }

                remove_file_watch_rows(items, &row_order_removals);
                row_insertions.extend(
                    row_reordered_replacements
                        .into_iter()
                        .map(|(_, _, item)| item),
                );
                insert_file_watch_rows_sorted(items, row_insertions, active_sort)
            };

            if let Some(row_insertion_indices) = row_insertion_indices {
                self.update_current_listing_child_indices_after_file_watch_insertions(
                    row_insertion_indices,
                );
            } else {
                self.current_listing_child_indices = None;
            }

            let viewport_restore_after_row_count_change = had_row_removals
                .then(|| ui::list_view_viewport(self.list_view))
                .flatten();
            self.reset_current_item_cell_text_caches()?;
            self.current_item_rows_synced_to_list_view = false;
            self.sync_current_item_rows(viewport_restore_after_row_count_change)?;
        }

        Ok(Some(folder_tree_changed))
    }

    fn on_file_operation_complete(&mut self, lparam: ui::MessageLong) -> ExplorerResult<()> {
        let Some(message) = self.workers.messages.take_file_operation_complete(lparam) else {
            return Ok(());
        };

        let result = self.handle_file_operation_complete(message);
        self.finish_deferred_shutdown_if_file_operation_idle();
        result
    }

    fn handle_file_operation_complete(
        &mut self,
        message: FileOperationCompleteMessage,
    ) -> ExplorerResult<()> {
        self.workers
            .finish_file_operation_worker_for_generation(message.generation);
        self.clear_file_operation_status()?;
        self.refresh_drop_feedback();

        if !self
            .workers
            .is_current_file_operation_generation(message.generation)
        {
            return Ok(());
        }

        let tab_id = message.tab_id;
        let location = message.location;
        match message.result {
            Ok(mut outcome) => {
                let completion_error = outcome.completion_error.take();
                self.apply_file_operation_outcome(tab_id, location, outcome)?;
                if let Some(error) = completion_error {
                    self.finish_file_operation_error(error)
                } else {
                    Ok(())
                }
            }
            Err(error) => {
                self.invalidate_location_icon_cache_entries();
                if self.is_active_location(tab_id, &location)? {
                    let _ = self.refresh_after_file_operation();
                }
                self.finish_file_operation_error(error)
            }
        }
    }

    fn finish_file_operation_error(&mut self, error: ExplorerError) -> ExplorerResult<()> {
        if error.is_cancelled() {
            return Ok(());
        }
        if self.shutdown_after_file_operation {
            self.show_error(&error);
            Ok(())
        } else {
            Err(error)
        }
    }

    fn apply_file_operation_outcome(
        &mut self,
        tab_id: TabId,
        location: NavigationLocation,
        outcome: FileOperationWorkerOutcome,
    ) -> ExplorerResult<()> {
        self.invalidate_location_icon_cache_entries();
        self.refresh_folder_tree_after_locations_changed(&outcome.affected_folders)?;

        let operation_context_active = self.is_active_location(tab_id, &location)?;
        if operation_context_active {
            self.undo_file_operation = outcome.undo_file_operation;
            self.app.active_tab_mut()?.selected_items = outcome.selected_items;
        }

        if operation_context_active || self.active_location_in(&outcome.affected_folders)? {
            self.workers.retire_active_listing_worker();
            self.workers.reap_retired_listing_workers();
            self.refresh_view_preserving_current_rows()
        } else {
            Ok(())
        }
    }

    fn is_active_location(
        &self,
        tab_id: TabId,
        location: &NavigationLocation,
    ) -> ExplorerResult<bool> {
        let active_tab = self.app.active_tab()?;
        Ok(active_tab.id == tab_id
            && active_tab
                .current_location()
                .has_same_path(location.as_path()))
    }

    fn active_location_in(&self, locations: &[NavigationLocation]) -> ExplorerResult<bool> {
        let active = self.app.active_tab()?.current_location();
        Ok(locations
            .iter()
            .any(|location| active.has_same_path(location.as_path())))
    }

    fn cancel_all_searches(&mut self) {
        self.workers.cancel_searches_for_shutdown();
        if let Err(error) = self.stop_search_completion_timer() {
            eprintln!("failed to stop search completion timer: {error}");
            self.workers.search_completion_timer_active = false;
        }
    }

    fn request_shutdown(&mut self) -> ExplorerResult<()> {
        self.recover_pending_complete_messages_after_timer_failure();
        if self.workers.has_file_operation_worker() {
            self.shutdown_after_file_operation = true;
            self.ensure_search_completion_timer();
            if let Err(error) =
                self.set_file_operation_status(FILE_OPERATION_SHUTDOWN_PENDING_MESSAGE)
            {
                eprintln!("failed to update file operation status: {error}");
            }
            Ok(())
        } else {
            ui::destroy_window(self.hwnd);
            Ok(())
        }
    }

    fn finish_deferred_shutdown_if_file_operation_idle(&mut self) {
        if self.shutdown_after_file_operation && !self.workers.has_file_operation_worker() {
            self.shutdown_after_file_operation = false;
            ui::destroy_window(self.hwnd);
        }
    }

    fn cleanup_background_workers_for_shutdown(&mut self) {
        self.icon_load_shutdown_requested
            .store(true, Ordering::Relaxed);
        self.pending_icon_load_tasks.clear();
        self.icon_load_messages.clear();
        reap_finished_icon_load_workers(&mut self.icon_load_workers);
        if let Err(error) = self.stop_file_watch_refresh_timer() {
            eprintln!("failed to stop file watch refresh timer: {error}");
            self.workers.file_watch_refresh_timer_active = false;
        }
        self.cancel_all_folder_tree_child_workers();
        self.reap_finished_folder_tree_child_workers();
        self.workers.cleanup_background_workers_for_shutdown();
        while let Some(message) = self.workers.messages.take_next_file_operation_complete() {
            if let Err(error) = self.handle_file_operation_complete(message) {
                self.show_error(&error);
            }
        }
    }

    fn on_notify(&mut self, lparam: ui::MessageLong) -> ExplorerResult<()> {
        // SAFETY: WM_NOTIFY lparam is valid for the duration of this message dispatch.
        let Some(notification) = (unsafe { ui::notification(lparam) }) else {
            return Ok(());
        };

        if notification.id_from == ID_TAB_CONTROL as usize {
            if notification.code == ui::TAB_SELECTION_CHANGED {
                self.switch_to_selected_tab()?;
            } else if notification.code == ui::TAB_RIGHT_CLICK {
                self.show_tab_menu()?;
            }
            return Ok(());
        }

        if notification.id_from == ID_FOLDER_TREE as usize {
            if notification.code == ui::TREE_VIEW_SELECTION_CHANGED {
                self.on_folder_tree_selection_changed()?;
            } else if notification.code == ui::TREE_VIEW_ITEM_EXPANDING {
                self.on_folder_tree_item_expanding(lparam)?;
            } else if notification.code == ui::TREE_VIEW_BEGIN_DRAG {
                self.begin_folder_tree_drag(lparam)?;
            } else if notification.code == ui::TREE_VIEW_RIGHT_CLICK {
                self.show_folder_tree_context_menu()?;
            }
            return Ok(());
        }

        if notification.id_from != ID_FILE_LIST as usize {
            return Ok(());
        }

        if notification.code == ui::LIST_VIEW_GET_DISPLAY_INFO {
            self.on_list_view_get_display_info(lparam)?;
            return Ok(());
        }

        if notification.code == ui::LIST_VIEW_COLUMN_CLICK {
            // SAFETY: the notification code above identifies a ListView column-click payload.
            if let Some(column_index) = unsafe { ui::list_view_column_click_index(lparam) } {
                self.sort_by_list_column(column_index)?;
            }
            return Ok(());
        }

        if notification.code == ui::LIST_VIEW_RIGHT_CLICK {
            self.show_context_menu(lparam)?;
            return Ok(());
        }

        if notification.code == ui::LIST_VIEW_BEGIN_DRAG {
            self.begin_list_view_drag(lparam)?;
            return Ok(());
        }

        if notification.code == ui::LIST_VIEW_END_LABEL_EDIT {
            self.on_list_view_label_edit(lparam)?;
            return Ok(());
        }

        if !list_view_notification_activates_item(notification.code) {
            return Ok(());
        }

        // SAFETY: the notification code above identifies a ListView activation payload.
        if let Some(index) = unsafe { ui::list_view_activation_index(lparam) } {
            self.activate_list_item(index)?;
        }

        Ok(())
    }

    fn switch_to_selected_tab(&mut self) -> ExplorerResult<()> {
        let Some(index) = ui::tab_current_selection(self.tab_control) else {
            return Ok(());
        };

        if index == self.app.active_tab_index()? {
            return Ok(());
        }

        self.app.switch_to_tab_index(index)?;
        self.search_controls_requested = false;
        self.create_menu()?;
        self.refresh_view()
    }

    fn show_tab_menu(&mut self) -> ExplorerResult<()> {
        let point = ui::cursor_position()?;
        self.activate_tab_at_point(point)?;

        let tab_menu = ui::create_popup_menu()?;
        self.populate_tab_menu(tab_menu)?;
        let selected_command = ui::track_popup_menu(self.hwnd, tab_menu, point);
        let destroy_result = ui::destroy_menu(tab_menu);

        match (selected_command, destroy_result) {
            (Err(error), _) => Err(error),
            (_, Err(error)) => Err(error),
            (Ok(Some(command_id)), Ok(())) => {
                self.on_command(command_id, 0);
                Ok(())
            }
            (Ok(None), Ok(())) => Ok(()),
        }
    }

    fn activate_tab_at_point(&mut self, point: ui::ScreenPoint) -> ExplorerResult<()> {
        let Some(index) = ui::tab_index_at_screen_point(self.tab_control, point)? else {
            return Ok(());
        };
        if index >= self.app.state().tabs.len() {
            return Ok(());
        }

        ui::set_tab_current_selection(self.tab_control, index);
        self.switch_to_selected_tab()
    }

    fn activate_list_item(&mut self, index: usize) -> ExplorerResult<()> {
        let Some(item) = self.current_item_slice()?.get(index).cloned() else {
            return Ok(());
        };

        self.activate_item(item)
    }

    fn activate_item(&mut self, item: FileItem) -> ExplorerResult<()> {
        let tab_id = self.app.state().active_tab_id;
        match self.app.activate_item_in_active(&item)? {
            ItemActivation::Navigated => self.finish_successful_navigation(tab_id),
            ItemActivation::Opened => Ok(()),
        }
    }

    fn show_context_menu(&mut self, lparam: ui::MessageLong) -> ExplorerResult<()> {
        let point = ui::cursor_position()?;
        let item_count = self.current_item_count()?;
        // SAFETY: callers pass the NM_RCLICK notification payload from the ListView.
        let clicked_index = unsafe { ui::list_view_activation_index(lparam) };
        let clicked_index = match clicked_index {
            Some(index) if index < item_count => Some(index),
            _ => ui::list_view_item_at_screen_point(self.list_view, point)?
                .filter(|index| *index < item_count),
        };

        let Some(index) = clicked_index else {
            return self.show_folder_background_context_menu(point);
        };

        if !ui::list_view_item_is_selected(self.list_view, index) {
            ui::set_list_view_selected_index(self.list_view, Some(index))?;
        }

        let targets = self.selected_locations();
        if targets.is_empty() {
            return Ok(());
        }

        let outcome = self.app.show_context_menu_for_items(
            &targets,
            ContextMenuPosition {
                x: point.x,
                y: point.y,
            },
        )?;

        self.app.active_tab_mut()?.selected_items = targets;
        if outcome.refresh_current_folder {
            self.refresh_active_view()?;
        }

        Ok(())
    }

    fn show_folder_background_context_menu(
        &mut self,
        point: ui::ScreenPoint,
    ) -> ExplorerResult<()> {
        let folder = self.app.active_tab()?.current_location().clone();
        let outcome = self.app.show_context_menu_for_folder_background(
            &folder,
            ContextMenuPosition {
                x: point.x,
                y: point.y,
            },
        )?;

        if outcome.refresh_current_folder {
            self.refresh_active_view()?;
        }

        Ok(())
    }

    fn show_folder_tree_context_menu(&mut self) -> ExplorerResult<()> {
        let point = ui::cursor_position()?;
        let Some(item) = ui::tree_view_item_at_screen_point(self.tree_view, point)? else {
            return Ok(());
        };
        let Some(value) = ui::tree_view_item_value(self.tree_view, item)? else {
            return Ok(());
        };
        let Some(node) = self.folder_tree_nodes.get(value.get()).cloned() else {
            return Ok(());
        };

        if node.kind == FolderTreeItemKind::Bookmark {
            return self.show_bookmark_tree_context_menu(item, node.location, point);
        }

        self.suppress_folder_tree_selection_while(|window| {
            ui::set_tree_view_selected_item(window.tree_view, Some(item))
        })?;

        let targets = vec![node.location];
        let outcome = self.app.show_context_menu_for_items(
            &targets,
            ContextMenuPosition {
                x: point.x,
                y: point.y,
            },
        );

        match outcome {
            Ok(outcome) if outcome.refresh_current_folder => self.refresh_active_view(),
            Ok(_) => self.sync_folder_tree_selection(),
            Err(error) => {
                let _ = self.sync_folder_tree_selection();
                Err(error)
            }
        }
    }

    fn show_bookmark_tree_context_menu(
        &mut self,
        item: ui::TreeViewItemHandle,
        location: NavigationLocation,
        point: ui::ScreenPoint,
    ) -> ExplorerResult<()> {
        self.suppress_folder_tree_selection_while(|window| {
            ui::set_tree_view_selected_item(window.tree_view, Some(item))
        })?;

        let menu = ui::create_popup_menu()?;
        ui::append_menu_item(menu, ID_BOOKMARK_OPEN_TREE_ITEM, "Open")?;
        ui::append_menu_separator(menu)?;
        ui::append_menu_item(menu, ID_BOOKMARK_REMOVE_TREE_ITEM, "Remove Bookmark")?;
        let selected_command = ui::track_popup_menu(self.hwnd, menu, point);
        let destroy_result = ui::destroy_menu(menu);

        match (selected_command, destroy_result) {
            (Err(error), _) => Err(error),
            (_, Err(error)) => Err(error),
            (Ok(Some(ID_BOOKMARK_OPEN_TREE_ITEM)), Ok(())) => self.navigate_to_location(location),
            (Ok(Some(ID_BOOKMARK_REMOVE_TREE_ITEM)), Ok(())) => {
                self.remove_bookmark_for_location(&location)
            }
            (Ok(_), Ok(())) => self.sync_folder_tree_selection(),
        }
    }

    fn selected_item(&self) -> Option<FileItem> {
        let Ok(current_items) = self.current_item_slice() else {
            return None;
        };
        ui::selected_list_view_indices(self.list_view)
            .into_iter()
            .find_map(|index| current_items.get(index).cloned())
    }

    fn selected_locations(&self) -> Vec<NavigationLocation> {
        let Ok(current_items) = self.current_item_slice() else {
            return Vec::new();
        };
        ui::selected_list_view_indices(self.list_view)
            .into_iter()
            .filter_map(|index| current_items.get(index).map(|item| item.location.clone()))
            .collect()
    }

    fn remember_current_selection(&mut self) -> ExplorerResult<()> {
        let selected_items = self.selected_locations();
        self.app.active_tab_mut()?.selected_items = selected_items;
        Ok(())
    }

    fn sync_tabs(&self) -> ExplorerResult<()> {
        let labels = self
            .app
            .state()
            .tabs
            .iter()
            .map(tab_display_label)
            .collect::<Vec<_>>();
        ui::set_tab_items(self.tab_control, &labels, self.app.active_tab_index()?)
    }

    fn sync_window_title(&self) -> ExplorerResult<()> {
        ui::set_window_text(self.hwnd, OsStr::new(WINDOW_TITLE))
    }

    fn sync_address(&self) -> ExplorerResult<()> {
        let active_tab = self.app.active_tab()?;
        ui::set_window_text(
            self.address_edit,
            active_tab.current_location().as_path().as_os_str(),
        )
    }

    fn show_error(&self, error: &ExplorerError) {
        if !should_show_user_error_dialog(error) {
            return;
        }
        eprintln!("detail: {error}");
        ui::show_error_message(self.hwnd, WINDOW_TITLE, &error.user_message());
    }

    fn recover_after_error(&mut self, error: &ExplorerError) {
        self.show_error(error);
        if let Err(recovery_error) = self.remove_missing_current_items_after_error(error) {
            eprintln!("failed to remove missing list items after error: {recovery_error}");
        }
        let _ = self.sync_address();
        let _ = self.sync_folder_tree_selection();
    }

    fn remove_missing_current_items_after_error(
        &mut self,
        error: &ExplorerError,
    ) -> ExplorerResult<()> {
        let missing_locations = missing_list_item_locations_from_error(
            error,
            self.current_item_slice()?,
            path_is_missing,
        );
        if missing_locations.is_empty() {
            return Ok(());
        }

        let previous_count = self.current_item_count()?;
        self.remove_locations_from_active_tab(&missing_locations)?;
        let removed = if let Some(items) = self.current_items.as_listing_mut() {
            remove_file_items_by_location(items, &missing_locations)
        } else {
            self.current_item_count()? != previous_count
        };
        if removed {
            self.current_listing_child_indices = None;
            let current_count = self.current_item_count()?;
            reset_file_item_cell_text_caches(
                &mut self.current_item_cell_text_caches,
                current_count,
            );
            if let Some(rows) = &mut self.current_search_rows {
                rows.item_count = current_count;
            }
            let viewport_restore = ui::list_view_viewport(self.list_view);
            self.current_item_rows_synced_to_list_view = false;
            self.sync_current_item_rows(viewport_restore)?;
            self.refresh_drop_feedback();
        }

        Ok(())
    }

    fn remove_locations_from_active_tab(
        &mut self,
        locations: &[NavigationLocation],
    ) -> ExplorerResult<()> {
        let active_tab = self.app.active_tab_mut()?;
        remove_navigation_locations_by_path(&mut active_tab.selected_items, locations);
        match &mut active_tab.search {
            SearchState::Results { items, .. } => {
                remove_file_items_by_location(items, locations);
            }
            SearchState::Cancelled { partial_items, .. } => {
                remove_file_items_by_location(partial_items, locations);
            }
            SearchState::Idle | SearchState::Running { .. } => {}
        }
        Ok(())
    }

    fn schedule_user_settings_save(&mut self) -> ExplorerResult<()> {
        if !self.settings_save_enabled {
            self.pending_user_settings_save = false;
            return Ok(());
        }

        self.pending_user_settings_save = true;
        if self.hwnd.is_null() {
            return self.save_pending_user_settings();
        }

        match ui::set_window_timer(
            self.hwnd,
            ID_USER_SETTINGS_SAVE_TIMER,
            USER_SETTINGS_SAVE_DEBOUNCE_MS,
        ) {
            Ok(()) => {
                self.user_settings_save_timer_active = true;
                Ok(())
            }
            Err(error) if self.user_settings_save_timer_active => {
                eprintln!("failed to reset user settings save timer: {error}");
                Ok(())
            }
            Err(error) => {
                eprintln!("failed to start user settings save timer; saving immediately: {error}");
                self.save_pending_user_settings()
            }
        }
    }

    fn stop_user_settings_save_timer(&mut self) {
        if !self.user_settings_save_timer_active {
            return;
        }

        if let Err(error) = ui::kill_window_timer(self.hwnd, ID_USER_SETTINGS_SAVE_TIMER) {
            eprintln!("failed to stop user settings save timer: {error}");
        }
        self.user_settings_save_timer_active = false;
    }

    fn save_pending_user_settings(&mut self) -> ExplorerResult<()> {
        if !self.pending_user_settings_save {
            self.stop_user_settings_save_timer();
            return Ok(());
        }

        self.save_user_settings()
    }

    fn save_user_settings(&mut self) -> ExplorerResult<()> {
        self.stop_user_settings_save_timer();
        if !self.settings_save_enabled {
            self.pending_user_settings_save = false;
            return Ok(());
        }

        let _ = self.apply_pending_startup_restore()?;
        self.app.save_user_settings(&self.settings_store)?;
        self.pending_user_settings_save = false;
        Ok(())
    }
}

fn build_horizontal_pane_layout(
    client_width: i32,
    margin: i32,
    splitter_width: i32,
    desired_tree_width: i32,
    min_tree_width: i32,
    min_right_width: i32,
) -> HorizontalPaneLayout {
    let margin = margin.max(0);
    let splitter_width = splitter_width.max(0);
    let content_width = (client_width - margin * 2).max(0);
    let tree_width = constrained_folder_tree_width(
        content_width,
        desired_tree_width,
        splitter_width,
        min_tree_width,
        min_right_width,
    );
    let available_after_tree = (content_width - tree_width).max(0);
    let actual_splitter_width = splitter_width.min(available_after_tree);
    let right_width = (content_width - tree_width - actual_splitter_width).max(0);
    let splitter_x = margin + tree_width;

    HorizontalPaneLayout {
        tree_width,
        splitter_x,
        splitter_width: actual_splitter_width,
        right_x: splitter_x + actual_splitter_width,
        right_width,
    }
}

fn constrained_folder_tree_width(
    content_width: i32,
    desired_tree_width: i32,
    splitter_width: i32,
    min_tree_width: i32,
    min_right_width: i32,
) -> i32 {
    let content_width = content_width.max(0);
    if content_width == 0 {
        return 0;
    }

    let splitter_width = splitter_width.max(0);
    let min_tree_width = min_tree_width.max(0);
    let min_right_width = min_right_width.max(0);
    if content_width <= min_tree_width + splitter_width {
        return desired_tree_width.max(min_tree_width).min(content_width);
    }

    let max_with_min_right = content_width - splitter_width - min_right_width;
    let max_tree_width = if max_with_min_right >= min_tree_width {
        max_with_min_right
    } else {
        (content_width - splitter_width).max(0)
    };
    let min_tree_width = min_tree_width.min(max_tree_width);

    desired_tree_width
        .max(0)
        .clamp(min_tree_width, max_tree_width.max(min_tree_width))
}

fn scale_px_between_dpi(value: i32, from: ui::DpiMetrics, to: ui::DpiMetrics) -> i32 {
    if value == 0 || from == to {
        return value;
    }

    let numerator = i64::from(value) * i64::from(to.current_dpi());
    let denominator = i64::from(from.current_dpi().max(1));
    let rounded = if numerator >= 0 {
        (numerator + denominator / 2) / denominator
    } else {
        (numerator - denominator / 2) / denominator
    };
    let scaled = rounded.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32;

    if value > 0 {
        scaled.max(1)
    } else {
        scaled.min(-1)
    }
}

fn notify_error_result(lparam: ui::MessageLong) -> ui::MessageResult {
    // SAFETY: callers pass the WM_NOTIFY lparam for the current message dispatch.
    let Some(notification) = (unsafe { ui::notification(lparam) }) else {
        return 0;
    };
    if notification.id_from == ID_FOLDER_TREE as usize
        && notification.code == ui::TREE_VIEW_ITEM_EXPANDING
    {
        1
    } else {
        0
    }
}

fn should_recover_pending_complete_messages_after_timer_failure(message: u32) -> bool {
    !matches!(
        message,
        ui::MESSAGE_NC_CREATE | ui::MESSAGE_DESTROY | ui::MESSAGE_NC_DESTROY
    )
}

unsafe extern "system" fn window_proc(
    hwnd: ui::RawWindowHandle,
    message: u32,
    wparam: ui::MessageWord,
    lparam: ui::MessageLong,
) -> ui::MessageResult {
    let hwnd = ui::WindowHandle::from_raw(hwnd);
    if should_recover_pending_complete_messages_after_timer_failure(message) {
        // SAFETY: user data remains owned by the window until WM_NCDESTROY.
        if let Some(window) = unsafe { ui::window_state_mut::<MainWindow>(hwnd) } {
            window.recover_pending_complete_messages_after_timer_failure();
        }
    }

    match message {
        ui::MESSAGE_NC_CREATE => {
            // SAFETY: WM_NCCREATE lparam is a CREATESTRUCTW pointer supplied by Windows.
            if unsafe { ui::attach_window_state_from_nccreate::<MainWindow>(hwnd, lparam) } {
                // SAFETY: the MainWindow pointer was just attached to the window user data.
                if let Some(window) = unsafe { ui::window_state_mut::<MainWindow>(hwnd) } {
                    window.mark_window_proc_owner();
                }
                1
            } else {
                0
            }
        }
        ui::MESSAGE_CREATE => {
            // SAFETY: user data was attached during WM_NCCREATE for this window.
            let Some(window) = (unsafe { ui::window_state_mut::<MainWindow>(hwnd) }) else {
                return -1;
            };

            match window.on_create(hwnd) {
                Ok(()) => 0,
                Err(error) => {
                    if should_show_user_error_dialog(&error) {
                        ui::show_error_message(hwnd, WINDOW_TITLE, &error.user_message());
                    }
                    -1
                }
            }
        }
        MESSAGE_CLOSE => {
            // SAFETY: user data remains owned by the window until WM_NCDESTROY.
            if let Some(window) = unsafe { ui::window_state_mut::<MainWindow>(hwnd) } {
                if let Err(error) = window.request_shutdown() {
                    window.recover_after_error(&error);
                }
            }
            0
        }
        ui::MESSAGE_COMMAND => {
            // SAFETY: user data remains owned by the window until WM_NCDESTROY.
            if let Some(window) = unsafe { ui::window_state_mut::<MainWindow>(hwnd) } {
                window.on_command(ui::command_id(wparam), ui::command_notification(wparam));
            }
            0
        }
        ui::MESSAGE_NOTIFY => {
            // SAFETY: user data remains owned by the window until WM_NCDESTROY.
            if let Some(window) = unsafe { ui::window_state_mut::<MainWindow>(hwnd) } {
                if let Err(error) = window.on_notify(lparam) {
                    let result = notify_error_result(lparam);
                    window.recover_after_error(&error);
                    return result;
                }
            }
            0
        }
        ui::MESSAGE_DRAW_ITEM => {
            // SAFETY: user data remains owned by the window until WM_NCDESTROY.
            if let Some(window) = unsafe { ui::window_state_mut::<MainWindow>(hwnd) } {
                if ui::command_id(wparam) == ID_TAB_CONTROL {
                    if let Some(result) = ui::draw_tab_item(
                        window.app.appearance_theme(),
                        &window.font_resource,
                        lparam,
                    ) {
                        return result;
                    }
                }
                if let Some(icon) = window.navigation_icons.for_command(ui::command_id(wparam)) {
                    if let Some(result) = ui::draw_material_icon_button(
                        window.app.appearance_theme(),
                        &window.theme_resources,
                        icon,
                        lparam,
                    ) {
                        return result;
                    }
                }
            }
            ui::default_window_proc(hwnd, message, wparam, lparam)
        }
        ui::MESSAGE_ERASE_BACKGROUND => {
            // SAFETY: user data remains owned by the window until WM_NCDESTROY.
            if let Some(window) = unsafe { ui::window_state_mut::<MainWindow>(hwnd) } {
                if let Some(result) = ui::erase_window_background(
                    hwnd,
                    window.app.appearance_theme(),
                    &window.theme_resources,
                    wparam,
                ) {
                    return result;
                }
            }
            ui::default_window_proc(hwnd, message, wparam, lparam)
        }
        ui::MESSAGE_CONTROL_COLOR_EDIT
        | ui::MESSAGE_CONTROL_COLOR_STATIC
        | ui::MESSAGE_CONTROL_COLOR_BUTTON => {
            // SAFETY: user data remains owned by the window until WM_NCDESTROY.
            if let Some(window) = unsafe { ui::window_state_mut::<MainWindow>(hwnd) } {
                if let Some(result) = ui::control_color_brush(
                    window.app.appearance_theme(),
                    &window.theme_resources,
                    wparam,
                ) {
                    return result;
                }
            }
            ui::default_window_proc(hwnd, message, wparam, lparam)
        }
        ui::MESSAGE_TIMER if wparam == ID_SEARCH_COMPLETION_TIMER => {
            // SAFETY: user data remains owned by the window until WM_NCDESTROY.
            if let Some(window) = unsafe { ui::window_state_mut::<MainWindow>(hwnd) } {
                if !window.workers.search_completion_timer_active
                    && window.workers.messages.has_pending_complete()
                {
                    window.workers.search_completion_timer_active = true;
                }
                if let Err(error) = window.on_search_completion_timer() {
                    window.recover_after_error(&error);
                }
            }
            0
        }
        ui::MESSAGE_TIMER if wparam == ID_TREE_DRAG_FEEDBACK_TIMER => {
            // SAFETY: user data remains owned by the window until WM_NCDESTROY.
            if let Some(window) = unsafe { ui::window_state_mut::<MainWindow>(hwnd) } {
                if let Some(target) = &window.tree_drop_target {
                    target.tick_drag_feedback();
                }
            }
            0
        }
        ui::MESSAGE_TIMER if wparam == ID_LIST_DRAG_FEEDBACK_TIMER => {
            // SAFETY: user data remains owned by the window until WM_NCDESTROY.
            if let Some(window) = unsafe { ui::window_state_mut::<MainWindow>(hwnd) } {
                if let Some(target) = &window.list_drop_target {
                    target.tick_drag_feedback();
                }
            }
            0
        }
        ui::MESSAGE_TIMER if wparam == ID_FILE_WATCH_REFRESH_TIMER => {
            // SAFETY: user data remains owned by the window until WM_NCDESTROY.
            if let Some(window) = unsafe { ui::window_state_mut::<MainWindow>(hwnd) } {
                if let Err(error) = window.on_file_watch_refresh_timer() {
                    window.recover_after_error(&error);
                }
            }
            0
        }
        ui::MESSAGE_TIMER if wparam == ID_USER_SETTINGS_SAVE_TIMER => {
            // SAFETY: user data remains owned by the window until WM_NCDESTROY.
            if let Some(window) = unsafe { ui::window_state_mut::<MainWindow>(hwnd) } {
                if let Err(error) = window.save_pending_user_settings() {
                    window.recover_after_error(&error);
                }
            }
            0
        }
        ui::MESSAGE_TIMER if wparam == ID_DEFERRED_STARTUP_TIMER => {
            // SAFETY: user data remains owned by the window until WM_NCDESTROY.
            if let Some(window) = unsafe { ui::window_state_mut::<MainWindow>(hwnd) } {
                if let Err(error) = window.on_deferred_startup_timer() {
                    window.recover_after_error(&error);
                }
            }
            0
        }
        MESSAGE_SEARCH_PROGRESS => {
            // SAFETY: user data remains owned by the window until WM_NCDESTROY.
            if let Some(window) = unsafe { ui::window_state_mut::<MainWindow>(hwnd) } {
                if let Err(error) = window.on_search_progress(lparam) {
                    window.show_error(&error);
                }
            }
            0
        }
        MESSAGE_SEARCH_COMPLETE => {
            // SAFETY: user data remains owned by the window until WM_NCDESTROY.
            if let Some(window) = unsafe { ui::window_state_mut::<MainWindow>(hwnd) } {
                if let Err(error) = window.on_search_complete(lparam) {
                    window.recover_after_error(&error);
                }
            }
            0
        }
        MESSAGE_LISTING_COMPLETE => {
            // SAFETY: user data remains owned by the window until WM_NCDESTROY.
            if let Some(window) = unsafe { ui::window_state_mut::<MainWindow>(hwnd) } {
                if let Err(error) = window.on_listing_complete(lparam) {
                    window.recover_after_error(&error);
                }
            }
            0
        }
        MESSAGE_FILE_OPERATION_COMPLETE => {
            // SAFETY: user data remains owned by the window until WM_NCDESTROY.
            if let Some(window) = unsafe { ui::window_state_mut::<MainWindow>(hwnd) } {
                if let Err(error) = window.on_file_operation_complete(lparam) {
                    window.recover_after_error(&error);
                }
            }
            0
        }
        MESSAGE_OLE_DROP_EVENT => {
            // SAFETY: user data remains owned by the window until WM_NCDESTROY.
            if let Some(window) = unsafe { ui::window_state_mut::<MainWindow>(hwnd) } {
                if let Err(error) = window.on_ole_drop_event_message() {
                    window.recover_after_error(&error);
                }
            }
            0
        }
        MESSAGE_FILE_WATCH_CHANGED => {
            // SAFETY: user data remains owned by the window until WM_NCDESTROY.
            if let Some(window) = unsafe { ui::window_state_mut::<MainWindow>(hwnd) } {
                if let Err(error) = window.on_file_watch_changed(lparam) {
                    window.recover_after_error(&error);
                }
            }
            0
        }
        MESSAGE_FOLDER_TREE_CHILDREN_COMPLETE => {
            // SAFETY: user data remains owned by the window until WM_NCDESTROY.
            if let Some(window) = unsafe { ui::window_state_mut::<MainWindow>(hwnd) } {
                if let Err(error) = window.on_folder_tree_children_complete(lparam) {
                    window.recover_after_error(&error);
                }
            }
            0
        }
        MESSAGE_ICON_LOAD_COMPLETE => {
            // SAFETY: user data remains owned by the window until WM_NCDESTROY.
            if let Some(window) = unsafe { ui::window_state_mut::<MainWindow>(hwnd) } {
                if let Err(error) = window.on_icon_load_complete(lparam) {
                    window.recover_after_error(&error);
                }
            }
            0
        }
        ui::MESSAGE_SET_CURSOR => {
            // SAFETY: user data remains owned by the window until WM_NCDESTROY.
            if let Some(window) = unsafe { ui::window_state_mut::<MainWindow>(hwnd) } {
                match window.on_set_cursor() {
                    Ok(true) => return 1,
                    Ok(false) => {}
                    Err(error) => window.recover_after_error(&error),
                }
            }
            ui::default_window_proc(hwnd, message, wparam, lparam)
        }
        ui::MESSAGE_MOUSE_MOVE => {
            // SAFETY: user data remains owned by the window until WM_NCDESTROY.
            if let Some(window) = unsafe { ui::window_state_mut::<MainWindow>(hwnd) } {
                match window.on_mouse_move(lparam) {
                    Ok(true) => return 0,
                    Ok(false) => {}
                    Err(error) => window.recover_after_error(&error),
                }
            }
            ui::default_window_proc(hwnd, message, wparam, lparam)
        }
        ui::MESSAGE_LEFT_BUTTON_DOWN => {
            // SAFETY: user data remains owned by the window until WM_NCDESTROY.
            if let Some(window) = unsafe { ui::window_state_mut::<MainWindow>(hwnd) } {
                match window.on_left_button_down(lparam) {
                    Ok(true) => return 0,
                    Ok(false) => {}
                    Err(error) => window.recover_after_error(&error),
                }
            }
            ui::default_window_proc(hwnd, message, wparam, lparam)
        }
        ui::MESSAGE_LEFT_BUTTON_UP => {
            // SAFETY: user data remains owned by the window until WM_NCDESTROY.
            if let Some(window) = unsafe { ui::window_state_mut::<MainWindow>(hwnd) } {
                if window.on_left_button_up() {
                    return 0;
                }
            }
            ui::default_window_proc(hwnd, message, wparam, lparam)
        }
        ui::MESSAGE_CAPTURE_CHANGED => {
            // SAFETY: user data remains owned by the window until WM_NCDESTROY.
            if let Some(window) = unsafe { ui::window_state_mut::<MainWindow>(hwnd) } {
                window.on_capture_changed();
            }
            0
        }
        ui::MESSAGE_GET_MIN_MAX_INFO => {
            // SAFETY: user data remains owned by the window until WM_NCDESTROY.
            if let Some(window) = unsafe { ui::window_state_mut::<MainWindow>(hwnd) } {
                if window.apply_minimum_tracking_size(lparam) {
                    return 0;
                }
            }
            ui::default_window_proc(hwnd, message, wparam, lparam)
        }
        ui::MESSAGE_ENTER_SIZE_MOVE => {
            // SAFETY: user data remains owned by the window until WM_NCDESTROY.
            if let Some(window) = unsafe { ui::window_state_mut::<MainWindow>(hwnd) } {
                window.enter_size_move();
            }
            0
        }
        ui::MESSAGE_EXIT_SIZE_MOVE => {
            // SAFETY: user data remains owned by the window until WM_NCDESTROY.
            if let Some(window) = unsafe { ui::window_state_mut::<MainWindow>(hwnd) } {
                if let Err(error) = window.exit_size_move(hwnd) {
                    window.show_error(&error);
                }
            }
            0
        }
        ui::MESSAGE_DPI_CHANGED => {
            // SAFETY: user data remains owned by the window until WM_NCDESTROY.
            if let Some(window) = unsafe { ui::window_state_mut::<MainWindow>(hwnd) } {
                if let Err(error) = window.on_dpi_changed(ui::dpi_from_changed_message(wparam)) {
                    window.show_error(&error);
                }
            }
            0
        }
        ui::MESSAGE_SIZE => {
            // SAFETY: user data remains owned by the window until WM_NCDESTROY.
            if let Some(window) = unsafe { ui::window_state_mut::<MainWindow>(hwnd) } {
                if let Err(error) = window.on_size() {
                    window.show_error(&error);
                }
            }
            0
        }
        ui::MESSAGE_DESTROY => {
            // SAFETY: user data remains owned by the window until WM_NCDESTROY.
            if let Some(window) = unsafe { ui::window_state_mut::<MainWindow>(hwnd) } {
                window.unregister_drop_targets();
                window.cancel_all_searches();
                window.cleanup_background_workers_for_shutdown();
                if let Err(error) = window.save_user_settings() {
                    window.show_error(&error);
                }
            }
            ui::post_quit_message(0);
            0
        }
        ui::MESSAGE_NC_DESTROY => {
            // SAFETY: this is the final owner handoff for the Box passed to CreateWindowExW.
            let _ = unsafe { ui::take_window_state::<MainWindow>(hwnd) };
            ui::default_window_proc(hwnd, message, wparam, lparam)
        }
        _ => ui::default_window_proc(hwnd, message, wparam, lparam),
    }
}

fn collect_non_empty_drop_effect_hints<T, I, F>(
    total_len: usize,
    items: I,
    mut hint_for_item: F,
) -> Vec<platform::OleDropEffectHint>
where
    I: IntoIterator<Item = T>,
    F: FnMut(T) -> platform::OleDropEffectHint,
{
    let none_hint = platform::OleDropEffectHint::none();
    let mut hints = Vec::new();
    for (index, item) in items.into_iter().enumerate() {
        let hint = hint_for_item(item);
        if hints.is_empty() && hint == none_hint {
            continue;
        }
        if hints.is_empty() {
            hints = Vec::with_capacity(total_len);
            hints.resize(index, none_hint);
        }
        hints.push(hint);
    }
    hints
}

fn internal_empty_file_list_drop_destination(
    origin: InternalDragOrigin,
    search: &SearchState,
    current_location: &NavigationLocation,
) -> Option<NavigationLocation> {
    if origin == InternalDragOrigin::FolderTree && matches!(search, SearchState::Idle) {
        Some(current_location.clone())
    } else {
        None
    }
}

#[cfg(test)]
mod drop_feedback_hint_tests {
    use j3files::domain::{NavigationLocation, SearchCriteria, SearchState};
    use std::path::PathBuf;

    use super::{
        collect_non_empty_drop_effect_hints, internal_empty_file_list_drop_destination, platform,
        InternalDragOrigin,
    };

    #[test]
    fn collect_non_empty_drop_effect_hints_returns_empty_vec_for_all_none() {
        let hints =
            collect_non_empty_drop_effect_hints(3, 0..3, |_| platform::OleDropEffectHint::none());

        assert!(hints.is_empty());
        assert_eq!(hints.capacity(), 0);
    }

    #[test]
    fn collect_non_empty_drop_effect_hints_preserves_indices_after_enabled_hint() {
        let none = platform::OleDropEffectHint::none();
        let enabled =
            platform::OleDropEffectHint::copy_move(Some(platform::OleDropPreferredEffect::Move));
        let source = [none, none, enabled, none];

        let hints =
            collect_non_empty_drop_effect_hints(source.len(), source.iter().copied(), |hint| hint);

        assert_eq!(hints, source.to_vec());
    }

    #[test]
    fn folder_tree_drag_can_drop_on_empty_file_list_as_current_folder() {
        let current = NavigationLocation::LocalPath(PathBuf::from(r"C:\work"));

        assert_eq!(
            internal_empty_file_list_drop_destination(
                InternalDragOrigin::FolderTree,
                &SearchState::Idle,
                &current,
            ),
            Some(current)
        );
    }

    #[test]
    fn file_list_drag_cannot_drop_on_empty_file_list() {
        let current = NavigationLocation::LocalPath(PathBuf::from(r"C:\work"));

        assert_eq!(
            internal_empty_file_list_drop_destination(
                InternalDragOrigin::FileList,
                &SearchState::Idle,
                &current,
            ),
            None
        );
    }

    #[test]
    fn folder_tree_drag_empty_file_list_drop_is_disabled_while_search_results_are_shown() {
        let current = NavigationLocation::LocalPath(PathBuf::from(r"C:\work"));
        let search = SearchState::Results {
            criteria: SearchCriteria::default(),
            items: Vec::new(),
            diagnostics: Vec::new(),
            progress: Default::default(),
        };

        assert_eq!(
            internal_empty_file_list_drop_destination(
                InternalDragOrigin::FolderTree,
                &search,
                &current,
            ),
            None
        );
    }
}

fn file_item_cell_text(item: &FileItem, column_index: usize) -> Vec<u16> {
    match column_index {
        LIST_NAME_COLUMN_INDEX => display_os_cell_text(item.display_name.as_os_str()),
        LIST_TYPE_COLUMN_INDEX => display_os_cell_text(item.type_name.as_os_str()),
        LIST_SIZE_COLUMN_INDEX => display_cell_text(&format_size(item.size)),
        LIST_UPDATED_COLUMN_INDEX => display_cell_text(&format_updated_time(item.updated_at)),
        _ => display_cell_text(""),
    }
}

fn status_list_view_cell_text(status: &str, column_index: usize) -> Vec<u16> {
    match column_index {
        LIST_NAME_COLUMN_INDEX => display_cell_text(status),
        _ => display_cell_text(""),
    }
}

#[cfg(test)]
fn file_watch_child_index_map(items: &[FileItem]) -> Option<HashMap<Vec<u16>, usize>> {
    let mut child_indices = HashMap::with_capacity(items.len());
    for (item_index, item) in items.iter().enumerate() {
        let key = file_item_child_name_key(item)?;
        child_indices.entry(key).or_insert(item_index);
    }
    Some(child_indices)
}

#[cfg(test)]
fn file_watch_existing_child_indices(
    child_indices: &HashMap<Vec<u16>, usize>,
    changed_names: &[OsString],
) -> Option<Vec<Option<usize>>> {
    let mut seen_changed_names = HashMap::with_capacity(changed_names.len());
    let mut existing_indices = Vec::with_capacity(changed_names.len());
    for name in changed_names {
        let key = file_watch_child_name_key(name.as_os_str());
        let existing_index = child_indices.get(&key).copied();
        if seen_changed_names.insert(key, ()).is_some() {
            return None;
        }
        existing_indices.push(existing_index);
    }
    Some(existing_indices)
}

#[cfg(test)]
fn file_watch_existing_child_indices_from_items(
    items: &[FileItem],
    changed_names: &[OsString],
) -> Option<Vec<Option<usize>>> {
    let mut child_indices = HashMap::new();
    file_watch_existing_child_indices_from_items_with_cache(
        items,
        &mut child_indices,
        changed_names,
        SortState::default(),
    )
}

fn file_watch_existing_child_indices_from_items_with_cache(
    items: &[FileItem],
    child_indices: &mut HashMap<Vec<u16>, usize>,
    changed_names: &[OsString],
    active_sort: SortState,
) -> Option<Vec<Option<usize>>> {
    let mut changed_indices_by_key = HashMap::with_capacity(changed_names.len());
    let mut changed_keys = Vec::with_capacity(changed_names.len());
    for (changed_index, name) in changed_names.iter().enumerate() {
        let key = file_watch_child_name_key(name.as_os_str());
        if changed_indices_by_key
            .insert(key.clone(), changed_index)
            .is_some()
        {
            return None;
        }
        changed_keys.push(key);
    }

    let mut existing_indices = vec![None; changed_names.len()];
    let mut remaining_names = changed_names.len();
    let mut item_key = Vec::new();
    for (changed_index, key) in changed_keys.iter().enumerate() {
        let Some(cached_index) = child_indices.get(key).copied() else {
            continue;
        };
        let Some(item) = items.get(cached_index) else {
            child_indices.remove(key);
            continue;
        };
        file_item_child_name_key_into(item, &mut item_key)?;
        if item_key.as_slice() == key.as_slice() {
            existing_indices[changed_index] = Some(cached_index);
            remaining_names -= 1;
        } else {
            child_indices.remove(key);
        }
    }
    if remaining_names == 0 {
        trim_file_watch_child_index_cache(child_indices);
        return Some(existing_indices);
    }

    if active_sort.key == SortKey::Name {
        for (changed_index, key) in changed_keys.iter().enumerate() {
            if existing_indices[changed_index].is_some() {
                continue;
            }
            if let Some(item_index) = file_watch_existing_child_index_from_name_sorted_items(
                items,
                key,
                active_sort.direction,
                &mut item_key,
            )? {
                existing_indices[changed_index] = Some(item_index);
                child_indices.insert(key.clone(), item_index);
                remaining_names -= 1;
                if remaining_names == 0 {
                    break;
                }
            }
        }
        trim_file_watch_child_index_cache(child_indices);
        return Some(existing_indices);
    }

    for (item_index, item) in items.iter().enumerate() {
        file_item_child_name_key_into(item, &mut item_key)?;
        let Some(changed_index) = changed_indices_by_key.get(&item_key).copied() else {
            continue;
        };
        if existing_indices[changed_index].is_none() {
            existing_indices[changed_index] = Some(item_index);
            child_indices.insert(item_key.clone(), item_index);
            remaining_names -= 1;
            if remaining_names == 0 {
                break;
            }
        }
    }

    trim_file_watch_child_index_cache(child_indices);
    Some(existing_indices)
}

fn file_watch_existing_child_index_from_name_sorted_items(
    items: &[FileItem],
    key: &[u16],
    direction: SortDirection,
    item_key: &mut Vec<u16>,
) -> Option<Option<usize>> {
    let file_start = items.partition_point(|item| item.is_folder());
    file_watch_existing_child_index_from_name_sorted_range(
        items, 0, file_start, key, direction, item_key,
    )
    .and_then(|folder_index| {
        folder_index.map_or_else(
            || {
                file_watch_existing_child_index_from_name_sorted_range(
                    items,
                    file_start,
                    items.len(),
                    key,
                    direction,
                    item_key,
                )
            },
            |index| Some(Some(index)),
        )
    })
}

fn file_watch_existing_child_index_from_name_sorted_range(
    items: &[FileItem],
    start: usize,
    end: usize,
    key: &[u16],
    direction: SortDirection,
    item_key: &mut Vec<u16>,
) -> Option<Option<usize>> {
    let mut lower = start;
    let mut upper = end;
    while lower < upper {
        let middle = lower + (upper - lower) / 2;
        file_item_child_name_key_into(&items[middle], item_key)?;
        let order = match direction {
            SortDirection::Ascending => item_key.as_slice().cmp(key),
            SortDirection::Descending => item_key.as_slice().cmp(key).reverse(),
        };
        match order {
            std::cmp::Ordering::Less => lower = middle + 1,
            std::cmp::Ordering::Equal => return Some(Some(middle)),
            std::cmp::Ordering::Greater => upper = middle,
        }
    }
    Some(None)
}

fn trim_file_watch_child_index_cache(child_indices: &mut HashMap<Vec<u16>, usize>) {
    if child_indices.len() > MAX_FILE_WATCH_CHILD_INDEX_CACHE_KEYS {
        child_indices.clear();
    }
}

fn update_file_watch_child_index_map_after_changes(
    child_indices: &mut HashMap<Vec<u16>, usize>,
    row_replacements: &[(usize, FileItem, FileItem)],
    row_removals: &[(usize, FileItem)],
) -> bool {
    debug_assert!(row_removals
        .windows(2)
        .all(|window| window[0].0 <= window[1].0));

    for (index, existing, updated) in row_replacements {
        let Some(existing_key) = file_item_child_name_key(existing) else {
            return false;
        };
        let Some(updated_key) = file_item_child_name_key(updated) else {
            return false;
        };
        if existing_key != updated_key {
            child_indices.remove(&existing_key);
        }
        child_indices.insert(updated_key, *index);
    }

    for (_, item) in row_removals {
        let Some(key) = file_item_child_name_key(item) else {
            return false;
        };
        child_indices.remove(&key);
    }

    match row_removals {
        [] => {}
        [(removed_index, _)] => {
            for index in child_indices.values_mut() {
                if *index > *removed_index {
                    *index -= 1;
                }
            }
        }
        [(first_removed_index, _), (second_removed_index, _)] => {
            for index in child_indices.values_mut() {
                if *index > *second_removed_index {
                    *index -= 2;
                } else if *index > *first_removed_index {
                    *index -= 1;
                }
            }
        }
        [(first_removed_index, _), .., (last_removed_index, _)] => {
            let removal_count = row_removals.len();

            for index in child_indices.values_mut() {
                if *index <= *first_removed_index {
                    continue;
                }
                let removed_before = if *index > *last_removed_index {
                    removal_count
                } else {
                    row_removals.partition_point(|(removed_index, _)| *removed_index < *index)
                };
                *index -= removed_before;
            }
        }
    }

    trim_file_watch_child_index_cache(child_indices);
    true
}

fn update_file_watch_child_index_map_after_insertions(
    child_indices: &mut HashMap<Vec<u16>, usize>,
    row_insertions: Vec<(usize, Vec<u16>)>,
) -> bool {
    if row_insertions.is_empty() {
        return true;
    }

    debug_assert!(row_insertions
        .windows(2)
        .all(|window| window[0].0 <= window[1].0));
    debug_assert!(row_insertions
        .iter()
        .enumerate()
        .all(|(insertion_order, (inserted_index, _))| *inserted_index >= insertion_order));

    for index in child_indices.values_mut() {
        *index += file_watch_insertions_before_or_at_index(&row_insertions, *index);
    }

    for (index, key) in row_insertions {
        if child_indices.insert(key, index).is_some() {
            return false;
        }
    }

    trim_file_watch_child_index_cache(child_indices);
    true
}

fn file_watch_insertions_before_or_at_index(
    row_insertions: &[(usize, Vec<u16>)],
    existing_index: usize,
) -> usize {
    let mut lower = 0;
    let mut upper = row_insertions.len();
    while lower < upper {
        let middle = lower + (upper - lower) / 2;
        let inserted_index = row_insertions[middle].0;
        debug_assert!(inserted_index >= middle);
        if inserted_index - middle <= existing_index {
            lower = middle + 1;
        } else {
            upper = middle;
        }
    }
    lower
}

fn remove_file_watch_rows(items: &mut Vec<FileItem>, row_removals: &[(usize, FileItem)]) {
    debug_assert!(row_removals
        .windows(2)
        .all(|window| window[0].0 <= window[1].0));

    if row_removals.is_empty() {
        return;
    }

    let mut row_index = 0;
    let mut removal_index = 0;
    items.retain(|_| {
        let remove = match row_removals.get(removal_index) {
            Some((removed_index, _)) if *removed_index == row_index => {
                removal_index += 1;
                true
            }
            _ => false,
        };
        row_index += 1;
        !remove
    });
}

fn insert_file_watch_rows_sorted(
    items: &mut Vec<FileItem>,
    row_insertions: Vec<FileItem>,
    active_sort: SortState,
) -> Option<Vec<(usize, Vec<u16>)>> {
    if row_insertions.is_empty() {
        return Some(Vec::new());
    }

    let mut insertions = Vec::with_capacity(row_insertions.len());
    for item in row_insertions {
        let key = file_item_child_name_key(&item)?;
        insertions.push((item, key));
    }

    let inserted_child_indices = active_sort.insert_file_items_with_payload(items, insertions);
    Some(inserted_child_indices)
}

fn file_item_child_name_key(item: &FileItem) -> Option<Vec<u16>> {
    let mut key = Vec::new();
    file_item_child_name_key_into(item, &mut key)?;
    Some(key)
}

fn file_item_child_name_key_into(item: &FileItem, output: &mut Vec<u16>) -> Option<()> {
    let file_name = item.location.as_path().file_name()?;
    file_watch_child_name_key_into(file_name, output);
    Some(())
}

fn file_watch_child_name_key(name: &OsStr) -> Vec<u16> {
    let mut key = Vec::new();
    file_watch_child_name_key_into(name, &mut key);
    key
}

fn file_watch_child_name_key_into(name: &OsStr, output: &mut Vec<u16>) {
    let units = name.encode_wide();
    output.clear();
    output.reserve(units.size_hint().0);
    for decoded in std::char::decode_utf16(units) {
        match decoded {
            Ok(character) => push_file_watch_case_folded_char(character, output),
            Err(error) => output.push(error.unpaired_surrogate()),
        }
    }
}

fn push_file_watch_case_folded_char(character: char, output: &mut Vec<u16>) {
    for folded in character.to_lowercase() {
        let mut buffer = [0_u16; 2];
        output.extend_from_slice(folded.encode_utf16(&mut buffer));
    }
}

fn is_direct_child_file_name(name: &OsStr) -> bool {
    let mut components = Path::new(name).components();
    matches!(components.next(), Some(Component::Normal(_))) && components.next().is_none()
}

fn replacement_requires_resort(existing: &FileItem, updated: &FileItem, sort_key: SortKey) -> bool {
    match sort_key {
        SortKey::Name => {
            existing.is_folder() != updated.is_folder()
                || existing.display_name != updated.display_name
        }
        SortKey::Kind => {
            existing.is_folder() != updated.is_folder()
                || existing.kind != updated.kind
                || existing.type_name != updated.type_name
        }
        SortKey::Size => {
            existing.is_folder() != updated.is_folder() || existing.size != updated.size
        }
        SortKey::UpdatedAt => {
            existing.is_folder() != updated.is_folder() || existing.updated_at != updated.updated_at
        }
    }
}

fn replacement_may_change_folder_tree(existing: &FileItem, updated: &FileItem) -> bool {
    (existing.is_folder() || updated.is_folder())
        && (existing.is_folder() != updated.is_folder()
            || existing.display_name != updated.display_name
            || existing.attributes.hidden != updated.attributes.hidden
            || existing.attributes.system != updated.attributes.system)
}

fn display_os_cell_text(value: &OsStr) -> Vec<u16> {
    let wide = value.encode_wide();
    let mut text = Vec::with_capacity(wide.size_hint().0 + 1);
    text.extend(wide);
    text.push(0);
    text
}

fn display_cell_text(value: &str) -> Vec<u16> {
    let mut text = Vec::with_capacity(value.len() + 1);
    text.extend(value.encode_utf16());
    text.push(0);
    text
}

#[cfg(test)]
mod list_view_cell_text_cache_tests {
    use std::ffi::OsString;
    use std::path::PathBuf;
    use std::time::{Duration, UNIX_EPOCH};

    use j3files::domain::{
        ExplorerResult, FileAttributes, FileItem, FileItemKind, NavigationLocation,
    };

    use super::{
        cached_file_item_cell_text, display_cell_text, reset_file_item_cell_text_caches,
        FileItemCellTextCaches, LIST_SIZE_COLUMN_INDEX, LIST_UPDATED_COLUMN_INDEX,
    };

    fn file_item(size: Option<u64>) -> ExplorerResult<FileItem> {
        Ok(FileItem {
            location: NavigationLocation::from_path(PathBuf::from(r"C:\root\cached.txt"))?,
            display_name: OsString::from("cached.txt"),
            kind: FileItemKind::File,
            type_name: OsString::from("Text Document"),
            size,
            updated_at: Some(UNIX_EPOCH + Duration::from_secs(60)),
            attributes: FileAttributes::default(),
        })
    }

    #[test]
    fn cached_file_item_cell_text_reuses_utf16_buffer_for_same_cell() -> ExplorerResult<()> {
        let item = file_item(Some(42))?;
        let mut caches = FileItemCellTextCaches::new();

        let first_ptr = {
            let text = cached_file_item_cell_text(&mut caches, 0, &item, LIST_SIZE_COLUMN_INDEX);
            assert_eq!(text, display_cell_text("42").as_slice());
            text.as_ptr()
        };
        let second_ptr = {
            let text = cached_file_item_cell_text(&mut caches, 0, &item, LIST_SIZE_COLUMN_INDEX);
            assert_eq!(text, display_cell_text("42").as_slice());
            text.as_ptr()
        };

        assert_eq!(first_ptr, second_ptr);
        assert_eq!(
            cached_file_item_cell_text(&mut caches, 0, &item, LIST_UPDATED_COLUMN_INDEX),
            display_cell_text("1970-01-01 00:01 UTC").as_slice()
        );
        Ok(())
    }

    #[test]
    fn reset_file_item_cell_text_cache_drops_stale_formatted_cells() -> ExplorerResult<()> {
        let old_item = file_item(Some(1))?;
        let updated_item = file_item(Some(2))?;
        let mut caches = FileItemCellTextCaches::new();

        assert_eq!(
            cached_file_item_cell_text(&mut caches, 0, &old_item, LIST_SIZE_COLUMN_INDEX),
            display_cell_text("1").as_slice()
        );

        reset_file_item_cell_text_caches(&mut caches, 1);

        assert_eq!(
            cached_file_item_cell_text(&mut caches, 0, &updated_item, LIST_SIZE_COLUMN_INDEX),
            display_cell_text("2").as_slice()
        );
        Ok(())
    }

    #[test]
    fn reset_file_item_cell_text_cache_does_not_preallocate_rows() -> ExplorerResult<()> {
        let item = file_item(Some(1))?;
        let mut caches = FileItemCellTextCaches::new();

        let _ = cached_file_item_cell_text(&mut caches, 0, &item, LIST_SIZE_COLUMN_INDEX);

        reset_file_item_cell_text_caches(&mut caches, 10_000);

        assert!(caches.is_empty());
        Ok(())
    }

    #[test]
    fn cached_file_item_cell_text_keeps_sparse_high_rows_compact() -> ExplorerResult<()> {
        let item = file_item(Some(7))?;
        let mut caches = FileItemCellTextCaches::new();
        let high_row_index = 500_000;

        {
            let text = cached_file_item_cell_text(
                &mut caches,
                high_row_index,
                &item,
                LIST_SIZE_COLUMN_INDEX,
            );
            assert_eq!(text, display_cell_text("7").as_slice());
        }

        assert_eq!(caches.len(), 1);
        assert!(caches.contains_key(&high_row_index));
        assert!(!caches.contains_key(&0));
        Ok(())
    }
}

fn search_running_text(progress: SearchProgress, cancel_requested: bool) -> String {
    let prefix = if cancel_requested {
        "Cancelling search"
    } else {
        "Searching"
    };
    format!(
        "{prefix}: {} folders, {} items, {} matches, {} skipped",
        progress.visited_folders,
        progress.scanned_items,
        progress.matched_items,
        progress.skipped_folders
    )
}

fn search_finished_empty_text(progress: SearchProgress, diagnostic_count: usize) -> String {
    format!(
        "No search results: {} items scanned, {} folders skipped, {} diagnostics",
        progress.scanned_items, progress.skipped_folders, diagnostic_count
    )
}

fn search_cancelled_empty_text(progress: SearchProgress, diagnostic_count: usize) -> String {
    format!(
        "Search cancelled: {} items scanned, {} folders skipped, {} diagnostics",
        progress.scanned_items, progress.skipped_folders, diagnostic_count
    )
}

fn spawn_background_worker<F>(
    thread_name: &'static str,
    operation: &'static str,
    worker: F,
) -> ExplorerResult<JoinHandle<()>>
where
    F: FnOnce() + Send + 'static,
{
    ThreadBuilder::new()
        .name(thread_name.to_string())
        .spawn(worker)
        .map_err(|source| ExplorerError::io(operation, None, source))
}

fn start_or_queue_icon_load_task(
    workers: &mut Vec<JoinHandle<()>>,
    pending_tasks: &mut VecDeque<IconLoadTask>,
    hwnd_value: isize,
    messages: IconLoadMessageStore,
    shutdown_requested: Arc<AtomicBool>,
    task: IconLoadTask,
) -> ExplorerResult<()> {
    reap_finished_icon_load_workers(workers);
    if task.is_stale() {
        return Ok(());
    }

    if workers.len() >= MAX_CONCURRENT_ICON_LOAD_WORKERS {
        pending_tasks.push_back(task);
        return Ok(());
    }

    spawn_icon_load_task(workers, hwnd_value, messages, shutdown_requested, task)
}

fn start_pending_icon_load_tasks(
    workers: &mut Vec<JoinHandle<()>>,
    pending_tasks: &mut VecDeque<IconLoadTask>,
    hwnd_value: isize,
    messages: IconLoadMessageStore,
    shutdown_requested: Arc<AtomicBool>,
) {
    reap_finished_icon_load_workers(workers);
    while workers.len() < MAX_CONCURRENT_ICON_LOAD_WORKERS {
        let Some(task) = pending_tasks.pop_front() else {
            break;
        };
        if task.is_stale() {
            continue;
        }

        if let Err(error) = spawn_icon_load_task(
            workers,
            hwnd_value,
            messages.clone(),
            Arc::clone(&shutdown_requested),
            task,
        ) {
            eprintln!("failed to start queued icon load worker: {error}");
        }
    }
}

fn spawn_icon_load_task(
    workers: &mut Vec<JoinHandle<()>>,
    hwnd_value: isize,
    messages: IconLoadMessageStore,
    shutdown_requested: Arc<AtomicBool>,
    task: IconLoadTask,
) -> ExplorerResult<()> {
    let handle = spawn_background_worker(
        "j3files-icon-load-worker",
        "start icon load worker thread",
        move || {
            let completion = task.run();
            if shutdown_requested.load(Ordering::Relaxed) {
                return;
            }
            messages.post_complete(hwnd_value, completion);
        },
    )?;
    workers.push(handle);
    Ok(())
}

fn reap_finished_icon_load_workers(workers: &mut Vec<JoinHandle<()>>) {
    let mut index = 0;
    while index < workers.len() {
        if workers[index].is_finished() {
            let handle = workers.swap_remove(index);
            if handle.join().is_err() {
                eprintln!("icon load worker panicked");
            }
        } else {
            index += 1;
        }
    }
}

fn log_search_diagnostics(outcome: &SearchOutcome) {
    for diagnostic in outcome
        .diagnostics
        .iter()
        .take(MAX_LOGGED_SEARCH_DIAGNOSTICS)
    {
        eprintln!(
            "search skipped {:?}: {}",
            diagnostic.path, diagnostic.detail
        );
    }
    let omitted = outcome
        .diagnostics
        .len()
        .saturating_sub(MAX_LOGGED_SEARCH_DIAGNOSTICS);
    if omitted > 0 {
        eprintln!("search diagnostics log omitted {omitted} additional entries");
    }
}

fn run_file_operation<S>(
    shell_gateway: &S,
    request: FileOperationRequest,
) -> ExplorerResult<FileOperationWorkerOutcome>
where
    S: ShellTransferGateway + ShellDeleteGateway + ShellRenameGateway + ?Sized,
{
    match request {
        FileOperationRequest::Transfer {
            operation,
            sources,
            destination,
            select_completed_items,
            ..
        } => {
            let expected =
                expected_transfer_locations_for_completion(operation, &destination, &sources)?;
            if operation == DropOperation::Move {
                validate_move_drop(&sources, &destination)?;
            }
            let affected_folders =
                file_transfer_refresh_locations(&sources, &destination, operation)?;
            match operation {
                DropOperation::Copy => {
                    shell_gateway.copy_items(&sources, &destination)?;
                }
                DropOperation::Move => {
                    shell_gateway.move_items(&sources, &destination)?;
                }
            }
            let completed_transfer =
                completed_transfer_after_operation(operation, &sources, expected.as_deref());

            Ok(FileOperationWorkerOutcome {
                affected_folders,
                selected_items: if select_completed_items {
                    completed_transfer_selection(completed_transfer)
                } else {
                    Vec::new()
                },
                undo_file_operation: undo_operation_for_completed_transfer(
                    &sources,
                    completed_transfer,
                ),
                completion_error: None,
            })
        }
        FileOperationRequest::Delete {
            operation, targets, ..
        } => {
            match operation {
                DeleteFileOperation::ToRecycleBin => {
                    shell_gateway.delete_to_recycle_bin(&targets)?;
                }
                DeleteFileOperation::Permanently => {
                    shell_gateway.delete_permanently(&targets)?;
                }
            }

            Ok(FileOperationWorkerOutcome {
                affected_folders: source_parent_locations(&targets)?,
                selected_items: Vec::new(),
                undo_file_operation: None,
                completion_error: None,
            })
        }
        FileOperationRequest::Rename {
            target,
            new_name,
            undo_original_name,
            ..
        } => {
            let new_name = RenameItemName::new(new_name)?;
            shell_gateway.rename_item(&target, &new_name)?;
            let renamed = renamed_sibling_location(&target, new_name.as_os_str())?;
            let selected_items = renamed.iter().cloned().collect();
            let undo_file_operation = match (undo_original_name, renamed) {
                (Some(original_name), Some(current)) => Some(UndoFileOperation::Rename {
                    current,
                    original_name,
                }),
                _ => None,
            };

            Ok(FileOperationWorkerOutcome {
                affected_folders: rename_refresh_locations(&target)?,
                selected_items,
                undo_file_operation,
                completion_error: None,
            })
        }
        FileOperationRequest::UndoMove { moved, .. } => {
            run_undo_move_file_operation(shell_gateway, moved)
        }
    }
}

fn run_undo_move_file_operation<S>(
    shell_gateway: &S,
    moved: Vec<(NavigationLocation, NavigationLocation)>,
) -> ExplorerResult<FileOperationWorkerOutcome>
where
    S: ShellTransferGateway + ?Sized,
{
    let mut restored = Vec::new();
    let mut remaining = Vec::new();
    let mut affected_folders = Vec::new();
    let mut first_error = None;

    for (current, original_parent) in moved {
        if let Some(parent) = current.as_path().parent() {
            match NavigationLocation::from_path(parent.to_path_buf()) {
                Ok(parent_location) => affected_folders.push(parent_location),
                Err(error) => set_first_error(&mut first_error, error),
            }
        }
        affected_folders.push(original_parent.clone());

        let move_result = validate_move_drop(std::slice::from_ref(&current), &original_parent)
            .and_then(|()| {
                shell_gateway.move_items(std::slice::from_ref(&current), &original_parent)
            });

        match move_result {
            Ok(()) => {
                if let Some(file_name) = current.as_path().file_name() {
                    let mut restored_path = original_parent.as_path().to_path_buf();
                    restored_path.push(file_name);
                    match NavigationLocation::from_path(restored_path) {
                        Ok(location) => restored.push(location),
                        Err(error) => set_first_error(&mut first_error, error),
                    }
                }
            }
            Err(error) => {
                set_first_error(&mut first_error, error);
                remaining.push((current, original_parent));
            }
        }
    }

    Ok(FileOperationWorkerOutcome {
        affected_folders,
        selected_items: restored,
        undo_file_operation: if remaining.is_empty() {
            None
        } else {
            Some(UndoFileOperation::Move { moved: remaining })
        },
        completion_error: first_error,
    })
}

fn set_first_error(first_error: &mut Option<ExplorerError>, error: ExplorerError) {
    if first_error.is_none() {
        *first_error = Some(error);
    }
}

fn external_drop_default_operation(
    effects: platform::OleDropEffects,
    preferred_effect: Option<platform::OleDropPreferredEffect>,
) -> Option<DropOperation> {
    default_external_drop_operation(
        DropAllowedOperations {
            copy: effects.copy,
            move_: effects.move_,
        },
        preferred_effect.map(drop_operation_from_ole_preferred),
    )
}

fn drop_operation_from_ole_preferred(
    preferred_effect: platform::OleDropPreferredEffect,
) -> DropOperation {
    match preferred_effect {
        platform::OleDropPreferredEffect::Copy => DropOperation::Copy,
        platform::OleDropPreferredEffect::Move => DropOperation::Move,
    }
}

fn drop_operation_from_clipboard(operation: ClipboardFileOperation) -> DropOperation {
    match operation {
        ClipboardFileOperation::Copy => DropOperation::Copy,
        ClipboardFileOperation::Move => DropOperation::Move,
    }
}

fn drag_source_completion_from_ole(
    outcome: platform::OleDragSourceOutcome,
) -> DragSourceCompletion {
    match outcome {
        platform::OleDragSourceOutcome::Cancelled => DragSourceCompletion::Cancelled,
        platform::OleDragSourceOutcome::NoDrop => DragSourceCompletion::NoDrop,
        platform::OleDragSourceOutcome::Copy => DragSourceCompletion::Copy,
        platform::OleDragSourceOutcome::Move => DragSourceCompletion::Move,
    }
}

fn unique_navigation_location_paths_by_path(
    locations: &[NavigationLocation],
) -> Vec<PreparedNavigationPath> {
    let mut unique = Vec::new();
    for location in locations {
        let path = location.prepared_path();
        if !unique
            .iter()
            .any(|existing: &PreparedNavigationPath| existing.has_same_path(&path))
        {
            unique.push(path);
        }
    }
    unique
}

fn live_folder_tree_node_indices_at_location(
    nodes: &[FolderTreeNodeState],
    location_path: &PreparedNavigationPath,
) -> Vec<usize> {
    nodes
        .iter()
        .enumerate()
        .filter(|(_, node)| {
            node.handle.is_some() && node.prepared_location_path.has_same_path(location_path)
        })
        .map(|(index, _)| index)
        .collect()
}

fn reusable_folder_tree_node_indices(nodes: &[FolderTreeNodeState]) -> Vec<usize> {
    nodes
        .iter()
        .enumerate()
        // Keep pop() aligned with the previous lowest-index reuse order.
        .rev()
        .filter_map(|(index, node)| {
            if node.handle.is_none() {
                Some(index)
            } else {
                None
            }
        })
        .collect()
}

fn take_folder_tree_child_indices(
    child_indices_by_parent: &mut [Vec<usize>],
    parent_index: usize,
) -> Vec<usize> {
    child_indices_by_parent
        .get_mut(parent_index)
        .map(std::mem::take)
        .unwrap_or_default()
}

fn request_cancel_for_folder_tree_child_workers(
    workers: &[ActiveFolderTreeChildWorker],
    pending_workers: &mut VecDeque<PendingFolderTreeChildWorker>,
    parent_index: usize,
) -> Option<u64> {
    for worker in workers {
        if worker.request.parent_index == parent_index {
            worker.request_cancel();
        }
    }
    remove_pending_folder_tree_child_workers_for_parent(pending_workers, parent_index);
    pending_cancelled_folder_tree_child_generation(workers, parent_index)
}

fn enqueue_pending_folder_tree_child_worker(
    pending_workers: &mut VecDeque<PendingFolderTreeChildWorker>,
    pending_worker: PendingFolderTreeChildWorker,
) {
    remove_pending_folder_tree_child_workers_for_parent(
        pending_workers,
        pending_worker.request.parent_index,
    );
    pending_workers.push_back(pending_worker);
}

fn remove_pending_folder_tree_child_workers_for_parent(
    pending_workers: &mut VecDeque<PendingFolderTreeChildWorker>,
    parent_index: usize,
) {
    pending_workers.retain(|worker| worker.request.parent_index != parent_index);
}

fn recoverable_folder_tree_child_loading_generation_on_spawn_error(
    workers: &[ActiveFolderTreeChildWorker],
    parent_index: usize,
    loading_generation_on_spawn_error: Option<u64>,
) -> Option<u64> {
    let generation = loading_generation_on_spawn_error?;
    if workers.iter().any(|worker| {
        worker.request.parent_index == parent_index
            && worker.request.generation == generation
            && worker.is_cancel_requested()
            && !worker.is_finished()
    }) {
        Some(generation)
    } else {
        None
    }
}

fn clear_finished_recoverable_folder_tree_child_loading(
    nodes: &mut [FolderTreeNodeState],
    workers: &[ActiveFolderTreeChildWorker],
    worker: &ActiveFolderTreeChildWorker,
) {
    if !worker.is_cancel_requested() && !worker.is_completion_message_abandoned() {
        return;
    }

    let Some(node) = nodes.get_mut(worker.request.parent_index) else {
        return;
    };
    if node.children_loading_generation == Some(worker.request.generation) {
        node.children_loading_generation =
            pending_cancelled_folder_tree_child_generation(workers, worker.request.parent_index);
    }
}

fn pending_cancelled_folder_tree_child_generation(
    workers: &[ActiveFolderTreeChildWorker],
    parent_index: usize,
) -> Option<u64> {
    workers
        .iter()
        .find(|worker| {
            worker.request.parent_index == parent_index
                && worker.is_cancel_requested()
                && !worker.is_finished()
        })
        .map(|worker| worker.request.generation)
}

#[cfg(test)]
mod file_watch_refresh_tests {
    use std::collections::HashMap;
    use std::ffi::OsString;
    use std::path::PathBuf;

    use j3files::domain::{
        ExplorerResult, FileAttributes, FileItem, FileItemKind, NavigationLocation, SortDirection,
        SortKey, SortState,
    };
    use j3files::platform::{DirectoryChange, DirectoryChangeBatch, DirectoryChangeKind};

    use super::{
        file_watch_child_index_map, file_watch_existing_child_indices,
        file_watch_existing_child_indices_from_items,
        file_watch_existing_child_indices_from_items_with_cache, insert_file_watch_rows_sorted,
        remove_file_watch_rows, replacement_requires_resort,
        update_file_watch_child_index_map_after_changes,
        update_file_watch_child_index_map_after_insertions, PendingFileWatchRefresh,
        MAX_INCREMENTAL_FILE_WATCH_CHANGES,
    };

    fn batch(names: &[&str]) -> DirectoryChangeBatch {
        DirectoryChangeBatch {
            overflowed: false,
            changes: names
                .iter()
                .map(|name| DirectoryChange {
                    file_name: OsString::from(name),
                    kind: DirectoryChangeKind::Modified,
                })
                .collect(),
        }
    }

    fn file_item(path: &str, display_name: &str) -> ExplorerResult<FileItem> {
        Ok(FileItem {
            location: NavigationLocation::from_path(PathBuf::from(path))?,
            display_name: OsString::from(display_name),
            kind: FileItemKind::File,
            type_name: OsString::from("File"),
            size: None,
            updated_at: None,
            attributes: FileAttributes::default(),
        })
    }

    #[test]
    fn pending_file_watch_refresh_coalesces_repeated_names() {
        let mut refresh = PendingFileWatchRefresh::default();

        refresh.merge(batch(&["same.txt", "same.txt"]));

        assert!(!refresh.requires_full_refresh);
        assert_eq!(refresh.changed_names, vec![OsString::from("same.txt")]);
    }

    #[test]
    fn pending_file_watch_refresh_falls_back_after_unique_name_limit() {
        let mut refresh = PendingFileWatchRefresh::default();
        let names = (0..=MAX_INCREMENTAL_FILE_WATCH_CHANGES)
            .map(|index| format!("changed-{index}.txt"))
            .collect::<Vec<_>>();
        let refs = names.iter().map(String::as_str).collect::<Vec<_>>();

        refresh.merge(batch(&refs));

        assert!(refresh.requires_full_refresh);
        assert!(refresh.changed_names.is_empty());
    }

    #[test]
    fn file_watch_existing_child_indices_match_case_insensitively() -> ExplorerResult<()> {
        let items = vec![
            file_item(r"C:\root\Alpha.txt", "Alpha.txt")?,
            file_item(r"C:\root\Beta.txt", "Beta.txt")?,
            file_item(r"C:\root\Gamma.txt", "Gamma.txt")?,
        ];
        let changed_names = vec![
            OsString::from("gamma.TXT"),
            OsString::from("missing.txt"),
            OsString::from("ALPHA.txt"),
        ];
        let child_indices = file_watch_child_index_map(&items)
            .ok_or_else(|| j3files::domain::ExplorerError::state_conflict("missing file name"))?;

        assert_eq!(
            file_watch_existing_child_indices(&child_indices, &changed_names),
            Some(vec![Some(2), None, Some(0)])
        );
        Ok(())
    }

    #[test]
    fn file_watch_existing_child_indices_reject_duplicate_names() -> ExplorerResult<()> {
        let items = vec![file_item(r"C:\root\Alpha.txt", "Alpha.txt")?];
        let changed_names = vec![OsString::from("Alpha.txt"), OsString::from("alpha.TXT")];
        let child_indices = file_watch_child_index_map(&items)
            .ok_or_else(|| j3files::domain::ExplorerError::state_conflict("missing file name"))?;

        assert_eq!(
            file_watch_existing_child_indices(&child_indices, &changed_names),
            None
        );
        Ok(())
    }

    #[test]
    fn file_watch_existing_child_indices_from_items_handles_medium_batches() -> ExplorerResult<()> {
        let items = (0..16)
            .map(|index| {
                let name = format!("Item-{index:02}.txt");
                file_item(&format!(r"C:\root\{name}"), &name)
            })
            .collect::<ExplorerResult<Vec<_>>>()?;
        let changed_names = (0..16)
            .map(|index| {
                if index % 4 == 0 {
                    OsString::from(format!("missing-{index:02}.txt"))
                } else {
                    OsString::from(format!("item-{index:02}.TXT"))
                }
            })
            .collect::<Vec<_>>();

        assert_eq!(
            file_watch_existing_child_indices_from_items(&items, &changed_names),
            Some(
                (0..16)
                    .map(|index| (index % 4 != 0).then_some(index))
                    .collect()
            )
        );
        Ok(())
    }

    #[test]
    fn file_watch_existing_child_indices_handles_name_sorted_descending() -> ExplorerResult<()> {
        let items = vec![
            file_item(r"C:\root\Gamma.txt", "Gamma.txt")?,
            file_item(r"C:\root\Delta.txt", "Delta.txt")?,
            file_item(r"C:\root\Beta.txt", "Beta.txt")?,
            file_item(r"C:\root\Alpha.txt", "Alpha.txt")?,
        ];
        let changed_names = vec![
            OsString::from("alpha.TXT"),
            OsString::from("missing.txt"),
            OsString::from("GAMMA.txt"),
        ];
        let active_sort = SortState {
            key: SortKey::Name,
            direction: SortDirection::Descending,
        };
        let mut child_indices = HashMap::new();

        assert_eq!(
            file_watch_existing_child_indices_from_items_with_cache(
                &items,
                &mut child_indices,
                &changed_names,
                active_sort,
            ),
            Some(vec![Some(3), None, Some(0)])
        );
        assert_eq!(child_indices.len(), 2);
        Ok(())
    }

    #[test]
    fn file_watch_existing_child_indices_keeps_linear_lookup_for_non_name_sort(
    ) -> ExplorerResult<()> {
        let items = vec![
            file_item(r"C:\root\Gamma.txt", "Gamma.txt")?,
            file_item(r"C:\root\Alpha.txt", "Alpha.txt")?,
            file_item(r"C:\root\Beta.txt", "Beta.txt")?,
        ];
        let changed_names = vec![OsString::from("BETA.txt")];
        let active_sort = SortState {
            key: SortKey::Size,
            direction: SortDirection::Ascending,
        };
        let mut child_indices = HashMap::new();

        assert_eq!(
            file_watch_existing_child_indices_from_items_with_cache(
                &items,
                &mut child_indices,
                &changed_names,
                active_sort,
            ),
            Some(vec![Some(2)])
        );
        assert_eq!(child_indices.len(), 1);
        Ok(())
    }

    #[test]
    fn file_watch_existing_child_indices_cache_stays_sparse() -> ExplorerResult<()> {
        let items = (0..8)
            .map(|index| {
                let name = format!("Item-{index}.txt");
                file_item(&format!(r"C:\root\{name}"), &name)
            })
            .collect::<ExplorerResult<Vec<_>>>()?;
        let changed_names = vec![OsString::from("item-5.TXT"), OsString::from("missing.txt")];
        let mut child_indices = HashMap::new();

        assert_eq!(
            file_watch_existing_child_indices_from_items_with_cache(
                &items,
                &mut child_indices,
                &changed_names,
                SortState::default(),
            ),
            Some(vec![Some(5), None])
        );

        assert_eq!(child_indices.len(), 1);
        assert_eq!(
            file_watch_existing_child_indices(&child_indices, &[OsString::from("ITEM-5.txt")]),
            Some(vec![Some(5)])
        );
        Ok(())
    }

    #[test]
    fn sparse_file_watch_child_index_cache_tracks_row_shifts() -> ExplorerResult<()> {
        let alpha = file_item(r"C:\root\Alpha.txt", "Alpha.txt")?;
        let beta = file_item(r"C:\root\Beta.txt", "Beta.txt")?;
        let gamma = file_item(r"C:\root\Gamma.txt", "Gamma.txt")?;
        let delta = file_item(r"C:\root\Delta.txt", "Delta.txt")?;
        let mut items = vec![alpha, beta.clone(), delta.clone(), gamma];
        let mut child_indices = HashMap::new();

        assert_eq!(
            file_watch_existing_child_indices_from_items_with_cache(
                &items,
                &mut child_indices,
                &[OsString::from("Delta.txt")],
                SortState::default(),
            ),
            Some(vec![Some(2)])
        );
        assert_eq!(child_indices.len(), 1);

        let row_removals = vec![(1, beta.clone())];
        assert!(update_file_watch_child_index_map_after_changes(
            &mut child_indices,
            &[],
            &row_removals,
        ));
        remove_file_watch_rows(&mut items, &row_removals);
        assert_eq!(
            file_watch_existing_child_indices_from_items_with_cache(
                &items,
                &mut child_indices,
                &[OsString::from("Delta.txt")],
                SortState::default(),
            ),
            Some(vec![Some(1)])
        );

        let inserted_indices =
            insert_file_watch_rows_sorted(&mut items, vec![beta], SortState::default())
                .ok_or_else(|| {
                    j3files::domain::ExplorerError::state_conflict("missing file name")
                })?;
        assert!(update_file_watch_child_index_map_after_insertions(
            &mut child_indices,
            inserted_indices,
        ));
        assert_eq!(
            file_watch_existing_child_indices_from_items_with_cache(
                &items,
                &mut child_indices,
                &[OsString::from("Beta.txt"), OsString::from("Delta.txt")],
                SortState::default(),
            ),
            Some(vec![Some(1), Some(2)])
        );
        assert_eq!(child_indices.len(), 2);
        Ok(())
    }

    #[test]
    fn file_watch_child_index_map_updates_after_replacement_and_removal() -> ExplorerResult<()> {
        let alpha = file_item(r"C:\root\Alpha.txt", "Alpha.txt")?;
        let beta = file_item(r"C:\root\Beta.txt", "Beta.txt")?;
        let gamma = file_item(r"C:\root\Gamma.txt", "Gamma.txt")?;
        let beta_updated = file_item(r"C:\root\beta-renamed.txt", "beta-renamed.txt")?;
        let mut child_indices =
            file_watch_child_index_map(&[alpha.clone(), beta.clone(), gamma.clone()]).ok_or_else(
                || j3files::domain::ExplorerError::state_conflict("missing file name"),
            )?;

        assert!(update_file_watch_child_index_map_after_changes(
            &mut child_indices,
            &[(1, beta, beta_updated)],
            &[(0, alpha)]
        ));

        let changed_names = vec![
            OsString::from("Alpha.txt"),
            OsString::from("beta-renamed.TXT"),
            OsString::from("Gamma.txt"),
        ];
        assert_eq!(
            file_watch_existing_child_indices(&child_indices, &changed_names),
            Some(vec![None, Some(0), Some(1)])
        );
        Ok(())
    }

    #[test]
    fn file_watch_child_index_map_updates_after_multiple_removals() -> ExplorerResult<()> {
        let items = vec![
            file_item(r"C:\root\Alpha.txt", "Alpha.txt")?,
            file_item(r"C:\root\Beta.txt", "Beta.txt")?,
            file_item(r"C:\root\Gamma.txt", "Gamma.txt")?,
            file_item(r"C:\root\Delta.txt", "Delta.txt")?,
            file_item(r"C:\root\Epsilon.txt", "Epsilon.txt")?,
            file_item(r"C:\root\Zeta.txt", "Zeta.txt")?,
        ];
        let mut child_indices = file_watch_child_index_map(&items)
            .ok_or_else(|| j3files::domain::ExplorerError::state_conflict("missing file name"))?;

        assert!(update_file_watch_child_index_map_after_changes(
            &mut child_indices,
            &[],
            &[
                (1, items[1].clone()),
                (3, items[3].clone()),
                (5, items[5].clone()),
            ],
        ));

        let changed_names = vec![
            OsString::from("Alpha.txt"),
            OsString::from("Beta.txt"),
            OsString::from("Gamma.txt"),
            OsString::from("Delta.txt"),
            OsString::from("Epsilon.txt"),
            OsString::from("Zeta.txt"),
        ];
        assert_eq!(
            file_watch_existing_child_indices(&child_indices, &changed_names),
            Some(vec![Some(0), None, Some(1), None, Some(2), None])
        );
        Ok(())
    }

    #[test]
    fn file_watch_child_index_map_updates_after_two_removals() -> ExplorerResult<()> {
        let items = vec![
            file_item(r"C:\root\Alpha.txt", "Alpha.txt")?,
            file_item(r"C:\root\Beta.txt", "Beta.txt")?,
            file_item(r"C:\root\Gamma.txt", "Gamma.txt")?,
            file_item(r"C:\root\Delta.txt", "Delta.txt")?,
            file_item(r"C:\root\Epsilon.txt", "Epsilon.txt")?,
            file_item(r"C:\root\Zeta.txt", "Zeta.txt")?,
        ];
        let mut child_indices = file_watch_child_index_map(&items)
            .ok_or_else(|| j3files::domain::ExplorerError::state_conflict("missing file name"))?;

        assert!(update_file_watch_child_index_map_after_changes(
            &mut child_indices,
            &[],
            &[(1, items[1].clone()), (4, items[4].clone())],
        ));

        let changed_names = vec![
            OsString::from("Alpha.txt"),
            OsString::from("Beta.txt"),
            OsString::from("Gamma.txt"),
            OsString::from("Delta.txt"),
            OsString::from("Epsilon.txt"),
            OsString::from("Zeta.txt"),
        ];
        assert_eq!(
            file_watch_existing_child_indices(&child_indices, &changed_names),
            Some(vec![Some(0), None, Some(1), Some(2), None, Some(3)])
        );
        Ok(())
    }

    #[test]
    fn insert_file_watch_rows_sorted_keeps_listing_order() -> ExplorerResult<()> {
        let mut items = vec![
            file_item(r"C:\root\Alpha.txt", "Alpha.txt")?,
            file_item(r"C:\root\Delta.txt", "Delta.txt")?,
        ];
        let row_insertions = vec![
            file_item(r"C:\root\Gamma.txt", "Gamma.txt")?,
            file_item(r"C:\root\Beta.txt", "Beta.txt")?,
        ];

        let inserted_indices =
            insert_file_watch_rows_sorted(&mut items, row_insertions, SortState::default())
                .ok_or_else(|| {
                    j3files::domain::ExplorerError::state_conflict("missing file name")
                })?;

        let names = items
            .iter()
            .map(|item| item.display_name.to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        let indices = inserted_indices
            .iter()
            .map(|(index, _)| *index)
            .collect::<Vec<_>>();
        assert_eq!(
            names,
            vec!["Alpha.txt", "Beta.txt", "Delta.txt", "Gamma.txt"]
        );
        assert_eq!(indices, vec![1, 3]);
        Ok(())
    }

    #[test]
    fn insert_file_watch_rows_sorted_reuses_existing_capacity() -> ExplorerResult<()> {
        let mut items = Vec::with_capacity(8);
        items.push(file_item(r"C:\root\Alpha.txt", "Alpha.txt")?);
        items.push(file_item(r"C:\root\Delta.txt", "Delta.txt")?);
        let original_capacity = items.capacity();

        let inserted_indices = insert_file_watch_rows_sorted(
            &mut items,
            vec![file_item(r"C:\root\Beta.txt", "Beta.txt")?],
            SortState::default(),
        )
        .ok_or_else(|| j3files::domain::ExplorerError::state_conflict("missing file name"))?;

        let names = items
            .iter()
            .map(|item| item.display_name.to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        let indices = inserted_indices
            .iter()
            .map(|(index, _)| *index)
            .collect::<Vec<_>>();
        assert_eq!(items.capacity(), original_capacity);
        assert_eq!(names, vec!["Alpha.txt", "Beta.txt", "Delta.txt"]);
        assert_eq!(indices, vec![1]);
        Ok(())
    }

    #[test]
    fn insert_file_watch_rows_sorted_keeps_equal_existing_first_by_size() -> ExplorerResult<()> {
        let mut alpha = file_item(r"C:\root\Alpha.txt", "Alpha.txt")?;
        alpha.size = Some(100);
        let mut delta = file_item(r"C:\root\Delta.txt", "Delta.txt")?;
        delta.size = Some(50);
        let mut omega = file_item(r"C:\root\Omega.txt", "Omega.txt")?;
        omega.size = Some(10);
        let mut beta = file_item(r"C:\root\Beta.txt", "Beta.txt")?;
        beta.size = Some(100);
        let mut gamma = file_item(r"C:\root\Gamma.txt", "Gamma.txt")?;
        gamma.size = Some(40);
        let mut items = vec![alpha, delta, omega];
        let row_insertions = vec![gamma, beta];
        let active_sort = SortState {
            key: SortKey::Size,
            direction: SortDirection::Descending,
        };

        let inserted_indices =
            insert_file_watch_rows_sorted(&mut items, row_insertions, active_sort).ok_or_else(
                || j3files::domain::ExplorerError::state_conflict("missing file name"),
            )?;

        let names = items
            .iter()
            .map(|item| item.display_name.to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        let indices = inserted_indices
            .iter()
            .map(|(index, _)| *index)
            .collect::<Vec<_>>();
        assert_eq!(
            names,
            vec![
                "Alpha.txt",
                "Beta.txt",
                "Delta.txt",
                "Gamma.txt",
                "Omega.txt"
            ]
        );
        assert_eq!(indices, vec![1, 3]);
        Ok(())
    }

    #[test]
    fn file_watch_reordered_replacement_reinserts_without_full_resort() -> ExplorerResult<()> {
        let mut alpha = file_item(r"C:\root\Alpha.txt", "Alpha.txt")?;
        alpha.size = Some(300);
        let mut beta = file_item(r"C:\root\Beta.txt", "Beta.txt")?;
        beta.size = Some(200);
        let mut gamma = file_item(r"C:\root\Gamma.txt", "Gamma.txt")?;
        gamma.size = Some(100);
        let mut beta_updated = beta.clone();
        beta_updated.size = Some(50);

        let mut items = vec![alpha, beta.clone(), gamma];
        let mut child_indices = file_watch_child_index_map(&items)
            .ok_or_else(|| j3files::domain::ExplorerError::state_conflict("missing file name"))?;
        assert!(replacement_requires_resort(
            &beta,
            &beta_updated,
            SortKey::Size
        ));
        let row_order_removals = vec![(1, beta)];

        assert!(update_file_watch_child_index_map_after_changes(
            &mut child_indices,
            &[],
            &row_order_removals,
        ));

        remove_file_watch_rows(&mut items, &row_order_removals);
        let inserted_indices = insert_file_watch_rows_sorted(
            &mut items,
            vec![beta_updated],
            SortState {
                key: SortKey::Size,
                direction: SortDirection::Descending,
            },
        )
        .ok_or_else(|| j3files::domain::ExplorerError::state_conflict("missing file name"))?;

        assert!(update_file_watch_child_index_map_after_insertions(
            &mut child_indices,
            inserted_indices,
        ));

        let names = items
            .iter()
            .map(|item| item.display_name.to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        let changed_names = vec![
            OsString::from("Alpha.txt"),
            OsString::from("Beta.txt"),
            OsString::from("Gamma.txt"),
        ];
        assert_eq!(names, vec!["Alpha.txt", "Gamma.txt", "Beta.txt"]);
        assert_eq!(
            file_watch_existing_child_indices(&child_indices, &changed_names),
            Some(vec![Some(0), Some(2), Some(1)])
        );
        Ok(())
    }

    #[test]
    fn file_watch_child_index_map_updates_after_insertions() -> ExplorerResult<()> {
        let mut items = vec![
            file_item(r"C:\root\Alpha.txt", "Alpha.txt")?,
            file_item(r"C:\root\Delta.txt", "Delta.txt")?,
        ];
        let mut child_indices = file_watch_child_index_map(&items)
            .ok_or_else(|| j3files::domain::ExplorerError::state_conflict("missing file name"))?;
        let row_insertions = vec![
            file_item(r"C:\root\Gamma.txt", "Gamma.txt")?,
            file_item(r"C:\root\Beta.txt", "Beta.txt")?,
        ];
        let inserted_indices =
            insert_file_watch_rows_sorted(&mut items, row_insertions, SortState::default())
                .ok_or_else(|| {
                    j3files::domain::ExplorerError::state_conflict("missing file name")
                })?;

        assert!(update_file_watch_child_index_map_after_insertions(
            &mut child_indices,
            inserted_indices
        ));

        let changed_names = vec![
            OsString::from("Alpha.txt"),
            OsString::from("Beta.txt"),
            OsString::from("Delta.txt"),
            OsString::from("Gamma.txt"),
        ];
        assert_eq!(
            file_watch_existing_child_indices(&child_indices, &changed_names),
            Some(vec![Some(0), Some(1), Some(2), Some(3)])
        );
        Ok(())
    }

    #[test]
    fn file_watch_child_index_map_updates_after_adjacent_insertions() -> ExplorerResult<()> {
        let mut items = vec![
            file_item(r"C:\root\Alpha.txt", "Alpha.txt")?,
            file_item(r"C:\root\Delta.txt", "Delta.txt")?,
        ];
        let mut child_indices = file_watch_child_index_map(&items)
            .ok_or_else(|| j3files::domain::ExplorerError::state_conflict("missing file name"))?;
        let row_insertions = vec![
            file_item(r"C:\root\Gamma.txt", "Gamma.txt")?,
            file_item(r"C:\root\Beta.txt", "Beta.txt")?,
            file_item(r"C:\root\Charlie.txt", "Charlie.txt")?,
        ];
        let inserted_indices =
            insert_file_watch_rows_sorted(&mut items, row_insertions, SortState::default())
                .ok_or_else(|| {
                    j3files::domain::ExplorerError::state_conflict("missing file name")
                })?;

        assert!(update_file_watch_child_index_map_after_insertions(
            &mut child_indices,
            inserted_indices
        ));

        let changed_names = vec![
            OsString::from("Alpha.txt"),
            OsString::from("Beta.txt"),
            OsString::from("Charlie.txt"),
            OsString::from("Delta.txt"),
            OsString::from("Gamma.txt"),
        ];
        assert_eq!(
            file_watch_existing_child_indices(&child_indices, &changed_names),
            Some(vec![Some(0), Some(1), Some(2), Some(3), Some(4)])
        );
        Ok(())
    }

    #[test]
    fn remove_file_watch_rows_removes_multiple_rows_without_reordering() -> ExplorerResult<()> {
        let mut items = vec![
            file_item(r"C:\root\Alpha.txt", "Alpha.txt")?,
            file_item(r"C:\root\Beta.txt", "Beta.txt")?,
            file_item(r"C:\root\Gamma.txt", "Gamma.txt")?,
            file_item(r"C:\root\Delta.txt", "Delta.txt")?,
            file_item(r"C:\root\Epsilon.txt", "Epsilon.txt")?,
            file_item(r"C:\root\Zeta.txt", "Zeta.txt")?,
        ];
        let row_removals = vec![
            (1, items[1].clone()),
            (3, items[3].clone()),
            (4, items[4].clone()),
        ];

        remove_file_watch_rows(&mut items, &row_removals);

        let names = items
            .iter()
            .map(|item| item.display_name.to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        assert_eq!(names, vec!["Alpha.txt", "Gamma.txt", "Zeta.txt"]);
        Ok(())
    }
}

#[cfg(test)]
mod folder_tree_refresh_tests {
    use std::collections::VecDeque;
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc,
    };
    use std::thread;

    use super::*;

    fn location(path: &str) -> ExplorerResult<NavigationLocation> {
        NavigationLocation::from_path(PathBuf::from(path))
    }

    fn folder_tree_node(
        children_loading_generation: Option<u64>,
    ) -> ExplorerResult<FolderTreeNodeState> {
        let location = location(r"C:\root")?;
        let prepared_location_path = location.prepared_path();
        Ok(FolderTreeNodeState {
            handle: None,
            parent: None,
            kind: FolderTreeItemKind::Bookmark,
            location,
            prepared_location_path,
            children_loaded: false,
            children_loading_generation,
        })
    }

    fn child_request(
        parent_index: usize,
        generation: u64,
    ) -> ExplorerResult<FolderTreeChildrenRequest> {
        Ok(FolderTreeChildrenRequest {
            generation,
            parent_index,
            location: location(r"C:\root")?,
            display_options: DisplayOptions::default(),
            kind: FolderTreeChildrenRequestKind::LoadChildren,
            selection_sync: false,
        })
    }

    fn cancellable_child_worker(
        request: FolderTreeChildrenRequest,
        release: mpsc::Receiver<()>,
    ) -> ActiveFolderTreeChildWorker {
        let cancel_requested = Arc::new(AtomicBool::new(false));
        let handle = thread::spawn(move || {
            let _ = release.recv();
        });
        ActiveFolderTreeChildWorker {
            request,
            cancel_requested,
            completion_message_abandoned: Arc::new(AtomicBool::new(false)),
            io_cancellation: Arc::new(platform::SynchronousIoCancellation::new()),
            handle,
        }
    }

    fn pending_child_worker(request: FolderTreeChildrenRequest) -> PendingFolderTreeChildWorker {
        PendingFolderTreeChildWorker {
            request,
            loading_generation_on_spawn_error: None,
        }
    }

    #[test]
    fn tree_refresh_locations_are_deduplicated_by_path() -> ExplorerResult<()> {
        let locations = vec![
            location(r"C:\root")?,
            location(r"c:\root\")?,
            location(r"D:\other")?,
        ];

        assert_eq!(
            unique_navigation_location_paths_by_path(&locations),
            vec![
                location(r"C:\root")?.prepared_path(),
                location(r"D:\other")?.prepared_path()
            ]
        );
        Ok(())
    }

    #[test]
    fn taking_folder_tree_child_indices_clears_only_requested_parent() {
        let mut child_indices_by_parent = vec![Vec::new(), vec![2, 3], vec![4], Vec::new()];

        assert_eq!(
            take_folder_tree_child_indices(&mut child_indices_by_parent, 1),
            vec![2, 3]
        );

        assert!(child_indices_by_parent[1].is_empty());
        assert_eq!(child_indices_by_parent[2], vec![4]);
    }

    #[test]
    fn taking_folder_tree_child_indices_returns_empty_for_missing_parent() {
        let mut child_indices_by_parent = vec![vec![1]];

        assert!(take_folder_tree_child_indices(&mut child_indices_by_parent, 4).is_empty());

        assert_eq!(child_indices_by_parent, vec![vec![1]]);
    }

    #[test]
    fn reusable_folder_tree_node_indices_pop_in_lowest_index_order() -> ExplorerResult<()> {
        let nodes = vec![
            folder_tree_node(None)?,
            folder_tree_node(None)?,
            folder_tree_node(None)?,
        ];
        let mut reusable_indices = reusable_folder_tree_node_indices(&nodes);

        assert_eq!(reusable_indices.pop(), Some(0));
        assert_eq!(reusable_indices.pop(), Some(1));
        assert_eq!(reusable_indices.pop(), Some(2));
        assert_eq!(reusable_indices.pop(), None);
        Ok(())
    }

    #[test]
    fn cancelling_child_worker_keeps_loading_generation_until_reaped() -> ExplorerResult<()> {
        let (release_tx, release_rx) = mpsc::channel();
        let mut workers = vec![cancellable_child_worker(child_request(0, 10)?, release_rx)];
        let mut pending_workers = VecDeque::new();
        let mut nodes = vec![folder_tree_node(Some(10))?];

        let pending_generation =
            request_cancel_for_folder_tree_child_workers(&workers, &mut pending_workers, 0);

        assert_eq!(pending_generation, Some(10));
        assert!(workers[0].cancel_requested.load(Ordering::Relaxed));

        let _ = release_tx.send(());
        while !workers[0].is_finished() {
            thread::yield_now();
        }
        let worker = workers.swap_remove(0);
        clear_finished_recoverable_folder_tree_child_loading(&mut nodes, &workers, &worker);
        MainWindow::join_folder_tree_child_worker(worker);

        assert_eq!(nodes[0].children_loading_generation, None);
        Ok(())
    }

    #[test]
    fn queued_child_worker_keeps_latest_request_for_parent() -> ExplorerResult<()> {
        let mut pending_workers = VecDeque::new();

        enqueue_pending_folder_tree_child_worker(
            &mut pending_workers,
            pending_child_worker(child_request(0, 10)?),
        );
        enqueue_pending_folder_tree_child_worker(
            &mut pending_workers,
            pending_child_worker(child_request(0, 11)?),
        );

        assert_eq!(pending_workers.len(), 1);
        assert_eq!(
            pending_workers
                .front()
                .map(|worker| worker.request.generation),
            Some(11)
        );
        Ok(())
    }

    #[test]
    fn cancelling_parent_removes_queued_child_worker() -> ExplorerResult<()> {
        let workers = Vec::new();
        let mut pending_workers = VecDeque::new();
        enqueue_pending_folder_tree_child_worker(
            &mut pending_workers,
            pending_child_worker(child_request(0, 10)?),
        );
        enqueue_pending_folder_tree_child_worker(
            &mut pending_workers,
            pending_child_worker(child_request(1, 20)?),
        );

        let pending_generation =
            request_cancel_for_folder_tree_child_workers(&workers, &mut pending_workers, 0);

        assert_eq!(pending_generation, None);
        assert_eq!(pending_workers.len(), 1);
        assert_eq!(
            pending_workers
                .front()
                .map(|worker| worker.request.parent_index),
            Some(1)
        );
        Ok(())
    }

    #[test]
    fn spawn_error_fallback_requires_running_cancelled_child_worker() -> ExplorerResult<()> {
        let workers = Vec::new();
        assert_eq!(
            recoverable_folder_tree_child_loading_generation_on_spawn_error(&workers, 0, Some(10)),
            None
        );

        let (release_tx, release_rx) = mpsc::channel();
        let mut workers = vec![cancellable_child_worker(child_request(0, 10)?, release_rx)];
        workers[0].request_cancel();

        assert_eq!(
            recoverable_folder_tree_child_loading_generation_on_spawn_error(&workers, 0, Some(10)),
            Some(10)
        );

        let _ = release_tx.send(());
        while !workers[0].is_finished() {
            thread::yield_now();
        }
        let worker = workers.swap_remove(0);
        MainWindow::join_folder_tree_child_worker(worker);
        Ok(())
    }

    #[test]
    fn abandoned_child_completion_clears_loading_generation_when_reaped() -> ExplorerResult<()> {
        let (release_tx, release_rx) = mpsc::channel();
        let mut workers = vec![cancellable_child_worker(child_request(0, 12)?, release_rx)];
        let mut nodes = vec![folder_tree_node(Some(12))?];
        workers[0]
            .completion_message_abandoned
            .store(true, Ordering::Relaxed);

        let _ = release_tx.send(());
        while !workers[0].is_finished() {
            thread::yield_now();
        }
        let worker = workers.swap_remove(0);
        clear_finished_recoverable_folder_tree_child_loading(&mut nodes, &workers, &worker);
        MainWindow::join_folder_tree_child_worker(worker);

        assert_eq!(nodes[0].children_loading_generation, None);
        Ok(())
    }
}

#[cfg(test)]
mod drop_default_tests {
    use j3files::domain::DropOperation;

    use super::{external_drop_default_operation, platform};

    #[test]
    fn external_drop_default_uses_preferred_effect_when_allowed() {
        let effects = platform::OleDropEffects {
            copy: true,
            move_: true,
        };

        assert_eq!(
            external_drop_default_operation(effects, Some(platform::OleDropPreferredEffect::Move)),
            Some(DropOperation::Move)
        );
    }

    #[test]
    fn external_drop_default_falls_back_to_single_allowed_effect() {
        assert_eq!(
            external_drop_default_operation(
                platform::OleDropEffects {
                    copy: false,
                    move_: true,
                },
                None,
            ),
            Some(DropOperation::Move)
        );
    }

    #[test]
    fn external_drop_default_leaves_ambiguous_copy_move_to_domain_fallback() {
        assert_eq!(
            external_drop_default_operation(
                platform::OleDropEffects {
                    copy: true,
                    move_: true,
                },
                None,
            ),
            None
        );
    }
}

#[cfg(test)]
mod error_dialog_policy_tests {
    use j3files::domain::ExplorerError;

    use super::should_show_user_error_dialog;

    #[test]
    fn cancelled_file_operations_do_not_show_user_error_dialog() {
        let error = ExplorerError::Cancelled {
            operation: "file operation",
        };

        assert!(!should_show_user_error_dialog(&error));
    }

    #[test]
    fn non_cancelled_failures_still_show_user_error_dialog() {
        let error = ExplorerError::invalid_input("파일 작업을 완료할 수 없습니다.");

        assert!(should_show_user_error_dialog(&error));
    }
}

#[cfg(test)]
mod stale_list_item_recovery_tests {
    use std::ffi::OsString;
    use std::path::{Path, PathBuf};

    use j3files::domain::{
        ExplorerError, ExplorerResult, FileAttributes, FileItem, FileItemKind, NavigationLocation,
        ShellOperation,
    };

    use super::{
        missing_list_item_locations_from_error, remove_file_items_by_location,
        remove_navigation_locations_by_path,
    };

    fn location(path: &str) -> ExplorerResult<NavigationLocation> {
        NavigationLocation::from_path(PathBuf::from(path))
    }

    fn file_item(path: &str) -> ExplorerResult<FileItem> {
        let location = location(path)?;
        Ok(FileItem {
            display_name: location.display_name(),
            location,
            kind: FileItemKind::File,
            type_name: OsString::from("Text Document"),
            size: Some(1),
            updated_at: None,
            attributes: FileAttributes::default(),
        })
    }

    fn shell_open_not_found(path: &str) -> ExplorerError {
        ExplorerError::shell_operation_failed_with_context(
            ShellOperation::Open,
            "ShellExecuteExW",
            Some(2),
            None,
            vec![PathBuf::from(path)],
            false,
            false,
        )
    }

    #[test]
    fn not_found_error_marks_matching_missing_list_item() -> ExplorerResult<()> {
        let error = shell_open_not_found(r"C:\root\missing.txt");
        let items = vec![
            file_item(r"C:\root\missing.txt")?,
            file_item(r"C:\root\present.txt")?,
        ];

        let missing = missing_list_item_locations_from_error(&error, &items, |path| {
            path == Path::new(r"C:\root\missing.txt")
        });

        assert_eq!(missing, vec![location(r"C:\root\missing.txt")?]);
        Ok(())
    }

    #[test]
    fn not_found_error_keeps_reported_item_when_disk_check_does_not_confirm_missing(
    ) -> ExplorerResult<()> {
        let error = shell_open_not_found(r"C:\root\present.txt");
        let items = vec![file_item(r"C:\root\present.txt")?];

        let missing = missing_list_item_locations_from_error(&error, &items, |_| false);

        assert!(missing.is_empty());
        Ok(())
    }

    #[test]
    fn item_removal_uses_windows_path_comparison() -> ExplorerResult<()> {
        let mut items = vec![
            file_item(r"C:\Root\Missing.txt")?,
            file_item(r"C:\root\present.txt")?,
        ];
        let mut selected = vec![
            location(r"C:\ROOT\MISSING.txt")?,
            location(r"C:\root\present.txt")?,
        ];
        let missing = vec![location(r"c:\root\missing.txt")?];

        assert!(remove_file_items_by_location(&mut items, &missing));
        assert!(remove_navigation_locations_by_path(&mut selected, &missing));

        assert_eq!(
            items
                .iter()
                .map(|item| item.location.clone())
                .collect::<Vec<_>>(),
            vec![location(r"C:\root\present.txt")?]
        );
        assert_eq!(selected, vec![location(r"C:\root\present.txt")?]);
        Ok(())
    }
}

fn expected_transfer_locations(
    destination: &NavigationLocation,
    sources: &[NavigationLocation],
) -> ExplorerResult<Option<Vec<ExpectedTransferLocation>>> {
    let Some(locations) = expected_transfer_target_locations(destination, sources)? else {
        return Ok(None);
    };

    Ok(Some(expected_transfer_locations_with_existence(locations)))
}

fn expected_transfer_locations_for_completion(
    operation: DropOperation,
    destination: &NavigationLocation,
    sources: &[NavigationLocation],
) -> ExplorerResult<Option<Vec<ExpectedTransferLocation>>> {
    if !exact_transfer_completion_existence_checks_exceed_limit(operation, sources.len()) {
        return expected_transfer_locations(destination, sources);
    }

    // Preserve target path validation while skipping existence probes for large transfers.
    let _ = expected_transfer_target_locations(destination, sources)?;
    Ok(None)
}

fn expected_transfer_target_locations(
    destination: &NavigationLocation,
    sources: &[NavigationLocation],
) -> ExplorerResult<Option<Vec<NavigationLocation>>> {
    let mut locations: Vec<NavigationLocation> = Vec::with_capacity(sources.len());
    let mut seen_target_indices_by_key: HashMap<Vec<u16>, Vec<usize>> =
        HashMap::with_capacity(sources.len());

    for source in sources {
        let Some(file_name) = source.as_path().file_name() else {
            return Ok(None);
        };
        let mut target_path = destination.as_path().to_path_buf();
        target_path.push(file_name);
        let location = NavigationLocation::from_path(target_path)?;
        let key = expected_transfer_location_key(&location);
        let same_key_indices = seen_target_indices_by_key.entry(key).or_default();
        if same_key_indices
            .iter()
            .any(|index| locations[*index].has_same_path(location.as_path()))
        {
            return Ok(None);
        }
        same_key_indices.push(locations.len());
        locations.push(location);
    }

    Ok(Some(locations))
}

fn expected_transfer_locations_with_existence(
    locations: Vec<NavigationLocation>,
) -> Vec<ExpectedTransferLocation> {
    locations
        .into_iter()
        .map(|location| {
            let existed_before = TransferTargetExistence::from_path(location.as_path());
            ExpectedTransferLocation {
                location,
                existed_before,
            }
        })
        .collect()
}

fn exact_transfer_completion_existence_checks_exceed_limit(
    operation: DropOperation,
    item_count: usize,
) -> bool {
    // Exact completion only enables undo and completed-item selection; bound metadata probes so
    // large shell transfers do not keep the file-operation worker busy on slow storage.
    let checks_per_item = match operation {
        DropOperation::Copy => 2,
        DropOperation::Move => 3,
    };

    match item_count.checked_mul(checks_per_item) {
        Some(check_count) => check_count > MAX_EXACT_TRANSFER_COMPLETION_EXISTENCE_CHECKS,
        None => true,
    }
}

fn expected_transfer_location_key(location: &NavigationLocation) -> Vec<u16> {
    file_watch_child_name_key(location.as_path().as_os_str())
}

fn completed_transfer_selection(
    completed_transfer: CompletedTransfer<'_>,
) -> Vec<NavigationLocation> {
    let Some(expected) = completed_transfer.expected() else {
        return Vec::new();
    };

    expected
        .iter()
        .map(|expected| expected.location.clone())
        .collect()
}

fn completed_transfer_after_operation<'a>(
    operation: DropOperation,
    sources: &[NavigationLocation],
    expected: Option<&'a [ExpectedTransferLocation]>,
) -> CompletedTransfer<'a> {
    let Some(expected) = expected else {
        return CompletedTransfer::Incomplete;
    };
    if exact_transfer_completion_existence_checks_exceed_limit(operation, expected.len()) {
        return CompletedTransfer::Incomplete;
    }
    if !transfer_completed_exactly_as_expected(operation, sources, expected) {
        return CompletedTransfer::Incomplete;
    }

    match operation {
        DropOperation::Copy => CompletedTransfer::Copy { expected },
        DropOperation::Move => CompletedTransfer::Move { expected },
    }
}

fn transfer_completed_exactly_as_expected(
    operation: DropOperation,
    sources: &[NavigationLocation],
    expected: &[ExpectedTransferLocation],
) -> bool {
    match operation {
        DropOperation::Copy => transfer_created_expected_targets(expected),
        DropOperation::Move => transfer_moved_expected_targets(sources, expected),
    }
}

fn transfer_created_expected_targets(expected: &[ExpectedTransferLocation]) -> bool {
    expected.iter().all(|expected| {
        expected.existed_before == TransferTargetExistence::Missing
            && TransferTargetExistence::from_path(expected.location.as_path())
                == TransferTargetExistence::Exists
    })
}

fn transfer_moved_expected_targets(
    sources: &[NavigationLocation],
    expected: &[ExpectedTransferLocation],
) -> bool {
    sources.len() == expected.len()
        && transfer_created_expected_targets(expected)
        && sources.iter().all(|source| {
            TransferTargetExistence::from_path(source.as_path()) == TransferTargetExistence::Missing
        })
}

fn undo_operation_for_completed_transfer(
    sources: &[NavigationLocation],
    completed_transfer: CompletedTransfer<'_>,
) -> Option<UndoFileOperation> {
    match completed_transfer {
        CompletedTransfer::Copy { expected } => Some(UndoFileOperation::Copy {
            copied: expected
                .iter()
                .map(|expected| expected.location.clone())
                .collect(),
        }),
        CompletedTransfer::Move { expected } => {
            let mut moved = Vec::with_capacity(sources.len());
            for (source, current) in sources.iter().zip(expected.iter()) {
                let original_parent = source.parent().ok()??;
                moved.push((current.location.clone(), original_parent));
            }
            Some(UndoFileOperation::Move { moved })
        }
        CompletedTransfer::Incomplete => None,
    }
}

fn renamed_sibling_location(
    location: &NavigationLocation,
    new_name: &OsStr,
) -> ExplorerResult<Option<NavigationLocation>> {
    let Some(parent) = location.parent()? else {
        return Ok(None);
    };

    let mut renamed_path = parent.as_path().to_path_buf();
    renamed_path.push(Path::new(new_name));
    NavigationLocation::from_path(renamed_path).map(Some)
}

fn rename_refresh_locations(
    location: &NavigationLocation,
) -> ExplorerResult<Vec<NavigationLocation>> {
    match location.parent()? {
        Some(parent) => Ok(vec![parent]),
        None => Ok(Vec::new()),
    }
}

fn selected_list_item_index(
    items: &[FileItem],
    selected_items: &[NavigationLocation],
) -> Option<usize> {
    if selected_items.is_empty() {
        return None;
    }

    const HASH_LOOKUP_SELECTION_THRESHOLD: usize = 8;
    if selected_items.len() <= HASH_LOOKUP_SELECTION_THRESHOLD {
        return items.iter().position(|item| {
            selected_items
                .iter()
                .any(|selected| selected == &item.location)
        });
    }

    let mut selected_locations = HashSet::with_capacity(selected_items.len());
    selected_locations.extend(selected_items.iter());
    items
        .iter()
        .position(|item| selected_locations.contains(&item.location))
}

fn current_search_rows_match(
    current: Option<&CurrentSearchRows>,
    current_item_count: usize,
    expected: &CurrentSearchRows,
) -> bool {
    current == Some(expected) && current_item_count == expected.item_count
}

#[cfg(test)]
mod current_search_rows_tests {
    use super::{
        current_search_rows_match, CurrentSearchRows, CurrentSearchRowsKind, DisplayOptions, TabId,
    };

    fn rows(item_count: usize, show_hidden: bool) -> CurrentSearchRows {
        CurrentSearchRows {
            tab_id: TabId(1),
            kind: CurrentSearchRowsKind::Results,
            item_count,
            display_options: DisplayOptions {
                show_hidden,
                show_system: false,
            },
        }
    }

    #[test]
    fn search_rows_match_when_marker_and_item_count_are_current() {
        let current = rows(3, false);

        assert!(current_search_rows_match(Some(&current), 3, &current));
    }

    #[test]
    fn search_rows_do_not_match_stale_count_or_display_options() {
        let current = rows(3, false);

        assert!(!current_search_rows_match(Some(&current), 2, &current));
        assert!(!current_search_rows_match(
            Some(&current),
            3,
            &rows(3, true)
        ));
    }
}

fn missing_list_item_locations_from_error(
    error: &ExplorerError,
    items: &[FileItem],
    path_is_missing: impl Fn(&Path) -> bool,
) -> Vec<NavigationLocation> {
    let target_paths = error.not_found_target_paths();
    if target_paths.is_empty() {
        return Vec::new();
    }

    let mut missing = Vec::new();
    for item in items {
        let is_reported_target = target_paths
            .iter()
            .any(|target_path| item.location.has_same_path(target_path));
        if is_reported_target
            && path_is_missing(item.location.as_path())
            && !location_matches_any(&item.location, &missing)
        {
            missing.push(item.location.clone());
        }
    }
    missing
}

fn path_is_missing(path: &Path) -> bool {
    matches!(path.try_exists(), Ok(false))
}

fn remove_file_items_by_location(
    items: &mut Vec<FileItem>,
    locations: &[NavigationLocation],
) -> bool {
    let original_len = items.len();
    items.retain(|item| !location_matches_any(&item.location, locations));
    items.len() != original_len
}

fn remove_navigation_locations_by_path(
    items: &mut Vec<NavigationLocation>,
    locations: &[NavigationLocation],
) -> bool {
    let original_len = items.len();
    items.retain(|item| !location_matches_any(item, locations));
    items.len() != original_len
}

fn location_matches_any(location: &NavigationLocation, locations: &[NavigationLocation]) -> bool {
    locations
        .iter()
        .any(|target| location.has_same_path(target.as_path()))
}

fn listing_request_matches_source_and_sort(
    current: &ListingRequest,
    requested: &ListingRequest,
) -> bool {
    current.has_same_listing_source_as(requested) && current.sort == requested.sort
}

fn list_column_sort_key(column_index: usize) -> Option<SortKey> {
    match column_index {
        LIST_NAME_COLUMN_INDEX => Some(SortKey::Name),
        LIST_SIZE_COLUMN_INDEX => Some(SortKey::Size),
        LIST_UPDATED_COLUMN_INDEX => Some(SortKey::UpdatedAt),
        LIST_TYPE_COLUMN_INDEX => Some(SortKey::Kind),
        _ => None,
    }
}

fn list_column_click_sort_state(current: SortState, key: SortKey) -> SortState {
    let direction = if current.key == key {
        toggled_sort_direction(current.direction)
    } else {
        SortDirection::Ascending
    };
    SortState { key, direction }
}

fn list_view_notification_activates_item(notification_code: u32) -> bool {
    notification_code == ui::LIST_VIEW_ITEM_ACTIVATE
}

fn toggled_sort_direction(direction: SortDirection) -> SortDirection {
    match direction {
        SortDirection::Ascending => SortDirection::Descending,
        SortDirection::Descending => SortDirection::Ascending,
    }
}

#[cfg(test)]
mod list_column_sort_tests {
    use super::{
        list_column_click_sort_state, list_column_sort_key, SortDirection, SortKey, SortState,
        LIST_NAME_COLUMN_INDEX, LIST_SIZE_COLUMN_INDEX, LIST_TYPE_COLUMN_INDEX,
        LIST_UPDATED_COLUMN_INDEX,
    };

    #[test]
    fn list_column_indices_follow_visible_order() {
        assert_eq!(
            [
                LIST_NAME_COLUMN_INDEX,
                LIST_TYPE_COLUMN_INDEX,
                LIST_SIZE_COLUMN_INDEX,
                LIST_UPDATED_COLUMN_INDEX
            ],
            [0, 1, 2, 3]
        );
    }

    #[test]
    fn list_columns_map_to_sort_keys() {
        assert_eq!(
            list_column_sort_key(LIST_NAME_COLUMN_INDEX),
            Some(SortKey::Name)
        );
        assert_eq!(
            list_column_sort_key(LIST_SIZE_COLUMN_INDEX),
            Some(SortKey::Size)
        );
        assert_eq!(
            list_column_sort_key(LIST_UPDATED_COLUMN_INDEX),
            Some(SortKey::UpdatedAt)
        );
        assert_eq!(
            list_column_sort_key(LIST_TYPE_COLUMN_INDEX),
            Some(SortKey::Kind)
        );
        assert_eq!(list_column_sort_key(4), None);
    }

    #[test]
    fn clicking_current_column_toggles_direction() {
        let current = SortState {
            key: SortKey::Name,
            direction: SortDirection::Ascending,
        };

        assert_eq!(
            list_column_click_sort_state(current, SortKey::Name),
            SortState {
                key: SortKey::Name,
                direction: SortDirection::Descending,
            }
        );
    }

    #[test]
    fn clicking_new_column_starts_ascending() {
        let current = SortState {
            key: SortKey::Name,
            direction: SortDirection::Descending,
        };

        assert_eq!(
            list_column_click_sort_state(current, SortKey::Size),
            SortState {
                key: SortKey::Size,
                direction: SortDirection::Ascending,
            }
        );
    }
}

#[cfg(test)]
mod list_view_activation_tests {
    use super::{list_view_notification_activates_item, ui};

    #[test]
    fn item_activate_notification_opens_list_item() {
        assert!(list_view_notification_activates_item(
            ui::LIST_VIEW_ITEM_ACTIVATE
        ));
    }

    #[test]
    fn double_click_notification_is_not_a_second_open_signal() {
        assert!(!list_view_notification_activates_item(
            ui::NOTIFICATION_DBL_CLICK
        ));
    }
}

#[cfg(test)]
mod tab_label_tests {
    use std::path::PathBuf;

    use super::{tab_display_label, ExplorerResult, NavigationLocation, TabId, TabState};

    #[test]
    fn tab_display_label_uses_location_name_without_position_prefix() -> ExplorerResult<()> {
        let tab = TabState::new(
            TabId(3),
            NavigationLocation::from_path(PathBuf::from(r"C:\Work"))?,
        );

        assert_eq!(tab_display_label(&tab), "Work");

        Ok(())
    }
}

fn display_os(value: &OsStr) -> String {
    value.to_string_lossy().into_owned()
}

fn tab_display_label(tab: &TabState) -> String {
    display_os(tab.current_location().display_name().as_os_str())
}

fn appearance_font_menu_label(font: &AppearanceFont) -> String {
    match font.family_name() {
        Some(family_name) => format!(
            "Font... ({} {}pt)",
            display_os(family_name),
            font.point_size()
        ),
        None if font.is_custom() => format!("Font... (Default {}pt)", font.point_size()),
        None => "Font... (System Default)".to_string(),
    }
}

fn is_drive_menu_id(id: u16) -> bool {
    id >= ID_DRIVE_BASE && usize::from(id - ID_DRIVE_BASE) < MAX_DRIVE_MENU_ITEMS
}

fn is_bookmark_menu_id(id: u16) -> bool {
    id >= ID_BOOKMARK_BASE && usize::from(id - ID_BOOKMARK_BASE) < MAX_BOOKMARK_MENU_ITEMS
}

fn command_for_appearance_theme(theme: AppearanceTheme) -> u16 {
    match theme {
        AppearanceTheme::Light => ID_THEME_LIGHT,
        AppearanceTheme::ClassicDark => ID_THEME_CLASSIC_DARK,
        AppearanceTheme::SepiaTeal => ID_THEME_SEPIA_TEAL,
        AppearanceTheme::Graphite => ID_THEME_GRAPHITE,
        AppearanceTheme::Forest => ID_THEME_FOREST,
        AppearanceTheme::SteelBlue => ID_THEME_STEEL_BLUE,
    }
}

fn appearance_theme_for_command(id: u16) -> Option<AppearanceTheme> {
    match id {
        ID_THEME_LIGHT => Some(AppearanceTheme::Light),
        ID_THEME_CLASSIC_DARK => Some(AppearanceTheme::ClassicDark),
        ID_THEME_SEPIA_TEAL => Some(AppearanceTheme::SepiaTeal),
        ID_THEME_GRAPHITE => Some(AppearanceTheme::Graphite),
        ID_THEME_FOREST => Some(AppearanceTheme::Forest),
        ID_THEME_STEEL_BLUE => Some(AppearanceTheme::SteelBlue),
        _ => None,
    }
}

fn format_size(size: Option<u64>) -> String {
    size.map(|value| value.to_string()).unwrap_or_default()
}

fn format_updated_time(value: Option<SystemTime>) -> String {
    let Some(value) = value else {
        return String::new();
    };

    let Ok(duration) = value.duration_since(UNIX_EPOCH) else {
        return "before 1970".to_string();
    };

    let total_seconds = duration.as_secs();
    let days = (total_seconds / 86_400) as i64;
    let seconds_of_day = total_seconds % 86_400;
    let (year, month, day) = civil_from_days(days);
    let hour = seconds_of_day / 3_600;
    let minute = (seconds_of_day % 3_600) / 60;

    format!("{year:04}-{month:02}-{day:02} {hour:02}:{minute:02} UTC")
}

fn civil_from_days(days: i64) -> (i32, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = mp + if mp < 10 { 3 } else { -9 };
    let year = y + if month <= 2 { 1 } else { 0 };

    (year as i32, month as u32, day as u32)
}

#[cfg(test)]
mod main_window_create_ownership_tests {
    use super::MainWindowCreateOwnership;

    #[test]
    fn caller_observes_window_proc_ownership_after_nccreate_attach() {
        let caller_ownership = MainWindowCreateOwnership::new();
        let window_ownership = caller_ownership.clone();

        assert!(!caller_ownership.is_window_proc_owner());

        window_ownership.mark_window_proc_owner();

        assert!(caller_ownership.is_window_proc_owner());
    }
}

#[cfg(test)]
mod transfer_undo_tests {
    use std::fs;
    use std::io;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    use j3files::domain::{DropOperation, ExplorerError, ExplorerResult, NavigationLocation};

    use super::{
        completed_transfer_after_operation, completed_transfer_selection,
        exact_transfer_completion_existence_checks_exceed_limit, expected_transfer_locations,
        expected_transfer_locations_for_completion, undo_operation_for_completed_transfer,
        ExpectedTransferLocation, TransferTargetExistence, UndoFileOperation,
        MAX_EXACT_TRANSFER_COMPLETION_EXISTENCE_CHECKS,
    };

    struct TestDir {
        path: PathBuf,
    }

    static NEXT_TEST_DIR_ID: AtomicU64 = AtomicU64::new(0);

    impl TestDir {
        fn new() -> ExplorerResult<Self> {
            let timestamp = match SystemTime::now().duration_since(UNIX_EPOCH) {
                Ok(duration) => duration.as_nanos(),
                Err(_) => 0,
            };
            let id = NEXT_TEST_DIR_ID.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "j3files-transfer-undo-{}-{timestamp}-{id}",
                std::process::id()
            ));

            fs::create_dir_all(&path).map_err(|source| {
                ExplorerError::io(
                    "create transfer undo test directory",
                    Some(path.clone()),
                    source,
                )
            })?;

            Ok(Self { path })
        }

        fn child(&self, file_name: &str) -> PathBuf {
            self.path.join(file_name)
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    fn write_test_file(path: &Path) -> ExplorerResult<()> {
        fs::write(path, b"transfer target").map_err(|source| {
            ExplorerError::io(
                "write transfer undo test file",
                Some(path.to_path_buf()),
                source,
            )
        })
    }

    fn transfer_sources(count: usize) -> ExplorerResult<Vec<NavigationLocation>> {
        (0..count)
            .map(|index| {
                NavigationLocation::from_path(PathBuf::from(format!(r"C:\from\file-{index}.txt")))
            })
            .collect()
    }

    fn completed_transfer<'a>(
        operation: DropOperation,
        sources: &[NavigationLocation],
        expected: &'a [ExpectedTransferLocation],
    ) -> super::CompletedTransfer<'a> {
        completed_transfer_after_operation(operation, sources, Some(expected))
    }

    #[test]
    fn expected_transfer_locations_reject_case_variant_targets() -> ExplorerResult<()> {
        let destination = NavigationLocation::from_path(PathBuf::from(r"C:\drop"))?;
        let sources = vec![
            NavigationLocation::from_path(PathBuf::from(r"C:\from\Report.txt"))?,
            NavigationLocation::from_path(PathBuf::from(r"D:\other\report.TXT"))?,
        ];

        assert!(expected_transfer_locations(&destination, &sources)?.is_none());

        Ok(())
    }

    #[test]
    fn transfer_target_existence_preserves_try_exists_errors() {
        assert_eq!(
            TransferTargetExistence::from_try_exists(Ok(true)),
            TransferTargetExistence::Exists
        );
        assert_eq!(
            TransferTargetExistence::from_try_exists(Ok(false)),
            TransferTargetExistence::Missing
        );
        assert_eq!(
            TransferTargetExistence::from_try_exists(Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "denied"
            ))),
            TransferTargetExistence::Unknown
        );
    }

    #[test]
    fn exact_transfer_completion_limit_counts_pre_and_post_existence_checks() {
        assert!(!exact_transfer_completion_existence_checks_exceed_limit(
            DropOperation::Copy,
            MAX_EXACT_TRANSFER_COMPLETION_EXISTENCE_CHECKS / 2
        ));
        assert!(exact_transfer_completion_existence_checks_exceed_limit(
            DropOperation::Copy,
            (MAX_EXACT_TRANSFER_COMPLETION_EXISTENCE_CHECKS / 2) + 1
        ));
        assert!(!exact_transfer_completion_existence_checks_exceed_limit(
            DropOperation::Move,
            MAX_EXACT_TRANSFER_COMPLETION_EXISTENCE_CHECKS / 3
        ));
        assert!(exact_transfer_completion_existence_checks_exceed_limit(
            DropOperation::Move,
            (MAX_EXACT_TRANSFER_COMPLETION_EXISTENCE_CHECKS / 3) + 1
        ));
    }

    #[test]
    fn large_transfer_skips_exact_completion_tracking() -> ExplorerResult<()> {
        let destination = NavigationLocation::from_path(PathBuf::from(r"C:\drop"))?;
        let source_count = (MAX_EXACT_TRANSFER_COMPLETION_EXISTENCE_CHECKS / 2) + 1;
        let sources = transfer_sources(source_count)?;

        assert!(expected_transfer_locations_for_completion(
            DropOperation::Copy,
            &destination,
            &sources
        )?
        .is_none());

        Ok(())
    }

    #[test]
    fn copy_undo_is_created_for_known_new_transfer_target() -> ExplorerResult<()> {
        let test_dir = TestDir::new()?;
        let target_path = test_dir.child("copied.txt");
        let target = NavigationLocation::from_path(target_path.clone())?;
        let expected = vec![ExpectedTransferLocation {
            location: target.clone(),
            existed_before: TransferTargetExistence::Missing,
        }];

        write_test_file(&target_path)?;

        let completed = completed_transfer(DropOperation::Copy, &[], expected.as_slice());

        match undo_operation_for_completed_transfer(&[], completed) {
            Some(UndoFileOperation::Copy { copied }) => assert_eq!(copied, vec![target]),
            other => panic!("expected copy undo operation, got {other:?}"),
        }

        Ok(())
    }

    #[test]
    fn copy_selection_and_undo_reuse_completed_transfer_check() -> ExplorerResult<()> {
        let test_dir = TestDir::new()?;
        let target_path = test_dir.child("reused-check.txt");
        let target = NavigationLocation::from_path(target_path.clone())?;
        let expected = vec![ExpectedTransferLocation {
            location: target.clone(),
            existed_before: TransferTargetExistence::Missing,
        }];

        write_test_file(&target_path)?;
        let completed = completed_transfer(DropOperation::Copy, &[], expected.as_slice());
        fs::remove_file(&target_path).map_err(|source| {
            ExplorerError::io(
                "remove transfer target after completion check",
                Some(target_path.clone()),
                source,
            )
        })?;

        assert_eq!(
            completed_transfer_selection(completed),
            vec![target.clone()]
        );
        match undo_operation_for_completed_transfer(&[], completed) {
            Some(UndoFileOperation::Copy { copied }) => assert_eq!(copied, vec![target]),
            other => panic!("expected copy undo operation, got {other:?}"),
        }

        Ok(())
    }

    #[test]
    fn copy_undo_is_not_created_for_preexisting_transfer_target() -> ExplorerResult<()> {
        let test_dir = TestDir::new()?;
        let target_path = test_dir.child("preexisting.txt");
        write_test_file(&target_path)?;
        let target = NavigationLocation::from_path(target_path)?;
        let expected = vec![ExpectedTransferLocation {
            location: target,
            existed_before: TransferTargetExistence::Exists,
        }];

        let completed = completed_transfer(DropOperation::Copy, &[], expected.as_slice());

        assert!(undo_operation_for_completed_transfer(&[], completed).is_none());

        Ok(())
    }

    #[test]
    fn copy_undo_is_not_created_when_pre_transfer_target_state_is_unknown() -> ExplorerResult<()> {
        let test_dir = TestDir::new()?;
        let target_path = test_dir.child("unknown-before.txt");
        let target = NavigationLocation::from_path(target_path.clone())?;
        let expected = vec![ExpectedTransferLocation {
            location: target,
            existed_before: TransferTargetExistence::Unknown,
        }];

        write_test_file(&target_path)?;

        let completed = completed_transfer(DropOperation::Copy, &[], expected.as_slice());

        assert!(undo_operation_for_completed_transfer(&[], completed).is_none());

        Ok(())
    }

    #[test]
    fn copy_undo_is_not_created_when_expected_target_is_missing_after_transfer(
    ) -> ExplorerResult<()> {
        let test_dir = TestDir::new()?;
        let target = NavigationLocation::from_path(test_dir.child("missing-after.txt"))?;
        let expected = vec![ExpectedTransferLocation {
            location: target,
            existed_before: TransferTargetExistence::Missing,
        }];

        let completed = completed_transfer(DropOperation::Copy, &[], expected.as_slice());

        assert!(undo_operation_for_completed_transfer(&[], completed).is_none());

        Ok(())
    }

    #[test]
    fn copy_undo_is_not_created_for_partial_expected_copy() -> ExplorerResult<()> {
        let test_dir = TestDir::new()?;
        let first_path = test_dir.child("copied-a.txt");
        let first = NavigationLocation::from_path(first_path.clone())?;
        let second = NavigationLocation::from_path(test_dir.child("copied-b.txt"))?;
        let expected = vec![
            ExpectedTransferLocation {
                location: first,
                existed_before: TransferTargetExistence::Missing,
            },
            ExpectedTransferLocation {
                location: second,
                existed_before: TransferTargetExistence::Missing,
            },
        ];

        write_test_file(&first_path)?;

        let completed = completed_transfer(DropOperation::Copy, &[], expected.as_slice());

        assert!(undo_operation_for_completed_transfer(&[], completed).is_none());

        Ok(())
    }

    #[test]
    fn move_undo_is_created_only_when_sources_disappear() -> ExplorerResult<()> {
        let test_dir = TestDir::new()?;
        let source_parent_path = test_dir.child("source");
        let destination_path = test_dir.child("destination");
        fs::create_dir_all(&source_parent_path).map_err(|source| {
            ExplorerError::io(
                "create move undo source directory",
                Some(source_parent_path.clone()),
                source,
            )
        })?;
        fs::create_dir_all(&destination_path).map_err(|source| {
            ExplorerError::io(
                "create move undo destination directory",
                Some(destination_path.clone()),
                source,
            )
        })?;
        let source = NavigationLocation::from_path(source_parent_path.join("moved.txt"))?;
        let target_path = destination_path.join("moved.txt");
        let target = NavigationLocation::from_path(target_path.clone())?;
        let expected = vec![ExpectedTransferLocation {
            location: target.clone(),
            existed_before: TransferTargetExistence::Missing,
        }];

        write_test_file(&target_path)?;

        let sources = std::slice::from_ref(&source);
        let completed = completed_transfer(DropOperation::Move, sources, expected.as_slice());

        match undo_operation_for_completed_transfer(sources, completed) {
            Some(UndoFileOperation::Move { moved }) => assert_eq!(
                moved,
                vec![(
                    target,
                    NavigationLocation::from_path(source_parent_path.clone())?
                )]
            ),
            other => panic!("expected move undo operation, got {other:?}"),
        }

        Ok(())
    }

    #[test]
    fn move_undo_and_selection_are_not_created_when_source_still_exists() -> ExplorerResult<()> {
        let test_dir = TestDir::new()?;
        let source_parent_path = test_dir.child("source");
        let destination_path = test_dir.child("destination");
        fs::create_dir_all(&source_parent_path).map_err(|source| {
            ExplorerError::io(
                "create partial move source directory",
                Some(source_parent_path.clone()),
                source,
            )
        })?;
        fs::create_dir_all(&destination_path).map_err(|source| {
            ExplorerError::io(
                "create partial move destination directory",
                Some(destination_path.clone()),
                source,
            )
        })?;
        let source_path = source_parent_path.join("stayed.txt");
        let target_path = destination_path.join("stayed.txt");
        let source = NavigationLocation::from_path(source_path.clone())?;
        let target = NavigationLocation::from_path(target_path.clone())?;
        let expected = vec![ExpectedTransferLocation {
            location: target,
            existed_before: TransferTargetExistence::Missing,
        }];

        write_test_file(&source_path)?;
        write_test_file(&target_path)?;

        let sources = std::slice::from_ref(&source);
        let completed = completed_transfer(DropOperation::Move, sources, expected.as_slice());

        assert!(undo_operation_for_completed_transfer(sources, completed).is_none());
        assert!(completed_transfer_selection(completed).is_empty());

        Ok(())
    }
}

#[cfg(test)]
mod horizontal_pane_layout_tests {
    use j3files::platform::win32_ui as ui;

    use super::{build_horizontal_pane_layout, scale_px_between_dpi};

    #[test]
    fn pane_layout_uses_requested_tree_width() {
        let layout = build_horizontal_pane_layout(1000, 8, 8, 240, 120, 360);

        assert_eq!(layout.tree_width, 240);
        assert_eq!(layout.splitter_x, 248);
        assert_eq!(layout.splitter_width, 8);
        assert_eq!(layout.right_x, 256);
        assert_eq!(layout.right_width, 736);
        assert!(layout.contains_splitter_x(248));
        assert!(layout.contains_splitter_x(255));
        assert!(!layout.contains_splitter_x(256));
    }

    #[test]
    fn pane_layout_clamps_tree_width_to_keep_right_pane_visible() {
        let layout = build_horizontal_pane_layout(1000, 8, 8, 900, 120, 360);

        assert_eq!(layout.tree_width, 616);
        assert_eq!(layout.right_width, 360);
    }

    #[test]
    fn pane_layout_preserves_minimum_tree_width_when_possible() {
        let layout = build_horizontal_pane_layout(1000, 8, 8, 10, 120, 360);

        assert_eq!(layout.tree_width, 120);
    }

    #[test]
    fn pane_layout_handles_tiny_client_width() {
        let layout = build_horizontal_pane_layout(20, 8, 8, 240, 120, 360);

        assert_eq!(layout.tree_width, 4);
        assert_eq!(layout.splitter_width, 0);
        assert_eq!(layout.right_width, 0);
    }

    #[test]
    fn dpi_scaling_preserves_adjusted_pane_width_ratio() {
        assert_eq!(
            scale_px_between_dpi(240, ui::DpiMetrics::new(96), ui::DpiMetrics::new(144)),
            360
        );
    }
}

#[cfg(test)]
mod size_move_dpi_tests {
    use super::{SizeMoveDpiState, PROGRAM_NAME, WINDOW_TITLE};

    #[test]
    fn window_title_uses_program_name() {
        assert_eq!(WINDOW_TITLE, PROGRAM_NAME);
        assert_eq!(WINDOW_TITLE, "j3Files");
    }

    #[test]
    fn dpi_refresh_is_not_deferred_outside_native_size_move() {
        let state = SizeMoveDpiState::default();

        assert!(!state.should_defer_dpi_refresh());
        assert!(!state.should_defer_layout());
    }

    #[test]
    fn dpi_refresh_is_pending_during_native_size_move() {
        let mut state = SizeMoveDpiState::default();

        state.enter();
        state.defer_dpi_refresh();

        assert!(state.should_defer_dpi_refresh());
        assert!(state.should_defer_layout());
    }

    #[test]
    fn exiting_native_size_move_returns_pending_state_and_resets() {
        let mut state = SizeMoveDpiState::default();

        state.enter();
        state.defer_dpi_refresh();
        let exit = state.exit();

        assert!(exit.dpi_refresh_pending);
        assert!(!state.should_defer_dpi_refresh());
        assert!(!state.should_defer_layout());
    }
}

#[cfg(test)]
mod search_worker_tests {
    use std::path::PathBuf;
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    };
    use std::thread;

    use j3files::app::SearchRequest;
    use j3files::domain::{
        DisplayOptions, NavigationLocation, SearchCriteria, SearchRunId, SortState, TabId,
    };
    use j3files::platform::SynchronousIoCancellation;

    use super::{
        cancel_search_workers, detach_cancelled_search_workers, reap_finished_search_workers,
        replace_pending_search_worker, ActiveSearchWorker, PendingSearchWorker,
    };

    fn request(tab_id: TabId, run_id: SearchRunId) -> SearchRequest {
        SearchRequest {
            run_id,
            tab_id,
            root: NavigationLocation::LocalPath(PathBuf::from(r"C:\root")),
            criteria: SearchCriteria::default(),
            display_options: DisplayOptions::default(),
            sort: SortState::default(),
        }
    }

    #[test]
    fn finished_search_workers_are_joined_without_waiting_for_running_workers() {
        let running_cancel = Arc::new(AtomicBool::new(false));
        let running_cancel_for_thread = Arc::clone(&running_cancel);
        let running_handle = thread::spawn(move || {
            while !running_cancel_for_thread.load(Ordering::Relaxed) {
                thread::yield_now();
            }
        });
        let finished_handle = thread::spawn(|| {});
        while !finished_handle.is_finished() {
            thread::yield_now();
        }

        let mut workers = vec![
            ActiveSearchWorker {
                tab_id: TabId(1),
                run_id: SearchRunId(1),
                cancel_requested: Arc::new(AtomicBool::new(false)),
                io_cancellation: Arc::new(SynchronousIoCancellation::new()),
                handle: Some(finished_handle),
            },
            ActiveSearchWorker {
                tab_id: TabId(1),
                run_id: SearchRunId(2),
                cancel_requested: Arc::clone(&running_cancel),
                io_cancellation: Arc::new(SynchronousIoCancellation::new()),
                handle: Some(running_handle),
            },
        ];

        reap_finished_search_workers(&mut workers);

        assert_eq!(workers.len(), 1);
        assert_eq!(workers[0].run_id, SearchRunId(2));

        workers[0].request_cancel();
        while !workers[0].is_finished() {
            thread::yield_now();
        }
        reap_finished_search_workers(&mut workers);

        assert!(workers.is_empty());
    }

    #[test]
    fn replacing_pending_search_worker_cancels_older_request_for_tab() {
        let first_cancel = Arc::new(AtomicBool::new(false));
        let second_cancel = Arc::new(AtomicBool::new(false));
        let mut pending_workers = vec![PendingSearchWorker {
            request: request(TabId(3), SearchRunId(10)),
            cancel_requested: Arc::clone(&first_cancel),
        }];

        replace_pending_search_worker(
            &mut pending_workers,
            PendingSearchWorker {
                request: request(TabId(3), SearchRunId(11)),
                cancel_requested: Arc::clone(&second_cancel),
            },
        );

        assert!(first_cancel.load(Ordering::Relaxed));
        assert!(!second_cancel.load(Ordering::Relaxed));
        assert_eq!(pending_workers.len(), 1);
        assert_eq!(pending_workers[0].tab_id(), TabId(3));
        assert_eq!(pending_workers[0].run_id(), SearchRunId(11));
    }

    #[test]
    fn cancelled_search_workers_are_detached_from_active_slots() {
        let cancelled = Arc::new(AtomicBool::new(true));
        let cancelled_for_thread = Arc::clone(&cancelled);
        let cancelled_handle = thread::spawn(move || {
            while !cancelled_for_thread.load(Ordering::Relaxed) {
                thread::yield_now();
            }
        });
        let retained_handle = thread::spawn(|| {});
        while !retained_handle.is_finished() {
            thread::yield_now();
        }

        let mut workers = vec![
            ActiveSearchWorker {
                tab_id: TabId(2),
                run_id: SearchRunId(20),
                cancel_requested: Arc::clone(&cancelled),
                io_cancellation: Arc::new(SynchronousIoCancellation::new()),
                handle: Some(cancelled_handle),
            },
            ActiveSearchWorker {
                tab_id: TabId(3),
                run_id: SearchRunId(21),
                cancel_requested: Arc::new(AtomicBool::new(false)),
                io_cancellation: Arc::new(SynchronousIoCancellation::new()),
                handle: Some(retained_handle),
            },
        ];

        detach_cancelled_search_workers(&mut workers);

        assert_eq!(workers.len(), 1);
        assert_eq!(workers[0].tab_id, TabId(3));
        assert_eq!(workers[0].run_id, SearchRunId(21));

        reap_finished_search_workers(&mut workers);
        assert!(workers.is_empty());
    }

    #[test]
    fn cancelling_all_search_workers_does_not_join_running_handles() {
        let cancel = Arc::new(AtomicBool::new(false));
        let cancel_for_thread = Arc::clone(&cancel);
        let handle = thread::spawn(move || {
            while !cancel_for_thread.load(Ordering::Relaxed) {
                thread::yield_now();
            }
        });
        let mut workers = vec![ActiveSearchWorker {
            tab_id: TabId(2),
            run_id: SearchRunId(20),
            cancel_requested: Arc::clone(&cancel),
            io_cancellation: Arc::new(SynchronousIoCancellation::new()),
            handle: Some(handle),
        }];

        cancel_search_workers(&mut workers);

        assert!(cancel.load(Ordering::Relaxed));
        assert!(workers.is_empty());
    }
}

#[cfg(test)]
mod listing_worker_tests {
    use std::path::PathBuf;
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    };
    use std::thread;

    use j3files::domain::{
        DisplayOptions, NavigationLocation, SortDirection, SortKey, SortState, TabId,
    };
    use j3files::platform::SynchronousIoCancellation;

    use super::{
        join_listing_worker, listing_request_matches_source_and_sort,
        reap_finished_listing_workers, retire_listing_worker, ActiveListingWorker, ListingRequest,
        WorkerController,
    };

    fn request(generation: u64) -> ListingRequest {
        ListingRequest {
            generation,
            tab_id: TabId(1),
            location: NavigationLocation::LocalPath(PathBuf::from(r"C:\root")),
            display_options: DisplayOptions::default(),
            sort: SortState::default(),
        }
    }

    fn cancellable_listing_worker(generation: u64) -> (ActiveListingWorker, Arc<AtomicBool>) {
        let cancel = Arc::new(AtomicBool::new(false));
        let cancel_for_thread = Arc::clone(&cancel);
        let handle = thread::spawn(move || {
            while !cancel_for_thread.load(Ordering::Relaxed) {
                thread::yield_now();
            }
        });

        (
            ActiveListingWorker {
                request: request(generation),
                cancel_requested: Arc::clone(&cancel),
                io_cancellation: Arc::new(SynchronousIoCancellation::new()),
                handle: Some(handle),
            },
            cancel,
        )
    }

    fn blocked_listing_worker(
        generation: u64,
    ) -> (ActiveListingWorker, Arc<AtomicBool>, Arc<AtomicBool>) {
        let cancel = Arc::new(AtomicBool::new(false));
        let release = Arc::new(AtomicBool::new(false));
        let release_for_thread = Arc::clone(&release);
        let handle = thread::spawn(move || {
            while !release_for_thread.load(Ordering::Relaxed) {
                thread::yield_now();
            }
        });

        (
            ActiveListingWorker {
                request: request(generation),
                cancel_requested: Arc::clone(&cancel),
                io_cancellation: Arc::new(SynchronousIoCancellation::new()),
                handle: Some(handle),
            },
            cancel,
            release,
        )
    }

    #[test]
    fn listing_request_source_match_ignores_sort_state() {
        let current = request(1);
        let mut resorted = request(2);
        resorted.sort = SortState {
            key: SortKey::UpdatedAt,
            direction: SortDirection::Descending,
        };

        assert_ne!(current, resorted);
        assert!(current.has_same_listing_source_as(&resorted));
    }

    #[test]
    fn listing_request_source_and_sort_match_rejects_changed_sort_state() {
        let current = request(1);
        let mut resorted = request(2);
        resorted.sort = SortState {
            key: SortKey::UpdatedAt,
            direction: SortDirection::Descending,
        };

        assert!(current.has_same_listing_source_as(&resorted));
        assert!(!listing_request_matches_source_and_sort(
            &current, &resorted
        ));
    }

    #[test]
    fn listing_request_source_and_sort_match_accepts_new_generation_with_same_sort() {
        let current = request(1);
        let requested = request(2);

        assert!(listing_request_matches_source_and_sort(
            &current, &requested
        ));
    }

    #[test]
    fn listing_request_source_match_keeps_display_options_in_source() {
        let current = request(1);
        let mut show_hidden = request(2);
        show_hidden.display_options = DisplayOptions {
            show_hidden: true,
            show_system: false,
        };

        assert!(!current.has_same_listing_source_as(&show_hidden));
    }

    #[test]
    fn retiring_listing_worker_requests_cancel_and_reaps_finished_handle() {
        let (active_worker, cancel) = cancellable_listing_worker(1);
        let mut listing_worker = Some(active_worker);
        let mut retired_listing_workers = Vec::new();

        assert!(retire_listing_worker(
            &mut listing_worker,
            &mut retired_listing_workers
        ));

        assert!(listing_worker.is_none());
        assert!(cancel.load(Ordering::Relaxed));
        assert_eq!(retired_listing_workers.len(), 1);

        while !retired_listing_workers[0].is_finished() {
            thread::yield_now();
        }
        reap_finished_listing_workers(&mut retired_listing_workers);

        assert!(retired_listing_workers.is_empty());
    }

    #[test]
    fn retiring_listing_worker_at_capacity_defers_without_detaching_running_worker() {
        let (retired_worker, retired_cancel, retired_release) = blocked_listing_worker(1);
        retired_cancel.store(true, Ordering::Relaxed);
        let (active_worker, active_cancel, active_release) = blocked_listing_worker(2);
        let mut listing_worker = Some(active_worker);
        let mut retired_listing_workers = vec![retired_worker];

        assert!(!retire_listing_worker(
            &mut listing_worker,
            &mut retired_listing_workers
        ));

        assert!(listing_worker.is_some());
        assert!(retired_cancel.load(Ordering::Relaxed));
        assert!(active_cancel.load(Ordering::Relaxed));
        assert_eq!(retired_listing_workers.len(), 1);
        assert_eq!(retired_listing_workers[0].request.generation, 1);
        assert!(retired_listing_workers[0].handle.is_some());

        active_release.store(true, Ordering::Relaxed);
        let Some(active_worker) = listing_worker.take() else {
            panic!("active listing worker should remain joinable");
        };
        join_listing_worker(active_worker);

        retired_release.store(true, Ordering::Relaxed);

        while retired_listing_workers
            .iter()
            .any(|worker| !worker.is_finished())
        {
            thread::yield_now();
        }
        reap_finished_listing_workers(&mut retired_listing_workers);
        assert!(retired_listing_workers.is_empty());
    }

    #[test]
    fn pending_listing_request_keeps_only_latest_request() {
        let mut workers = WorkerController::new();

        workers.replace_pending_listing_request(request(2));
        workers.replace_pending_listing_request(request(3));

        let Some(pending) = workers.take_pending_listing_request() else {
            panic!("latest pending listing request should be kept");
        };
        assert_eq!(pending.generation, 3);
        assert!(workers.take_pending_listing_request().is_none());
    }
}

#[cfg(test)]
mod file_operation_run_tests {
    use std::cell::RefCell;
    use std::ffi::OsString;
    use std::path::PathBuf;

    use j3files::app::{ShellDeleteGateway, ShellRenameGateway, ShellTransferGateway};
    use j3files::domain::{
        DropOperation, ExplorerError, ExplorerResult, NavigationLocation, RenameItemName, TabId,
    };

    use super::{run_file_operation, DeleteFileOperation, FileOperationRequest, UndoFileOperation};

    type TransferLog = Vec<(Vec<NavigationLocation>, NavigationLocation)>;

    #[derive(Default)]
    struct RecordingShell {
        copied: RefCell<TransferLog>,
        moved: RefCell<TransferLog>,
        move_errors: RefCell<Vec<NavigationLocation>>,
        recycled: RefCell<Vec<Vec<NavigationLocation>>>,
        permanently_deleted: RefCell<Vec<Vec<NavigationLocation>>>,
        renamed: RefCell<Vec<(NavigationLocation, OsString)>>,
    }

    impl ShellTransferGateway for RecordingShell {
        fn copy_items(
            &self,
            sources: &[NavigationLocation],
            destination: &NavigationLocation,
        ) -> ExplorerResult<()> {
            self.copied
                .borrow_mut()
                .push((sources.to_vec(), destination.clone()));
            Ok(())
        }

        fn move_items(
            &self,
            sources: &[NavigationLocation],
            destination: &NavigationLocation,
        ) -> ExplorerResult<()> {
            self.moved
                .borrow_mut()
                .push((sources.to_vec(), destination.clone()));
            if sources.iter().any(|source| {
                self.move_errors
                    .borrow()
                    .iter()
                    .any(|failed| failed == source)
            }) {
                return Err(ExplorerError::invalid_input("move failed"));
            }
            Ok(())
        }
    }

    impl ShellDeleteGateway for RecordingShell {
        fn delete_to_recycle_bin(&self, targets: &[NavigationLocation]) -> ExplorerResult<()> {
            self.recycled.borrow_mut().push(targets.to_vec());
            Ok(())
        }

        fn delete_permanently(&self, targets: &[NavigationLocation]) -> ExplorerResult<()> {
            self.permanently_deleted.borrow_mut().push(targets.to_vec());
            Ok(())
        }
    }

    impl ShellRenameGateway for RecordingShell {
        fn rename_item(
            &self,
            target: &NavigationLocation,
            new_name: &RenameItemName,
        ) -> ExplorerResult<()> {
            self.renamed
                .borrow_mut()
                .push((target.clone(), new_name.as_os_str().to_os_string()));
            Ok(())
        }
    }

    fn location(path: &str) -> ExplorerResult<NavigationLocation> {
        NavigationLocation::from_path(PathBuf::from(path))
    }

    fn transfer_request(
        operation: DropOperation,
        sources: Vec<NavigationLocation>,
        destination: NavigationLocation,
    ) -> FileOperationRequest {
        FileOperationRequest::Transfer {
            tab_id: TabId(1),
            location: destination.clone(),
            operation,
            sources,
            destination,
            select_completed_items: false,
        }
    }

    #[test]
    fn move_transfer_rejects_descendant_destination_before_shell_call() -> ExplorerResult<()> {
        let shell = RecordingShell::default();
        let request = transfer_request(
            DropOperation::Move,
            vec![location(r"C:\source\folder")?],
            location(r"C:\source\folder\child")?,
        );

        let error = run_file_operation(&shell, request)
            .expect_err("move into source descendant must fail before shell call");

        assert_eq!(
            error.user_message(),
            "이동 대상이 원본과 같거나 원본의 하위 폴더입니다."
        );
        assert!(shell.moved.borrow().is_empty());
        Ok(())
    }

    #[test]
    fn copy_worker_outcome_refreshes_destination() -> ExplorerResult<()> {
        let shell = RecordingShell::default();
        let destination = location(r"D:\to")?;
        let request = transfer_request(
            DropOperation::Copy,
            vec![location(r"C:\from\a.txt")?],
            destination.clone(),
        );

        let outcome = run_file_operation(&shell, request)?;

        assert_eq!(outcome.affected_folders, vec![destination]);
        assert_eq!(shell.copied.borrow().len(), 1);
        Ok(())
    }

    #[test]
    fn move_worker_outcome_refreshes_source_parents_and_destination() -> ExplorerResult<()> {
        let shell = RecordingShell::default();
        let destination = location(r"D:\to")?;
        let request = transfer_request(
            DropOperation::Move,
            vec![
                location(r"C:\from\a.txt")?,
                location(r"C:\from\nested\b.txt")?,
            ],
            destination.clone(),
        );

        let outcome = run_file_operation(&shell, request)?;

        assert_eq!(
            outcome.affected_folders,
            vec![
                location(r"C:\from")?,
                location(r"C:\from\nested")?,
                destination
            ]
        );
        assert_eq!(shell.moved.borrow().len(), 1);
        Ok(())
    }

    #[test]
    fn delete_worker_outcome_refreshes_source_parents() -> ExplorerResult<()> {
        let shell = RecordingShell::default();
        let request = FileOperationRequest::Delete {
            tab_id: TabId(1),
            location: location(r"C:\from")?,
            operation: DeleteFileOperation::ToRecycleBin,
            targets: vec![
                location(r"C:\from\a.txt")?,
                location(r"C:\from\nested\b.txt")?,
            ],
        };

        let outcome = run_file_operation(&shell, request)?;

        assert_eq!(
            outcome.affected_folders,
            vec![location(r"C:\from")?, location(r"C:\from\nested")?]
        );
        assert_eq!(shell.recycled.borrow().len(), 1);
        Ok(())
    }

    #[test]
    fn rename_worker_outcome_selects_renamed_item_and_records_undo() -> ExplorerResult<()> {
        let shell = RecordingShell::default();
        let target = location(r"C:\from\a.txt")?;
        let request = FileOperationRequest::Rename {
            tab_id: TabId(1),
            location: location(r"C:\from")?,
            target: target.clone(),
            new_name: OsString::from("b.txt"),
            undo_original_name: Some(OsString::from("a.txt")),
        };

        let outcome = run_file_operation(&shell, request)?;

        assert_eq!(
            *shell.renamed.borrow(),
            vec![(target, OsString::from("b.txt"))]
        );
        assert_eq!(outcome.affected_folders, vec![location(r"C:\from")?]);
        assert_eq!(outcome.selected_items, vec![location(r"C:\from\b.txt")?]);
        match outcome.undo_file_operation {
            Some(UndoFileOperation::Rename {
                current,
                original_name,
            }) => {
                assert_eq!(current, location(r"C:\from\b.txt")?);
                assert_eq!(original_name, OsString::from("a.txt"));
            }
            other => panic!("expected rename undo operation, got {other:?}"),
        }
        assert!(outcome.completion_error.is_none());
        Ok(())
    }

    #[test]
    fn undo_move_worker_keeps_remaining_items_after_partial_failure() -> ExplorerResult<()> {
        let shell = RecordingShell::default();
        let failed_current = location(r"D:\to\b.txt")?;
        shell.move_errors.borrow_mut().push(failed_current.clone());
        let request = FileOperationRequest::UndoMove {
            tab_id: TabId(1),
            location: location(r"D:\to")?,
            moved: vec![
                (location(r"D:\to\a.txt")?, location(r"C:\from")?),
                (failed_current.clone(), location(r"C:\from\nested")?),
            ],
        };

        let outcome = run_file_operation(&shell, request)?;

        assert_eq!(shell.moved.borrow().len(), 2);
        assert_eq!(outcome.selected_items, vec![location(r"C:\from\a.txt")?]);
        match outcome.undo_file_operation {
            Some(UndoFileOperation::Move { moved }) => {
                assert_eq!(moved, vec![(failed_current, location(r"C:\from\nested")?)]);
            }
            other => panic!("expected remaining move undo operation, got {other:?}"),
        }
        let Some(error) = outcome.completion_error else {
            panic!("partial undo move failure should report the first error");
        };
        assert_eq!(error.user_message(), "move failed");
        Ok(())
    }
}

#[cfg(test)]
mod file_operation_worker_tests {
    use std::sync::mpsc;
    use std::thread;
    use std::time::Duration;

    use j3files::domain::TabId;

    use super::{join_file_operation_worker, ActiveFileOperationWorker, WorkerController};

    #[test]
    fn shutdown_file_operation_cleanup_waits_for_running_handle() {
        let (started_tx, started_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let (done_tx, done_rx) = mpsc::channel();
        let (cleanup_started_tx, cleanup_started_rx) = mpsc::channel();
        let handle = thread::spawn(move || {
            let _ = started_tx.send(());
            let _ = release_rx.recv_timeout(Duration::from_millis(500));
            let _ = done_tx.send(());
        });
        assert!(started_rx.recv().is_ok());

        let mut worker = Some(ActiveFileOperationWorker {
            generation: 42,
            tab_id: TabId(9),
            handle: Some(handle),
        });

        let cleanup_handle = thread::spawn(move || {
            let _ = cleanup_started_tx.send(());
            join_file_operation_worker(&mut worker);
            worker.is_none()
        });

        assert!(cleanup_started_rx.recv().is_ok());
        assert!(done_rx.recv_timeout(Duration::from_millis(50)).is_err());
        assert!(release_tx.send(()).is_ok());
        assert!(cleanup_handle.join().unwrap_or(false));
        assert!(done_rx.recv_timeout(Duration::from_secs(1)).is_ok());
    }

    #[test]
    fn shutdown_file_operation_reaps_finished_handle_without_complete_message() {
        let handle = thread::spawn(|| {});
        while !handle.is_finished() {
            thread::yield_now();
        }

        let mut workers = WorkerController::new();
        workers.start_file_operation_worker(42, TabId(9), handle);

        workers.reap_finished_file_operation_worker_for_shutdown();

        assert!(!workers.has_file_operation_worker());
    }

    #[test]
    fn active_file_operation_worker_rejects_second_start() {
        let handle = thread::spawn(|| {});
        let mut workers = WorkerController::new();
        workers.start_file_operation_worker(42, TabId(9), handle);

        let error = workers
            .ensure_file_operation_worker_idle()
            .expect_err("active file operation worker must block a second operation");

        assert_eq!(
            error.user_message(),
            "다른 파일 작업이 아직 완료되지 않았습니다."
        );
        workers.finish_file_operation_worker_for_generation(42);
        assert!(!workers.has_file_operation_worker());
    }
}

#[cfg(test)]
mod search_message_tests {
    use j3files::domain::{ExplorerError, SearchProgress, SearchRunId, TabId};

    use super::{SearchCompleteMessage, SearchProgressMessage, WorkerMessageStore};

    #[test]
    fn invalid_search_message_token_is_ignored() {
        let messages = WorkerMessageStore::new();
        assert!(messages.take_search_progress(-1).is_none());
        assert!(messages.take_search_complete(-1).is_none());
    }

    #[test]
    fn progress_message_token_can_be_taken_once() {
        let messages = WorkerMessageStore::new();
        let insertion = messages
            .lock()
            .search
            .insert_progress(SearchProgressMessage {
                tab_id: TabId(7),
                run_id: SearchRunId(11),
                progress: SearchProgress {
                    visited_folders: 3,
                    scanned_items: 5,
                    matched_items: 2,
                    skipped_folders: 1,
                },
            });
        assert!(insertion.should_post);

        let Some(message) = messages.take_search_progress(insertion.token) else {
            panic!("registered progress message was not returned");
        };

        assert_eq!(message.tab_id, TabId(7));
        assert_eq!(message.run_id, SearchRunId(11));
        assert_eq!(message.progress.scanned_items, 5);
        assert!(messages.take_search_progress(insertion.token).is_none());
    }

    #[test]
    fn progress_messages_for_same_search_are_coalesced() {
        let messages = WorkerMessageStore::new();
        let first = messages
            .lock()
            .search
            .insert_progress(SearchProgressMessage {
                tab_id: TabId(7),
                run_id: SearchRunId(11),
                progress: SearchProgress {
                    visited_folders: 1,
                    scanned_items: 64,
                    matched_items: 2,
                    skipped_folders: 0,
                },
            });
        let second = messages
            .lock()
            .search
            .insert_progress(SearchProgressMessage {
                tab_id: TabId(7),
                run_id: SearchRunId(11),
                progress: SearchProgress {
                    visited_folders: 3,
                    scanned_items: 128,
                    matched_items: 5,
                    skipped_folders: 1,
                },
            });

        assert!(first.should_post);
        assert!(!second.should_post);
        assert_eq!(first.token, second.token);

        let Some(message) = messages.take_search_progress(first.token) else {
            panic!("coalesced progress message was not returned");
        };

        assert_eq!(message.tab_id, TabId(7));
        assert_eq!(message.run_id, SearchRunId(11));
        assert_eq!(message.progress.scanned_items, 128);
        assert_eq!(message.progress.matched_items, 5);
        assert!(messages.take_search_progress(first.token).is_none());
    }

    #[test]
    fn progress_messages_for_different_searches_keep_distinct_tokens() {
        let messages = WorkerMessageStore::new();
        let first = messages
            .lock()
            .search
            .insert_progress(SearchProgressMessage {
                tab_id: TabId(7),
                run_id: SearchRunId(11),
                progress: SearchProgress {
                    visited_folders: 1,
                    scanned_items: 64,
                    matched_items: 2,
                    skipped_folders: 0,
                },
            });
        let second = messages
            .lock()
            .search
            .insert_progress(SearchProgressMessage {
                tab_id: TabId(7),
                run_id: SearchRunId(12),
                progress: SearchProgress {
                    visited_folders: 2,
                    scanned_items: 96,
                    matched_items: 3,
                    skipped_folders: 0,
                },
            });

        assert!(first.should_post);
        assert!(second.should_post);
        assert_ne!(first.token, second.token);

        let Some(first_message) = messages.take_search_progress(first.token) else {
            panic!("first progress message was not returned");
        };
        let Some(second_message) = messages.take_search_progress(second.token) else {
            panic!("second progress message was not returned");
        };

        assert_eq!(first_message.run_id, SearchRunId(11));
        assert_eq!(first_message.progress.scanned_items, 64);
        assert_eq!(second_message.run_id, SearchRunId(12));
        assert_eq!(second_message.progress.scanned_items, 96);
    }

    #[test]
    fn mismatched_search_message_type_does_not_consume_token() {
        let messages = WorkerMessageStore::new();
        let token = messages
            .lock()
            .search
            .insert_complete(SearchCompleteMessage {
                tab_id: TabId(13),
                run_id: SearchRunId(17),
                result: Err(ExplorerError::Cancelled { operation: "test" }),
            });

        assert!(messages.take_search_progress(token).is_none());

        let Some(message) = messages.take_search_complete(token) else {
            panic!("registered complete message was consumed by mismatched type");
        };

        assert_eq!(message.tab_id, TabId(13));
        assert_eq!(message.run_id, SearchRunId(17));
        assert!(message.result.is_err());
    }

    #[test]
    fn complete_message_is_kept_pending_when_window_post_cannot_deliver() {
        let messages = WorkerMessageStore::new();

        messages.post_search_complete(
            1,
            SearchCompleteMessage {
                tab_id: TabId(21),
                run_id: SearchRunId(34),
                result: Err(ExplorerError::Cancelled { operation: "test" }),
            },
        );

        let Some(message) = messages.take_next_search_complete() else {
            panic!("complete message was dropped after failed window post");
        };

        assert_eq!(message.tab_id, TabId(21));
        assert_eq!(message.run_id, SearchRunId(34));
        assert!(message.result.is_err());
    }
}

#[cfg(test)]
mod completion_message_tests {
    use std::path::PathBuf;

    use j3files::domain::{DisplayOptions, ExplorerError, NavigationLocation, SortState, TabId};
    use j3files::platform::DirectoryChangeBatch;

    use super::{
        FileOperationCompleteMessage, FileWatchChangeMessage, ListingCompleteMessage,
        ListingRequest, WorkerMessageStore,
    };

    fn local_location(path: &str) -> NavigationLocation {
        NavigationLocation::LocalPath(PathBuf::from(path))
    }

    fn listing_request(generation: u64) -> ListingRequest {
        ListingRequest {
            generation,
            tab_id: TabId(55),
            location: local_location(r"C:\root"),
            display_options: DisplayOptions::default(),
            sort: SortState::default(),
        }
    }

    #[test]
    fn listing_complete_message_is_kept_pending_when_window_post_cannot_deliver() {
        let messages = WorkerMessageStore::new();

        messages.post_listing_complete(
            1,
            ListingCompleteMessage {
                request: listing_request(89),
                result: Err(ExplorerError::Cancelled { operation: "test" }),
            },
        );

        let Some(message) = messages.take_next_listing_complete() else {
            panic!("listing complete message was dropped after failed window post");
        };

        assert_eq!(message.request.generation, 89);
        assert!(message.result.is_err());
    }

    #[test]
    fn file_operation_complete_message_is_kept_pending_when_window_post_cannot_deliver() {
        let messages = WorkerMessageStore::new();

        messages.post_file_operation_complete(
            1,
            FileOperationCompleteMessage {
                generation: 144,
                tab_id: TabId(55),
                location: local_location(r"C:\root"),
                result: Err(ExplorerError::Cancelled { operation: "test" }),
            },
        );

        let Some(message) = messages.take_next_file_operation_complete() else {
            panic!("file operation complete message was dropped after failed window post");
        };

        assert_eq!(message.generation, 144);
        assert_eq!(message.tab_id, TabId(55));
        assert!(message.result.is_err());
    }

    #[test]
    fn completion_recovery_request_is_raised_when_post_and_timer_fail() {
        let messages = WorkerMessageStore::new();

        messages.post_file_operation_complete(
            1,
            FileOperationCompleteMessage {
                generation: 377,
                tab_id: TabId(55),
                location: local_location(r"C:\root"),
                result: Err(ExplorerError::Cancelled { operation: "test" }),
            },
        );

        assert!(messages.take_completion_recovery_request());
        assert!(!messages.take_completion_recovery_request());
        assert!(messages.take_next_file_operation_complete().is_some());
    }

    #[test]
    fn file_watch_change_message_is_kept_pending_when_window_post_cannot_deliver() {
        let messages = WorkerMessageStore::new();

        messages.post_file_watch_changed(
            1,
            FileWatchChangeMessage {
                generation: 233,
                changes: DirectoryChangeBatch {
                    overflowed: true,
                    changes: Vec::new(),
                },
            },
        );

        let Some(message) = messages.take_next_file_watch_changed() else {
            panic!("file watch change message was dropped after failed window post");
        };

        assert_eq!(message.generation, 233);
        assert!(message.changes.overflowed);
    }
}
