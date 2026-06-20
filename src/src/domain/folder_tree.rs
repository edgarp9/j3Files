use std::ffi::{OsStr, OsString};

use super::{
    BookmarkItem, ExplorerError, ExplorerResult, FileItem, KnownFolderKind, NavigationLocation,
};

pub const DEFAULT_FOLDER_TREE_KNOWN_FOLDERS: [KnownFolderKind; 4] = [
    KnownFolderKind::Home,
    KnownFolderKind::Desktop,
    KnownFolderKind::Downloads,
    KnownFolderKind::Documents,
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FolderTreeSection {
    KnownFolders,
    Drives,
    Bookmarks,
    FolderChildren,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FolderTreeItemKind {
    KnownFolder(KnownFolderKind),
    DriveRoot,
    Bookmark,
    FolderChild,
}

impl FolderTreeItemKind {
    pub fn section(self) -> FolderTreeSection {
        match self {
            Self::KnownFolder(_) => FolderTreeSection::KnownFolders,
            Self::DriveRoot => FolderTreeSection::Drives,
            Self::Bookmark => FolderTreeSection::Bookmarks,
            Self::FolderChild => FolderTreeSection::FolderChildren,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FolderTreeItem {
    location: NavigationLocation,
    display_name: OsString,
    kind: FolderTreeItemKind,
    depth: u16,
    has_children: bool,
}

impl FolderTreeItem {
    pub fn new(
        kind: FolderTreeItemKind,
        location: NavigationLocation,
        display_name: Option<OsString>,
        depth: u16,
        has_children: bool,
    ) -> ExplorerResult<Self> {
        validate_kind_location(kind, &location)?;
        Ok(Self {
            display_name: display_name
                .filter(|name| name.as_os_str() != OsStr::new(""))
                .unwrap_or_else(|| location.display_name()),
            location,
            kind,
            depth,
            has_children,
        })
    }

    pub fn known_folder(
        kind: KnownFolderKind,
        location: NavigationLocation,
        display_name: Option<OsString>,
    ) -> ExplorerResult<Self> {
        Self::new(
            FolderTreeItemKind::KnownFolder(kind),
            location,
            display_name,
            0,
            true,
        )
    }

    pub fn drive_root(
        location: NavigationLocation,
        display_name: Option<OsString>,
    ) -> ExplorerResult<Self> {
        Self::new(
            FolderTreeItemKind::DriveRoot,
            location,
            display_name,
            0,
            true,
        )
    }

    pub fn bookmark(bookmark: &BookmarkItem) -> ExplorerResult<Self> {
        Self::new(
            FolderTreeItemKind::Bookmark,
            bookmark.target.clone(),
            Some(bookmark.display_name.clone()),
            0,
            true,
        )
    }

    pub fn folder_child(item: &FileItem, depth: u16, has_children: bool) -> ExplorerResult<Self> {
        if !item.is_folder() {
            return Err(ExplorerError::invalid_input(
                "폴더 트리에는 파일 항목을 표시할 수 없습니다.",
            ));
        }

        Self::folder_child_from_parts(
            item.location.clone(),
            item.display_name.clone(),
            depth,
            has_children,
        )
    }

    pub(crate) fn folder_child_from_parts(
        location: NavigationLocation,
        display_name: OsString,
        depth: u16,
        has_children: bool,
    ) -> ExplorerResult<Self> {
        Self::new(
            FolderTreeItemKind::FolderChild,
            location,
            Some(display_name),
            depth,
            has_children,
        )
    }

    pub fn location(&self) -> &NavigationLocation {
        &self.location
    }

    pub fn navigation_target(&self) -> NavigationLocation {
        self.location.clone()
    }

    pub fn display_name(&self) -> &OsStr {
        self.display_name.as_os_str()
    }

    pub fn kind(&self) -> FolderTreeItemKind {
        self.kind
    }

    pub fn section(&self) -> FolderTreeSection {
        self.kind.section()
    }

    pub fn depth(&self) -> u16 {
        self.depth
    }

    pub fn has_children(&self) -> bool {
        self.has_children
    }
}

fn validate_kind_location(
    kind: FolderTreeItemKind,
    location: &NavigationLocation,
) -> ExplorerResult<()> {
    match (kind, location) {
        (
            FolderTreeItemKind::KnownFolder(expected),
            NavigationLocation::KnownFolder { kind: actual, .. },
        ) if expected == *actual => Ok(()),
        (FolderTreeItemKind::KnownFolder(_), _) => Err(ExplorerError::invalid_input(
            "폴더 트리 기본 폴더 항목은 Windows 기본 폴더 탐색 위치를 사용해야 합니다.",
        )),
        (FolderTreeItemKind::DriveRoot, NavigationLocation::DriveRoot(_)) => Ok(()),
        (FolderTreeItemKind::DriveRoot, _) => Err(ExplorerError::invalid_input(
            "폴더 트리 드라이브 항목은 드라이브 루트 탐색 위치를 사용해야 합니다.",
        )),
        (FolderTreeItemKind::Bookmark | FolderTreeItemKind::FolderChild, _) => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{BookmarkAccessibility, FileAttributes, FileItemKind};
    use std::path::{Path, PathBuf};
    use std::time::UNIX_EPOCH;

    fn location(path: &str) -> ExplorerResult<NavigationLocation> {
        NavigationLocation::from_path(PathBuf::from(path))
    }

    fn file_item(path: &str, kind: FileItemKind) -> ExplorerResult<FileItem> {
        let location = NavigationLocation::from_path(PathBuf::from(path))?;
        Ok(FileItem {
            display_name: location.display_name(),
            location,
            kind,
            type_name: OsString::from("test"),
            size: None,
            updated_at: None,
            attributes: FileAttributes::default(),
        })
    }

    #[test]
    fn default_known_folder_order_matches_left_tree_roots() {
        assert_eq!(
            DEFAULT_FOLDER_TREE_KNOWN_FOLDERS,
            [
                KnownFolderKind::Home,
                KnownFolderKind::Desktop,
                KnownFolderKind::Downloads,
                KnownFolderKind::Documents
            ]
        );
    }

    #[test]
    fn known_folder_item_keeps_navigation_location_as_identifier() -> ExplorerResult<()> {
        let path = PathBuf::from(r"C:\Users\Test\Desktop");
        let location = NavigationLocation::known_folder(KnownFolderKind::Desktop, path.clone())?;
        let item = FolderTreeItem::known_folder(
            KnownFolderKind::Desktop,
            location,
            Some(OsString::from("Desktop")),
        )?;

        assert_eq!(item.display_name(), OsStr::new("Desktop"));
        assert_eq!(
            item.location().as_path(),
            Path::new(r"C:\Users\Test\Desktop")
        );
        assert_eq!(item.navigation_target().as_path(), path.as_path());
        assert_eq!(
            item.kind(),
            FolderTreeItemKind::KnownFolder(KnownFolderKind::Desktop)
        );
        assert_eq!(item.section(), FolderTreeSection::KnownFolders);

        Ok(())
    }

    #[test]
    fn drive_root_item_requires_drive_root_location() -> ExplorerResult<()> {
        let drive = FolderTreeItem::drive_root(location(r"C:\")?, None)?;
        assert_eq!(drive.location().as_path(), Path::new(r"C:\"));
        assert_eq!(drive.section(), FolderTreeSection::Drives);

        let error = match FolderTreeItem::drive_root(location(r"C:\Users")?, None) {
            Err(error) => error,
            Ok(_) => {
                return Err(ExplorerError::state_conflict(
                    "non-root drive path should not create a drive tree item",
                ));
            }
        };

        assert_eq!(
            error.user_message(),
            "폴더 트리 드라이브 항목은 드라이브 루트 탐색 위치를 사용해야 합니다."
        );

        Ok(())
    }

    #[test]
    fn bookmark_item_uses_bookmark_target_not_display_name_reverse_lookup() -> ExplorerResult<()> {
        let bookmark = BookmarkItem::from_parts(
            location(r"\\server\share\자료")?,
            OsString::from("Team Share"),
            0,
            UNIX_EPOCH,
            None,
            BookmarkAccessibility::Unknown,
        );

        let item = FolderTreeItem::bookmark(&bookmark)?;

        assert_eq!(item.display_name(), OsStr::new("Team Share"));
        assert_eq!(
            item.navigation_target().as_path(),
            Path::new(r"\\server\share\자료")
        );
        assert_eq!(item.section(), FolderTreeSection::Bookmarks);

        Ok(())
    }

    #[test]
    fn folder_child_accepts_only_folder_file_items() -> ExplorerResult<()> {
        let folder = file_item(r"C:\root\child", FileItemKind::Folder)?;
        let item = FolderTreeItem::folder_child(&folder, 2, false)?;

        assert_eq!(item.location().as_path(), Path::new(r"C:\root\child"));
        assert_eq!(item.display_name(), OsStr::new("child"));
        assert_eq!(item.depth(), 2);
        assert!(!item.has_children());
        assert_eq!(item.kind(), FolderTreeItemKind::FolderChild);

        let file = file_item(r"C:\root\readme.txt", FileItemKind::File)?;
        let error = match FolderTreeItem::folder_child(&file, 2, false) {
            Err(error) => error,
            Ok(_) => {
                return Err(ExplorerError::state_conflict(
                    "file items should not create folder tree children",
                ));
            }
        };

        assert_eq!(
            error.user_message(),
            "폴더 트리에는 파일 항목을 표시할 수 없습니다."
        );

        Ok(())
    }

    #[test]
    fn folder_child_keeps_depth_and_expand_hint() -> ExplorerResult<()> {
        let folder = file_item(r"C:\root\child", FileItemKind::Folder)?;
        let item = FolderTreeItem::folder_child(&folder, 1, true)?;

        assert_eq!(item.section(), FolderTreeSection::FolderChildren);
        assert_eq!(item.depth(), 1);
        assert!(item.has_children());
        assert_eq!(item.display_name(), OsStr::new("child"));

        Ok(())
    }
}
