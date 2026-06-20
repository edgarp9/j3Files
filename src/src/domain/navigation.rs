use std::ffi::OsString;
use std::os::windows::ffi::OsStrExt;
use std::path::{Component, Path, PathBuf, Prefix};

use super::{text::case_fold_os, ExplorerError, ExplorerResult};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum KnownFolderKind {
    Desktop,
    Downloads,
    Documents,
    Home,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum NavigationLocation {
    LocalPath(PathBuf),
    DriveRoot(PathBuf),
    NetworkShare(PathBuf),
    KnownFolder {
        kind: KnownFolderKind,
        path: PathBuf,
    },
}

impl NavigationLocation {
    pub fn from_path(path: impl Into<PathBuf>) -> ExplorerResult<Self> {
        let path = path.into();
        validate_path(path.as_path(), "탐색 위치 경로가 비어 있습니다.")?;

        if is_network_path(&path) {
            Ok(Self::NetworkShare(path))
        } else if is_drive_root(&path) {
            Ok(Self::DriveRoot(path))
        } else {
            Ok(Self::LocalPath(path))
        }
    }

    pub fn known_folder(kind: KnownFolderKind, path: impl Into<PathBuf>) -> ExplorerResult<Self> {
        let path = path.into();
        validate_path(path.as_path(), "Windows 기본 폴더 경로가 비어 있습니다.")?;
        Ok(Self::KnownFolder { kind, path })
    }

    pub fn as_path(&self) -> &Path {
        match self {
            Self::LocalPath(path)
            | Self::DriveRoot(path)
            | Self::NetworkShare(path)
            | Self::KnownFolder { path, .. } => path,
        }
    }

    pub fn into_path(self) -> PathBuf {
        match self {
            Self::LocalPath(path)
            | Self::DriveRoot(path)
            | Self::NetworkShare(path)
            | Self::KnownFolder { path, .. } => path,
        }
    }

    pub fn parent(&self) -> ExplorerResult<Option<Self>> {
        let current = self.as_path();
        match current.parent() {
            Some(parent)
                if !parent.as_os_str().is_empty() && parent.as_os_str() != current.as_os_str() =>
            {
                NavigationLocation::from_path(parent.to_path_buf()).map(Some)
            }
            _ => Ok(None),
        }
    }

    pub fn display_name(&self) -> OsString {
        self.as_path()
            .file_name()
            .map(OsString::from)
            .unwrap_or_else(|| self.as_path().as_os_str().to_os_string())
    }

    pub fn has_same_path(&self, other: &Path) -> bool {
        PreparedNavigationPath::from_path(other)
            .compare_location(self)
            .is_exact_match()
    }

    pub fn contains_path(&self, target: &Path) -> bool {
        PreparedNavigationPath::from_path(target)
            .compare_location(self)
            .contains_target()
    }

    pub fn path_specificity(&self) -> usize {
        self.normalized_path_key().len()
    }

    pub fn best_containing_path_index<'a>(
        target: &Path,
        candidates: impl IntoIterator<Item = (usize, &'a NavigationLocation)>,
    ) -> Option<usize> {
        let target = PreparedNavigationPath::from_path(target);
        candidates
            .into_iter()
            .filter_map(|(index, location)| {
                let comparison = target.compare_location(location);
                comparison
                    .contains_target()
                    .then_some((index, comparison.specificity()))
            })
            .max_by_key(|(_, specificity)| *specificity)
            .map(|(index, _)| index)
    }

    pub fn prepared_path(&self) -> PreparedNavigationPath {
        PreparedNavigationPath::from_path(self.as_path())
    }

    pub(super) fn normalized_path_key(&self) -> NormalizedPathKey {
        NormalizedPathKey::from_path(self.as_path())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PreparedNavigationPath {
    key: NormalizedPathKey,
}

impl PreparedNavigationPath {
    pub fn from_path(path: &Path) -> Self {
        Self {
            key: NormalizedPathKey::from_path(path),
        }
    }

    pub fn has_same_path(&self, other: &Self) -> bool {
        self.key == other.key
    }

    pub fn contains_path(&self, target: &Self) -> bool {
        self.key.contains(&target.key)
    }

    pub fn best_containing_path_index<'a>(
        target: &Self,
        candidates: impl IntoIterator<Item = (usize, &'a PreparedNavigationPath)>,
    ) -> Option<usize> {
        candidates
            .into_iter()
            .filter_map(|(index, location)| {
                location
                    .contains_path(target)
                    .then_some((index, location.key.len()))
            })
            .max_by_key(|(_, specificity)| *specificity)
            .map(|(index, _)| index)
    }

    fn compare_location(&self, candidate: &NavigationLocation) -> PreparedPathComparison {
        let candidate_key = candidate.normalized_path_key();
        PreparedPathComparison {
            exact_match: candidate_key.eq(&self.key),
            contains_target: candidate_key.contains(&self.key),
            specificity: candidate_key.len(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PreparedPathComparison {
    exact_match: bool,
    contains_target: bool,
    specificity: usize,
}

impl PreparedPathComparison {
    fn is_exact_match(self) -> bool {
        self.exact_match
    }

    fn contains_target(self) -> bool {
        self.contains_target
    }

    fn specificity(self) -> usize {
        self.specificity
    }
}

fn is_network_path(path: &Path) -> bool {
    let mut components = path.components();
    matches!(
        components.next(),
        Some(Component::Prefix(prefix))
            if matches!(prefix.kind(), Prefix::UNC(_, _) | Prefix::VerbatimUNC(_, _))
    )
}

fn validate_path(path: &Path, empty_message: &str) -> ExplorerResult<()> {
    if path.as_os_str().is_empty() {
        return Err(ExplorerError::invalid_input(empty_message));
    }

    if !path.is_absolute() {
        return Err(ExplorerError::invalid_input(
            "탐색 위치 경로는 절대 경로여야 합니다.",
        ));
    }

    if path
        .components()
        .any(|component| matches!(component, Component::ParentDir))
    {
        return Err(ExplorerError::invalid_input(
            "탐색 위치 경로에 상위 디렉터리 성분이 포함되어 있습니다.",
        ));
    }

    if path.as_os_str().encode_wide().any(|unit| unit == 0) {
        return Err(ExplorerError::invalid_input(
            "탐색 위치 경로에 NUL 문자가 포함되어 있습니다.",
        ));
    }

    Ok(())
}

fn is_drive_root(path: &Path) -> bool {
    let mut components = path.components();
    let has_drive_prefix = matches!(
        components.next(),
        Some(Component::Prefix(prefix))
            if matches!(prefix.kind(), Prefix::Disk(_) | Prefix::VerbatimDisk(_))
    );
    has_drive_prefix
        && matches!(components.next(), Some(Component::RootDir))
        && components.next().is_none()
}

fn normalized_path_components(path: &Path) -> Vec<Vec<u16>> {
    let has_root = path.has_root();
    let mut components = Vec::new();

    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if matches!(
                    components.last().map(|(_, kind)| *kind),
                    Some(NormalizedComponentKind::Normal)
                ) {
                    components.pop();
                } else if !has_root {
                    components.push((
                        case_fold_os(component.as_os_str()),
                        NormalizedComponentKind::Parent,
                    ));
                }
            }
            Component::Normal(_) => components.push((
                case_fold_os(component.as_os_str()),
                NormalizedComponentKind::Normal,
            )),
            Component::Prefix(_) | Component::RootDir => components.push((
                case_fold_os(component.as_os_str()),
                NormalizedComponentKind::Anchor,
            )),
        }
    }

    components
        .into_iter()
        .map(|(component, _)| component)
        .collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NormalizedComponentKind {
    Anchor,
    Normal,
    Parent,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(super) struct NormalizedPathKey {
    components: Vec<Vec<u16>>,
}

impl NormalizedPathKey {
    fn from_path(path: &Path) -> Self {
        Self {
            components: normalized_path_components(path),
        }
    }

    fn len(&self) -> usize {
        self.components.len()
    }

    pub(super) fn contains(&self, target: &Self) -> bool {
        self.components.len() <= target.components.len()
            && self
                .components
                .iter()
                .zip(target.components.iter())
                .all(|(candidate, target)| candidate == target)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_path_preserves_unicode_unc_paths() -> ExplorerResult<()> {
        let raw = PathBuf::from(r"\\server\share\한글");
        let location = NavigationLocation::from_path(raw.clone())?;

        assert_eq!(location.as_path(), raw.as_path());
        assert!(matches!(location, NavigationLocation::NetworkShare(_)));

        Ok(())
    }

    #[test]
    fn from_path_accepts_absolute_local_paths() -> ExplorerResult<()> {
        let raw = PathBuf::from(r"C:\safe\folder");
        let location = NavigationLocation::from_path(raw.clone())?;

        assert_eq!(location.as_path(), raw.as_path());
        assert!(matches!(location, NavigationLocation::LocalPath(_)));

        Ok(())
    }

    #[test]
    fn from_path_rejects_relative_paths() -> ExplorerResult<()> {
        for path in [
            PathBuf::from(r"relative\file.txt"),
            PathBuf::from(r"C:relative\file.txt"),
            PathBuf::from(r"\current-drive\file.txt"),
        ] {
            let result = NavigationLocation::from_path(path);

            assert!(matches!(
                result,
                Err(ExplorerError::InvalidInput { message }) if message.contains("절대 경로")
            ));
        }

        Ok(())
    }

    #[test]
    fn from_path_rejects_parent_dir_components() -> ExplorerResult<()> {
        for path in [
            PathBuf::from(r"C:\root\..\target"),
            PathBuf::from(r"\\server\share\root\..\target"),
        ] {
            let result = NavigationLocation::from_path(path);

            assert!(matches!(
                result,
                Err(ExplorerError::InvalidInput { message }) if message.contains("상위 디렉터리")
            ));
        }

        Ok(())
    }

    #[test]
    fn from_path_rejects_embedded_nul() -> ExplorerResult<()> {
        let result = NavigationLocation::from_path(PathBuf::from("C:\\safe\0tail"));

        assert!(result.is_err());
        Ok(())
    }

    #[test]
    fn known_folder_rejects_embedded_nul() -> ExplorerResult<()> {
        let result = NavigationLocation::known_folder(
            KnownFolderKind::Downloads,
            PathBuf::from("C:\\safe\0tail"),
        );

        assert!(result.is_err());
        Ok(())
    }

    #[test]
    fn drive_root_has_no_parent_navigation_target() -> ExplorerResult<()> {
        let location = NavigationLocation::from_path(PathBuf::from(r"C:\"))?;

        assert!(location.parent()?.is_none());

        Ok(())
    }

    #[test]
    fn child_path_parent_is_navigation_location() -> ExplorerResult<()> {
        let location = NavigationLocation::from_path(PathBuf::from(r"C:\root\child"))?;
        let parent = location
            .parent()?
            .ok_or_else(|| ExplorerError::state_conflict("expected parent location"))?;

        assert_eq!(parent.as_path(), Path::new(r"C:\root"));
        assert!(matches!(parent, NavigationLocation::LocalPath(_)));

        Ok(())
    }

    #[test]
    fn path_comparison_uses_windows_case_and_separator_normalization() -> ExplorerResult<()> {
        let location = NavigationLocation::from_path(PathBuf::from(r"C:\Work"))?;

        assert!(location.has_same_path(Path::new(r"c:\work\")));
        assert!(location.contains_path(Path::new(r"c:/WORK/Child")));
        assert!(!location.contains_path(Path::new(r"C:\Workspace")));

        Ok(())
    }

    #[test]
    fn path_comparison_collapses_parent_dir_components() {
        let location = NavigationLocation::LocalPath(PathBuf::from(r"C:\root\folder"));

        assert!(location.has_same_path(Path::new(r"C:\root\folder\child\..")));
        assert!(location.contains_path(Path::new(r"C:\root\folder\child\..\nested")));
        assert!(!location.contains_path(Path::new(r"C:\root\folder\..\sibling")));
    }

    #[test]
    fn unc_path_containment_uses_component_boundaries() -> ExplorerResult<()> {
        let location = NavigationLocation::from_path(PathBuf::from(r"\\Server\Share"))?;

        assert!(location.has_same_path(Path::new(r"\\server\share\")));
        assert!(location.contains_path(Path::new(r"\\SERVER\SHARE\자료")));
        assert!(!location.contains_path(Path::new(r"\\SERVER\SHARE2\자료")));

        Ok(())
    }

    #[test]
    fn best_containing_path_index_uses_most_specific_candidate() -> ExplorerResult<()> {
        let locations = [
            NavigationLocation::from_path(PathBuf::from(r"C:\"))?,
            NavigationLocation::from_path(PathBuf::from(r"C:\root"))?,
            NavigationLocation::from_path(PathBuf::from(r"C:\root\folder"))?,
            NavigationLocation::from_path(PathBuf::from(r"D:\root\folder"))?,
        ];

        let best = NavigationLocation::best_containing_path_index(
            Path::new(r"c:\ROOT\folder\leaf"),
            locations.iter().enumerate(),
        );
        let target = PreparedNavigationPath::from_path(Path::new(r"c:\ROOT\folder\leaf"));
        let prepared_locations = locations
            .iter()
            .map(NavigationLocation::prepared_path)
            .collect::<Vec<_>>();
        let prepared_best = PreparedNavigationPath::best_containing_path_index(
            &target,
            prepared_locations.iter().enumerate(),
        );

        assert_eq!(best, Some(2));
        assert_eq!(prepared_best, Some(2));
        Ok(())
    }
}
