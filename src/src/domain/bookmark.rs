use std::ffi::{OsStr, OsString};
use std::time::SystemTime;

use super::{ExplorerError, ExplorerResult, NavigationLocation};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BookmarkAccessibility {
    Unknown,
    Accessible,
    Inaccessible,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BookmarkItem {
    pub display_name: OsString,
    pub target: NavigationLocation,
    pub sort_order: u32,
    pub created_at: SystemTime,
    pub last_used_at: Option<SystemTime>,
    pub accessibility: BookmarkAccessibility,
}

impl BookmarkItem {
    pub fn new(
        target: NavigationLocation,
        display_name: Option<OsString>,
        sort_order: u32,
    ) -> Self {
        let display_name = display_name
            .filter(|name| name.as_os_str() != OsStr::new(""))
            .unwrap_or_else(|| default_display_name(&target));
        Self {
            display_name,
            target,
            sort_order,
            created_at: SystemTime::now(),
            last_used_at: None,
            accessibility: BookmarkAccessibility::Unknown,
        }
    }

    pub fn from_parts(
        target: NavigationLocation,
        display_name: OsString,
        sort_order: u32,
        created_at: SystemTime,
        last_used_at: Option<SystemTime>,
        accessibility: BookmarkAccessibility,
    ) -> Self {
        let display_name = if display_name.as_os_str() == OsStr::new("") {
            default_display_name(&target)
        } else {
            display_name
        };

        Self {
            display_name,
            target,
            sort_order,
            created_at,
            last_used_at,
            accessibility,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BookmarkList {
    items: Vec<BookmarkItem>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BookmarkAddOutcome {
    Added(usize),
    AlreadyExists(usize),
}

impl BookmarkList {
    pub fn from_items(mut items: Vec<BookmarkItem>) -> Self {
        items.sort_by_key(|item| item.sort_order);

        let mut unique_items = Vec::with_capacity(items.len());
        for item in items {
            if unique_items
                .iter()
                .any(|existing: &BookmarkItem| same_target_path(&existing.target, &item.target))
            {
                continue;
            }
            unique_items.push(item);
        }

        let mut list = Self {
            items: unique_items,
        };
        list.reassign_sort_order();
        list
    }

    pub fn items(&self) -> &[BookmarkItem] {
        &self.items
    }

    pub fn get(&self, index: usize) -> ExplorerResult<&BookmarkItem> {
        self.items
            .get(index)
            .ok_or_else(|| ExplorerError::state_conflict("북마크 항목을 찾을 수 없습니다."))
    }

    pub fn index_of_target(&self, target: &NavigationLocation) -> Option<usize> {
        self.items
            .iter()
            .position(|item| same_target_path(&item.target, target))
    }

    pub fn add(
        &mut self,
        target: NavigationLocation,
        display_name: Option<OsString>,
    ) -> BookmarkAddOutcome {
        if let Some(index) = self
            .items
            .iter()
            .position(|item| same_target_path(&item.target, &target))
        {
            return BookmarkAddOutcome::AlreadyExists(index);
        }

        let sort_order = self.items.len() as u32;
        self.items
            .push(BookmarkItem::new(target, display_name, sort_order));
        BookmarkAddOutcome::Added(self.items.len() - 1)
    }

    pub fn rename(&mut self, index: usize, display_name: OsString) -> ExplorerResult<()> {
        if display_name.as_os_str() == OsStr::new("") {
            return Err(ExplorerError::invalid_input(
                "북마크 표시 이름이 비어 있습니다.",
            ));
        }

        let item = self
            .items
            .get_mut(index)
            .ok_or_else(|| ExplorerError::state_conflict("북마크 항목을 찾을 수 없습니다."))?;
        item.display_name = display_name;
        Ok(())
    }

    pub fn remove(&mut self, index: usize) -> ExplorerResult<BookmarkItem> {
        if index >= self.items.len() {
            return Err(ExplorerError::state_conflict(
                "삭제할 북마크 항목을 찾을 수 없습니다.",
            ));
        }
        let item = self.items.remove(index);
        self.reassign_sort_order();
        Ok(item)
    }

    pub fn move_item(&mut self, from_index: usize, to_index: usize) -> ExplorerResult<()> {
        if from_index >= self.items.len() || to_index >= self.items.len() {
            return Err(ExplorerError::state_conflict(
                "이동할 북마크 항목을 찾을 수 없습니다.",
            ));
        }

        let item = self.items.remove(from_index);
        self.items.insert(to_index, item);
        self.reassign_sort_order();
        Ok(())
    }

    pub fn mark_selected(&mut self, index: usize, selected_at: SystemTime) -> ExplorerResult<()> {
        let item = self
            .items
            .get_mut(index)
            .ok_or_else(|| ExplorerError::state_conflict("북마크 항목을 찾을 수 없습니다."))?;
        item.last_used_at = Some(selected_at);
        item.accessibility = BookmarkAccessibility::Accessible;
        Ok(())
    }

    pub fn mark_accessibility(
        &mut self,
        index: usize,
        accessibility: BookmarkAccessibility,
    ) -> ExplorerResult<()> {
        let item = self
            .items
            .get_mut(index)
            .ok_or_else(|| ExplorerError::state_conflict("북마크 항목을 찾을 수 없습니다."))?;
        item.accessibility = accessibility;
        Ok(())
    }

    fn reassign_sort_order(&mut self) {
        for (index, item) in self.items.iter_mut().enumerate() {
            item.sort_order = index as u32;
        }
    }
}

fn default_display_name(target: &NavigationLocation) -> OsString {
    target.display_name()
}

fn same_target_path(left: &NavigationLocation, right: &NavigationLocation) -> bool {
    left.has_same_path(right.as_path())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::KnownFolderKind;
    use std::path::PathBuf;
    use std::time::{Duration, UNIX_EPOCH};

    fn location(path: &str) -> ExplorerResult<NavigationLocation> {
        NavigationLocation::from_path(PathBuf::from(path))
    }

    #[test]
    fn duplicate_target_is_not_added_and_existing_index_is_returned() -> ExplorerResult<()> {
        let mut bookmarks = BookmarkList::default();
        let first = bookmarks.add(location(r"C:\work")?, None);
        let duplicate = bookmarks.add(location(r"C:\work")?, Some(OsString::from("Work")));

        assert_eq!(first, BookmarkAddOutcome::Added(0));
        assert_eq!(duplicate, BookmarkAddOutcome::AlreadyExists(0));
        assert_eq!(bookmarks.items().len(), 1);
        assert_eq!(bookmarks.items()[0].display_name, OsString::from("work"));

        Ok(())
    }

    #[test]
    fn duplicate_target_path_is_rejected_across_location_variants() -> ExplorerResult<()> {
        let path = PathBuf::from(r"C:\Users\Test\Desktop");
        let mut bookmarks = BookmarkList::default();
        let first = bookmarks.add(
            NavigationLocation::known_folder(KnownFolderKind::Desktop, path.clone())?,
            Some(OsString::from("Desktop")),
        );
        let duplicate = bookmarks.add(NavigationLocation::from_path(path)?, None);

        assert_eq!(first, BookmarkAddOutcome::Added(0));
        assert_eq!(duplicate, BookmarkAddOutcome::AlreadyExists(0));
        assert_eq!(bookmarks.items().len(), 1);
        assert!(matches!(
            bookmarks.items()[0].target,
            NavigationLocation::KnownFolder {
                kind: KnownFolderKind::Desktop,
                ..
            }
        ));

        Ok(())
    }

    #[test]
    fn duplicate_target_path_uses_windows_path_normalization() -> ExplorerResult<()> {
        let mut bookmarks = BookmarkList::default();
        let first = bookmarks.add(location(r"C:\Work")?, Some(OsString::from("Work")));
        let duplicate = bookmarks.add(location(r"c:\work\")?, Some(OsString::from("Duplicate")));

        assert_eq!(first, BookmarkAddOutcome::Added(0));
        assert_eq!(duplicate, BookmarkAddOutcome::AlreadyExists(0));
        assert_eq!(bookmarks.items().len(), 1);
        assert_eq!(bookmarks.items()[0].display_name, OsString::from("Work"));

        Ok(())
    }

    #[test]
    fn index_of_target_uses_windows_path_normalization() -> ExplorerResult<()> {
        let mut bookmarks = BookmarkList::default();
        bookmarks.add(location(r"C:\Work")?, Some(OsString::from("Work")));
        bookmarks.add(location(r"D:\Media")?, Some(OsString::from("Media")));

        assert_eq!(bookmarks.index_of_target(&location(r"c:\work\")?), Some(0));
        assert_eq!(bookmarks.index_of_target(&location(r"E:\Other")?), None);

        Ok(())
    }

    #[test]
    fn restored_items_are_sorted_reindexed_and_deduplicated() -> ExplorerResult<()> {
        let old = UNIX_EPOCH + Duration::from_secs(10);
        let items = vec![
            BookmarkItem::from_parts(
                location(r"C:\third")?,
                OsString::from("Third"),
                30,
                old,
                None,
                BookmarkAccessibility::Unknown,
            ),
            BookmarkItem::from_parts(
                location(r"C:\first")?,
                OsString::from("First"),
                10,
                old,
                None,
                BookmarkAccessibility::Unknown,
            ),
            BookmarkItem::from_parts(
                location(r"C:\first")?,
                OsString::from("Duplicate"),
                20,
                old,
                None,
                BookmarkAccessibility::Unknown,
            ),
        ];

        let bookmarks = BookmarkList::from_items(items);

        assert_eq!(bookmarks.items().len(), 2);
        assert_eq!(
            bookmarks.items()[0].target.as_path(),
            PathBuf::from(r"C:\first")
        );
        assert_eq!(bookmarks.items()[0].sort_order, 0);
        assert_eq!(
            bookmarks.items()[1].target.as_path(),
            PathBuf::from(r"C:\third")
        );
        assert_eq!(bookmarks.items()[1].sort_order, 1);

        Ok(())
    }

    #[test]
    fn restored_items_are_deduplicated_by_target_path() -> ExplorerResult<()> {
        let old = UNIX_EPOCH + Duration::from_secs(10);
        let path = PathBuf::from(r"C:\Users\Test\Documents");
        let items = vec![
            BookmarkItem::from_parts(
                NavigationLocation::known_folder(KnownFolderKind::Documents, path.clone())?,
                OsString::from("Documents"),
                0,
                old,
                None,
                BookmarkAccessibility::Unknown,
            ),
            BookmarkItem::from_parts(
                NavigationLocation::from_path(path)?,
                OsString::from("Duplicate"),
                1,
                old,
                None,
                BookmarkAccessibility::Unknown,
            ),
        ];

        let bookmarks = BookmarkList::from_items(items);

        assert_eq!(bookmarks.items().len(), 1);
        assert_eq!(
            bookmarks.items()[0].display_name,
            OsString::from("Documents")
        );

        Ok(())
    }

    #[test]
    fn selecting_bookmark_updates_last_used_and_accessibility() -> ExplorerResult<()> {
        let mut bookmarks = BookmarkList::default();
        bookmarks.add(location(r"C:\work")?, None);
        let selected_at = UNIX_EPOCH + Duration::from_secs(42);

        bookmarks.mark_selected(0, selected_at)?;

        assert_eq!(bookmarks.items()[0].last_used_at, Some(selected_at));
        assert_eq!(
            bookmarks.items()[0].accessibility,
            BookmarkAccessibility::Accessible
        );

        Ok(())
    }
}
