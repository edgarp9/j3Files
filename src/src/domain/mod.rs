mod appearance;
mod bookmark;
mod drop;
mod error;
mod file_item;
mod folder;
mod folder_tree;
mod navigation;
mod search;
mod tab;
mod text;

pub use appearance::{
    dark_theme_storage_value, AppearanceFont, AppearanceTheme, DEFAULT_APPEARANCE_FONT_POINT_SIZE,
    DEFAULT_APPEARANCE_THEME, MAX_APPEARANCE_FONT_POINT_SIZE, MIN_APPEARANCE_FONT_POINT_SIZE,
};
pub use bookmark::{BookmarkAccessibility, BookmarkAddOutcome, BookmarkItem, BookmarkList};
pub use drop::{
    compare_storage_roots, decide_drop_operation, decide_vertical_auto_scroll_direction,
    default_external_drop_operation, drag_source_refresh_locations,
    file_transfer_refresh_locations, same_storage_root, snapshot_drag_source_locations,
    source_parent_locations, unique_drag_sources, validate_move_drop, AutoScrollDirection,
    DragSourceCompletion, DropAllowedOperations, DropModifierKeys, DropOperation, DropSourceKind,
    HoverExpandAction, HoverExpandState, StorageRootRelation,
};
pub use error::{ExplorerError, FileNameErrorKind, ShellOperation};
pub use file_item::{
    sort_file_items, sort_file_items_with_payload, DisplayOptions, FileAttributes, FileItem,
    FileItemKind, SortDirection, SortKey, SortState,
};
pub use folder::{NewFolderName, RenameItemName};
pub use folder_tree::{
    FolderTreeItem, FolderTreeItemKind, FolderTreeSection, DEFAULT_FOLDER_TREE_KNOWN_FOLDERS,
};
pub use navigation::{KnownFolderKind, NavigationLocation, PreparedNavigationPath};
pub use search::{
    matches_search_criteria, PreparedSearchCriteria, SearchCriteria, SearchDiagnostic,
    SearchProgress, SearchRunId, SearchScope,
};
pub use tab::{SearchState, TabId, TabState};

pub type ExplorerResult<T> = Result<T, ExplorerError>;
