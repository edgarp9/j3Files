use std::collections::HashMap;
use std::ffi::{OsStr, OsString};
use std::path::PathBuf;
use std::time::SystemTime;

pub use super::explorer_ports::{
    ContextMenuOutcome, ContextMenuPosition, FileSystemGateway, FolderCreationGateway,
    FolderTreeGateway, ItemListingGateway, LocationAccessGateway, NeverCancelSearch,
    NoopSearchProgressReporter, SearchCancellation, SearchFileSystemGateway,
    SearchFileSystemOutcome, SearchProgressReporter, ShellContextMenuGateway, ShellDeleteGateway,
    ShellFileOperationGateway, ShellGateway, ShellOpenGateway, ShellOpenWithGateway,
    ShellPropertiesGateway, ShellRenameGateway, ShellTransferGateway,
};
use crate::domain::{
    decide_drop_operation, file_transfer_refresh_locations, source_parent_locations,
    validate_move_drop, AppearanceFont, AppearanceTheme, BookmarkAccessibility, BookmarkAddOutcome,
    BookmarkItem, BookmarkList, DisplayOptions, DropModifierKeys, DropOperation, DropSourceKind,
    ExplorerError, ExplorerResult, FileItem, FolderTreeItem, KnownFolderKind, NavigationLocation,
    NewFolderName, RenameItemName, SearchCriteria, SearchDiagnostic, SearchProgress, SearchRunId,
    SearchState, ShellOperation, SortDirection, SortKey, SortState, TabId, TabState,
    DEFAULT_FOLDER_TREE_KNOWN_FOLDERS,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchRequest {
    pub run_id: SearchRunId,
    pub tab_id: TabId,
    pub root: NavigationLocation,
    pub criteria: SearchCriteria,
    pub display_options: DisplayOptions,
    pub sort: SortState,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchOutcome {
    pub run_id: SearchRunId,
    pub tab_id: TabId,
    pub criteria: SearchCriteria,
    pub items: Vec<FileItem>,
    pub diagnostics: Vec<SearchDiagnostic>,
    pub progress: SearchProgress,
    pub cancelled: bool,
}

impl SearchOutcome {
    pub fn from_request(request: SearchRequest, outcome: SearchFileSystemOutcome) -> Self {
        Self {
            run_id: request.run_id,
            tab_id: request.tab_id,
            criteria: request.criteria,
            items: outcome.items,
            diagnostics: outcome.diagnostics,
            progress: outcome.progress,
            cancelled: outcome.cancelled,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct UserSettings {
    pub bookmarks: BookmarkList,
    pub display_options: DisplayOptions,
    pub appearance_theme: AppearanceTheme,
    pub appearance_font: AppearanceFont,
    pub startup_folder: Option<NavigationLocation>,
    pub restore_tabs_on_startup: bool,
    pub session: UserSession,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct UserSession {
    pub tabs: Vec<TabState>,
    pub active_tab_id: Option<TabId>,
    pub closed_tabs: Vec<TabState>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StartupSessionRestore {
    session: UserSession,
    access_cache: HashMap<PathBuf, bool>,
}

pub trait UserSettingsGateway {
    fn load_user_settings(&self) -> ExplorerResult<UserSettings>;

    fn save_user_settings(&self, settings: &UserSettings) -> ExplorerResult<()>;
}

pub trait FolderTreeChildPresenceGateway {
    fn has_child_folders(
        &self,
        location: &NavigationLocation,
        options: DisplayOptions,
    ) -> ExplorerResult<bool>;
}

#[derive(Debug, Clone)]
pub struct ExplorerState {
    pub tabs: Vec<TabState>,
    pub active_tab_id: TabId,
    pub closed_tabs: Vec<TabState>,
    pub bookmarks: BookmarkList,
    pub display_options: DisplayOptions,
    pub appearance_theme: AppearanceTheme,
    pub appearance_font: AppearanceFont,
    pub startup_folder: Option<NavigationLocation>,
    pub restore_tabs_on_startup: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ItemActivation {
    Navigated,
    Opened,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct FileOperationOutcome {
    pub affected_folders: Vec<NavigationLocation>,
}

impl FileOperationOutcome {
    fn new(affected_folders: Vec<NavigationLocation>) -> Self {
        Self { affected_folders }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileDropPlan {
    pub operation: DropOperation,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateFolderOutcome {
    pub created_folder: NavigationLocation,
    pub affected_folders: Vec<NavigationLocation>,
}

impl CreateFolderOutcome {
    fn new(created_folder: NavigationLocation, affected_folders: Vec<NavigationLocation>) -> Self {
        Self {
            created_folder,
            affected_folders,
        }
    }
}

pub struct ExplorerApp<F, S> {
    state: ExplorerState,
    file_system: F,
    shell: S,
    next_tab_id: u64,
    next_search_run_id: u64,
}

impl<F, S> ExplorerApp<F, S> {
    pub fn new(start_location: NavigationLocation, file_system: F, shell: S) -> Self {
        Self::with_user_settings(start_location, file_system, shell, UserSettings::default())
    }

    pub fn with_user_settings(
        start_location: NavigationLocation,
        file_system: F,
        shell: S,
        settings: UserSettings,
    ) -> Self {
        let first_tab_id = TabId(1);
        Self {
            state: ExplorerState {
                tabs: vec![TabState::new(first_tab_id, start_location)],
                active_tab_id: first_tab_id,
                closed_tabs: Vec::new(),
                bookmarks: settings.bookmarks,
                display_options: settings.display_options,
                appearance_theme: settings.appearance_theme,
                appearance_font: settings.appearance_font,
                startup_folder: settings.startup_folder,
                restore_tabs_on_startup: settings.restore_tabs_on_startup,
            },
            file_system,
            shell,
            next_tab_id: 2,
            next_search_run_id: 1,
        }
    }
}

impl<F, S> ExplorerApp<F, S>
where
    F: LocationAccessGateway,
{
    pub fn new_at_accessible_start<I>(
        start_locations: I,
        file_system: F,
        shell: S,
    ) -> ExplorerResult<Self>
    where
        I: IntoIterator<Item = NavigationLocation>,
    {
        Self::new_at_accessible_start_with_settings(
            start_locations,
            file_system,
            shell,
            UserSettings::default(),
        )
    }

    pub fn new_at_accessible_start_with_settings<I>(
        start_locations: I,
        file_system: F,
        shell: S,
        settings: UserSettings,
    ) -> ExplorerResult<Self>
    where
        I: IntoIterator<Item = NavigationLocation>,
    {
        let mut first_error = None;

        let UserSettings {
            bookmarks,
            display_options,
            appearance_theme,
            appearance_font,
            startup_folder,
            restore_tabs_on_startup,
            session,
        } = settings;
        let settings = UserSettings {
            bookmarks,
            display_options,
            appearance_theme,
            appearance_font,
            startup_folder,
            restore_tabs_on_startup,
            session: UserSession::default(),
        };

        let mut access_cache = HashMap::new();

        if restore_tabs_on_startup {
            let mut saved_start_location = None;
            for tab in &session.tabs {
                let location = tab.current_location();
                let accessible =
                    if let Some(accessible) = access_cache.get(location.as_path()).copied() {
                        accessible
                    } else {
                        let accessible = match file_system.ensure_accessible(location) {
                            Ok(()) => true,
                            Err(error) => {
                                if first_error.is_none() {
                                    first_error = Some(error);
                                }
                                false
                            }
                        };
                        access_cache.insert(location.as_path().to_path_buf(), accessible);
                        accessible
                    };

                if accessible {
                    saved_start_location = Some(location.clone());
                    break;
                }
            }

            if let Some(start_location) = saved_start_location {
                let mut app =
                    Self::with_user_settings(start_location, file_system, shell, settings);
                app.apply_startup_user_session(session, access_cache)?;
                return Ok(app);
            }
        }

        for location in start_locations {
            match file_system.ensure_accessible(&location) {
                Ok(()) => {
                    access_cache.insert(location.as_path().to_path_buf(), true);
                    let mut app = Self::with_user_settings(location, file_system, shell, settings);
                    app.apply_startup_user_session(session, access_cache)?;
                    return Ok(app);
                }
                Err(error) if first_error.is_none() => first_error = Some(error),
                Err(_) => {}
            }
        }

        Err(first_error.unwrap_or_else(|| {
            ExplorerError::invalid_input("접근 가능한 시작 위치를 찾을 수 없습니다.")
        }))
    }

    pub fn new_at_accessible_start_deferring_startup_session<I>(
        start_locations: I,
        file_system: F,
        shell: S,
        settings: UserSettings,
    ) -> ExplorerResult<(Self, Option<StartupSessionRestore>)>
    where
        I: IntoIterator<Item = NavigationLocation>,
    {
        let mut first_error = None;

        let UserSettings {
            bookmarks,
            display_options,
            appearance_theme,
            appearance_font,
            startup_folder,
            restore_tabs_on_startup,
            session,
        } = settings;
        let settings = UserSettings {
            bookmarks,
            display_options,
            appearance_theme,
            appearance_font,
            startup_folder,
            restore_tabs_on_startup,
            session: UserSession::default(),
        };

        let mut access_cache = HashMap::new();

        if restore_tabs_on_startup {
            if let Some(location) = active_session_location(&session) {
                if ensure_accessible_with_cache(
                    &file_system,
                    location,
                    &mut access_cache,
                    &mut first_error,
                ) {
                    let start_location = location.clone();
                    let restore =
                        startup_session_restore(restore_tabs_on_startup, session, access_cache);
                    let app =
                        Self::with_user_settings(start_location, file_system, shell, settings);
                    return Ok((app, restore));
                }
            }
        }

        for location in start_locations {
            if ensure_accessible_with_cache(
                &file_system,
                &location,
                &mut access_cache,
                &mut first_error,
            ) {
                let restore =
                    startup_session_restore(restore_tabs_on_startup, session, access_cache);
                let app = Self::with_user_settings(location, file_system, shell, settings);
                return Ok((app, restore));
            }
        }

        if restore_tabs_on_startup {
            for tab in &session.tabs {
                let location = tab.current_location();
                if ensure_accessible_with_cache(
                    &file_system,
                    location,
                    &mut access_cache,
                    &mut first_error,
                ) {
                    let start_location = location.clone();
                    let restore =
                        startup_session_restore(restore_tabs_on_startup, session, access_cache);
                    let app =
                        Self::with_user_settings(start_location, file_system, shell, settings);
                    return Ok((app, restore));
                }
            }
        }

        Err(first_error.unwrap_or_else(|| {
            ExplorerError::invalid_input("접근 가능한 시작 위치를 찾을 수 없습니다.")
        }))
    }

    pub fn apply_user_settings(&mut self, settings: UserSettings) -> ExplorerResult<()> {
        let UserSettings {
            bookmarks,
            display_options,
            appearance_theme,
            appearance_font,
            startup_folder,
            restore_tabs_on_startup,
            session,
        } = settings;
        self.state.bookmarks = bookmarks;
        self.state.display_options = display_options;
        self.state.appearance_theme = appearance_theme;
        self.state.appearance_font = appearance_font;
        self.state.startup_folder = startup_folder;
        self.state.restore_tabs_on_startup = restore_tabs_on_startup;
        self.apply_user_session(session)
    }
}

impl<F, S> ExplorerApp<F, S> {
    pub fn user_settings(&self) -> UserSettings {
        UserSettings {
            bookmarks: self.state.bookmarks.clone(),
            display_options: self.state.display_options,
            appearance_theme: self.state.appearance_theme,
            appearance_font: self.state.appearance_font.clone(),
            startup_folder: self.state.startup_folder.clone(),
            restore_tabs_on_startup: self.state.restore_tabs_on_startup,
            session: self.user_session(),
        }
    }
}

impl<F, S> ExplorerApp<F, S>
where
    F: LocationAccessGateway,
{
    pub fn apply_deferred_startup_restore(
        &mut self,
        restore: StartupSessionRestore,
    ) -> ExplorerResult<()> {
        self.apply_startup_user_session(restore.session, restore.access_cache)
    }

    pub fn load_user_settings<G>(&mut self, gateway: &G) -> ExplorerResult<()>
    where
        G: UserSettingsGateway,
    {
        let settings = gateway.load_user_settings()?;
        self.apply_user_settings(settings)
    }
}

impl<F, S> ExplorerApp<F, S> {
    pub fn save_user_settings<G>(&self, gateway: &G) -> ExplorerResult<()>
    where
        G: UserSettingsGateway,
    {
        gateway.save_user_settings(&self.user_settings())
    }

    pub fn display_options(&self) -> DisplayOptions {
        self.state.display_options
    }

    pub fn set_show_hidden(&mut self, show_hidden: bool) {
        self.state.display_options.show_hidden = show_hidden;
    }

    pub fn set_show_system(&mut self, show_system: bool) {
        self.state.display_options.show_system = show_system;
    }

    pub fn appearance_theme(&self) -> AppearanceTheme {
        self.state.appearance_theme
    }

    pub fn set_appearance_theme(&mut self, theme: AppearanceTheme) {
        self.state.appearance_theme = theme;
    }

    pub fn appearance_font(&self) -> &AppearanceFont {
        &self.state.appearance_font
    }

    pub fn set_appearance_font(&mut self, font: AppearanceFont) {
        self.state.appearance_font = font;
    }

    pub fn startup_folder(&self) -> Option<&NavigationLocation> {
        self.state.startup_folder.as_ref()
    }

    pub fn set_startup_folder(&mut self, startup_folder: Option<NavigationLocation>) {
        self.state.startup_folder = startup_folder;
    }

    pub fn set_restore_tabs_on_startup(&mut self, restore_tabs_on_startup: bool) {
        self.state.restore_tabs_on_startup = restore_tabs_on_startup;
    }

    pub fn set_active_sort_key(&mut self, key: SortKey) -> ExplorerResult<()> {
        self.active_tab_mut()?.sort.key = key;
        Ok(())
    }

    pub fn set_active_sort_direction(&mut self, direction: SortDirection) -> ExplorerResult<()> {
        self.active_tab_mut()?.sort.direction = direction;
        Ok(())
    }

    pub fn state(&self) -> &ExplorerState {
        &self.state
    }

    pub fn active_tab(&self) -> ExplorerResult<&TabState> {
        let active_id = self.state.active_tab_id;
        self.state
            .tabs
            .iter()
            .find(|tab| tab.id == active_id)
            .ok_or_else(|| ExplorerError::state_conflict("활성 탭을 찾을 수 없습니다."))
    }

    pub fn active_tab_mut(&mut self) -> ExplorerResult<&mut TabState> {
        let active_id = self.state.active_tab_id;
        self.state
            .tabs
            .iter_mut()
            .find(|tab| tab.id == active_id)
            .ok_or_else(|| ExplorerError::state_conflict("활성 탭을 찾을 수 없습니다."))
    }

    pub fn active_tab_index(&self) -> ExplorerResult<usize> {
        let active_id = self.state.active_tab_id;
        self.state
            .tabs
            .iter()
            .position(|tab| tab.id == active_id)
            .ok_or_else(|| ExplorerError::state_conflict("활성 탭을 찾을 수 없습니다."))
    }
}

impl<F, S> ExplorerApp<F, S>
where
    F: LocationAccessGateway,
{
    pub fn open_tab(&mut self, location: NavigationLocation) -> ExplorerResult<TabId> {
        self.file_system.ensure_accessible(&location)?;
        let tab_id = self.allocate_tab_id()?;
        self.state.tabs.push(TabState::new(tab_id, location));
        self.state.active_tab_id = tab_id;
        Ok(tab_id)
    }

    pub fn open_folder_in_new_tab(&mut self, item: &FileItem) -> ExplorerResult<TabId> {
        if !item.is_folder() {
            return Err(ExplorerError::invalid_input(
                "폴더 항목만 새 탭으로 열 수 있습니다.",
            ));
        }

        self.open_tab(item.location.clone())
    }

    pub fn close_tab(&mut self, tab_id: TabId) -> ExplorerResult<()> {
        if self.state.tabs.len() == 1 {
            return Err(ExplorerError::state_conflict(
                "마지막 탭은 닫을 수 없습니다.",
            ));
        }

        let index = self
            .state
            .tabs
            .iter()
            .position(|tab| tab.id == tab_id)
            .ok_or_else(|| ExplorerError::state_conflict("닫을 탭을 찾을 수 없습니다."))?;
        let closed = self.state.tabs.remove(index);
        self.state.closed_tabs.push(closed);

        if self.state.active_tab_id == tab_id {
            let fallback_index = index.min(self.state.tabs.len().saturating_sub(1));
            let fallback = self
                .state
                .tabs
                .get(fallback_index)
                .map(|tab| tab.id)
                .ok_or_else(|| ExplorerError::state_conflict("활성화할 탭이 없습니다."))?;
            self.state.active_tab_id = fallback;
        }

        Ok(())
    }

    pub fn close_active_tab(&mut self) -> ExplorerResult<()> {
        self.close_tab(self.state.active_tab_id)
    }

    pub fn reopen_last_closed_tab(&mut self) -> ExplorerResult<TabId> {
        let tab = self
            .state
            .closed_tabs
            .last()
            .ok_or_else(|| ExplorerError::state_conflict("다시 열 닫힌 탭이 없습니다."))?;
        if self.state.tabs.iter().any(|open_tab| open_tab.id == tab.id) {
            return Err(ExplorerError::state_conflict(
                "같은 식별자의 열린 탭이 이미 있습니다.",
            ));
        }

        self.file_system.ensure_accessible(tab.current_location())?;
        let tab = self
            .state
            .closed_tabs
            .pop()
            .ok_or_else(|| ExplorerError::state_conflict("다시 열 닫힌 탭이 없습니다."))?;
        let tab_id = tab.id;
        self.state.tabs.push(tab);
        self.state.active_tab_id = tab_id;
        Ok(tab_id)
    }

    pub fn switch_tab(&mut self, tab_id: TabId) -> ExplorerResult<()> {
        if self.state.tabs.iter().any(|tab| tab.id == tab_id) {
            self.state.active_tab_id = tab_id;
            Ok(())
        } else {
            Err(ExplorerError::state_conflict(
                "전환할 탭을 찾을 수 없습니다.",
            ))
        }
    }

    pub fn switch_to_tab_index(&mut self, index: usize) -> ExplorerResult<()> {
        let tab_id = self
            .state
            .tabs
            .get(index)
            .map(|tab| tab.id)
            .ok_or_else(|| ExplorerError::state_conflict("전환할 탭을 찾을 수 없습니다."))?;
        self.switch_tab(tab_id)
    }

    pub fn move_tab(&mut self, tab_id: TabId, new_index: usize) -> ExplorerResult<()> {
        if new_index >= self.state.tabs.len() {
            return Err(ExplorerError::state_conflict(
                "탭을 이동할 위치가 열린 탭 범위를 벗어났습니다.",
            ));
        }

        let current_index = self
            .state
            .tabs
            .iter()
            .position(|tab| tab.id == tab_id)
            .ok_or_else(|| ExplorerError::state_conflict("이동할 탭을 찾을 수 없습니다."))?;

        if current_index == new_index {
            return Ok(());
        }

        let tab = self.state.tabs.remove(current_index);
        self.state.tabs.insert(new_index, tab);
        Ok(())
    }

    pub fn navigate_active(&mut self, location: NavigationLocation) -> ExplorerResult<()> {
        self.file_system.ensure_accessible(&location)?;
        self.active_tab_mut()?.navigate_to(location);
        Ok(())
    }

    pub fn navigate_active_path(&mut self, path: impl Into<PathBuf>) -> ExplorerResult<()> {
        self.navigate_active(NavigationLocation::from_path(path.into())?)
    }
}

impl<F, S> ExplorerApp<F, S>
where
    F: LocationAccessGateway,
    S: ShellOpenGateway,
{
    pub fn activate_item_in_active(&mut self, item: &FileItem) -> ExplorerResult<ItemActivation> {
        if item.is_folder() {
            self.navigate_active(item.location.clone())?;
            Ok(ItemActivation::Navigated)
        } else {
            self.shell.open_path(&item.location)?;
            Ok(ItemActivation::Opened)
        }
    }
}

impl<F, S> ExplorerApp<F, S>
where
    F: LocationAccessGateway,
{
    pub fn go_back(&mut self) -> ExplorerResult<()> {
        let location = self.active_tab()?.back_location()?.clone();
        self.file_system.ensure_accessible(&location)?;
        self.active_tab_mut()?.go_back()?;
        Ok(())
    }

    pub fn go_forward(&mut self) -> ExplorerResult<()> {
        let location = self.active_tab()?.forward_location()?.clone();
        self.file_system.ensure_accessible(&location)?;
        self.active_tab_mut()?.go_forward()?;
        Ok(())
    }

    pub fn go_up(&mut self) -> ExplorerResult<()> {
        let location = self.active_tab()?.current_location().parent()?;
        if let Some(location) = location {
            self.file_system.ensure_accessible(&location)?;
            self.active_tab_mut()?.navigate_to(location);
        }
        Ok(())
    }
}

impl<F, S> ExplorerApp<F, S>
where
    F: ItemListingGateway,
{
    pub fn list_active_items(&self) -> ExplorerResult<Vec<FileItem>> {
        let active_tab = self.active_tab()?;
        self.file_system.list_items(
            active_tab.current_location(),
            self.state.display_options,
            active_tab.sort,
        )
    }
}

impl<F, S> ExplorerApp<F, S>
where
    F: FolderTreeGateway,
{
    pub fn folder_tree_roots(&self) -> ExplorerResult<Vec<FolderTreeItem>> {
        let mut roots = Vec::new();

        for kind in DEFAULT_FOLDER_TREE_KNOWN_FOLDERS {
            let location = self.file_system.known_folder(kind)?;
            roots.push(FolderTreeItem::known_folder(kind, location, None)?);
        }

        for location in self.file_system.drive_roots()? {
            roots.push(FolderTreeItem::drive_root(location, None)?);
        }

        for bookmark in self.state.bookmarks.items() {
            roots.push(FolderTreeItem::bookmark(bookmark)?);
        }

        Ok(roots)
    }

    pub fn folder_tree_children(
        &self,
        location: &NavigationLocation,
    ) -> ExplorerResult<Vec<FolderTreeItem>> {
        self.file_system
            .list_child_folders(location, self.state.display_options, SortState::default())?
            .iter()
            .map(|item| FolderTreeItem::folder_child(item, 1, true))
            .collect()
    }
}

impl<F, S> ExplorerApp<F, S>
where
    F: FolderTreeChildPresenceGateway,
{
    pub fn folder_tree_has_child_folders(
        &self,
        location: &NavigationLocation,
    ) -> ExplorerResult<bool> {
        self.file_system
            .has_child_folders(location, self.state.display_options)
    }
}

struct SearchStateTransitions;

impl SearchStateTransitions {
    fn start(tab: &mut TabState, run_id: SearchRunId, criteria: &SearchCriteria) {
        tab.search = SearchState::Running {
            run_id,
            criteria: criteria.clone(),
            progress: SearchProgress::default(),
            cancel_requested: false,
        };
    }

    fn request_cancel(tab: &mut TabState) -> Option<SearchRunId> {
        match &mut tab.search {
            SearchState::Running {
                run_id,
                cancel_requested,
                ..
            } => {
                *cancel_requested = true;
                Some(*run_id)
            }
            _ => None,
        }
    }

    fn update_progress(tab: &mut TabState, run_id: SearchRunId, progress: SearchProgress) -> bool {
        match &mut tab.search {
            SearchState::Running {
                run_id: active_run_id,
                progress: active_progress,
                ..
            } if *active_run_id == run_id => {
                *active_progress = progress;
                true
            }
            _ => false,
        }
    }

    fn finish(tab: &mut TabState, outcome: SearchOutcome) -> bool {
        let cancel_requested = match &tab.search {
            SearchState::Running {
                run_id,
                cancel_requested,
                ..
            } if *run_id == outcome.run_id => *cancel_requested,
            _ => return false,
        };

        tab.search = if outcome.cancelled || cancel_requested {
            SearchState::Cancelled {
                criteria: outcome.criteria,
                partial_items: outcome.items,
                diagnostics: outcome.diagnostics,
                progress: outcome.progress,
            }
        } else {
            SearchState::Results {
                criteria: outcome.criteria,
                items: outcome.items,
                diagnostics: outcome.diagnostics,
                progress: outcome.progress,
            }
        };

        true
    }

    fn fail(tab: &mut TabState, run_id: SearchRunId) -> bool {
        match &tab.search {
            SearchState::Running {
                run_id: active_run_id,
                ..
            } if *active_run_id == run_id => {
                tab.search = SearchState::Idle;
                true
            }
            _ => false,
        }
    }

    fn clear(tab: &mut TabState) {
        tab.search = SearchState::Idle;
    }
}

// Search state use cases.
impl<F, S> ExplorerApp<F, S>
where
    F: LocationAccessGateway,
{
    pub fn start_search_in_active(
        &mut self,
        criteria: SearchCriteria,
    ) -> ExplorerResult<SearchRequest> {
        let tab_id = self.state.active_tab_id;
        let root = self.active_tab()?.current_location().clone();
        self.file_system.ensure_accessible(&root)?;
        let sort = self.active_tab()?.sort;
        let run_id = self.allocate_search_run_id()?;
        let display_options = self.state.display_options;

        SearchStateTransitions::start(self.active_tab_mut()?, run_id, &criteria);

        Ok(SearchRequest {
            run_id,
            tab_id,
            root,
            criteria,
            display_options,
            sort,
        })
    }

    pub fn request_active_search_cancel(&mut self) -> ExplorerResult<Option<SearchRunId>> {
        self.request_search_cancel(self.state.active_tab_id)
    }

    pub fn request_search_cancel(&mut self, tab_id: TabId) -> ExplorerResult<Option<SearchRunId>> {
        let tab = self.tab_mut(tab_id)?;
        Ok(SearchStateTransitions::request_cancel(tab))
    }

    pub fn update_search_progress(
        &mut self,
        tab_id: TabId,
        run_id: SearchRunId,
        progress: SearchProgress,
    ) -> ExplorerResult<bool> {
        let Some(tab) = self.try_tab_mut(tab_id) else {
            return Ok(false);
        };
        Ok(SearchStateTransitions::update_progress(
            tab, run_id, progress,
        ))
    }

    pub fn finish_search(&mut self, outcome: SearchOutcome) -> ExplorerResult<bool> {
        let Some(tab) = self.try_tab_mut(outcome.tab_id) else {
            return Ok(false);
        };
        Ok(SearchStateTransitions::finish(tab, outcome))
    }

    pub fn fail_search(&mut self, tab_id: TabId, run_id: SearchRunId) -> ExplorerResult<bool> {
        let Some(tab) = self.try_tab_mut(tab_id) else {
            return Ok(false);
        };
        Ok(SearchStateTransitions::fail(tab, run_id))
    }

    pub fn clear_active_search(&mut self) -> ExplorerResult<()> {
        SearchStateTransitions::clear(self.active_tab_mut()?);
        Ok(())
    }
}

impl<F, S> ExplorerApp<F, S>
where
    F: FolderCreationGateway,
{
    pub fn create_folder_in_active(
        &mut self,
        name: &OsStr,
        select_created: bool,
    ) -> ExplorerResult<CreateFolderOutcome> {
        let name = NewFolderName::new(name)?;
        let parent = self.active_tab()?.current_location().clone();
        let created_folder = self.file_system.create_folder(&parent, &name)?;
        if select_created {
            self.active_tab_mut()?.select_only(created_folder.clone());
        }

        Ok(CreateFolderOutcome::new(created_folder, vec![parent]))
    }
}

impl<F, S> ExplorerApp<F, S>
where
    S: ShellOpenGateway,
{
    pub fn open_location(&self, location: &NavigationLocation) -> ExplorerResult<()> {
        self.shell.open_path(location)
    }
}

impl<F, S> ExplorerApp<F, S>
where
    S: ShellOpenWithGateway,
{
    pub fn open_with(&self, location: &NavigationLocation) -> ExplorerResult<()> {
        self.shell.open_with(location)
    }

    pub fn open_item_with_picker(&self, item: &FileItem) -> ExplorerResult<()> {
        if item.is_folder() {
            return Err(ExplorerError::invalid_input(
                "파일 항목만 연결 프로그램을 선택할 수 있습니다.",
            ));
        }

        self.open_with(&item.location)
    }
}

impl<F, S> ExplorerApp<F, S>
where
    S: ShellPropertiesGateway,
{
    pub fn show_properties(&self, location: &NavigationLocation) -> ExplorerResult<()> {
        self.shell.show_properties(location)
    }
}

impl<F, S> ExplorerApp<F, S>
where
    S: ShellContextMenuGateway,
{
    pub fn show_context_menu_for_items(
        &self,
        targets: &[NavigationLocation],
        position: ContextMenuPosition,
    ) -> ExplorerResult<ContextMenuOutcome> {
        ensure_selection(targets)?;
        self.shell.show_context_menu(targets, position)
    }

    pub fn show_context_menu_for_folder_background(
        &self,
        folder: &NavigationLocation,
        position: ContextMenuPosition,
    ) -> ExplorerResult<ContextMenuOutcome> {
        self.shell
            .show_folder_background_context_menu(folder, position)
    }
}

impl<F, S> ExplorerApp<F, S> {
    pub fn add_bookmark(
        &mut self,
        location: NavigationLocation,
        display_name: Option<OsString>,
    ) -> BookmarkAddOutcome {
        self.state.bookmarks.add(location, display_name)
    }

    pub fn add_active_location_bookmark(
        &mut self,
        display_name: Option<OsString>,
    ) -> ExplorerResult<BookmarkAddOutcome> {
        let location = self.active_tab()?.current_location().clone();
        Ok(self.add_bookmark(location, display_name))
    }

    pub fn add_selected_folder_bookmark(
        &mut self,
        item: &FileItem,
        display_name: Option<OsString>,
    ) -> ExplorerResult<BookmarkAddOutcome> {
        if !item.is_folder() {
            return Err(ExplorerError::invalid_input(
                "폴더 항목만 북마크로 추가할 수 있습니다.",
            ));
        }

        Ok(self.add_bookmark(item.location.clone(), display_name))
    }

    pub fn rename_bookmark(&mut self, index: usize, display_name: OsString) -> ExplorerResult<()> {
        self.state.bookmarks.rename(index, display_name)
    }

    pub fn bookmark_index_for_location(&self, location: &NavigationLocation) -> Option<usize> {
        self.state.bookmarks.index_of_target(location)
    }

    pub fn delete_bookmark(&mut self, index: usize) -> ExplorerResult<BookmarkItem> {
        self.state.bookmarks.remove(index)
    }

    pub fn move_bookmark(&mut self, from_index: usize, to_index: usize) -> ExplorerResult<()> {
        self.state.bookmarks.move_item(from_index, to_index)
    }
}

impl<F, S> ExplorerApp<F, S>
where
    F: LocationAccessGateway,
{
    pub fn navigate_active_to_bookmark(&mut self, index: usize) -> ExplorerResult<()> {
        let location = self.state.bookmarks.get(index)?.target.clone();

        match self.file_system.ensure_accessible(&location) {
            Ok(()) => {
                self.active_tab_mut()?.navigate_to(location);
                self.state
                    .bookmarks
                    .mark_selected(index, SystemTime::now())?;
                Ok(())
            }
            Err(error) => {
                self.state
                    .bookmarks
                    .mark_accessibility(index, BookmarkAccessibility::Inaccessible)?;
                Err(error)
            }
        }
    }
}

// Shell file operation use cases.
impl<F, S> ExplorerApp<F, S>
where
    F: LocationAccessGateway,
    S: ShellTransferGateway,
{
    pub fn copy_items_to(
        &self,
        selected_items: &[NavigationLocation],
        destination: &NavigationLocation,
    ) -> ExplorerResult<FileOperationOutcome> {
        ensure_selection(selected_items)?;
        self.file_system.ensure_accessible(destination)?;
        self.shell.copy_items(selected_items, destination)?;
        Ok(FileOperationOutcome::new(file_transfer_refresh_locations(
            selected_items,
            destination,
            DropOperation::Copy,
        )?))
    }

    pub fn move_items_to(
        &self,
        selected_items: &[NavigationLocation],
        destination: &NavigationLocation,
    ) -> ExplorerResult<FileOperationOutcome> {
        ensure_selection(selected_items)?;
        self.file_system.ensure_accessible(destination)?;
        validate_move_drop(selected_items, destination)?;
        self.shell.move_items(selected_items, destination)?;
        Ok(FileOperationOutcome::new(file_transfer_refresh_locations(
            selected_items,
            destination,
            DropOperation::Move,
        )?))
    }

    pub fn copy_to_active(
        &self,
        sources: &[NavigationLocation],
    ) -> ExplorerResult<FileOperationOutcome> {
        let destination = self.active_tab()?.current_location().clone();
        self.copy_items_to(sources, &destination)
    }

    pub fn move_to_active(
        &self,
        sources: &[NavigationLocation],
    ) -> ExplorerResult<FileOperationOutcome> {
        let destination = self.active_tab()?.current_location().clone();
        self.move_items_to(sources, &destination)
    }

    pub fn prepare_file_drop(
        &self,
        sources: &[NavigationLocation],
        destination: &NavigationLocation,
        source_kind: DropSourceKind,
        modifiers: DropModifierKeys,
    ) -> ExplorerResult<FileDropPlan> {
        ensure_selection(sources)?;
        self.file_system.ensure_accessible(destination)?;
        let operation = decide_drop_operation(sources, destination, source_kind, modifiers);
        if operation == DropOperation::Move {
            validate_move_drop(sources, destination)?;
        }

        Ok(FileDropPlan { operation })
    }
}

impl<F, S> ExplorerApp<F, S>
where
    S: ShellDeleteGateway,
{
    pub fn delete_to_recycle_bin(
        &self,
        targets: &[NavigationLocation],
    ) -> ExplorerResult<FileOperationOutcome> {
        ensure_selection(targets)?;
        self.shell.delete_to_recycle_bin(targets)?;
        Ok(FileOperationOutcome::new(source_parent_locations(targets)?))
    }

    pub fn delete_permanently(
        &self,
        targets: &[NavigationLocation],
    ) -> ExplorerResult<FileOperationOutcome> {
        ensure_selection(targets)?;
        self.shell.delete_permanently(targets)?;
        Ok(FileOperationOutcome::new(source_parent_locations(targets)?))
    }
}

impl<F, S> ExplorerApp<F, S>
where
    S: ShellRenameGateway,
{
    pub fn rename_item(
        &self,
        target: &NavigationLocation,
        new_name: &OsStr,
    ) -> ExplorerResult<FileOperationOutcome> {
        let new_name = RenameItemName::new(new_name)?;
        self.shell.rename_item(target, &new_name)?;
        let affected_folders = match target.parent()? {
            Some(parent) => vec![parent],
            None => Vec::new(),
        };
        Ok(FileOperationOutcome::new(affected_folders))
    }
}

impl<F, S> ExplorerApp<F, S>
where
    F: FolderTreeGateway,
{
    pub fn drive_roots(&self) -> ExplorerResult<Vec<NavigationLocation>> {
        self.file_system.drive_roots()
    }

    pub fn known_folder(&self, kind: KnownFolderKind) -> ExplorerResult<NavigationLocation> {
        self.file_system.known_folder(kind)
    }
}

impl<F, S> ExplorerApp<F, S> {
    fn user_session(&self) -> UserSession {
        UserSession {
            tabs: self.state.tabs.iter().map(session_tab).collect(),
            active_tab_id: Some(self.state.active_tab_id),
            closed_tabs: self.state.closed_tabs.iter().map(session_tab).collect(),
        }
    }
}

impl<F, S> ExplorerApp<F, S>
where
    F: LocationAccessGateway,
{
    fn apply_user_session(&mut self, session: UserSession) -> ExplorerResult<()> {
        self.apply_user_session_with_access_cache(session, None)
    }

    fn apply_startup_user_session(
        &mut self,
        session: UserSession,
        mut access_cache: HashMap<PathBuf, bool>,
    ) -> ExplorerResult<()> {
        let UserSession {
            tabs,
            active_tab_id,
            closed_tabs,
        } = session;

        if self.state.restore_tabs_on_startup {
            self.restore_accessible_open_tabs_from_session(tabs, active_tab_id, &mut access_cache)?;
        }
        self.restore_closed_tabs_from_session(closed_tabs)
    }

    fn apply_user_session_with_access_cache(
        &mut self,
        session: UserSession,
        known_accessible_path: Option<PathBuf>,
    ) -> ExplorerResult<()> {
        let UserSession {
            tabs,
            active_tab_id,
            closed_tabs,
        } = session;

        let mut access_cache = HashMap::new();
        if let Some(path) = known_accessible_path {
            access_cache.insert(path, true);
        }

        if self.state.restore_tabs_on_startup {
            self.restore_accessible_open_tabs_from_session(tabs, active_tab_id, &mut access_cache)?;
        }
        self.restore_closed_tabs_from_session(closed_tabs)
    }

    fn restore_accessible_open_tabs_from_session(
        &mut self,
        tabs: Vec<TabState>,
        active_tab_id: Option<TabId>,
        access_cache: &mut HashMap<PathBuf, bool>,
    ) -> ExplorerResult<()> {
        let accessible_tabs = tabs
            .into_iter()
            .filter(|tab| {
                self.is_location_accessible_with_cache(tab.current_location(), access_cache)
            })
            .collect();
        self.restore_open_tabs_from_session(accessible_tabs, active_tab_id)
    }

    fn restore_open_tabs_from_session(
        &mut self,
        tabs: Vec<TabState>,
        active_tab_id: Option<TabId>,
    ) -> ExplorerResult<()> {
        let mut restored_tabs = Vec::new();
        let mut restored_active_tab_id = None;
        let mut next_id = 1_u64;

        for tab in tabs {
            let restored_id = TabId(next_id);
            next_id = next_tab_id_after(next_id)?;
            if Some(tab.id) == active_tab_id {
                restored_active_tab_id = Some(restored_id);
            }
            restored_tabs.push(tab.with_id(restored_id));
        }

        if restored_tabs.is_empty() {
            self.next_tab_id = self.next_available_tab_id()?;
            return Ok(());
        }

        let active_tab_id = restored_active_tab_id.unwrap_or(restored_tabs[0].id);
        self.state.tabs = restored_tabs;
        self.state.active_tab_id = active_tab_id;
        self.next_tab_id = next_id;
        Ok(())
    }

    fn is_location_accessible_with_cache(
        &self,
        location: &NavigationLocation,
        access_cache: &mut HashMap<PathBuf, bool>,
    ) -> bool {
        if let Some(accessible) = access_cache.get(location.as_path()).copied() {
            return accessible;
        }

        let accessible = self.file_system.ensure_accessible(location).is_ok();
        access_cache.insert(location.as_path().to_path_buf(), accessible);
        accessible
    }

    fn restore_closed_tabs_from_session(&mut self, tabs: Vec<TabState>) -> ExplorerResult<()> {
        let mut next_id = self.next_available_tab_id()?;
        let mut restored_tabs = Vec::with_capacity(tabs.len());

        for tab in tabs {
            let restored_id = TabId(next_id);
            next_id = next_tab_id_after(next_id)?;
            restored_tabs.push(tab.with_id(restored_id));
        }

        self.state.closed_tabs = restored_tabs;
        self.next_tab_id = next_id;
        Ok(())
    }

    fn next_available_tab_id(&self) -> ExplorerResult<u64> {
        let max_id = self
            .state
            .tabs
            .iter()
            .chain(self.state.closed_tabs.iter())
            .map(|tab| tab.id.0)
            .max()
            .unwrap_or(0);
        next_tab_id_after(max_id)
    }

    fn allocate_tab_id(&mut self) -> ExplorerResult<TabId> {
        let id = self.next_tab_id;
        self.next_tab_id = next_tab_id_after(self.next_tab_id)?;
        Ok(TabId(id))
    }

    fn allocate_search_run_id(&mut self) -> ExplorerResult<SearchRunId> {
        let id = self.next_search_run_id;
        self.next_search_run_id = self
            .next_search_run_id
            .checked_add(1)
            .ok_or_else(|| ExplorerError::state_conflict("검색 식별자를 더 만들 수 없습니다."))?;
        Ok(SearchRunId(id))
    }

    fn tab_mut(&mut self, tab_id: TabId) -> ExplorerResult<&mut TabState> {
        self.try_tab_mut(tab_id)
            .ok_or_else(|| ExplorerError::state_conflict("탭을 찾을 수 없습니다."))
    }

    fn try_tab_mut(&mut self, tab_id: TabId) -> Option<&mut TabState> {
        self.state.tabs.iter_mut().find(|tab| tab.id == tab_id)
    }
}

fn ensure_selection(selected_items: &[NavigationLocation]) -> ExplorerResult<()> {
    if selected_items.is_empty() {
        Err(ExplorerError::invalid_input(
            "선택된 파일 또는 폴더가 없습니다.",
        ))
    } else {
        Ok(())
    }
}

fn session_tab(tab: &TabState) -> TabState {
    TabState::from_parts(
        tab.id,
        tab.current_location().clone(),
        tab.back_history().to_vec(),
        tab.forward_history().to_vec(),
        tab.sort,
    )
}

fn active_session_location(session: &UserSession) -> Option<&NavigationLocation> {
    let active_tab_id = session.active_tab_id?;
    session
        .tabs
        .iter()
        .find(|tab| tab.id == active_tab_id)
        .map(TabState::current_location)
}

fn ensure_accessible_with_cache<F>(
    file_system: &F,
    location: &NavigationLocation,
    access_cache: &mut HashMap<PathBuf, bool>,
    first_error: &mut Option<ExplorerError>,
) -> bool
where
    F: LocationAccessGateway,
{
    if let Some(accessible) = access_cache.get(location.as_path()).copied() {
        return accessible;
    }

    let accessible = match file_system.ensure_accessible(location) {
        Ok(()) => true,
        Err(error) => {
            if first_error.is_none() {
                *first_error = Some(error);
            }
            false
        }
    };
    access_cache.insert(location.as_path().to_path_buf(), accessible);
    accessible
}

fn startup_session_restore(
    restore_tabs_on_startup: bool,
    session: UserSession,
    access_cache: HashMap<PathBuf, bool>,
) -> Option<StartupSessionRestore> {
    let restore_open_tabs = restore_tabs_on_startup && !session.tabs.is_empty();
    if !restore_open_tabs && session.closed_tabs.is_empty() {
        return None;
    }

    Some(StartupSessionRestore {
        session,
        access_cache,
    })
}

fn next_tab_id_after(id: u64) -> ExplorerResult<u64> {
    id.checked_add(1)
        .ok_or_else(|| ExplorerError::state_conflict("탭 식별자를 더 만들 수 없습니다."))
}

pub fn unsupported_shell_operation(operation: ShellOperation) -> ExplorerError {
    ExplorerError::unsupported(
        "shell file operation",
        format!("{operation} is reserved for the IFileOperation bridge"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{
        sort_file_items, BookmarkAccessibility, FileAttributes, FileItemKind, FileNameErrorKind,
        FolderTreeItemKind, SearchCriteria, SearchState,
    };
    use std::cell::RefCell;
    use std::ffi::OsString;
    use std::os::windows::ffi::OsStringExt;
    use std::path::Path;
    use std::rc::Rc;

    type Shared<T> = Rc<RefCell<T>>;
    type PathList = Vec<PathBuf>;
    type ContextMenuLog = Vec<(PathList, ContextMenuPosition)>;
    type TransferLog = Vec<(PathList, PathBuf)>;

    fn tree_navigation_failure_cases() -> [(&'static str, u32, &'static str); 3] {
        [
            (
                r"C:\blocked",
                5,
                "권한이 없어 작업을 완료할 수 없습니다.",
            ),
            (r"C:\missing", 3, "위치를 찾을 수 없습니다."),
            (
                r"\\offline\share",
                53,
                "네트워크 위치에 연결할 수 없습니다. 서버 이름, 공유 이름 또는 네트워크 연결을 확인해 주세요.",
            ),
        ]
    }

    #[derive(Debug, Clone, Default)]
    struct FakeFileSystemGateway {
        checked: Shared<Vec<PathBuf>>,
        inaccessible: Shared<Vec<(PathBuf, u32)>>,
        created_folders: Shared<Vec<(PathBuf, OsString)>>,
        listed: Shared<Vec<(PathBuf, DisplayOptions, SortState)>>,
        child_listed: Shared<Vec<(PathBuf, DisplayOptions, SortState)>>,
        child_presence_checked: Shared<Vec<(PathBuf, DisplayOptions)>>,
        items: Shared<Vec<(PathBuf, Vec<FileItem>)>>,
        drive_roots: Shared<Vec<NavigationLocation>>,
    }

    impl FakeFileSystemGateway {
        fn deny(&self, path: impl Into<PathBuf>) {
            self.fail_with_code(path, 5);
        }

        fn fail_with_code(&self, path: impl Into<PathBuf>, code: u32) {
            self.inaccessible.borrow_mut().push((path.into(), code));
        }

        fn add_items(&self, parent: impl AsRef<Path>, items: Vec<FileItem>) {
            self.items
                .borrow_mut()
                .push((parent.as_ref().to_path_buf(), items));
        }

        fn set_drive_roots(&self, roots: Vec<NavigationLocation>) {
            *self.drive_roots.borrow_mut() = roots;
        }

        fn configured_items(
            &self,
            location: &NavigationLocation,
            options: DisplayOptions,
            sort: SortState,
            mut include_item: impl FnMut(&FileItem) -> bool,
        ) -> ExplorerResult<Vec<FileItem>> {
            self.ensure_accessible(location)?;
            let source_items = self
                .items
                .borrow()
                .iter()
                .find(|(parent, _)| parent.as_path() == location.as_path())
                .map(|(_, items)| items.clone())
                .unwrap_or_default();
            let mut items = source_items
                .into_iter()
                .filter(|item| options.allows(item) && include_item(item))
                .collect::<Vec<_>>();
            sort_file_items(&mut items, sort);
            Ok(items)
        }
    }

    impl ItemListingGateway for FakeFileSystemGateway {
        fn list_items(
            &self,
            location: &NavigationLocation,
            options: DisplayOptions,
            sort: SortState,
        ) -> ExplorerResult<Vec<FileItem>> {
            self.listed
                .borrow_mut()
                .push((location.as_path().to_path_buf(), options, sort));
            self.configured_items(location, options, sort, |_| true)
        }
    }

    impl FolderTreeGateway for FakeFileSystemGateway {
        fn list_child_folders(
            &self,
            location: &NavigationLocation,
            options: DisplayOptions,
            sort: SortState,
        ) -> ExplorerResult<Vec<FileItem>> {
            self.child_listed
                .borrow_mut()
                .push((location.as_path().to_path_buf(), options, sort));
            self.configured_items(location, options, sort, FileItem::is_folder)
        }

        fn drive_roots(&self) -> ExplorerResult<Vec<NavigationLocation>> {
            Ok(self.drive_roots.borrow().clone())
        }

        fn known_folder(&self, kind: KnownFolderKind) -> ExplorerResult<NavigationLocation> {
            let path = match kind {
                KnownFolderKind::Desktop => PathBuf::from(r"C:\Users\Test\Desktop"),
                KnownFolderKind::Downloads => PathBuf::from(r"C:\Users\Test\Downloads"),
                KnownFolderKind::Documents => PathBuf::from(r"C:\Users\Test\Documents"),
                KnownFolderKind::Home => PathBuf::from(r"C:\Users\Test"),
            };
            NavigationLocation::known_folder(kind, path)
        }
    }

    impl FolderTreeChildPresenceGateway for FakeFileSystemGateway {
        fn has_child_folders(
            &self,
            location: &NavigationLocation,
            options: DisplayOptions,
        ) -> ExplorerResult<bool> {
            self.child_presence_checked
                .borrow_mut()
                .push((location.as_path().to_path_buf(), options));
            self.ensure_accessible(location)?;

            let has_child_folder = self
                .items
                .borrow()
                .iter()
                .find(|(parent, _)| parent.as_path() == location.as_path())
                .map(|(_, items)| {
                    items
                        .iter()
                        .any(|item| options.allows(item) && item.is_folder())
                })
                .unwrap_or(false);

            Ok(has_child_folder)
        }
    }

    impl LocationAccessGateway for FakeFileSystemGateway {
        fn ensure_accessible(&self, location: &NavigationLocation) -> ExplorerResult<()> {
            let path = location.as_path();
            self.checked.borrow_mut().push(path.to_path_buf());
            if let Some((_, code)) = self
                .inaccessible
                .borrow()
                .iter()
                .find(|(blocked, _)| blocked.as_path() == path)
            {
                return Err(ExplorerError::windows_api(
                    "read file attributes",
                    "GetFileAttributesW",
                    *code,
                    Some(path.to_path_buf()),
                ));
            }

            Ok(())
        }
    }

    impl FolderCreationGateway for FakeFileSystemGateway {
        fn create_folder(
            &self,
            parent: &NavigationLocation,
            name: &NewFolderName,
        ) -> ExplorerResult<NavigationLocation> {
            self.ensure_accessible(parent)?;
            self.created_folders.borrow_mut().push((
                parent.as_path().to_path_buf(),
                name.as_os_str().to_os_string(),
            ));
            NavigationLocation::from_path(parent.as_path().join(name.as_os_str()))
        }
    }

    impl SearchFileSystemGateway for FakeFileSystemGateway {
        fn search_items(
            &self,
            _root: &NavigationLocation,
            _criteria: &SearchCriteria,
            _options: DisplayOptions,
            _sort: SortState,
            _cancellation: &dyn SearchCancellation,
            _progress: &dyn SearchProgressReporter,
        ) -> ExplorerResult<SearchFileSystemOutcome> {
            Ok(SearchFileSystemOutcome::default())
        }
    }

    impl FileSystemGateway for FakeFileSystemGateway {}

    #[derive(Debug, Clone, Default)]
    struct FakeShellGateway {
        opened: Shared<Vec<PathBuf>>,
        open_with_paths: Shared<Vec<PathBuf>>,
        context_menus: Shared<ContextMenuLog>,
        background_context_menus: Shared<ContextMenuLog>,
        copied: Shared<TransferLog>,
        moved: Shared<TransferLog>,
        deleted: Shared<Vec<PathList>>,
        permanently_deleted: Shared<Vec<PathList>>,
        renamed: Shared<Vec<(PathBuf, OsString)>>,
    }

    impl ShellOpenGateway for FakeShellGateway {
        fn open_path(&self, location: &NavigationLocation) -> ExplorerResult<()> {
            self.opened
                .borrow_mut()
                .push(location.as_path().to_path_buf());
            Ok(())
        }
    }

    impl ShellOpenWithGateway for FakeShellGateway {
        fn open_with(&self, location: &NavigationLocation) -> ExplorerResult<()> {
            self.open_with_paths
                .borrow_mut()
                .push(location.as_path().to_path_buf());
            Ok(())
        }
    }

    impl ShellPropertiesGateway for FakeShellGateway {
        fn show_properties(&self, _location: &NavigationLocation) -> ExplorerResult<()> {
            Ok(())
        }
    }

    impl ShellContextMenuGateway for FakeShellGateway {
        fn show_context_menu(
            &self,
            targets: &[NavigationLocation],
            position: ContextMenuPosition,
        ) -> ExplorerResult<ContextMenuOutcome> {
            self.context_menus.borrow_mut().push((
                targets
                    .iter()
                    .map(|location| location.as_path().to_path_buf())
                    .collect(),
                position,
            ));
            Ok(ContextMenuOutcome {
                command_invoked: true,
                refresh_current_folder: true,
            })
        }

        fn show_folder_background_context_menu(
            &self,
            folder: &NavigationLocation,
            position: ContextMenuPosition,
        ) -> ExplorerResult<ContextMenuOutcome> {
            self.background_context_menus
                .borrow_mut()
                .push((vec![folder.as_path().to_path_buf()], position));
            Ok(ContextMenuOutcome {
                command_invoked: true,
                refresh_current_folder: true,
            })
        }
    }

    impl ShellTransferGateway for FakeShellGateway {
        fn copy_items(
            &self,
            sources: &[NavigationLocation],
            destination: &NavigationLocation,
        ) -> ExplorerResult<()> {
            self.copied.borrow_mut().push((
                sources
                    .iter()
                    .map(|location| location.as_path().to_path_buf())
                    .collect(),
                destination.as_path().to_path_buf(),
            ));
            Ok(())
        }

        fn move_items(
            &self,
            sources: &[NavigationLocation],
            destination: &NavigationLocation,
        ) -> ExplorerResult<()> {
            self.moved.borrow_mut().push((
                sources
                    .iter()
                    .map(|location| location.as_path().to_path_buf())
                    .collect(),
                destination.as_path().to_path_buf(),
            ));
            Ok(())
        }
    }

    impl ShellDeleteGateway for FakeShellGateway {
        fn delete_to_recycle_bin(&self, targets: &[NavigationLocation]) -> ExplorerResult<()> {
            self.deleted.borrow_mut().push(
                targets
                    .iter()
                    .map(|location| location.as_path().to_path_buf())
                    .collect(),
            );
            Ok(())
        }

        fn delete_permanently(&self, targets: &[NavigationLocation]) -> ExplorerResult<()> {
            self.permanently_deleted.borrow_mut().push(
                targets
                    .iter()
                    .map(|location| location.as_path().to_path_buf())
                    .collect(),
            );
            Ok(())
        }
    }

    impl ShellRenameGateway for FakeShellGateway {
        fn rename_item(
            &self,
            target: &NavigationLocation,
            new_name: &RenameItemName,
        ) -> ExplorerResult<()> {
            self.renamed.borrow_mut().push((
                target.as_path().to_path_buf(),
                new_name.as_os_str().to_os_string(),
            ));
            Ok(())
        }
    }

    impl ShellFileOperationGateway for FakeShellGateway {}

    impl ShellGateway for FakeShellGateway {}

    #[derive(Debug, Clone, Default)]
    struct AccessOnlyGateway {
        checked: Shared<Vec<PathBuf>>,
    }

    impl LocationAccessGateway for AccessOnlyGateway {
        fn ensure_accessible(&self, location: &NavigationLocation) -> ExplorerResult<()> {
            self.checked
                .borrow_mut()
                .push(location.as_path().to_path_buf());
            Ok(())
        }
    }

    #[derive(Debug, Clone, Copy, Default)]
    struct NoShellGateway;

    #[derive(Debug, Clone, Default)]
    struct TransferOnlyShellGateway {
        copied: Shared<TransferLog>,
        moved: Shared<TransferLog>,
    }

    impl ShellTransferGateway for TransferOnlyShellGateway {
        fn copy_items(
            &self,
            sources: &[NavigationLocation],
            destination: &NavigationLocation,
        ) -> ExplorerResult<()> {
            self.copied.borrow_mut().push((
                sources
                    .iter()
                    .map(|location| location.as_path().to_path_buf())
                    .collect(),
                destination.as_path().to_path_buf(),
            ));
            Ok(())
        }

        fn move_items(
            &self,
            sources: &[NavigationLocation],
            destination: &NavigationLocation,
        ) -> ExplorerResult<()> {
            self.moved.borrow_mut().push((
                sources
                    .iter()
                    .map(|location| location.as_path().to_path_buf())
                    .collect(),
                destination.as_path().to_path_buf(),
            ));
            Ok(())
        }
    }

    fn location(path: impl AsRef<Path>) -> ExplorerResult<NavigationLocation> {
        NavigationLocation::from_path(path.as_ref().to_path_buf())
    }

    fn explorer(
        start: impl AsRef<Path>,
    ) -> ExplorerResult<(
        ExplorerApp<FakeFileSystemGateway, FakeShellGateway>,
        FakeFileSystemGateway,
        FakeShellGateway,
    )> {
        let file_system = FakeFileSystemGateway::default();
        let shell = FakeShellGateway::default();
        let app = ExplorerApp::new(location(start)?, file_system.clone(), shell.clone());
        Ok((app, file_system, shell))
    }

    fn file_item(path: impl AsRef<Path>, kind: FileItemKind) -> ExplorerResult<FileItem> {
        let path = path.as_ref().to_path_buf();
        let display_name = path
            .file_name()
            .map(OsString::from)
            .unwrap_or_else(|| path.as_os_str().to_os_string());

        Ok(FileItem {
            location: NavigationLocation::from_path(path)?,
            display_name,
            kind,
            type_name: OsString::from("test item"),
            size: None,
            updated_at: None,
            attributes: FileAttributes::default(),
        })
    }

    fn display_names(items: &[FolderTreeItem]) -> Vec<OsString> {
        items
            .iter()
            .map(|item| item.display_name().to_os_string())
            .collect()
    }

    #[test]
    fn start_search_requires_only_location_access_port() -> ExplorerResult<()> {
        let file_system = AccessOnlyGateway::default();
        let mut app = ExplorerApp::new(location(r"C:\root")?, file_system.clone(), NoShellGateway);
        let criteria = SearchCriteria {
            query: "report".to_string(),
            ..SearchCriteria::default()
        };

        let request = app.start_search_in_active(criteria.clone())?;

        assert_eq!(request.root, location(r"C:\root")?);
        assert_eq!(request.criteria, criteria);
        assert_eq!(
            *file_system.checked.borrow(),
            vec![PathBuf::from(r"C:\root")]
        );
        assert!(matches!(
            app.active_tab()?.search,
            SearchState::Running { run_id, .. } if run_id == request.run_id
        ));
        Ok(())
    }

    #[test]
    fn copy_items_to_requires_only_access_and_transfer_ports() -> ExplorerResult<()> {
        let file_system = AccessOnlyGateway::default();
        let shell = TransferOnlyShellGateway::default();
        let app = ExplorerApp::new(location(r"C:\start")?, file_system.clone(), shell.clone());
        let sources = vec![location(r"C:\source\a.txt")?];
        let destination = location(r"C:\dest")?;

        let outcome = app.copy_items_to(&sources, &destination)?;

        assert_eq!(
            *file_system.checked.borrow(),
            vec![PathBuf::from(r"C:\dest")]
        );
        assert_eq!(
            *shell.copied.borrow(),
            vec![(
                vec![PathBuf::from(r"C:\source\a.txt")],
                PathBuf::from(r"C:\dest")
            )]
        );
        assert_eq!(outcome.affected_folders, vec![destination]);
        Ok(())
    }

    #[test]
    fn navigate_active_path_accepts_unicode_unc_paths() -> ExplorerResult<()> {
        let (mut app, _, _) = explorer(r"C:\start")?;

        app.navigate_active_path(PathBuf::from(r"\\server\share\한글"))?;

        assert_eq!(
            app.active_tab()?.current_location().as_path(),
            Path::new(r"\\server\share\한글")
        );
        assert!(matches!(
            app.active_tab()?.current_location(),
            NavigationLocation::NetworkShare(_)
        ));

        Ok(())
    }

    #[test]
    fn tab_navigation_history_is_independent() -> ExplorerResult<()> {
        let (mut app, _, _) = explorer(r"C:\one")?;
        let first_tab_id = app.state().active_tab_id;
        let second_tab_id = app.open_tab(location(r"C:\two")?)?;

        app.navigate_active(location(r"C:\two\next")?)?;
        app.switch_tab(first_tab_id)?;
        app.navigate_active(location(r"C:\one\next")?)?;
        app.go_back()?;

        assert_eq!(
            app.active_tab()?.current_location().as_path(),
            Path::new(r"C:\one")
        );

        app.switch_tab(second_tab_id)?;
        assert_eq!(
            app.active_tab()?.current_location().as_path(),
            Path::new(r"C:\two\next")
        );
        app.go_back()?;
        assert_eq!(
            app.active_tab()?.current_location().as_path(),
            Path::new(r"C:\two")
        );

        Ok(())
    }

    #[test]
    fn go_forward_restores_location_and_updates_history() -> ExplorerResult<()> {
        let (mut app, _, _) = explorer(r"C:\one")?;

        app.navigate_active(location(r"C:\two")?)?;
        app.go_back()?;
        app.go_forward()?;

        assert_eq!(
            app.active_tab()?.current_location().as_path(),
            Path::new(r"C:\two")
        );
        assert_eq!(
            app.active_tab()?.back_history()[0].as_path(),
            Path::new(r"C:\one")
        );
        assert!(app.active_tab()?.forward_history().is_empty());

        Ok(())
    }

    #[test]
    fn failed_forward_and_up_navigation_preserve_current_location() -> ExplorerResult<()> {
        let (mut app, file_system, _) = explorer(r"C:\root")?;

        app.navigate_active(location(r"C:\root\child")?)?;
        app.navigate_active(location(r"C:\root\child\leaf")?)?;
        app.go_back()?;
        file_system.deny(r"C:\root\child\leaf");

        let forward_error = app
            .go_forward()
            .expect_err("inaccessible forward target should fail");
        assert_eq!(
            app.active_tab()?.current_location().as_path(),
            Path::new(r"C:\root\child")
        );
        assert_eq!(
            forward_error.user_message(),
            "권한이 없어 작업을 완료할 수 없습니다."
        );

        file_system.deny(r"C:\root");
        let up_error = app
            .go_up()
            .expect_err("inaccessible parent target should fail");
        assert_eq!(
            app.active_tab()?.current_location().as_path(),
            Path::new(r"C:\root\child")
        );
        assert_eq!(
            up_error.user_message(),
            "권한이 없어 작업을 완료할 수 없습니다."
        );

        Ok(())
    }

    #[test]
    fn moving_tabs_changes_order_without_changing_active_tab() -> ExplorerResult<()> {
        let (mut app, _, _) = explorer(r"C:\one")?;
        let first_tab_id = app.state().active_tab_id;
        let second_tab_id = app.open_tab(location(r"C:\two")?)?;
        let third_tab_id = app.open_tab(location(r"C:\three")?)?;

        app.move_tab(third_tab_id, 0)?;

        let ordered_ids = app
            .state()
            .tabs
            .iter()
            .map(|tab| tab.id)
            .collect::<Vec<_>>();
        assert_eq!(ordered_ids, vec![third_tab_id, first_tab_id, second_tab_id]);
        assert_eq!(app.state().active_tab_id, third_tab_id);
        assert_eq!(app.active_tab_index()?, 0);

        app.switch_to_tab_index(2)?;
        assert_eq!(app.state().active_tab_id, second_tab_id);

        Ok(())
    }

    #[test]
    fn closing_active_tab_selects_adjacent_tab_and_keeps_restore_stack() -> ExplorerResult<()> {
        let (mut app, _, _) = explorer(r"C:\one")?;
        let second_tab_id = app.open_tab(location(r"C:\two")?)?;
        let third_tab_id = app.open_tab(location(r"C:\three")?)?;
        app.switch_tab(second_tab_id)?;

        app.close_active_tab()?;

        assert_eq!(app.state().active_tab_id, third_tab_id);
        assert_eq!(
            app.state().closed_tabs.last().map(|tab| tab.id),
            Some(second_tab_id)
        );
        assert_eq!(app.state().tabs.len(), 2);

        let reopened = app.reopen_last_closed_tab()?;
        assert_eq!(reopened, second_tab_id);
        assert_eq!(app.state().active_tab_id, second_tab_id);
        assert_eq!(
            app.state()
                .tabs
                .last()
                .map(|tab| tab.current_location().as_path()),
            Some(Path::new(r"C:\two"))
        );

        Ok(())
    }

    #[test]
    fn open_folder_in_new_tab_activates_new_tab() -> ExplorerResult<()> {
        let (mut app, _, _) = explorer(r"C:\root")?;
        let folder = file_item(r"C:\root\child", FileItemKind::Folder)?;
        let file = file_item(r"C:\root\readme.txt", FileItemKind::File)?;

        let tab_id = app.open_folder_in_new_tab(&folder)?;

        assert_eq!(app.state().active_tab_id, tab_id);
        assert_eq!(
            app.active_tab()?.current_location().as_path(),
            Path::new(r"C:\root\child")
        );

        let error = app
            .open_folder_in_new_tab(&file)
            .expect_err("file items cannot be opened as folder tabs");
        assert_eq!(
            error.user_message(),
            "폴더 항목만 새 탭으로 열 수 있습니다."
        );

        Ok(())
    }

    #[test]
    fn display_options_are_global_settings_and_forwarded_to_listing() -> ExplorerResult<()> {
        let (mut app, file_system, _) = explorer(r"C:\root")?;

        app.set_show_hidden(true);
        app.set_show_system(true);
        let items = app.list_active_items()?;

        assert!(items.is_empty());
        assert_eq!(
            app.user_settings().display_options,
            DisplayOptions {
                show_hidden: true,
                show_system: true,
            }
        );
        let listed = file_system.listed.borrow().clone();
        assert_eq!(
            listed,
            vec![(
                PathBuf::from(r"C:\root"),
                DisplayOptions {
                    show_hidden: true,
                    show_system: true,
                },
                SortState::default(),
            )]
        );
        Ok(())
    }

    #[test]
    fn appearance_theme_is_global_user_setting() -> ExplorerResult<()> {
        let (mut app, _, _) = explorer(r"C:\root")?;

        app.set_appearance_theme(AppearanceTheme::Graphite);

        assert_eq!(app.appearance_theme(), AppearanceTheme::Graphite);
        assert_eq!(
            app.user_settings().appearance_theme,
            AppearanceTheme::Graphite
        );
        Ok(())
    }

    #[test]
    fn appearance_font_is_global_user_setting() -> ExplorerResult<()> {
        let (mut app, _, _) = explorer(r"C:\root")?;
        let font = AppearanceFont::custom(OsString::from("Segoe UI"), 12)
            .ok_or_else(|| ExplorerError::state_conflict("expected valid font"))?;

        app.set_appearance_font(font.clone());

        assert_eq!(app.appearance_font(), &font);
        assert_eq!(app.user_settings().appearance_font, font);
        Ok(())
    }

    #[test]
    fn folder_tree_roots_include_known_folders_drives_and_bookmarks() -> ExplorerResult<()> {
        let (mut app, file_system, _) = explorer(r"C:\root")?;
        file_system.set_drive_roots(vec![location(r"C:\")?, location(r"D:\")?]);
        app.add_bookmark(location(r"C:\Work")?, Some(OsString::from("Work")));

        let roots = app.folder_tree_roots()?;
        let kinds = roots.iter().map(|item| item.kind()).collect::<Vec<_>>();

        assert_eq!(
            kinds,
            vec![
                FolderTreeItemKind::KnownFolder(KnownFolderKind::Home),
                FolderTreeItemKind::KnownFolder(KnownFolderKind::Desktop),
                FolderTreeItemKind::KnownFolder(KnownFolderKind::Downloads),
                FolderTreeItemKind::KnownFolder(KnownFolderKind::Documents),
                FolderTreeItemKind::DriveRoot,
                FolderTreeItemKind::DriveRoot,
                FolderTreeItemKind::Bookmark,
            ]
        );
        assert_eq!(roots[0].location().as_path(), Path::new(r"C:\Users\Test"));
        assert_eq!(roots[4].location().as_path(), Path::new(r"C:\"));
        assert_eq!(roots[5].location().as_path(), Path::new(r"D:\"));
        assert_eq!(roots[6].display_name(), OsStr::new("Work"));
        assert_eq!(roots[6].location().as_path(), Path::new(r"C:\Work"));

        Ok(())
    }

    #[test]
    fn folder_tree_children_return_only_allowed_folders() -> ExplorerResult<()> {
        let (mut app, file_system, _) = explorer(r"C:\root")?;
        let visible = file_item(r"C:\root\visible", FileItemKind::Folder)?;
        let file = file_item(r"C:\root\readme.txt", FileItemKind::File)?;
        let mut hidden = file_item(r"C:\root\hidden", FileItemKind::Folder)?;
        hidden.attributes.hidden = true;
        let mut system = file_item(r"C:\root\system", FileItemKind::Folder)?;
        system.attributes.system = true;
        file_system.add_items(r"C:\root", vec![file, system, visible, hidden]);

        let default_children = app.folder_tree_children(&location(r"C:\root")?)?;
        assert_eq!(
            display_names(&default_children),
            vec![OsString::from("visible")]
        );

        app.set_show_hidden(true);
        let hidden_children = app.folder_tree_children(&location(r"C:\root")?)?;
        assert_eq!(
            display_names(&hidden_children),
            vec![OsString::from("hidden"), OsString::from("visible")]
        );
        assert!(hidden_children.iter().all(|item| {
            item.kind() == FolderTreeItemKind::FolderChild
                && item.depth() == 1
                && item.has_children()
        }));

        app.set_show_system(true);
        let all_children = app.folder_tree_children(&location(r"C:\root")?)?;
        assert_eq!(
            display_names(&all_children),
            vec![
                OsString::from("hidden"),
                OsString::from("system"),
                OsString::from("visible"),
            ]
        );
        assert_eq!(
            file_system.child_listed.borrow().last().cloned(),
            Some((
                PathBuf::from(r"C:\root"),
                DisplayOptions {
                    show_hidden: true,
                    show_system: true,
                },
                SortState::default(),
            ))
        );

        Ok(())
    }

    #[test]
    fn folder_tree_child_presence_uses_display_options_without_listing_children(
    ) -> ExplorerResult<()> {
        let (mut app, file_system, _) = explorer(r"C:\root")?;
        let file = file_item(r"C:\root\readme.txt", FileItemKind::File)?;
        let mut hidden = file_item(r"C:\root\hidden", FileItemKind::Folder)?;
        hidden.attributes.hidden = true;
        file_system.add_items(r"C:\root", vec![file, hidden]);

        assert!(!app.folder_tree_has_child_folders(&location(r"C:\root")?)?);
        assert_eq!(
            file_system.child_presence_checked.borrow().last().cloned(),
            Some((PathBuf::from(r"C:\root"), DisplayOptions::default()))
        );
        assert!(file_system.child_listed.borrow().is_empty());

        app.set_show_hidden(true);
        assert!(app.folder_tree_has_child_folders(&location(r"C:\root")?)?);
        assert_eq!(
            file_system.child_presence_checked.borrow().last().cloned(),
            Some((
                PathBuf::from(r"C:\root"),
                DisplayOptions {
                    show_hidden: true,
                    show_system: false,
                },
            ))
        );
        assert!(file_system.child_listed.borrow().is_empty());

        Ok(())
    }

    #[test]
    fn failed_folder_tree_child_lookup_preserves_current_location_and_user_message(
    ) -> ExplorerResult<()> {
        for (path, code, user_message) in tree_navigation_failure_cases() {
            let (app, file_system, _) = explorer(r"C:\current")?;
            file_system.fail_with_code(path, code);

            let error = app
                .folder_tree_children(&location(path)?)
                .expect_err("folder tree child lookup failure should be recoverable");

            assert_eq!(
                app.active_tab()?.current_location().as_path(),
                Path::new(r"C:\current")
            );
            assert_eq!(error.user_message(), user_message);
        }

        Ok(())
    }

    #[test]
    fn failed_folder_tree_selection_navigation_preserves_current_location_and_user_message(
    ) -> ExplorerResult<()> {
        for (path, code, user_message) in tree_navigation_failure_cases() {
            let (mut app, file_system, _) = explorer(r"C:\current")?;
            file_system.fail_with_code(path, code);

            let error = app
                .navigate_active(location(path)?)
                .expect_err("folder tree selection failure should not move the active tab");

            assert_eq!(
                app.active_tab()?.current_location().as_path(),
                Path::new(r"C:\current")
            );
            assert_eq!(error.user_message(), user_message);
        }

        Ok(())
    }

    #[test]
    fn active_sort_is_tab_state_and_forwarded_to_listing() -> ExplorerResult<()> {
        let (mut app, file_system, _) = explorer(r"C:\one")?;
        let first_tab_id = app.state().active_tab_id;
        app.open_tab(location(r"C:\two")?)?;

        app.set_active_sort_key(SortKey::Size)?;
        app.set_active_sort_direction(SortDirection::Descending)?;
        app.list_active_items()?;

        let last_listing = file_system.listed.borrow().last().cloned();
        assert_eq!(
            last_listing,
            Some((
                PathBuf::from(r"C:\two"),
                DisplayOptions::default(),
                SortState {
                    key: SortKey::Size,
                    direction: SortDirection::Descending,
                },
            ))
        );

        app.switch_tab(first_tab_id)?;
        app.list_active_items()?;

        let last_listing = file_system.listed.borrow().last().cloned();
        assert_eq!(
            last_listing,
            Some((
                PathBuf::from(r"C:\one"),
                DisplayOptions::default(),
                SortState::default(),
            ))
        );
        Ok(())
    }

    #[test]
    fn start_search_creates_request_and_running_state_for_active_tab() -> ExplorerResult<()> {
        let (mut app, _, _) = explorer(r"C:\root")?;
        let criteria = SearchCriteria {
            query: "report".to_string(),
            ..SearchCriteria::default()
        };

        let request = app.start_search_in_active(criteria.clone())?;

        assert_eq!(request.tab_id, app.state().active_tab_id);
        assert_eq!(request.root.as_path(), Path::new(r"C:\root"));
        assert_eq!(request.criteria, criteria.clone());
        assert_eq!(
            app.active_tab()?.search,
            SearchState::Running {
                run_id: request.run_id,
                criteria,
                progress: SearchProgress::default(),
                cancel_requested: false,
            }
        );

        Ok(())
    }

    #[test]
    fn startup_restore_uses_accessible_saved_tabs_and_active_tab() -> ExplorerResult<()> {
        let file_system = FakeFileSystemGateway::default();
        let shell = FakeShellGateway::default();
        let settings = UserSettings {
            restore_tabs_on_startup: true,
            session: UserSession {
                tabs: vec![
                    TabState::from_parts(
                        TabId(10),
                        location(r"C:\one\child")?,
                        vec![location(r"C:\one")?],
                        Vec::new(),
                        SortState {
                            key: SortKey::UpdatedAt,
                            direction: SortDirection::Descending,
                        },
                    ),
                    TabState::new(TabId(20), location(r"C:\two")?),
                ],
                active_tab_id: Some(TabId(20)),
                closed_tabs: vec![TabState::new(TabId(30), location(r"C:\closed")?)],
            },
            ..UserSettings::default()
        };

        let mut app = ExplorerApp::new_at_accessible_start_with_settings(
            vec![location(r"C:\fallback")?],
            file_system,
            shell,
            settings,
        )?;

        assert_eq!(app.state().tabs.len(), 2);
        assert_eq!(
            app.active_tab()?.current_location().as_path(),
            Path::new(r"C:\two")
        );
        assert_eq!(
            app.state().tabs[0].sort,
            SortState {
                key: SortKey::UpdatedAt,
                direction: SortDirection::Descending,
            }
        );
        app.switch_to_tab_index(0)?;
        app.go_back()?;
        assert_eq!(
            app.active_tab()?.current_location().as_path(),
            Path::new(r"C:\one")
        );
        assert_eq!(app.state().closed_tabs.len(), 1);

        Ok(())
    }

    #[test]
    fn startup_restore_checks_saved_open_tabs_once_per_location() -> ExplorerResult<()> {
        let file_system = FakeFileSystemGateway::default();
        let shell = FakeShellGateway::default();
        let settings = UserSettings {
            restore_tabs_on_startup: true,
            session: UserSession {
                tabs: vec![
                    TabState::new(TabId(10), location(r"C:\fallback")?),
                    TabState::new(TabId(20), location(r"C:\shared")?),
                    TabState::new(TabId(30), location(r"C:\shared")?),
                ],
                active_tab_id: Some(TabId(20)),
                closed_tabs: Vec::new(),
            },
            ..UserSettings::default()
        };

        let app = ExplorerApp::new_at_accessible_start_with_settings(
            vec![location(r"C:\fallback")?],
            file_system.clone(),
            shell,
            settings,
        )?;

        assert_eq!(app.state().tabs.len(), 3);
        assert_eq!(
            app.active_tab()?.current_location().as_path(),
            Path::new(r"C:\shared")
        );
        assert_eq!(
            *file_system.checked.borrow(),
            vec![PathBuf::from(r"C:\fallback"), PathBuf::from(r"C:\shared")]
        );

        Ok(())
    }

    #[test]
    fn deferred_startup_restore_checks_only_active_saved_tab_before_return() -> ExplorerResult<()> {
        let file_system = FakeFileSystemGateway::default();
        let shell = FakeShellGateway::default();
        let settings = UserSettings {
            restore_tabs_on_startup: true,
            session: UserSession {
                tabs: vec![
                    TabState::new(TabId(10), location(r"C:\one")?),
                    TabState::new(TabId(20), location(r"C:\active")?),
                    TabState::new(TabId(30), location(r"C:\other")?),
                ],
                active_tab_id: Some(TabId(20)),
                closed_tabs: Vec::new(),
            },
            ..UserSettings::default()
        };

        let (mut app, pending_restore) =
            ExplorerApp::new_at_accessible_start_deferring_startup_session(
                vec![location(r"C:\fallback")?],
                file_system.clone(),
                shell,
                settings,
            )?;

        assert_eq!(app.state().tabs.len(), 1);
        assert_eq!(
            app.active_tab()?.current_location().as_path(),
            Path::new(r"C:\active")
        );
        assert_eq!(
            *file_system.checked.borrow(),
            vec![PathBuf::from(r"C:\active")]
        );

        let Some(pending_restore) = pending_restore else {
            panic!("saved open tabs should produce a deferred restore");
        };
        app.apply_deferred_startup_restore(pending_restore)?;

        assert_eq!(app.state().tabs.len(), 3);
        assert_eq!(
            app.active_tab()?.current_location().as_path(),
            Path::new(r"C:\active")
        );
        assert_eq!(
            *file_system.checked.borrow(),
            vec![
                PathBuf::from(r"C:\active"),
                PathBuf::from(r"C:\one"),
                PathBuf::from(r"C:\other"),
            ]
        );

        Ok(())
    }

    #[test]
    fn deferred_startup_restore_uses_fallback_without_scanning_saved_tabs() -> ExplorerResult<()> {
        let file_system = FakeFileSystemGateway::default();
        file_system.fail_with_code(r"\\offline\share", 53);
        let shell = FakeShellGateway::default();
        let settings = UserSettings {
            restore_tabs_on_startup: true,
            session: UserSession {
                tabs: vec![
                    TabState::new(TabId(10), location(r"C:\one")?),
                    TabState::new(TabId(20), location(r"\\offline\share")?),
                ],
                active_tab_id: Some(TabId(20)),
                closed_tabs: Vec::new(),
            },
            ..UserSettings::default()
        };

        let (mut app, pending_restore) =
            ExplorerApp::new_at_accessible_start_deferring_startup_session(
                vec![location(r"C:\fallback")?],
                file_system.clone(),
                shell,
                settings,
            )?;

        assert_eq!(
            app.active_tab()?.current_location().as_path(),
            Path::new(r"C:\fallback")
        );
        assert_eq!(
            *file_system.checked.borrow(),
            vec![
                PathBuf::from(r"\\offline\share"),
                PathBuf::from(r"C:\fallback"),
            ]
        );

        let Some(pending_restore) = pending_restore else {
            panic!("saved open tabs should produce a deferred restore");
        };
        app.apply_deferred_startup_restore(pending_restore)?;

        assert_eq!(app.state().tabs.len(), 1);
        assert_eq!(
            app.active_tab()?.current_location().as_path(),
            Path::new(r"C:\one")
        );
        assert_eq!(
            *file_system.checked.borrow(),
            vec![
                PathBuf::from(r"\\offline\share"),
                PathBuf::from(r"C:\fallback"),
                PathBuf::from(r"C:\one"),
            ]
        );

        Ok(())
    }

    #[test]
    fn startup_restore_skips_inaccessible_saved_open_tabs() -> ExplorerResult<()> {
        let file_system = FakeFileSystemGateway::default();
        file_system.deny(r"C:\blocked");
        let shell = FakeShellGateway::default();
        let settings = UserSettings {
            restore_tabs_on_startup: true,
            session: UserSession {
                tabs: vec![
                    TabState::new(TabId(10), location(r"C:\blocked")?),
                    TabState::new(TabId(20), location(r"C:\open")?),
                ],
                active_tab_id: Some(TabId(10)),
                closed_tabs: Vec::new(),
            },
            ..UserSettings::default()
        };

        let app = ExplorerApp::new_at_accessible_start_with_settings(
            vec![location(r"C:\fallback")?],
            file_system.clone(),
            shell,
            settings,
        )?;

        assert_eq!(app.state().tabs.len(), 1);
        assert_eq!(
            app.active_tab()?.current_location().as_path(),
            Path::new(r"C:\open")
        );
        assert_eq!(
            *file_system.checked.borrow(),
            vec![PathBuf::from(r"C:\blocked"), PathBuf::from(r"C:\open")]
        );

        Ok(())
    }

    #[test]
    fn startup_restore_uses_fallback_for_inaccessible_network_tab() -> ExplorerResult<()> {
        let file_system = FakeFileSystemGateway::default();
        file_system.fail_with_code(r"\\offline\share", 53);
        let shell = FakeShellGateway::default();
        let settings = UserSettings {
            restore_tabs_on_startup: true,
            session: UserSession {
                tabs: vec![TabState::new(TabId(10), location(r"\\offline\share")?)],
                active_tab_id: Some(TabId(10)),
                closed_tabs: Vec::new(),
            },
            ..UserSettings::default()
        };

        let app = ExplorerApp::new_at_accessible_start_with_settings(
            vec![location(r"C:\fallback")?],
            file_system.clone(),
            shell,
            settings,
        )?;

        assert_eq!(app.state().tabs.len(), 1);
        assert_eq!(
            app.active_tab()?.current_location().as_path(),
            Path::new(r"C:\fallback")
        );
        assert_eq!(
            *file_system.checked.borrow(),
            vec![
                PathBuf::from(r"\\offline\share"),
                PathBuf::from(r"C:\fallback"),
            ]
        );

        Ok(())
    }

    #[test]
    fn startup_restore_checks_fallback_when_saved_open_tabs_are_empty() -> ExplorerResult<()> {
        let file_system = FakeFileSystemGateway::default();
        let shell = FakeShellGateway::default();
        let settings = UserSettings {
            restore_tabs_on_startup: true,
            session: UserSession {
                tabs: Vec::new(),
                active_tab_id: None,
                closed_tabs: Vec::new(),
            },
            ..UserSettings::default()
        };

        let app = ExplorerApp::new_at_accessible_start_with_settings(
            vec![location(r"C:\fallback")?],
            file_system.clone(),
            shell,
            settings,
        )?;

        assert_eq!(app.state().tabs.len(), 1);
        assert_eq!(
            app.active_tab()?.current_location().as_path(),
            Path::new(r"C:\fallback")
        );
        assert_eq!(
            *file_system.checked.borrow(),
            vec![PathBuf::from(r"C:\fallback")]
        );

        Ok(())
    }

    #[test]
    fn user_settings_snapshot_includes_open_active_and_closed_tabs() -> ExplorerResult<()> {
        let (mut app, _, _) = explorer(r"C:\one")?;
        let first_tab_id = app.state().active_tab_id;
        let second_tab_id = app.open_tab(location(r"C:\two")?)?;
        app.close_tab(second_tab_id)?;
        app.set_startup_folder(Some(location(r"C:\startup")?));

        let settings = app.user_settings();

        assert_eq!(
            settings
                .startup_folder
                .as_ref()
                .map(NavigationLocation::as_path),
            Some(Path::new(r"C:\startup"))
        );
        assert_eq!(settings.session.active_tab_id, Some(first_tab_id));
        assert_eq!(settings.session.tabs.len(), 1);
        assert_eq!(
            settings.session.tabs[0].current_location().as_path(),
            Path::new(r"C:\one")
        );
        assert_eq!(settings.session.closed_tabs.len(), 1);
        assert_eq!(
            settings.session.closed_tabs[0].current_location().as_path(),
            Path::new(r"C:\two")
        );

        Ok(())
    }

    #[test]
    fn failed_closed_tab_reopen_preserves_restore_stack() -> ExplorerResult<()> {
        let file_system = FakeFileSystemGateway::default();
        file_system.fail_with_code(r"\\offline\share", 53);
        let shell = FakeShellGateway::default();
        let settings = UserSettings {
            session: UserSession {
                tabs: Vec::new(),
                active_tab_id: None,
                closed_tabs: vec![TabState::new(TabId(10), location(r"\\offline\share")?)],
            },
            ..UserSettings::default()
        };
        let mut app = ExplorerApp::new_at_accessible_start_with_settings(
            vec![location(r"C:\fallback")?],
            file_system,
            shell,
            settings,
        )?;

        let error = app
            .reopen_last_closed_tab()
            .expect_err("offline network tab should not be removed from restore stack");

        assert_eq!(
            error.user_message(),
            "네트워크 위치에 연결할 수 없습니다. 서버 이름, 공유 이름 또는 네트워크 연결을 확인해 주세요."
        );
        assert_eq!(app.state().closed_tabs.len(), 1);
        assert_eq!(
            app.active_tab()?.current_location().as_path(),
            Path::new(r"C:\fallback")
        );

        Ok(())
    }

    #[test]
    fn search_progress_updates_only_matching_running_search() -> ExplorerResult<()> {
        let (mut app, _, _) = explorer(r"C:\root")?;
        let request = app.start_search_in_active(SearchCriteria::default())?;
        let progress = SearchProgress {
            visited_folders: 2,
            scanned_items: 10,
            matched_items: 3,
            skipped_folders: 1,
        };

        assert!(app.update_search_progress(request.tab_id, request.run_id, progress)?);
        assert!(!app.update_search_progress(request.tab_id, SearchRunId(999), progress)?);

        match &app.active_tab()?.search {
            SearchState::Running {
                progress: active_progress,
                ..
            } => assert_eq!(*active_progress, progress),
            state => panic!("expected running search, got {state:?}"),
        }

        Ok(())
    }

    #[test]
    fn finishing_search_records_results_separately_from_current_location() -> ExplorerResult<()> {
        let (mut app, _, _) = explorer(r"C:\root")?;
        let request = app.start_search_in_active(SearchCriteria::default())?;
        let result = file_item(r"C:\root\nested\report.txt", FileItemKind::File)?;
        let progress = SearchProgress {
            visited_folders: 2,
            scanned_items: 4,
            matched_items: 1,
            skipped_folders: 0,
        };
        let outcome = SearchOutcome::from_request(
            request.clone(),
            SearchFileSystemOutcome {
                items: vec![result.clone()],
                diagnostics: Vec::new(),
                progress,
                cancelled: false,
            },
        );

        assert!(app.finish_search(outcome)?);

        assert_eq!(
            app.active_tab()?.current_location().as_path(),
            Path::new(r"C:\root")
        );
        match &app.active_tab()?.search {
            SearchState::Results {
                criteria,
                items,
                diagnostics,
                progress: actual_progress,
            } => {
                assert_eq!(criteria, &request.criteria);
                assert_eq!(items, &vec![result]);
                assert!(diagnostics.is_empty());
                assert_eq!(*actual_progress, progress);
            }
            state => panic!("expected search results, got {state:?}"),
        }

        Ok(())
    }

    #[test]
    fn cancelled_search_keeps_partial_results() -> ExplorerResult<()> {
        let (mut app, _, _) = explorer(r"C:\root")?;
        let request = app.start_search_in_active(SearchCriteria::default())?;
        assert_eq!(app.request_active_search_cancel()?, Some(request.run_id));
        let partial = file_item(r"C:\root\partial.txt", FileItemKind::File)?;
        let diagnostic = SearchDiagnostic::new(PathBuf::from(r"C:\root\blocked"), "access denied");
        let progress = SearchProgress {
            visited_folders: 1,
            scanned_items: 1,
            matched_items: 1,
            skipped_folders: 1,
        };

        let outcome = SearchOutcome::from_request(
            request,
            SearchFileSystemOutcome {
                items: vec![partial.clone()],
                diagnostics: vec![diagnostic.clone()],
                progress,
                cancelled: true,
            },
        );

        assert!(app.finish_search(outcome)?);
        match &app.active_tab()?.search {
            SearchState::Cancelled {
                partial_items,
                diagnostics,
                progress: actual_progress,
                ..
            } => {
                assert_eq!(partial_items, &vec![partial]);
                assert_eq!(diagnostics, &vec![diagnostic]);
                assert_eq!(*actual_progress, progress);
            }
            state => panic!("expected cancelled search, got {state:?}"),
        }

        Ok(())
    }

    #[test]
    fn reopen_last_closed_tab_uses_most_recent_closed_tab() -> ExplorerResult<()> {
        let (mut app, _, _) = explorer(r"C:\one")?;
        let second_tab_id = app.open_tab(location(r"C:\two")?)?;
        let third_tab_id = app.open_tab(location(r"C:\three")?)?;

        app.close_tab(second_tab_id)?;
        app.close_tab(third_tab_id)?;

        let reopened = app.reopen_last_closed_tab()?;

        assert_eq!(reopened, third_tab_id);
        assert_eq!(app.state().active_tab_id, third_tab_id);
        assert_eq!(
            app.active_tab()?.current_location().as_path(),
            Path::new(r"C:\three")
        );

        Ok(())
    }

    #[test]
    fn reopen_last_closed_tab_restores_tab_state() -> ExplorerResult<()> {
        let (mut app, _, _) = explorer(r"C:\root")?;
        let tab_id = app.open_tab(location(r"C:\work")?)?;
        app.navigate_active(location(r"C:\work\child")?)?;

        let sort = SortState {
            key: SortKey::UpdatedAt,
            direction: SortDirection::Descending,
        };
        let criteria = SearchCriteria {
            query: "selected".to_string(),
            ..SearchCriteria::default()
        };
        let search = SearchState::Running {
            run_id: SearchRunId(42),
            criteria,
            progress: SearchProgress {
                visited_folders: 1,
                scanned_items: 2,
                matched_items: 1,
                skipped_folders: 0,
            },
            cancel_requested: true,
        };
        let selected_items = vec![location(r"C:\work\child\selected.txt")?];
        {
            let tab = app.active_tab_mut()?;
            tab.sort = sort;
            tab.search = search.clone();
            tab.selected_items = selected_items.clone();
        }

        app.close_tab(tab_id)?;
        let reopened = app.reopen_last_closed_tab()?;

        assert_eq!(reopened, tab_id);
        assert_eq!(
            app.active_tab()?.current_location().as_path(),
            Path::new(r"C:\work\child")
        );
        assert_eq!(app.active_tab()?.sort, sort);
        assert_eq!(app.active_tab()?.search, search);
        assert_eq!(app.active_tab()?.selected_items, selected_items);

        app.go_back()?;
        assert_eq!(
            app.active_tab()?.current_location().as_path(),
            Path::new(r"C:\work")
        );

        Ok(())
    }

    #[test]
    fn activating_folder_item_navigates_and_records_history() -> ExplorerResult<()> {
        let (mut app, _, _) = explorer(r"C:\root")?;
        let item = file_item(r"C:\root\child", FileItemKind::Folder)?;

        assert_eq!(
            app.activate_item_in_active(&item)?,
            ItemActivation::Navigated
        );
        assert_eq!(
            app.active_tab()?.current_location().as_path(),
            Path::new(r"C:\root\child")
        );

        app.go_back()?;
        assert_eq!(
            app.active_tab()?.current_location().as_path(),
            Path::new(r"C:\root")
        );

        Ok(())
    }

    #[test]
    fn known_folder_location_is_resolved_through_file_system_gateway() -> ExplorerResult<()> {
        let (app, _, _) = explorer(r"C:\root")?;
        let desktop = app.known_folder(KnownFolderKind::Desktop)?;

        assert_eq!(desktop.as_path(), Path::new(r"C:\Users\Test\Desktop"));
        assert!(matches!(
            desktop,
            NavigationLocation::KnownFolder {
                kind: KnownFolderKind::Desktop,
                ..
            }
        ));

        Ok(())
    }

    #[test]
    fn activating_file_item_uses_shell_without_changing_location() -> ExplorerResult<()> {
        let (mut app, _, shell) = explorer(r"C:\root")?;
        let item = file_item(r"C:\root\readme.txt", FileItemKind::File)?;

        assert_eq!(app.activate_item_in_active(&item)?, ItemActivation::Opened);
        assert_eq!(
            shell.opened.borrow().as_slice(),
            &[PathBuf::from(r"C:\root\readme.txt")]
        );
        assert_eq!(
            app.active_tab()?.current_location().as_path(),
            Path::new(r"C:\root")
        );

        Ok(())
    }

    #[test]
    fn open_item_with_picker_uses_shell_open_with_for_files() -> ExplorerResult<()> {
        let (app, _, shell) = explorer(r"C:\root")?;
        let item = file_item(r"C:\root\readme.txt", FileItemKind::File)?;

        app.open_item_with_picker(&item)?;

        assert_eq!(
            shell.open_with_paths.borrow().as_slice(),
            &[PathBuf::from(r"C:\root\readme.txt")]
        );

        Ok(())
    }

    #[test]
    fn open_item_with_picker_rejects_folders() -> ExplorerResult<()> {
        let (app, _, shell) = explorer(r"C:\root")?;
        let item = file_item(r"C:\root\child", FileItemKind::Folder)?;

        let error = app
            .open_item_with_picker(&item)
            .expect_err("folder items should not use Open With");

        assert_eq!(
            error.user_message(),
            "파일 항목만 연결 프로그램을 선택할 수 있습니다."
        );
        assert!(shell.open_with_paths.borrow().is_empty());

        Ok(())
    }

    #[test]
    fn context_menu_for_items_uses_shell_gateway_and_requests_refresh() -> ExplorerResult<()> {
        let (app, _, shell) = explorer(r"C:\root")?;
        let targets = vec![location(r"C:\root\a.txt")?, location(r"C:\root\b.txt")?];
        let position = ContextMenuPosition { x: 10, y: 20 };

        let outcome = app.show_context_menu_for_items(&targets, position)?;

        assert_eq!(
            shell.context_menus.borrow().as_slice(),
            &[(
                vec![
                    PathBuf::from(r"C:\root\a.txt"),
                    PathBuf::from(r"C:\root\b.txt")
                ],
                position
            )]
        );
        assert!(outcome.command_invoked);
        assert!(outcome.refresh_current_folder);

        Ok(())
    }

    #[test]
    fn folder_background_context_menu_uses_shell_gateway() -> ExplorerResult<()> {
        let (app, _, shell) = explorer(r"C:\root")?;
        let folder = location(r"C:\root")?;
        let position = ContextMenuPosition { x: 30, y: 40 };

        let outcome = app.show_context_menu_for_folder_background(&folder, position)?;

        assert_eq!(
            shell.background_context_menus.borrow().as_slice(),
            &[(vec![PathBuf::from(r"C:\root")], position)]
        );
        assert!(outcome.command_invoked);
        assert!(outcome.refresh_current_folder);

        Ok(())
    }

    #[test]
    fn create_folder_in_active_selects_created_folder() -> ExplorerResult<()> {
        let (mut app, file_system, _) = explorer(r"C:\root")?;

        let outcome = app.create_folder_in_active(OsStr::new("Reports"), true)?;

        assert_eq!(
            file_system.created_folders.borrow().as_slice(),
            &[(PathBuf::from(r"C:\root"), OsString::from("Reports"))]
        );
        assert_eq!(
            outcome.created_folder.as_path(),
            Path::new(r"C:\root\Reports")
        );
        assert_eq!(outcome.affected_folders, vec![location(r"C:\root")?]);
        assert_eq!(
            app.active_tab()?.selected_items,
            vec![location(r"C:\root\Reports")?]
        );

        Ok(())
    }

    #[test]
    fn create_folder_in_active_can_leave_selection_unchanged() -> ExplorerResult<()> {
        let (mut app, _, _) = explorer(r"C:\root")?;
        app.active_tab_mut()?.selected_items = vec![location(r"C:\root\old")?];

        app.create_folder_in_active(OsStr::new("Reports"), false)?;

        assert_eq!(
            app.active_tab()?.selected_items,
            vec![location(r"C:\root\old")?]
        );

        Ok(())
    }

    #[test]
    fn create_folder_in_active_rejects_invalid_name_before_gateway() -> ExplorerResult<()> {
        let (mut app, file_system, _) = explorer(r"C:\root")?;

        let error = app
            .create_folder_in_active(OsStr::new("bad:name"), true)
            .expect_err("invalid folder names should fail before file system calls");

        assert_eq!(
            error.user_message(),
            "폴더 이름이 올바르지 않습니다. 다른 이름을 입력해 주세요."
        );
        assert!(file_system.created_folders.borrow().is_empty());

        Ok(())
    }

    #[test]
    fn create_folder_in_active_denies_inaccessible_parent() -> ExplorerResult<()> {
        let (mut app, file_system, _) = explorer(r"C:\root")?;
        file_system.deny(r"C:\root");

        let error = app
            .create_folder_in_active(OsStr::new("Reports"), true)
            .expect_err("inaccessible parent should reject folder creation");

        assert_eq!(
            error.user_message(),
            "권한이 없어 작업을 완료할 수 없습니다."
        );
        assert!(app.active_tab()?.selected_items.is_empty());

        Ok(())
    }

    #[test]
    fn copy_to_active_returns_destination_refresh_folder() -> ExplorerResult<()> {
        let (app, _, shell) = explorer(r"C:\target")?;
        let sources = vec![location(r"C:\source\a.txt")?, location(r"C:\source\b.txt")?];

        let outcome = app.copy_to_active(&sources)?;

        assert_eq!(
            shell.copied.borrow().as_slice(),
            &[(
                vec![
                    PathBuf::from(r"C:\source\a.txt"),
                    PathBuf::from(r"C:\source\b.txt"),
                ],
                PathBuf::from(r"C:\target")
            )]
        );
        assert_eq!(outcome.affected_folders, vec![location(r"C:\target")?]);

        Ok(())
    }

    #[test]
    fn move_delete_and_rename_return_parent_refresh_folders() -> ExplorerResult<()> {
        let (app, _, shell) = explorer(r"C:\target")?;
        let sources = vec![location(r"C:\source\a.txt")?, location(r"C:\source\b.txt")?];

        let moved = app.move_to_active(&sources)?;
        let deleted = app.delete_to_recycle_bin(&sources)?;
        let permanently_deleted = app.delete_permanently(&sources)?;
        let renamed = app.rename_item(&sources[0], OsStr::new("renamed.txt"))?;

        assert_eq!(shell.moved.borrow().len(), 1);
        assert_eq!(shell.deleted.borrow().len(), 1);
        assert_eq!(shell.permanently_deleted.borrow().len(), 1);
        assert_eq!(
            shell.renamed.borrow().as_slice(),
            &[(
                PathBuf::from(r"C:\source\a.txt"),
                OsString::from("renamed.txt")
            )]
        );
        assert_eq!(
            moved.affected_folders,
            vec![location(r"C:\source")?, location(r"C:\target")?]
        );
        assert_eq!(deleted.affected_folders, vec![location(r"C:\source")?]);
        assert_eq!(
            permanently_deleted.affected_folders,
            vec![location(r"C:\source")?]
        );
        assert_eq!(renamed.affected_folders, vec![location(r"C:\source")?]);

        Ok(())
    }

    #[test]
    fn rename_rejects_invalid_name_before_shell_gateway() -> ExplorerResult<()> {
        let (app, _, shell) = explorer(r"C:\target")?;
        let target = location(r"C:\source\a.txt")?;
        let invalid_name = OsString::from_wide(&[b'a' as u16, 0, b'b' as u16]);

        let error = app
            .rename_item(&target, invalid_name.as_os_str())
            .expect_err("embedded NUL must be rejected before Shell rename");

        assert!(matches!(
            error,
            ExplorerError::InvalidFileName {
                reason: FileNameErrorKind::HasControlCharacter,
                ..
            }
        ));
        assert!(shell.renamed.borrow().is_empty());

        Ok(())
    }

    #[test]
    fn file_operations_reject_empty_selection() -> ExplorerResult<()> {
        let (app, _, _) = explorer(r"C:\target")?;

        let error = app
            .copy_to_active(&[])
            .expect_err("empty file operation selection should fail");

        assert_eq!(error.user_message(), "선택된 파일 또는 폴더가 없습니다.");

        Ok(())
    }

    #[test]
    fn prepare_file_drop_uses_internal_same_drive_default() -> ExplorerResult<()> {
        let (app, _, shell) = explorer(r"C:\target")?;
        let sources = vec![location(r"C:\source\a.txt")?];
        let destination = location(r"C:\target\folder")?;

        let plan = app.prepare_file_drop(
            &sources,
            &destination,
            DropSourceKind::Internal,
            DropModifierKeys::default(),
        )?;

        assert_eq!(plan.operation, DropOperation::Move);
        assert!(shell.copied.borrow().is_empty());
        assert!(shell.moved.borrow().is_empty());

        Ok(())
    }

    #[test]
    fn prepare_file_drop_modifiers_override_internal_default() -> ExplorerResult<()> {
        let (app, _, shell) = explorer(r"C:\target")?;
        let sources = vec![location(r"C:\source\a.txt")?];
        let destination = location(r"C:\target\folder")?;

        let ctrl_plan = app.prepare_file_drop(
            &sources,
            &destination,
            DropSourceKind::Internal,
            DropModifierKeys {
                control: true,
                shift: false,
            },
        )?;
        let shift_plan = app.prepare_file_drop(
            &sources,
            &destination,
            DropSourceKind::Internal,
            DropModifierKeys {
                control: false,
                shift: true,
            },
        )?;

        assert_eq!(ctrl_plan.operation, DropOperation::Copy);
        assert_eq!(shift_plan.operation, DropOperation::Move);
        assert!(shell.copied.borrow().is_empty());
        assert!(shell.moved.borrow().is_empty());

        Ok(())
    }

    #[test]
    fn prepare_file_drop_uses_external_shell_default() -> ExplorerResult<()> {
        let (app, _, shell) = explorer(r"C:\target")?;
        let sources = vec![location(r"D:\source\a.txt")?];
        let destination = location(r"C:\target")?;

        let plan = app.prepare_file_drop(
            &sources,
            &destination,
            DropSourceKind::External {
                default_operation: Some(DropOperation::Move),
            },
            DropModifierKeys::default(),
        )?;

        assert_eq!(plan.operation, DropOperation::Move);
        assert!(shell.copied.borrow().is_empty());
        assert!(shell.moved.borrow().is_empty());

        Ok(())
    }

    #[test]
    fn prepare_file_drop_rejects_move_into_source_descendant() -> ExplorerResult<()> {
        let (app, _, shell) = explorer(r"C:\target")?;
        let sources = vec![location(r"C:\source\folder")?];
        let destination = location(r"C:\source\folder\child")?;

        let error = app
            .prepare_file_drop(
                &sources,
                &destination,
                DropSourceKind::Internal,
                DropModifierKeys::default(),
            )
            .expect_err("moving a folder into a child must fail before shell calls");

        assert_eq!(
            error.user_message(),
            "이동 대상이 원본과 같거나 원본의 하위 폴더입니다."
        );
        assert!(shell.moved.borrow().is_empty());

        Ok(())
    }

    #[test]
    fn failed_back_navigation_preserves_current_location() -> ExplorerResult<()> {
        let (mut app, file_system, _) = explorer(r"C:\allowed")?;
        app.navigate_active(location(r"C:\current")?)?;
        file_system.deny(r"C:\allowed");

        let error = app.go_back().expect_err("back target should be denied");

        assert_eq!(
            app.active_tab()?.current_location().as_path(),
            Path::new(r"C:\current")
        );
        assert_eq!(
            error.user_message(),
            "권한이 없어 작업을 완료할 수 없습니다."
        );

        Ok(())
    }

    #[test]
    fn adding_active_location_bookmark_uses_duplicate_policy() -> ExplorerResult<()> {
        let (mut app, _, _) = explorer(r"C:\root")?;

        let first = app.add_active_location_bookmark(None)?;
        let duplicate = app.add_active_location_bookmark(Some(OsString::from("Root")))?;

        assert_eq!(first, BookmarkAddOutcome::Added(0));
        assert_eq!(duplicate, BookmarkAddOutcome::AlreadyExists(0));
        assert_eq!(app.state().bookmarks.items().len(), 1);
        assert_eq!(
            app.state().bookmarks.items()[0].target.as_path(),
            Path::new(r"C:\root")
        );

        Ok(())
    }

    #[test]
    fn adding_selected_folder_bookmark_rejects_files() -> ExplorerResult<()> {
        let (mut app, _, _) = explorer(r"C:\root")?;
        let folder = file_item(r"C:\root\child", FileItemKind::Folder)?;
        let file = file_item(r"C:\root\readme.txt", FileItemKind::File)?;

        assert_eq!(
            app.add_selected_folder_bookmark(&folder, Some(OsString::from("Child")))?,
            BookmarkAddOutcome::Added(0)
        );
        let error = app
            .add_selected_folder_bookmark(&file, None)
            .expect_err("file items cannot be bookmarked by the selected-folder use case");

        assert_eq!(
            error.user_message(),
            "폴더 항목만 북마크로 추가할 수 있습니다."
        );
        assert_eq!(app.state().bookmarks.items().len(), 1);

        Ok(())
    }

    #[test]
    fn bookmark_index_for_location_uses_normalized_path() -> ExplorerResult<()> {
        let (mut app, _, _) = explorer(r"C:\root")?;
        app.add_bookmark(location(r"C:\Work")?, Some(OsString::from("Work")));
        app.add_bookmark(location(r"D:\Media")?, Some(OsString::from("Media")));

        assert_eq!(
            app.bookmark_index_for_location(&location(r"c:\work\")?),
            Some(0)
        );
        assert_eq!(
            app.bookmark_index_for_location(&location(r"E:\Other")?),
            None
        );

        Ok(())
    }

    #[test]
    fn bookmark_selection_navigates_and_updates_usage() -> ExplorerResult<()> {
        let (mut app, _, _) = explorer(r"C:\root")?;
        app.add_bookmark(location(r"C:\target")?, None);

        app.navigate_active_to_bookmark(0)?;

        assert_eq!(
            app.active_tab()?.current_location().as_path(),
            Path::new(r"C:\target")
        );
        assert_eq!(
            app.state().bookmarks.items()[0].accessibility,
            BookmarkAccessibility::Accessible
        );
        assert!(app.state().bookmarks.items()[0].last_used_at.is_some());

        app.go_back()?;
        assert_eq!(
            app.active_tab()?.current_location().as_path(),
            Path::new(r"C:\root")
        );

        Ok(())
    }

    #[test]
    fn inaccessible_bookmark_preserves_current_location() -> ExplorerResult<()> {
        let (mut app, file_system, _) = explorer(r"C:\root")?;
        app.add_bookmark(location(r"C:\blocked")?, None);
        file_system.deny(r"C:\blocked");

        let error = app
            .navigate_active_to_bookmark(0)
            .expect_err("blocked bookmark should fail");

        assert_eq!(
            app.active_tab()?.current_location().as_path(),
            Path::new(r"C:\root")
        );
        assert_eq!(
            app.state().bookmarks.items()[0].accessibility,
            BookmarkAccessibility::Inaccessible
        );
        assert!(app.state().bookmarks.items()[0].last_used_at.is_none());
        assert_eq!(
            error.user_message(),
            "권한이 없어 작업을 완료할 수 없습니다."
        );

        Ok(())
    }

    #[test]
    fn missing_bookmark_target_preserves_current_location() -> ExplorerResult<()> {
        let (mut app, file_system, _) = explorer(r"C:\root")?;
        app.add_bookmark(location(r"C:\missing")?, Some(OsString::from("Missing")));
        file_system.fail_with_code(r"C:\missing", 3);

        let error = app
            .navigate_active_to_bookmark(0)
            .expect_err("missing bookmark target should fail without changing the tab");

        assert_eq!(
            app.active_tab()?.current_location().as_path(),
            Path::new(r"C:\root")
        );
        assert_eq!(
            app.state().bookmarks.items()[0].accessibility,
            BookmarkAccessibility::Inaccessible
        );
        assert_eq!(error.user_message(), "위치를 찾을 수 없습니다.");

        Ok(())
    }

    #[test]
    fn network_bookmark_failure_preserves_location_and_user_message() -> ExplorerResult<()> {
        let (mut app, file_system, _) = explorer(r"C:\root")?;
        app.add_bookmark(
            location(r"\\offline\share")?,
            Some(OsString::from("Offline")),
        );
        file_system.fail_with_code(r"\\offline\share", 53);

        let error = app
            .navigate_active_to_bookmark(0)
            .expect_err("offline network bookmark should fail without changing the tab");

        assert_eq!(
            app.active_tab()?.current_location().as_path(),
            Path::new(r"C:\root")
        );
        assert_eq!(
            app.state().bookmarks.items()[0].accessibility,
            BookmarkAccessibility::Inaccessible
        );
        assert_eq!(
            error.user_message(),
            "네트워크 위치에 연결할 수 없습니다. 서버 이름, 공유 이름 또는 네트워크 연결을 확인해 주세요."
        );

        Ok(())
    }

    #[test]
    fn bookmark_rename_delete_and_reorder_are_state_only() -> ExplorerResult<()> {
        let (mut app, _, _) = explorer(r"C:\root")?;
        app.add_bookmark(location(r"C:\one")?, None);
        app.add_bookmark(location(r"C:\two")?, None);

        app.rename_bookmark(1, OsString::from("Second"))?;
        app.move_bookmark(1, 0)?;
        let removed = app.delete_bookmark(1)?;

        assert_eq!(app.state().bookmarks.items().len(), 1);
        assert_eq!(
            app.state().bookmarks.items()[0].display_name,
            OsString::from("Second")
        );
        assert_eq!(
            app.state().bookmarks.items()[0].target.as_path(),
            Path::new(r"C:\two")
        );
        assert_eq!(app.state().bookmarks.items()[0].sort_order, 0);
        assert_eq!(removed.target.as_path(), Path::new(r"C:\one"));

        Ok(())
    }
}
