use crate::domain::{
    DisplayOptions, ExplorerResult, FileItem, KnownFolderKind, NavigationLocation, NewFolderName,
    RenameItemName, SearchCriteria, SearchDiagnostic, SearchProgress, SortState,
};

pub trait ItemListingGateway {
    fn list_items(
        &self,
        location: &NavigationLocation,
        options: DisplayOptions,
        sort: SortState,
    ) -> ExplorerResult<Vec<FileItem>>;
}

pub trait FolderTreeGateway {
    fn list_child_folders(
        &self,
        location: &NavigationLocation,
        options: DisplayOptions,
        sort: SortState,
    ) -> ExplorerResult<Vec<FileItem>>;

    fn drive_roots(&self) -> ExplorerResult<Vec<NavigationLocation>>;

    fn known_folder(&self, kind: KnownFolderKind) -> ExplorerResult<NavigationLocation>;
}

pub trait LocationAccessGateway {
    fn ensure_accessible(&self, location: &NavigationLocation) -> ExplorerResult<()>;
}

pub trait FolderCreationGateway {
    fn create_folder(
        &self,
        parent: &NavigationLocation,
        name: &NewFolderName,
    ) -> ExplorerResult<NavigationLocation>;
}

pub trait SearchFileSystemGateway {
    fn search_items(
        &self,
        root: &NavigationLocation,
        criteria: &SearchCriteria,
        options: DisplayOptions,
        sort: SortState,
        cancellation: &dyn SearchCancellation,
        progress: &dyn SearchProgressReporter,
    ) -> ExplorerResult<SearchFileSystemOutcome>;
}

pub trait FileSystemGateway:
    ItemListingGateway
    + FolderTreeGateway
    + LocationAccessGateway
    + FolderCreationGateway
    + SearchFileSystemGateway
{
}

pub trait SearchCancellation {
    fn is_cancel_requested(&self) -> bool;
}

#[derive(Debug, Clone, Copy, Default)]
pub struct NeverCancelSearch;

impl SearchCancellation for NeverCancelSearch {
    fn is_cancel_requested(&self) -> bool {
        false
    }
}

pub trait SearchProgressReporter {
    fn report(&self, progress: SearchProgress);
}

#[derive(Debug, Clone, Copy, Default)]
pub struct NoopSearchProgressReporter;

impl SearchProgressReporter for NoopSearchProgressReporter {
    fn report(&self, _progress: SearchProgress) {}
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SearchFileSystemOutcome {
    pub items: Vec<FileItem>,
    pub diagnostics: Vec<SearchDiagnostic>,
    pub progress: SearchProgress,
    pub cancelled: bool,
}

pub trait ShellOpenGateway {
    fn open_path(&self, location: &NavigationLocation) -> ExplorerResult<()>;
}

pub trait ShellOpenWithGateway {
    fn open_with(&self, location: &NavigationLocation) -> ExplorerResult<()>;
}

pub trait ShellPropertiesGateway {
    fn show_properties(&self, location: &NavigationLocation) -> ExplorerResult<()>;
}

pub trait ShellContextMenuGateway {
    fn show_context_menu(
        &self,
        targets: &[NavigationLocation],
        position: ContextMenuPosition,
    ) -> ExplorerResult<ContextMenuOutcome>;

    fn show_folder_background_context_menu(
        &self,
        folder: &NavigationLocation,
        position: ContextMenuPosition,
    ) -> ExplorerResult<ContextMenuOutcome>;
}

pub trait ShellTransferGateway {
    fn copy_items(
        &self,
        sources: &[NavigationLocation],
        destination: &NavigationLocation,
    ) -> ExplorerResult<()>;

    fn move_items(
        &self,
        sources: &[NavigationLocation],
        destination: &NavigationLocation,
    ) -> ExplorerResult<()>;
}

pub trait ShellDeleteGateway {
    fn delete_to_recycle_bin(&self, targets: &[NavigationLocation]) -> ExplorerResult<()>;

    fn delete_permanently(&self, targets: &[NavigationLocation]) -> ExplorerResult<()>;
}

pub trait ShellRenameGateway {
    fn rename_item(
        &self,
        target: &NavigationLocation,
        new_name: &RenameItemName,
    ) -> ExplorerResult<()>;
}

pub trait ShellFileOperationGateway:
    ShellTransferGateway + ShellDeleteGateway + ShellRenameGateway
{
}

pub trait ShellGateway:
    ShellOpenGateway
    + ShellOpenWithGateway
    + ShellPropertiesGateway
    + ShellContextMenuGateway
    + ShellFileOperationGateway
{
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ContextMenuPosition {
    pub x: i32,
    pub y: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ContextMenuOutcome {
    pub command_invoked: bool,
    pub refresh_current_folder: bool,
}
