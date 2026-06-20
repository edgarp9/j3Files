use std::collections::HashMap;
use std::ffi::{OsStr, OsString};
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

#[cfg(test)]
use std::fs;

use crate::app::explorer::FolderTreeChildPresenceGateway;
use crate::app::{
    FileSystemGateway, FolderCreationGateway, FolderTreeGateway, ItemListingGateway,
    LocationAccessGateway, NeverCancelSearch, SearchCancellation, SearchFileSystemGateway,
    SearchFileSystemOutcome, SearchProgressReporter,
};
use crate::domain::{
    DisplayOptions, ExplorerError, ExplorerResult, FileAttributes, FileItem, FileItemKind,
    FolderTreeItem, KnownFolderKind, NavigationLocation, NewFolderName, PreparedSearchCriteria,
    SearchCriteria, SearchDiagnostic, SearchProgress, SearchScope, SortState,
};
use crate::platform;

#[derive(Debug, Default, Clone, Copy)]
pub struct NativeFileSystemGateway;

const RECENT_ACCESSIBLE_DIRECTORY_TTL: Duration = Duration::from_secs(2);
const ERROR_FILE_NOT_FOUND_CODE: u32 = 2;
const ERROR_PATH_NOT_FOUND_CODE: u32 = 3;
const DIRECT_EXISTING_CHILD_LOOKUP_LIMIT: usize = 64;

#[derive(Debug)]
struct RecentAccessibleDirectory {
    path: PathBuf,
    checked_at: Instant,
}

static RECENT_ACCESSIBLE_DIRECTORY: OnceLock<Mutex<Option<RecentAccessibleDirectory>>> =
    OnceLock::new();

#[derive(Debug, Default)]
struct TypeNameCache {
    file_folder: Option<OsString>,
    drive: Option<OsString>,
    network_share: Option<OsString>,
    file_without_extension: Option<OsString>,
    other: Option<OsString>,
    file_type_names_by_extension: HashMap<OsString, OsString>,
}

impl TypeNameCache {
    fn type_name_for(&mut self, display_name: &OsStr, kind: FileItemKind) -> OsString {
        match kind {
            FileItemKind::Folder => cached_common_type_name(&mut self.file_folder, "File folder"),
            FileItemKind::Drive => cached_common_type_name(&mut self.drive, "Drive"),
            FileItemKind::NetworkShare => {
                cached_common_type_name(&mut self.network_share, "Network share")
            }
            FileItemKind::File => self.file_type_name(display_name),
            FileItemKind::Other => cached_common_type_name(&mut self.other, "Item"),
        }
    }

    fn file_type_name(&mut self, display_name: &OsStr) -> OsString {
        let Some(extension) = file_name_extension(display_name) else {
            return cached_common_type_name(&mut self.file_without_extension, "File");
        };

        if let Some(type_name) = self.file_type_names_by_extension.get(extension) {
            return type_name.clone();
        }

        let type_name = file_type_name_from_extension(extension);
        self.file_type_names_by_extension
            .insert(extension.to_os_string(), type_name.clone());
        type_name
    }
}

impl NativeFileSystemGateway {
    pub fn new() -> Self {
        Self
    }

    fn item_from_entry(
        &self,
        parent: &NavigationLocation,
        entry: platform::Win32DirectoryEntry,
    ) -> ExplorerResult<FileItem> {
        let path = parent.as_path().join(&entry.file_name);
        self.item_from_entry_at_path(path.as_path(), entry)
    }

    fn item_from_entry_with_type_names(
        &self,
        parent: &NavigationLocation,
        entry: platform::Win32DirectoryEntry,
        type_names: &mut TypeNameCache,
    ) -> ExplorerResult<FileItem> {
        let path = parent.as_path().join(&entry.file_name);
        self.item_from_entry_at_path_with_type_names(path.as_path(), entry, type_names)
    }

    fn item_from_entry_at_path(
        &self,
        path: &Path,
        entry: platform::Win32DirectoryEntry,
    ) -> ExplorerResult<FileItem> {
        self.item_from_entry_at_path_resolving_type_name(path, entry, type_name_for)
    }

    fn item_from_entry_at_path_with_type_names(
        &self,
        path: &Path,
        entry: platform::Win32DirectoryEntry,
        type_names: &mut TypeNameCache,
    ) -> ExplorerResult<FileItem> {
        self.item_from_entry_at_path_resolving_type_name(path, entry, |display_name, kind| {
            type_names.type_name_for(display_name, kind)
        })
    }

    fn item_from_entry_at_path_resolving_type_name(
        &self,
        path: &Path,
        entry: platform::Win32DirectoryEntry,
        mut type_name_for_entry: impl FnMut(&OsStr, FileItemKind) -> OsString,
    ) -> ExplorerResult<FileItem> {
        let display_name = entry.file_name;
        let kind = if entry.attributes.directory {
            FileItemKind::Folder
        } else {
            FileItemKind::File
        };
        let size = if entry.attributes.directory {
            None
        } else {
            Some(entry.file_size)
        };
        let type_name = type_name_for_entry(display_name.as_os_str(), kind);

        Ok(FileItem {
            location: NavigationLocation::from_path(path)?,
            display_name,
            kind,
            type_name,
            size,
            updated_at: entry.last_write_time,
            attributes: FileAttributes {
                hidden: entry.attributes.hidden,
                system: entry.attributes.system,
                read_only: entry.attributes.read_only,
            },
        })
    }

    fn list_items_matching(
        &self,
        location: &NavigationLocation,
        options: DisplayOptions,
        sort: SortState,
        cancellation: &dyn SearchCancellation,
        mut include_entry: impl FnMut(&platform::Win32DirectoryEntry) -> bool,
        mut include_item: impl FnMut(&FileItem) -> bool,
    ) -> ExplorerResult<Vec<FileItem>> {
        if cancellation.is_cancel_requested() {
            return Ok(Vec::new());
        }

        let path = location.as_path();
        consume_recent_accessible_directory(path);

        if cancellation.is_cancel_requested() {
            return Ok(Vec::new());
        }

        let mut items = Vec::new();
        let mut cancelled = false;
        let mut saw_entry = false;
        let mut type_names = TypeNameCache::default();
        let visit_result = platform::visit_directory_entries(path, |entry| {
            if cancellation.is_cancel_requested() {
                cancelled = true;
                return Ok(platform::DirectoryVisit::Stop);
            }

            saw_entry = true;
            if include_entry(&entry) {
                let item =
                    self.item_from_entry_with_type_names(location, entry, &mut type_names)?;
                if options.allows(&item) && include_item(&item) {
                    items.push(item);
                }
            }

            if cancellation.is_cancel_requested() {
                cancelled = true;
                Ok(platform::DirectoryVisit::Stop)
            } else {
                Ok(platform::DirectoryVisit::Continue)
            }
        });
        if let Err(error) = visit_result {
            return Err(refine_directory_visit_error(path, error));
        }

        if cancelled || cancellation.is_cancel_requested() {
            return Ok(items);
        }
        if !saw_entry {
            ensure_directory_location(path)?;
        }

        if !sort.sort_file_items_unless_cancelled(&mut items, || cancellation.is_cancel_requested())
        {
            return Ok(items);
        }
        Ok(items)
    }

    pub fn list_items_with_cancellation(
        &self,
        location: &NavigationLocation,
        options: DisplayOptions,
        sort: SortState,
        cancellation: &dyn SearchCancellation,
    ) -> ExplorerResult<Vec<FileItem>> {
        self.list_items_matching(location, options, sort, cancellation, |_| true, |_| true)
    }

    pub fn item_for_existing_child(
        &self,
        parent: &NavigationLocation,
        child_name: &OsStr,
    ) -> ExplorerResult<Option<FileItem>> {
        let child_path = parent.as_path().join(child_name);
        let Some(entry) = platform::directory_entry(&child_path)? else {
            return Ok(None);
        };

        self.item_from_entry(parent, entry).map(Some)
    }

    pub fn items_for_existing_children(
        &self,
        parent: &NavigationLocation,
        child_names: &[OsString],
    ) -> ExplorerResult<Vec<Option<FileItem>>> {
        if child_names.is_empty() {
            return Ok(Vec::new());
        }
        if should_use_direct_existing_child_lookup(child_names.len()) {
            let mut type_names = TypeNameCache::default();
            return child_names
                .iter()
                .map(|child_name| {
                    let child_path = parent.as_path().join(child_name);
                    let Some(entry) = platform::directory_entry(&child_path)? else {
                        return Ok(None);
                    };

                    self.item_from_entry_with_type_names(parent, entry, &mut type_names)
                        .map(Some)
                })
                .collect();
        }

        let mut pending_indices_by_key =
            HashMap::<Vec<u16>, Vec<usize>>::with_capacity(child_names.len());
        for (index, child_name) in child_names.iter().enumerate() {
            pending_indices_by_key
                .entry(directory_child_lookup_key(child_name.as_os_str()))
                .or_default()
                .push(index);
        }

        let mut items = vec![None; child_names.len()];
        let mut type_names = TypeNameCache::default();
        let visit_result = platform::visit_directory_entries(parent.as_path(), |entry| {
            let key = directory_child_lookup_key(entry.file_name.as_os_str());
            let Some(indices) = pending_indices_by_key.remove(&key) else {
                return Ok(platform::DirectoryVisit::Continue);
            };

            let item = self.item_from_entry_with_type_names(parent, entry, &mut type_names)?;
            if let Some((first_index, remaining_indices)) = indices.split_first() {
                for index in remaining_indices {
                    items[*index] = Some(item.clone());
                }
                items[*first_index] = Some(item);
            }

            if pending_indices_by_key.is_empty() {
                Ok(platform::DirectoryVisit::Stop)
            } else {
                Ok(platform::DirectoryVisit::Continue)
            }
        });

        match visit_result {
            Ok(()) => Ok(items),
            Err(error) if is_missing_directory_entry_error(&error) => {
                Ok(vec![None; child_names.len()])
            }
            Err(error) => Err(error),
        }
    }

    pub fn list_child_folders_with_cancellation(
        &self,
        location: &NavigationLocation,
        options: DisplayOptions,
        sort: SortState,
        cancellation: &dyn SearchCancellation,
    ) -> ExplorerResult<Vec<FileItem>> {
        self.list_items_matching(
            location,
            options,
            sort,
            cancellation,
            |entry| entry.attributes.directory,
            |_| true,
        )
    }

    pub fn list_folder_tree_children_with_cancellation(
        &self,
        location: &NavigationLocation,
        options: DisplayOptions,
        cancellation: &dyn SearchCancellation,
    ) -> ExplorerResult<Vec<FolderTreeItem>> {
        if cancellation.is_cancel_requested() {
            return Ok(Vec::new());
        }

        let path = location.as_path();
        consume_recent_accessible_directory(path);

        if cancellation.is_cancel_requested() {
            return Ok(Vec::new());
        }

        let mut children = Vec::new();
        let mut cancelled = false;
        let mut saw_entry = false;
        let visit_result = platform::visit_directory_entries(path, |entry| {
            if cancellation.is_cancel_requested() {
                cancelled = true;
                return Ok(platform::DirectoryVisit::Stop);
            }

            saw_entry = true;
            if entry.attributes.directory
                && display_options_allow_attributes(options, entry.attributes)
            {
                let child_path = path.join(&entry.file_name);
                children.push(FolderTreeItem::folder_child_from_parts(
                    NavigationLocation::from_path(child_path)?,
                    entry.file_name,
                    1,
                    true,
                )?);
            }

            if cancellation.is_cancel_requested() {
                cancelled = true;
                Ok(platform::DirectoryVisit::Stop)
            } else {
                Ok(platform::DirectoryVisit::Continue)
            }
        });
        if let Err(error) = visit_result {
            return Err(refine_directory_visit_error(path, error));
        }

        if cancelled || cancellation.is_cancel_requested() {
            return Ok(children);
        }
        if !saw_entry {
            ensure_directory_location(path)?;
        }

        sort_folder_tree_children_by_name(&mut children);
        Ok(children)
    }

    pub fn has_child_folders_with_cancellation(
        &self,
        location: &NavigationLocation,
        options: DisplayOptions,
        cancellation: &dyn SearchCancellation,
    ) -> ExplorerResult<bool> {
        if cancellation.is_cancel_requested() {
            return Ok(false);
        }

        let path = location.as_path();

        if cancellation.is_cancel_requested() {
            return Ok(false);
        }

        let mut has_child_folder = false;
        let mut cancelled = false;
        let mut saw_entry = false;
        let visit_result = platform::visit_directory_entries(path, |entry| {
            if cancellation.is_cancel_requested() {
                cancelled = true;
                return Ok(platform::DirectoryVisit::Stop);
            }

            saw_entry = true;
            if !entry.attributes.directory {
                return Ok(platform::DirectoryVisit::Continue);
            }

            if display_options_allow_attributes(options, entry.attributes) {
                has_child_folder = true;
                Ok(platform::DirectoryVisit::Stop)
            } else if cancellation.is_cancel_requested() {
                cancelled = true;
                Ok(platform::DirectoryVisit::Stop)
            } else {
                Ok(platform::DirectoryVisit::Continue)
            }
        });
        if let Err(error) = visit_result {
            return Err(refine_directory_visit_error(path, error));
        }
        if cancelled {
            return Ok(false);
        }
        if !saw_entry {
            ensure_directory_location(path)?;
        }

        Ok(has_child_folder)
    }
}

impl ItemListingGateway for NativeFileSystemGateway {
    fn list_items(
        &self,
        location: &NavigationLocation,
        options: DisplayOptions,
        sort: SortState,
    ) -> ExplorerResult<Vec<FileItem>> {
        self.list_items_matching(
            location,
            options,
            sort,
            &NeverCancelSearch,
            |_| true,
            |_| true,
        )
    }
}

impl FolderTreeGateway for NativeFileSystemGateway {
    fn list_child_folders(
        &self,
        location: &NavigationLocation,
        options: DisplayOptions,
        sort: SortState,
    ) -> ExplorerResult<Vec<FileItem>> {
        self.list_child_folders_with_cancellation(location, options, sort, &NeverCancelSearch)
    }

    fn drive_roots(&self) -> ExplorerResult<Vec<NavigationLocation>> {
        let mut locations = Vec::new();
        for root in platform::logical_drive_roots()? {
            locations.push(NavigationLocation::from_path(root)?);
        }
        Ok(locations)
    }

    fn known_folder(&self, kind: KnownFolderKind) -> ExplorerResult<NavigationLocation> {
        let path = platform::known_folder_path(win32_known_folder(kind))?;
        NavigationLocation::known_folder(kind, path)
    }
}

impl FolderTreeChildPresenceGateway for NativeFileSystemGateway {
    fn has_child_folders(
        &self,
        location: &NavigationLocation,
        options: DisplayOptions,
    ) -> ExplorerResult<bool> {
        self.has_child_folders_with_cancellation(location, options, &NeverCancelSearch)
    }
}

impl LocationAccessGateway for NativeFileSystemGateway {
    fn ensure_accessible(&self, location: &NavigationLocation) -> ExplorerResult<()> {
        let path = location.as_path();
        ensure_directory_location(path)?;
        platform::ensure_directory_listable(path)?;
        remember_accessible_directory(path);
        Ok(())
    }
}

impl FolderCreationGateway for NativeFileSystemGateway {
    fn create_folder(
        &self,
        parent: &NavigationLocation,
        name: &NewFolderName,
    ) -> ExplorerResult<NavigationLocation> {
        self.ensure_accessible(parent)?;
        let path = parent.as_path().join(name.as_os_str());
        platform::create_directory(&path)?;
        NavigationLocation::from_path(path)
    }
}

impl SearchFileSystemGateway for NativeFileSystemGateway {
    fn search_items(
        &self,
        root: &NavigationLocation,
        criteria: &SearchCriteria,
        options: DisplayOptions,
        sort: SortState,
        cancellation: &dyn SearchCancellation,
        progress_reporter: &dyn SearchProgressReporter,
    ) -> ExplorerResult<SearchFileSystemOutcome> {
        let mut outcome = SearchFileSystemOutcome::default();
        if cancellation.is_cancel_requested() {
            outcome.cancelled = true;
            return Ok(outcome);
        }

        let root_access_recently_checked = consume_recent_accessible_directory(root.as_path());
        if !root_access_recently_checked {
            self.ensure_accessible(root)?;
        }
        if cancellation.is_cancel_requested() {
            outcome.cancelled = true;
            return Ok(outcome);
        }
        if criteria.query.is_empty() {
            return Ok(outcome);
        }

        let mut pending_folders = vec![root.clone()];
        let include_subfolders = criteria.scope == SearchScope::IncludeSubfolders;
        let prepared_criteria = PreparedSearchCriteria::new(criteria);
        let mut folded_name = Vec::new();
        let mut last_reported_scanned = 0;
        let mut type_names = TypeNameCache::default();
        let mut omitted_diagnostics = 0;

        while let Some(folder) = pending_folders.pop() {
            if cancellation.is_cancel_requested() {
                outcome.cancelled = true;
                break;
            }

            let is_root = folder.as_path() == root.as_path();
            let scanned_before_visit = outcome.progress.scanned_items;
            outcome.progress.visited_folders += 1;
            let visit_result = platform::visit_directory_entries_until(
                folder.as_path(),
                || cancellation.is_cancel_requested(),
                |entry| {
                    if cancellation.is_cancel_requested() {
                        outcome.cancelled = true;
                        return Ok(platform::DirectoryVisit::Stop);
                    }

                    let is_reparse_point = entry.attributes.reparse_point;
                    let display_allowed =
                        display_options_allow_attributes(options, entry.attributes);
                    let name_matches = display_allowed
                        && prepared_criteria
                            .matches_display_name(entry.file_name.as_os_str(), &mut folded_name);
                    let should_descend = include_subfolders
                        && display_allowed
                        && entry.attributes.directory
                        && !is_reparse_point;

                    if name_matches {
                        let child_path = folder.as_path().join(&entry.file_name);
                        let item = match self.item_from_entry_at_path_with_type_names(
                            child_path.as_path(),
                            entry,
                            &mut type_names,
                        ) {
                            Ok(item) => item,
                            Err(error) => {
                                record_search_diagnostic(
                                    &mut outcome,
                                    &mut omitted_diagnostics,
                                    || child_path,
                                    || error.to_string(),
                                );
                                return Ok(platform::DirectoryVisit::Continue);
                            }
                        };

                        outcome.progress.scanned_items += 1;
                        outcome.progress.matched_items += 1;
                        if should_descend {
                            pending_folders.push(item.location.clone());
                        }
                        outcome.items.push(item);
                    } else {
                        let descend_location = if should_descend {
                            match child_location_from_name(&folder, entry.file_name.as_os_str()) {
                                Ok(location) => Some(location),
                                Err(error) => {
                                    record_search_diagnostic(
                                        &mut outcome,
                                        &mut omitted_diagnostics,
                                        || folder.as_path().join(&entry.file_name),
                                        || error.to_string(),
                                    );
                                    return Ok(platform::DirectoryVisit::Continue);
                                }
                            }
                        } else {
                            None
                        };

                        outcome.progress.scanned_items += 1;
                        if let Some(location) = descend_location {
                            pending_folders.push(location);
                        }
                    }

                    report_search_progress(
                        outcome.progress,
                        progress_reporter,
                        &mut last_reported_scanned,
                        false,
                    );

                    if cancellation.is_cancel_requested() {
                        outcome.cancelled = true;
                        Ok(platform::DirectoryVisit::Stop)
                    } else {
                        Ok(platform::DirectoryVisit::Continue)
                    }
                },
            );

            if let Err(error) = visit_result {
                if outcome.cancelled || cancellation.is_cancel_requested() {
                    outcome.cancelled = true;
                    break;
                }
                if root_access_recently_checked
                    && is_root
                    && outcome.progress.scanned_items == scanned_before_visit
                {
                    return Err(refine_recent_directory_visit_error(folder.as_path(), error));
                }
                record_search_error(
                    &mut outcome,
                    &mut omitted_diagnostics,
                    folder.as_path(),
                    &error,
                );
                report_search_progress(
                    outcome.progress,
                    progress_reporter,
                    &mut last_reported_scanned,
                    true,
                );
                continue;
            }

            if outcome.cancelled || cancellation.is_cancel_requested() {
                outcome.cancelled = true;
                break;
            }
        }

        summarize_omitted_search_diagnostics(&mut outcome, root.as_path(), omitted_diagnostics);
        finalize_search_items(&mut outcome, sort, cancellation);
        progress_reporter.report(outcome.progress);
        Ok(outcome)
    }
}

impl FileSystemGateway for NativeFileSystemGateway {}

fn child_location_from_name(
    parent: &NavigationLocation,
    child_name: &OsStr,
) -> ExplorerResult<NavigationLocation> {
    NavigationLocation::from_path(parent.as_path().join(child_name))
}

fn finalize_search_items(
    outcome: &mut SearchFileSystemOutcome,
    sort: SortState,
    cancellation: &dyn SearchCancellation,
) {
    if outcome.cancelled || cancellation.is_cancel_requested() {
        outcome.cancelled = true;
        return;
    }

    if !sort
        .sort_file_items_unless_cancelled(&mut outcome.items, || cancellation.is_cancel_requested())
    {
        outcome.cancelled = true;
        return;
    }
    if cancellation.is_cancel_requested() {
        outcome.cancelled = true;
    }
}

fn sort_folder_tree_children_by_name(children: &mut [FolderTreeItem]) {
    children.sort_by(|left, right| left.display_name().cmp(right.display_name()));
}

fn win32_known_folder(kind: KnownFolderKind) -> platform::Win32KnownFolder {
    match kind {
        KnownFolderKind::Desktop => platform::Win32KnownFolder::Desktop,
        KnownFolderKind::Downloads => platform::Win32KnownFolder::Downloads,
        KnownFolderKind::Documents => platform::Win32KnownFolder::Documents,
        KnownFolderKind::Home => platform::Win32KnownFolder::Profile,
    }
}

fn record_search_diagnostic(
    outcome: &mut SearchFileSystemOutcome,
    omitted_diagnostics: &mut usize,
    path: impl FnOnce() -> PathBuf,
    detail: impl FnOnce() -> String,
) {
    if outcome.diagnostics.len() < SearchDiagnostic::MAX_RECORDED_DETAILS {
        outcome
            .diagnostics
            .push(SearchDiagnostic::new(path(), detail()));
    } else {
        *omitted_diagnostics = omitted_diagnostics.saturating_add(1);
    }
}

fn summarize_omitted_search_diagnostics(
    outcome: &mut SearchFileSystemOutcome,
    root: &Path,
    omitted_diagnostics: usize,
) {
    if omitted_diagnostics == 0 {
        return;
    }

    outcome.diagnostics.push(SearchDiagnostic::new(
        root.to_path_buf(),
        format!(
            "{} additional search diagnostics omitted after recording the first {}",
            omitted_diagnostics,
            SearchDiagnostic::MAX_RECORDED_DETAILS
        ),
    ));
}

fn record_search_error(
    outcome: &mut SearchFileSystemOutcome,
    omitted_diagnostics: &mut usize,
    path: &Path,
    error: &ExplorerError,
) {
    outcome.progress.skipped_folders += 1;
    record_search_diagnostic(
        outcome,
        omitted_diagnostics,
        || path.to_path_buf(),
        || error.to_string(),
    );
}

fn display_options_allow_attributes(
    options: DisplayOptions,
    attributes: platform::Win32FileAttributes,
) -> bool {
    (options.show_hidden || !attributes.hidden) && (options.show_system || !attributes.system)
}

fn recent_accessible_directory() -> &'static Mutex<Option<RecentAccessibleDirectory>> {
    RECENT_ACCESSIBLE_DIRECTORY.get_or_init(|| Mutex::new(None))
}

fn remember_accessible_directory(path: &Path) {
    let Ok(mut recent) = recent_accessible_directory().lock() else {
        return;
    };

    *recent = Some(RecentAccessibleDirectory {
        path: path.to_path_buf(),
        checked_at: Instant::now(),
    });
}

fn consume_recent_accessible_directory(path: &Path) -> bool {
    let Ok(mut recent) = recent_accessible_directory().lock() else {
        return false;
    };

    let Some(checked) = recent.as_ref() else {
        return false;
    };

    if checked.checked_at.elapsed() > RECENT_ACCESSIBLE_DIRECTORY_TTL {
        *recent = None;
        return false;
    }

    if checked.path.as_path() != path {
        return false;
    }

    *recent = None;
    true
}

fn refine_recent_directory_visit_error(path: &Path, error: ExplorerError) -> ExplorerError {
    refine_directory_visit_error(path, error)
}

fn refine_directory_visit_error(path: &Path, error: ExplorerError) -> ExplorerError {
    match ensure_directory_location(path) {
        Ok(()) => error,
        Err(location_error) => location_error,
    }
}

fn ensure_directory_location(path: &Path) -> ExplorerResult<()> {
    let attributes = platform::file_attributes(path)?;
    if attributes.directory {
        Ok(())
    } else {
        Err(ExplorerError::invalid_location(
            path.to_path_buf(),
            "탐색 위치는 폴더, 드라이브 또는 네트워크 공유여야 합니다.",
        ))
    }
}

fn report_search_progress(
    progress: SearchProgress,
    progress_reporter: &dyn SearchProgressReporter,
    last_reported_scanned: &mut usize,
    force: bool,
) {
    const REPORT_EVERY_SCANNED_ITEMS: usize = 64;

    if force
        || progress
            .scanned_items
            .saturating_sub(*last_reported_scanned)
            >= REPORT_EVERY_SCANNED_ITEMS
    {
        progress_reporter.report(progress);
        *last_reported_scanned = progress.scanned_items;
    }
}

fn type_name_for(display_name: &OsStr, kind: FileItemKind) -> OsString {
    match kind {
        FileItemKind::Folder => OsString::from("File folder"),
        FileItemKind::Drive => OsString::from("Drive"),
        FileItemKind::NetworkShare => OsString::from("Network share"),
        FileItemKind::File => file_type_name(display_name),
        FileItemKind::Other => OsString::from("Item"),
    }
}

fn file_type_name(display_name: &OsStr) -> OsString {
    let Some(extension) = file_name_extension(display_name) else {
        return OsString::from("File");
    };

    file_type_name_from_extension(extension)
}

fn file_name_extension(display_name: &OsStr) -> Option<&OsStr> {
    let extension = Path::new(display_name).extension()?;
    if extension.is_empty() {
        None
    } else {
        Some(extension)
    }
}

fn file_type_name_from_extension(extension: &OsStr) -> OsString {
    let mut type_name = OsString::with_capacity(1 + extension.len() + 5);
    type_name.push(".");
    type_name.push(extension);
    type_name.push(" file");
    type_name
}

fn cached_common_type_name(slot: &mut Option<OsString>, label: &'static str) -> OsString {
    slot.get_or_insert_with(|| OsString::from(label)).clone()
}

fn should_use_direct_existing_child_lookup(child_count: usize) -> bool {
    child_count <= DIRECT_EXISTING_CHILD_LOOKUP_LIMIT
}

fn directory_child_lookup_key(name: &OsStr) -> Vec<u16> {
    let units = name.encode_wide();
    let mut key = Vec::with_capacity(units.size_hint().0);
    for decoded in std::char::decode_utf16(units) {
        match decoded {
            Ok(character) => push_directory_child_case_folded_char(character, &mut key),
            Err(error) => key.push(error.unpaired_surrogate()),
        }
    }
    key
}

fn push_directory_child_case_folded_char(character: char, output: &mut Vec<u16>) {
    for folded in character.to_lowercase() {
        let mut buffer = [0_u16; 2];
        output.extend_from_slice(folded.encode_utf16(&mut buffer));
    }
}

fn is_missing_directory_entry_error(error: &ExplorerError) -> bool {
    matches!(
        error,
        ExplorerError::WindowsApi { code, .. }
            if *code == ERROR_FILE_NOT_FOUND_CODE || *code == ERROR_PATH_NOT_FOUND_CODE
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{NeverCancelSearch, NoopSearchProgressReporter, SearchCancellation};
    use std::cell::Cell;
    use std::path::{Path, PathBuf};
    use std::time::{SystemTime, UNIX_EPOCH};

    struct CancelAfterChecks {
        remaining: Cell<usize>,
    }

    impl CancelAfterChecks {
        fn new(remaining: usize) -> Self {
            Self {
                remaining: Cell::new(remaining),
            }
        }
    }

    impl SearchCancellation for CancelAfterChecks {
        fn is_cancel_requested(&self) -> bool {
            let remaining = self.remaining.get();
            if remaining == 0 {
                true
            } else {
                self.remaining.set(remaining - 1);
                false
            }
        }
    }

    fn test_search_item(name: &str) -> ExplorerResult<FileItem> {
        let path = PathBuf::from(r"C:\search").join(name);
        Ok(FileItem {
            location: NavigationLocation::from_path(path)?,
            display_name: OsString::from(name),
            kind: FileItemKind::File,
            type_name: OsString::from("test item"),
            size: None,
            updated_at: None,
            attributes: FileAttributes::default(),
        })
    }

    fn item_names(items: &[FileItem]) -> Vec<OsString> {
        items.iter().map(|item| item.display_name.clone()).collect()
    }

    fn assert_missing_directory_error<T>(result: ExplorerResult<T>) {
        match result {
            Ok(_) => panic!("expected missing directory error"),
            Err(error) => assert!(
                is_missing_directory_entry_error(&error),
                "expected missing directory error, got {error}"
            ),
        }
    }

    #[test]
    fn search_diagnostics_are_capped_and_summarize_omitted_details() {
        let mut outcome = SearchFileSystemOutcome::default();
        let mut omitted_diagnostics = 0;
        let detail_calls = Cell::new(0);
        let diagnostic_count = SearchDiagnostic::MAX_RECORDED_DETAILS + 3;

        for index in 0..diagnostic_count {
            record_search_diagnostic(
                &mut outcome,
                &mut omitted_diagnostics,
                || PathBuf::from(format!(r"C:\search\{index}")),
                || {
                    detail_calls.set(detail_calls.get() + 1);
                    format!("error {index}")
                },
            );
        }
        summarize_omitted_search_diagnostics(
            &mut outcome,
            Path::new(r"C:\search"),
            omitted_diagnostics,
        );

        assert_eq!(detail_calls.get(), SearchDiagnostic::MAX_RECORDED_DETAILS);
        assert_eq!(
            outcome.diagnostics.len(),
            SearchDiagnostic::MAX_RECORDED_DETAILS + 1
        );
        assert_eq!(omitted_diagnostics, 3);
        assert_eq!(outcome.diagnostics[0].detail, "error 0");
        assert_eq!(
            outcome.diagnostics[SearchDiagnostic::MAX_RECORDED_DETAILS].path,
            PathBuf::from(r"C:\search")
        );
        assert!(outcome.diagnostics[SearchDiagnostic::MAX_RECORDED_DETAILS]
            .detail
            .contains("3 additional search diagnostics omitted"));
    }

    #[test]
    fn search_error_recording_keeps_skipped_count_after_diagnostic_cap() {
        let mut outcome = SearchFileSystemOutcome::default();
        let mut omitted_diagnostics = 0;
        let error = ExplorerError::invalid_input("test search diagnostic");
        let skipped_count = SearchDiagnostic::MAX_RECORDED_DETAILS + 2;

        for _ in 0..skipped_count {
            record_search_error(
                &mut outcome,
                &mut omitted_diagnostics,
                Path::new(r"C:\blocked"),
                &error,
            );
        }

        assert_eq!(outcome.progress.skipped_folders, skipped_count);
        assert_eq!(
            outcome.diagnostics.len(),
            SearchDiagnostic::MAX_RECORDED_DETAILS
        );
        assert_eq!(omitted_diagnostics, 2);
    }

    #[test]
    fn type_name_uses_extension_for_files() {
        assert_eq!(
            type_name_for(OsStr::new("notes.md"), FileItemKind::File),
            OsString::from(".md file")
        );
        assert_eq!(
            type_name_for(OsStr::new("LICENSE"), FileItemKind::File),
            OsString::from("File")
        );
    }

    #[test]
    fn type_name_uses_folder_label_for_folders() {
        assert_eq!(
            type_name_for(OsStr::new("src"), FileItemKind::Folder),
            OsString::from("File folder")
        );
    }

    #[test]
    fn lists_unicode_file_names_from_win32_directory_entries(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let temp_dir = TempDirectory::new()?;
        let file_path = temp_dir.path().join("한글.txt");
        let folder_path = temp_dir.path().join("폴더");
        fs::write(&file_path, b"hello")?;
        fs::create_dir(&folder_path)?;

        let gateway = NativeFileSystemGateway::new();
        let location = NavigationLocation::from_path(temp_dir.path().to_path_buf())?;
        let items =
            gateway.list_items(&location, DisplayOptions::default(), SortState::default())?;

        let file = items
            .iter()
            .find(|item| item.display_name.as_os_str() == OsStr::new("한글.txt"));
        let folder = items
            .iter()
            .find(|item| item.display_name.as_os_str() == OsStr::new("폴더"));

        assert!(matches!(
            file.map(|item| item.kind),
            Some(FileItemKind::File)
        ));
        assert_eq!(file.and_then(|item| item.size), Some(5));
        assert_eq!(
            file.map(|item| item.type_name.as_os_str()),
            Some(OsStr::new(".txt file"))
        );
        assert!(file.and_then(|item| item.updated_at).is_some());
        assert!(matches!(
            folder.map(|item| item.kind),
            Some(FileItemKind::Folder)
        ));

        Ok(())
    }

    #[test]
    fn items_for_existing_children_returns_matches_in_requested_order(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let temp_dir = TempDirectory::new()?;
        fs::write(temp_dir.path().join("Alpha.txt"), b"alpha")?;
        fs::write(temp_dir.path().join("beta.txt"), b"beta")?;

        let gateway = NativeFileSystemGateway::new();
        let location = NavigationLocation::from_path(temp_dir.path().to_path_buf())?;
        let requested = vec![
            OsString::from("BETA.TXT"),
            OsString::from("missing.txt"),
            OsString::from("alpha.txt"),
        ];

        let items = gateway.items_for_existing_children(&location, &requested)?;
        let names = items
            .iter()
            .map(|item| item.as_ref().map(|item| item.display_name.clone()))
            .collect::<Vec<_>>();

        assert_eq!(
            names,
            vec![
                Some(OsString::from("beta.txt")),
                None,
                Some(OsString::from("Alpha.txt"))
            ]
        );
        assert_eq!(items[0].as_ref().and_then(|item| item.size), Some(4));
        assert_eq!(items[2].as_ref().and_then(|item| item.size), Some(5));

        Ok(())
    }

    #[test]
    fn direct_existing_child_lookup_covers_incremental_file_watch_batches() {
        assert!(should_use_direct_existing_child_lookup(9));
        assert!(should_use_direct_existing_child_lookup(
            DIRECT_EXISTING_CHILD_LOOKUP_LIMIT
        ));
        assert!(!should_use_direct_existing_child_lookup(
            DIRECT_EXISTING_CHILD_LOOKUP_LIMIT + 1
        ));
    }

    #[test]
    fn items_for_existing_children_treats_missing_parent_as_missing_children(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let temp_dir = TempDirectory::new()?;
        let missing_location =
            NavigationLocation::from_path(temp_dir.path().join("missing-parent"))?;
        let requested = vec![OsString::from("a.txt"), OsString::from("b.txt")];

        let items = NativeFileSystemGateway::new()
            .items_for_existing_children(&missing_location, &requested)?;

        assert_eq!(items.len(), requested.len());
        assert!(items.iter().all(Option::is_none));

        Ok(())
    }

    #[test]
    fn listing_cancellation_stops_during_directory_visit() -> Result<(), Box<dyn std::error::Error>>
    {
        let temp_dir = TempDirectory::new()?;
        fs::write(temp_dir.path().join("a.txt"), b"a")?;
        fs::write(temp_dir.path().join("b.txt"), b"b")?;
        fs::write(temp_dir.path().join("c.txt"), b"c")?;

        let gateway = NativeFileSystemGateway::new();
        let location = NavigationLocation::from_path(temp_dir.path().to_path_buf())?;
        let cancellation = CancelAfterChecks::new(3);
        let items = gateway.list_items_with_cancellation(
            &location,
            DisplayOptions::default(),
            SortState::default(),
            &cancellation,
        )?;

        assert_eq!(items.len(), 1);

        Ok(())
    }

    #[test]
    fn listing_paths_accept_empty_directories() -> Result<(), Box<dyn std::error::Error>> {
        let temp_dir = TempDirectory::new()?;
        let gateway = NativeFileSystemGateway::new();
        let location = NavigationLocation::from_path(temp_dir.path().to_path_buf())?;

        let items =
            gateway.list_items(&location, DisplayOptions::default(), SortState::default())?;
        let tree_children = gateway.list_folder_tree_children_with_cancellation(
            &location,
            DisplayOptions::default(),
            &NeverCancelSearch,
        )?;
        let has_child_folders = gateway.has_child_folders_with_cancellation(
            &location,
            DisplayOptions::default(),
            &NeverCancelSearch,
        )?;

        assert!(items.is_empty());
        assert!(tree_children.is_empty());
        assert!(!has_child_folders);

        Ok(())
    }

    #[test]
    fn listing_paths_report_missing_directories() -> Result<(), Box<dyn std::error::Error>> {
        let temp_dir = TempDirectory::new()?;
        let gateway = NativeFileSystemGateway::new();
        let location = NavigationLocation::from_path(temp_dir.path().join("missing"))?;

        assert_missing_directory_error(gateway.list_items(
            &location,
            DisplayOptions::default(),
            SortState::default(),
        ));
        assert_missing_directory_error(gateway.list_folder_tree_children_with_cancellation(
            &location,
            DisplayOptions::default(),
            &NeverCancelSearch,
        ));
        assert_missing_directory_error(gateway.has_child_folders_with_cancellation(
            &location,
            DisplayOptions::default(),
            &NeverCancelSearch,
        ));

        Ok(())
    }

    #[test]
    fn cancelled_search_finalization_skips_partial_result_sort() -> ExplorerResult<()> {
        let mut outcome = SearchFileSystemOutcome {
            items: vec![
                test_search_item("b-report.txt")?,
                test_search_item("a-report.txt")?,
            ],
            cancelled: true,
            ..SearchFileSystemOutcome::default()
        };

        finalize_search_items(&mut outcome, SortState::default(), &NeverCancelSearch);

        assert!(outcome.cancelled);
        assert_eq!(
            item_names(&outcome.items),
            vec![
                OsString::from("b-report.txt"),
                OsString::from("a-report.txt")
            ]
        );
        Ok(())
    }

    #[test]
    fn search_finalization_honors_late_cancellation_before_sort() -> ExplorerResult<()> {
        let mut outcome = SearchFileSystemOutcome {
            items: vec![
                test_search_item("b-report.txt")?,
                test_search_item("a-report.txt")?,
            ],
            ..SearchFileSystemOutcome::default()
        };
        let cancellation = CancelAfterChecks::new(0);

        finalize_search_items(&mut outcome, SortState::default(), &cancellation);

        assert!(outcome.cancelled);
        assert_eq!(
            item_names(&outcome.items),
            vec![
                OsString::from("b-report.txt"),
                OsString::from("a-report.txt")
            ]
        );
        Ok(())
    }

    #[test]
    fn search_finalization_honors_late_cancellation_after_sort() -> ExplorerResult<()> {
        let mut outcome = SearchFileSystemOutcome {
            items: vec![
                test_search_item("b-report.txt")?,
                test_search_item("a-report.txt")?,
            ],
            ..SearchFileSystemOutcome::default()
        };
        let cancellation = CancelAfterChecks::new(8);

        finalize_search_items(&mut outcome, SortState::default(), &cancellation);

        assert!(outcome.cancelled);
        assert_eq!(
            item_names(&outcome.items),
            vec![
                OsString::from("a-report.txt"),
                OsString::from("b-report.txt")
            ]
        );
        Ok(())
    }

    #[test]
    fn search_finalization_honors_cancellation_during_sort() -> ExplorerResult<()> {
        let mut outcome = SearchFileSystemOutcome {
            items: vec![
                test_search_item("d-report.txt")?,
                test_search_item("c-report.txt")?,
                test_search_item("b-report.txt")?,
                test_search_item("a-report.txt")?,
            ],
            ..SearchFileSystemOutcome::default()
        };
        let cancellation = CancelAfterChecks::new(7);

        finalize_search_items(&mut outcome, SortState::default(), &cancellation);

        assert!(outcome.cancelled);
        assert_eq!(
            item_names(&outcome.items),
            vec![
                OsString::from("d-report.txt"),
                OsString::from("c-report.txt"),
                OsString::from("b-report.txt"),
                OsString::from("a-report.txt")
            ]
        );
        Ok(())
    }

    #[test]
    fn completed_search_finalization_sorts_result_items() -> ExplorerResult<()> {
        let mut outcome = SearchFileSystemOutcome {
            items: vec![
                test_search_item("b-report.txt")?,
                test_search_item("a-report.txt")?,
            ],
            ..SearchFileSystemOutcome::default()
        };

        finalize_search_items(&mut outcome, SortState::default(), &NeverCancelSearch);

        assert!(!outcome.cancelled);
        assert_eq!(
            item_names(&outcome.items),
            vec![
                OsString::from("a-report.txt"),
                OsString::from("b-report.txt")
            ]
        );
        Ok(())
    }

    #[test]
    fn list_child_folders_returns_only_folders() -> Result<(), Box<dyn std::error::Error>> {
        let temp_dir = TempDirectory::new()?;
        fs::create_dir(temp_dir.path().join("z-folder"))?;
        fs::create_dir(temp_dir.path().join("a-folder"))?;
        fs::write(temp_dir.path().join("middle.txt"), b"file")?;

        let gateway = NativeFileSystemGateway::new();
        let location = NavigationLocation::from_path(temp_dir.path().to_path_buf())?;
        let items = gateway.list_child_folders(
            &location,
            DisplayOptions::default(),
            SortState::default(),
        )?;
        let names = items
            .iter()
            .map(|item| item.display_name.clone())
            .collect::<Vec<_>>();

        assert_eq!(
            names,
            vec![OsString::from("a-folder"), OsString::from("z-folder")]
        );
        assert!(items.iter().all(FileItem::is_folder));

        Ok(())
    }

    #[test]
    fn list_folder_tree_children_returns_sorted_tree_items(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let temp_dir = TempDirectory::new()?;
        fs::create_dir(temp_dir.path().join("z-folder"))?;
        fs::create_dir(temp_dir.path().join("a-folder"))?;
        fs::write(temp_dir.path().join("middle.txt"), b"file")?;

        let gateway = NativeFileSystemGateway::new();
        let location = NavigationLocation::from_path(temp_dir.path().to_path_buf())?;
        let items = gateway.list_folder_tree_children_with_cancellation(
            &location,
            DisplayOptions::default(),
            &NeverCancelSearch,
        )?;
        let names = items
            .iter()
            .map(|item| item.display_name().to_os_string())
            .collect::<Vec<_>>();

        assert_eq!(
            names,
            vec![OsString::from("a-folder"), OsString::from("z-folder")]
        );
        assert!(items.iter().all(|item| item.depth() == 1));
        assert!(items.iter().all(FolderTreeItem::has_children));

        Ok(())
    }

    #[test]
    fn has_child_folders_reports_folder_presence_without_requiring_listing(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let temp_dir = TempDirectory::new()?;
        fs::write(temp_dir.path().join("readme.txt"), b"file")?;

        let gateway = NativeFileSystemGateway::new();
        let location = NavigationLocation::from_path(temp_dir.path().to_path_buf())?;
        assert!(!gateway.has_child_folders(&location, DisplayOptions::default())?);

        fs::create_dir(temp_dir.path().join("child"))?;
        assert!(gateway.has_child_folders(&location, DisplayOptions::default())?);

        Ok(())
    }

    #[test]
    fn child_presence_cancellation_can_stop_before_directory_access(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let gateway = NativeFileSystemGateway::new();
        let location = NavigationLocation::from_path(PathBuf::from(
            r"C:\j3files-cancelled-child-presence-missing",
        ))?;
        let cancellation = CancelAfterChecks::new(0);

        assert!(!gateway.has_child_folders_with_cancellation(
            &location,
            DisplayOptions::default(),
            &cancellation,
        )?);

        Ok(())
    }

    #[test]
    fn searches_current_folder_without_descending() -> Result<(), Box<dyn std::error::Error>> {
        let temp_dir = TempDirectory::new()?;
        fs::write(temp_dir.path().join("report.txt"), b"root")?;
        fs::write(temp_dir.path().join("notes.txt"), b"notes")?;
        let nested_dir = temp_dir.path().join("nested");
        fs::create_dir(&nested_dir)?;
        fs::write(nested_dir.join("report.txt"), b"nested")?;

        let gateway = NativeFileSystemGateway::new();
        let location = NavigationLocation::from_path(temp_dir.path().to_path_buf())?;
        let criteria = SearchCriteria {
            query: "report".to_string(),
            scope: SearchScope::CurrentFolder,
        };

        let outcome = gateway.search_items(
            &location,
            &criteria,
            DisplayOptions::default(),
            SortState::default(),
            &NeverCancelSearch,
            &NoopSearchProgressReporter,
        )?;

        let names = outcome
            .items
            .iter()
            .map(|item| item.display_name.clone())
            .collect::<Vec<_>>();
        assert_eq!(names, vec![OsString::from("report.txt")]);
        assert_eq!(outcome.progress.scanned_items, 3);
        assert_eq!(outcome.progress.matched_items, 1);
        assert!(!outcome.cancelled);

        Ok(())
    }

    #[test]
    fn empty_search_query_returns_without_scanning() -> Result<(), Box<dyn std::error::Error>> {
        let temp_dir = TempDirectory::new()?;
        fs::write(temp_dir.path().join("report.txt"), b"root")?;
        let nested_dir = temp_dir.path().join("nested");
        fs::create_dir(&nested_dir)?;
        fs::write(nested_dir.join("nested-report.txt"), b"nested")?;

        let gateway = NativeFileSystemGateway::new();
        let location = NavigationLocation::from_path(temp_dir.path().to_path_buf())?;
        let criteria = SearchCriteria {
            query: String::new(),
            scope: SearchScope::IncludeSubfolders,
        };

        let outcome = gateway.search_items(
            &location,
            &criteria,
            DisplayOptions::default(),
            SortState::default(),
            &NeverCancelSearch,
            &NoopSearchProgressReporter,
        )?;

        assert!(outcome.items.is_empty());
        assert_eq!(outcome.progress.scanned_items, 0);
        assert_eq!(outcome.progress.matched_items, 0);
        assert_eq!(outcome.progress.visited_folders, 0);
        assert!(!outcome.cancelled);

        Ok(())
    }

    #[test]
    fn search_cancellation_before_visit_marks_cancelled() -> Result<(), Box<dyn std::error::Error>>
    {
        let temp_dir = TempDirectory::new()?;
        fs::write(temp_dir.path().join("report.txt"), b"root")?;

        let gateway = NativeFileSystemGateway::new();
        let location = NavigationLocation::from_path(temp_dir.path().to_path_buf())?;
        let criteria = SearchCriteria {
            query: "report".to_string(),
            scope: SearchScope::CurrentFolder,
        };
        let cancellation = CancelAfterChecks::new(3);

        let outcome = gateway.search_items(
            &location,
            &criteria,
            DisplayOptions::default(),
            SortState::default(),
            &cancellation,
            &NoopSearchProgressReporter,
        )?;

        assert!(outcome.cancelled);
        assert_eq!(outcome.progress.scanned_items, 0);
        assert!(outcome.items.is_empty());

        Ok(())
    }

    #[test]
    fn searches_subfolders_when_requested() -> Result<(), Box<dyn std::error::Error>> {
        let temp_dir = TempDirectory::new()?;
        let nested_dir = temp_dir.path().join("nested");
        fs::create_dir(&nested_dir)?;
        fs::write(nested_dir.join("report.txt"), b"nested")?;

        let gateway = NativeFileSystemGateway::new();
        let location = NavigationLocation::from_path(temp_dir.path().to_path_buf())?;
        let criteria = SearchCriteria {
            query: "report".to_string(),
            scope: SearchScope::IncludeSubfolders,
        };

        let outcome = gateway.search_items(
            &location,
            &criteria,
            DisplayOptions::default(),
            SortState::default(),
            &NeverCancelSearch,
            &NoopSearchProgressReporter,
        )?;

        assert!(outcome
            .items
            .iter()
            .any(|item| item.location.as_path().ends_with(r"nested\report.txt")));
        assert_eq!(outcome.progress.scanned_items, 2);
        assert_eq!(outcome.progress.matched_items, 1);

        Ok(())
    }

    struct TempDirectory {
        path: PathBuf,
    }

    impl TempDirectory {
        fn new() -> Result<Self, Box<dyn std::error::Error>> {
            let unique = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
            let path = std::env::temp_dir().join(format!(
                "j3files-listing-test-{}-{unique}",
                std::process::id()
            ));
            fs::create_dir(&path)?;
            Ok(Self { path })
        }

        fn path(&self) -> &Path {
            &self.path
        }
    }

    impl Drop for TempDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }
}
