use std::collections::{HashMap, HashSet};
use std::ffi::{OsStr, OsString};
use std::path::PathBuf;
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc,
};

use crate::domain::{ExplorerResult, FileItem, FileItemKind, KnownFolderKind, NavigationLocation};
use crate::platform::{self, ShellIconQuery, ShellImageListHandle};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShellFileIcon {
    system_image_index: i32,
}

impl ShellFileIcon {
    pub fn system_image_index(self) -> i32 {
        self.system_image_index
    }
}

pub type ShellIconLoadCompletion = Box<dyn FnOnce(&mut ShellIconCache) -> bool + Send>;

pub struct ShellIconLoadTask {
    generation: u64,
    current_generation: Arc<AtomicU64>,
    load: Box<dyn FnOnce() -> ShellIconLoadCompletion + Send>,
}

impl ShellIconLoadTask {
    fn new(
        generation: u64,
        current_generation: Arc<AtomicU64>,
        load: impl FnOnce() -> ShellIconLoadCompletion + Send + 'static,
    ) -> Self {
        Self {
            generation,
            current_generation,
            load: Box::new(load),
        }
    }

    pub fn is_stale(&self) -> bool {
        self.current_generation.load(Ordering::Relaxed) != self.generation
    }

    pub fn run(self) -> ShellIconLoadCompletion {
        if self.current_generation.load(Ordering::Relaxed) != self.generation {
            return Box::new(|_: &mut ShellIconCache| false);
        }

        (self.load)()
    }
}

#[derive(Debug)]
pub struct ShellIconCache {
    image_list: ShellImageListHandle,
    entries: HashMap<ShellIconCacheKey, ShellFileIcon>,
    file_extension_entries: HashMap<OsString, ShellFileIcon>,
    normalized_file_extensions: HashMap<OsString, OsString>,
    pending_loads: HashSet<ShellIconCacheBucket>,
    generation: u64,
    current_generation: Arc<AtomicU64>,
    default_file_icon: ShellFileIcon,
    default_folder_icon: ShellFileIcon,
}

impl ShellIconCache {
    pub fn new() -> ExplorerResult<Self> {
        let default_file = platform::shell_file_icon(&ShellIconQuery::generic_file(
            default_file_placeholder_path(),
        ))?;
        let default_folder = platform::shell_file_icon(&ShellIconQuery::generic_folder(
            default_folder_placeholder_path(),
        ))?;

        Ok(Self {
            image_list: default_folder.image_list,
            entries: HashMap::new(),
            file_extension_entries: HashMap::new(),
            normalized_file_extensions: HashMap::new(),
            pending_loads: HashSet::new(),
            generation: 0,
            current_generation: Arc::new(AtomicU64::new(0)),
            default_file_icon: default_file.icon.into(),
            default_folder_icon: default_folder.icon.into(),
        })
    }

    pub fn system_image_list(&self) -> ShellImageListHandle {
        self.image_list
    }

    pub fn invalidate_all(&mut self) {
        self.entries.clear();
        self.file_extension_entries.clear();
        self.normalized_file_extensions.clear();
        self.advance_generation();
    }

    pub fn invalidate_location_entries(&mut self) {
        self.entries.retain(|key, _| !key.is_location_specific());
        self.advance_generation();
    }

    pub fn invalidate_location(&mut self, location: &NavigationLocation) {
        let mut invalidated_location_icon = false;
        match location {
            NavigationLocation::DriveRoot(path) => {
                self.entries
                    .remove(&ShellIconCacheKey::DriveRoot(path.clone()));
                invalidated_location_icon = true;
            }
            NavigationLocation::NetworkShare(path) => {
                self.entries
                    .remove(&ShellIconCacheKey::NetworkShare(path.clone()));
                invalidated_location_icon = true;
            }
            NavigationLocation::KnownFolder { kind, .. } => {
                self.entries.remove(&ShellIconCacheKey::KnownFolder(*kind));
                invalidated_location_icon = true;
            }
            NavigationLocation::LocalPath(_) => {}
        }
        if invalidated_location_icon {
            self.advance_generation();
        }
    }

    pub fn invalidate_item(&mut self, item: &FileItem) {
        if let Some(extension) = local_file_extension(item) {
            self.invalidate_file_extension(extension);
            return;
        }

        self.entries.remove(&ShellIconCacheKey::from_item(item));
        self.advance_generation();
    }

    pub fn invalidate_item_replacement_if_needed(
        &mut self,
        existing: &FileItem,
        updated: &FileItem,
    ) {
        if item_replacement_requires_icon_cache_invalidation(existing, updated) {
            self.invalidate_item(existing);
            self.invalidate_item(updated);
        }
    }

    pub fn invalidate_item_presence_change_if_needed(&mut self, item: &FileItem) {
        if item_presence_change_requires_icon_cache_invalidation(item) {
            self.invalidate_item(item);
        }
    }

    pub fn icon_for_item(&mut self, item: &FileItem) -> ShellFileIcon {
        if let Some(extension) = local_file_extension(item) {
            return self.icon_for_file_extension(extension, self.default_file_icon);
        }

        let key = ShellIconCacheKey::from_item(item);
        if let Some(icon) = self.entries.get(&key).copied() {
            return icon;
        }

        let fallback = self.fallback_for_item(item);
        let icon = icon_query_for_item(&key, item)
            .and_then(|query| platform::shell_file_icon(&query).ok())
            .map(|lookup| lookup.icon.into())
            .unwrap_or(fallback);

        self.entries.insert(key, icon);
        icon
    }

    pub fn cached_or_default_icon_for_item(&self, item: &FileItem) -> ShellFileIcon {
        let bucket = item_icon_cache_bucket(item);
        let fallback = self.fallback_for_item(item);
        self.cached_icon_for_bucket(&bucket).unwrap_or(fallback)
    }

    pub fn request_icon_load_for_item<E>(
        &mut self,
        item: &FileItem,
        start: impl FnOnce(ShellIconLoadTask) -> Result<(), E>,
    ) -> Result<bool, E> {
        let bucket = item_icon_cache_bucket(item);
        if self.cached_icon_for_bucket(&bucket).is_some() || self.pending_loads.contains(&bucket) {
            return Ok(false);
        }

        let Some(query) = icon_query_for_bucket(&bucket, item) else {
            return Ok(false);
        };

        self.pending_loads.insert(bucket.clone());
        let generation = self.generation;
        let current_generation = Arc::clone(&self.current_generation);
        let fallback = self.fallback_for_item(item);
        let load_bucket = bucket.clone();
        let task = ShellIconLoadTask::new(generation, current_generation, move || {
            let icon = platform::shell_file_icon(&query)
                .map(|lookup| lookup.icon.into())
                .unwrap_or(fallback);
            Box::new(move |cache: &mut ShellIconCache| {
                cache.finish_icon_load(generation, load_bucket, icon)
            }) as ShellIconLoadCompletion
        });

        match start(task) {
            Ok(()) => Ok(true),
            Err(error) => {
                self.pending_loads.remove(&bucket);
                Err(error)
            }
        }
    }

    fn icon_for_file_extension(
        &mut self,
        extension: &OsStr,
        fallback: ShellFileIcon,
    ) -> ShellFileIcon {
        if let Some(normalized_extension) = self.normalized_file_extensions.get(extension) {
            return icon_for_normalized_file_extension(
                &mut self.file_extension_entries,
                normalized_extension.as_os_str(),
                fallback,
            );
        }

        let normalized_extension = normalized_file_extension(extension);
        let icon = icon_for_normalized_file_extension(
            &mut self.file_extension_entries,
            normalized_extension.as_os_str(),
            fallback,
        );
        self.normalized_file_extensions
            .insert(extension.to_os_string(), normalized_extension);
        icon
    }

    fn invalidate_file_extension(&mut self, extension: &OsStr) {
        if let Some(normalized_extension) = self.normalized_file_extensions.get(extension) {
            self.file_extension_entries
                .remove(normalized_extension.as_os_str());
            self.advance_generation();
            return;
        }

        let normalized_extension = normalized_file_extension(extension);
        self.file_extension_entries
            .remove(normalized_extension.as_os_str());
        self.advance_generation();
    }

    fn fallback_for_item(&self, item: &FileItem) -> ShellFileIcon {
        if item.is_folder() {
            self.default_folder_icon
        } else {
            self.default_file_icon
        }
    }

    fn cached_icon_for_bucket(&self, bucket: &ShellIconCacheBucket) -> Option<ShellFileIcon> {
        match bucket {
            ShellIconCacheBucket::FileExtension(extension) => self
                .file_extension_entries
                .get(extension.as_os_str())
                .copied(),
            ShellIconCacheBucket::Entry(key) => self.entries.get(key).copied(),
        }
    }

    fn finish_icon_load(
        &mut self,
        generation: u64,
        bucket: ShellIconCacheBucket,
        icon: ShellFileIcon,
    ) -> bool {
        if generation != self.generation {
            return false;
        }

        self.pending_loads.remove(&bucket);
        self.insert_icon_for_bucket(bucket, icon)
    }

    fn insert_icon_for_bucket(
        &mut self,
        bucket: ShellIconCacheBucket,
        icon: ShellFileIcon,
    ) -> bool {
        let previous = match bucket {
            ShellIconCacheBucket::FileExtension(extension) => {
                self.file_extension_entries.insert(extension, icon)
            }
            ShellIconCacheBucket::Entry(key) => self.entries.insert(key, icon),
        };
        previous != Some(icon)
    }

    fn advance_generation(&mut self) {
        self.pending_loads.clear();
        self.generation = self.generation.wrapping_add(1);
        self.current_generation
            .store(self.generation, Ordering::Relaxed);
    }
}

fn local_file_extension(item: &FileItem) -> Option<&OsStr> {
    match (&item.location, item.kind) {
        (NavigationLocation::LocalPath(_), FileItemKind::File) => {
            item.extension().filter(|extension| !extension.is_empty())
        }
        _ => None,
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum ShellIconCacheKey {
    FileExtension(OsString),
    FileWithoutExtension,
    Folder,
    DriveRoot(PathBuf),
    NetworkShare(PathBuf),
    KnownFolder(KnownFolderKind),
    Other,
}

impl ShellIconCacheKey {
    fn from_item(item: &FileItem) -> Self {
        match &item.location {
            NavigationLocation::DriveRoot(path) => return Self::DriveRoot(path.clone()),
            NavigationLocation::NetworkShare(path) => return Self::NetworkShare(path.clone()),
            NavigationLocation::KnownFolder { kind, .. } => return Self::KnownFolder(*kind),
            NavigationLocation::LocalPath(_) => {}
        }

        match item.kind {
            FileItemKind::File => item
                .extension()
                .filter(|extension| !extension.is_empty())
                .map(|extension| Self::FileExtension(normalized_file_extension(extension)))
                .unwrap_or(Self::FileWithoutExtension),
            FileItemKind::Folder => Self::Folder,
            FileItemKind::Drive => Self::DriveRoot(item.location.as_path().to_path_buf()),
            FileItemKind::NetworkShare => Self::NetworkShare(item.location.as_path().to_path_buf()),
            FileItemKind::Other => Self::Other,
        }
    }

    fn is_location_specific(&self) -> bool {
        matches!(
            self,
            Self::DriveRoot(_) | Self::NetworkShare(_) | Self::KnownFolder(_)
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum ShellIconCacheBucket {
    FileExtension(OsString),
    Entry(ShellIconCacheKey),
}

impl ShellIconCacheBucket {
    fn is_location_specific(&self) -> bool {
        match self {
            Self::FileExtension(_) => false,
            Self::Entry(key) => key.is_location_specific(),
        }
    }
}

impl From<platform::ShellIconIndex> for ShellFileIcon {
    fn from(icon: platform::ShellIconIndex) -> Self {
        Self {
            system_image_index: icon.system_image_index(),
        }
    }
}

fn item_icon_cache_bucket(item: &FileItem) -> ShellIconCacheBucket {
    if let Some(extension) = local_file_extension(item) {
        return ShellIconCacheBucket::FileExtension(normalized_file_extension(extension));
    }

    ShellIconCacheBucket::Entry(ShellIconCacheKey::from_item(item))
}

fn item_replacement_requires_icon_cache_invalidation(
    existing: &FileItem,
    updated: &FileItem,
) -> bool {
    let existing_bucket = item_icon_cache_bucket(existing);
    let updated_bucket = item_icon_cache_bucket(updated);
    existing_bucket != updated_bucket
        || existing_bucket.is_location_specific()
        || updated_bucket.is_location_specific()
}

fn item_presence_change_requires_icon_cache_invalidation(item: &FileItem) -> bool {
    item_icon_cache_bucket(item).is_location_specific()
}

fn icon_query_for_item(key: &ShellIconCacheKey, item: &FileItem) -> Option<ShellIconQuery> {
    match key {
        ShellIconCacheKey::FileExtension(extension) => Some(ShellIconQuery::generic_file(
            file_placeholder_path(Some(extension.as_os_str())),
        )),
        ShellIconCacheKey::FileWithoutExtension => {
            Some(ShellIconQuery::generic_file(file_placeholder_path(None)))
        }
        ShellIconCacheKey::Folder => Some(ShellIconQuery::generic_folder(
            default_folder_placeholder_path(),
        )),
        ShellIconCacheKey::DriveRoot(path) => Some(ShellIconQuery::drive(path.clone())),
        ShellIconCacheKey::NetworkShare(path) => Some(ShellIconQuery::network_share(path.clone())),
        ShellIconCacheKey::KnownFolder(_) => Some(ShellIconQuery::known_folder(
            item.location.as_path().to_path_buf(),
        )),
        ShellIconCacheKey::Other => None,
    }
}

fn icon_query_for_bucket(bucket: &ShellIconCacheBucket, item: &FileItem) -> Option<ShellIconQuery> {
    match bucket {
        ShellIconCacheBucket::FileExtension(extension) => Some(ShellIconQuery::generic_file(
            file_placeholder_path(Some(extension.as_os_str())),
        )),
        ShellIconCacheBucket::Entry(key) => icon_query_for_item(key, item),
    }
}

fn icon_for_normalized_file_extension(
    file_extension_entries: &mut HashMap<OsString, ShellFileIcon>,
    normalized_extension: &OsStr,
    fallback: ShellFileIcon,
) -> ShellFileIcon {
    if let Some(icon) = file_extension_entries.get(normalized_extension).copied() {
        return icon;
    }

    let icon = platform::shell_file_icon(&ShellIconQuery::generic_file(file_placeholder_path(
        Some(normalized_extension),
    )))
    .map(|lookup| lookup.icon.into())
    .unwrap_or(fallback);
    file_extension_entries.insert(normalized_extension.to_os_string(), icon);
    icon
}

fn default_file_placeholder_path() -> PathBuf {
    file_placeholder_path(None)
}

fn default_folder_placeholder_path() -> PathBuf {
    PathBuf::from("j3files-folder")
}

fn file_placeholder_path(extension: Option<&OsStr>) -> PathBuf {
    let mut name = OsString::from("j3files-file");
    if let Some(extension) = extension {
        name.push(".");
        name.push(extension);
    }
    PathBuf::from(name)
}

fn normalized_file_extension(extension: &OsStr) -> OsString {
    extension
        .to_str()
        .map(|extension| OsString::from(extension.to_lowercase()))
        .unwrap_or_else(|| extension.to_os_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{FileAttributes, NavigationLocation};
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn item(path: &str, kind: FileItemKind) -> ExplorerResult<FileItem> {
        let location = NavigationLocation::from_path(PathBuf::from(path))?;
        let display_name = location.display_name();
        Ok(FileItem {
            location,
            display_name,
            kind,
            type_name: OsString::from("test item"),
            size: None,
            updated_at: None,
            attributes: FileAttributes::default(),
        })
    }

    #[test]
    fn file_icon_cache_key_normalizes_extension_case() -> ExplorerResult<()> {
        let upper = item(r"C:\root\readme.TXT", FileItemKind::File)?;
        let lower = item(r"C:\root\readme.txt", FileItemKind::File)?;

        assert_eq!(
            ShellIconCacheKey::from_item(&upper),
            ShellIconCacheKey::FileExtension(OsString::from("txt"))
        );
        assert_eq!(
            ShellIconCacheKey::from_item(&upper),
            ShellIconCacheKey::from_item(&lower)
        );

        Ok(())
    }

    #[test]
    fn folder_icon_cache_key_uses_folder_bucket() -> ExplorerResult<()> {
        let item = item(r"C:\root\child", FileItemKind::Folder)?;

        assert_eq!(
            ShellIconCacheKey::from_item(&item),
            ShellIconCacheKey::Folder
        );

        Ok(())
    }

    #[test]
    fn file_placeholder_keeps_extension() {
        assert_eq!(
            file_placeholder_path(Some(OsStr::new("rs"))),
            PathBuf::from("j3files-file.rs")
        );
    }

    #[test]
    fn same_extension_file_replacement_keeps_icon_cache() -> ExplorerResult<()> {
        let existing = item(r"C:\root\readme.TXT", FileItemKind::File)?;
        let mut updated = item(r"C:\root\readme.txt", FileItemKind::File)?;
        updated.size = Some(42);

        assert!(!item_replacement_requires_icon_cache_invalidation(
            &existing, &updated
        ));

        Ok(())
    }

    #[test]
    fn extension_change_requires_icon_cache_invalidation() -> ExplorerResult<()> {
        let existing = item(r"C:\root\readme.txt", FileItemKind::File)?;
        let updated = item(r"C:\root\readme.md", FileItemKind::File)?;

        assert!(item_replacement_requires_icon_cache_invalidation(
            &existing, &updated
        ));

        Ok(())
    }

    #[test]
    fn file_folder_kind_change_requires_icon_cache_invalidation() -> ExplorerResult<()> {
        let existing = item(r"C:\root\child.txt", FileItemKind::File)?;
        let updated = item(r"C:\root\child.txt", FileItemKind::Folder)?;

        assert!(item_replacement_requires_icon_cache_invalidation(
            &existing, &updated
        ));

        Ok(())
    }

    #[test]
    fn local_file_presence_change_keeps_shared_extension_cache() -> ExplorerResult<()> {
        let item = item(r"C:\root\readme.txt", FileItemKind::File)?;

        assert!(!item_presence_change_requires_icon_cache_invalidation(
            &item
        ));

        Ok(())
    }

    #[test]
    fn location_specific_presence_change_requires_icon_cache_invalidation() -> ExplorerResult<()> {
        let item = item(r"C:\", FileItemKind::Drive)?;

        assert!(item_presence_change_requires_icon_cache_invalidation(&item));

        Ok(())
    }

    #[test]
    fn stale_icon_load_task_skips_load_closure() {
        let current_generation = Arc::new(AtomicU64::new(1));
        let load_count = Arc::new(AtomicUsize::new(0));
        let load_count_for_task = Arc::clone(&load_count);
        let task = ShellIconLoadTask::new(0, current_generation, move || {
            load_count_for_task.fetch_add(1, Ordering::Relaxed);
            Box::new(|_: &mut ShellIconCache| true) as ShellIconLoadCompletion
        });

        assert!(task.is_stale());
        let _completion = task.run();

        assert_eq!(load_count.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn current_icon_load_task_runs_load_closure() {
        let current_generation = Arc::new(AtomicU64::new(1));
        let load_count = Arc::new(AtomicUsize::new(0));
        let load_count_for_task = Arc::clone(&load_count);
        let task = ShellIconLoadTask::new(1, current_generation, move || {
            load_count_for_task.fetch_add(1, Ordering::Relaxed);
            Box::new(|_: &mut ShellIconCache| true) as ShellIconLoadCompletion
        });

        assert!(!task.is_stale());
        let _completion = task.run();

        assert_eq!(load_count.load(Ordering::Relaxed), 1);
    }
}
