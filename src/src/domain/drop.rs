use std::collections::HashSet;
use std::path::{Component, Path, Prefix};

use super::{
    file_item::FileItem, navigation::NormalizedPathKey, text::case_fold_os, ExplorerError,
    ExplorerResult, NavigationLocation,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DropOperation {
    Copy,
    Move,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DropModifierKeys {
    pub control: bool,
    pub shift: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DropSourceKind {
    Internal,
    External {
        default_operation: Option<DropOperation>,
    },
}

impl DropSourceKind {
    pub fn internal_operation_resolver(
        sources: &[NavigationLocation],
    ) -> impl Fn(&NavigationLocation, DropModifierKeys) -> ExplorerResult<DropOperation> {
        let sources = PreparedInternalDropSources::new(sources);
        move |destination, modifiers| sources.operation_for_destination(destination, modifiers)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DropAllowedOperations {
    pub copy: bool,
    pub move_: bool,
}

#[derive(Debug, Clone)]
struct PreparedInternalDropSources {
    source_path_keys: Vec<NormalizedPathKey>,
    source_roots: Vec<Option<StorageRoot>>,
}

impl PreparedInternalDropSources {
    pub fn new(sources: &[NavigationLocation]) -> Self {
        let mut source_path_keys = Vec::with_capacity(sources.len());
        let mut source_roots = Vec::with_capacity(sources.len());

        for source in sources {
            source_path_keys.push(source.normalized_path_key());
            source_roots.push(storage_root(source.as_path()));
        }

        Self {
            source_path_keys,
            source_roots,
        }
    }

    pub fn operation_for_destination(
        &self,
        destination: &NavigationLocation,
        modifiers: DropModifierKeys,
    ) -> ExplorerResult<DropOperation> {
        let destination_key = destination.normalized_path_key();
        self.validate_destination_key(&destination_key)?;

        if modifiers.control {
            return Ok(DropOperation::Copy);
        }
        if modifiers.shift {
            return Ok(DropOperation::Move);
        }

        let destination_root = storage_root(destination.as_path());
        Ok(self.default_operation_for_destination_root(destination_root.as_ref()))
    }

    fn validate_destination_key(&self, destination_key: &NormalizedPathKey) -> ExplorerResult<()> {
        if self
            .source_path_keys
            .iter()
            .any(|source_key| source_key.contains(destination_key))
        {
            return Err(ExplorerError::invalid_input(
                "이동 대상이 원본과 같거나 원본의 하위 폴더입니다.",
            ));
        }

        Ok(())
    }

    fn default_operation_for_destination_root(
        &self,
        destination_root: Option<&StorageRoot>,
    ) -> DropOperation {
        if self.source_roots.is_empty() {
            return DropOperation::Copy;
        }

        if self
            .source_roots
            .iter()
            .all(|source_root| same_storage_root_value(source_root.as_ref(), destination_root))
        {
            DropOperation::Move
        } else {
            DropOperation::Copy
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DragSourceCompletion {
    Cancelled,
    NoDrop,
    Copy,
    Move,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StorageRootRelation {
    SameDrive,
    DifferentDrive,
    SameUncShare,
    DifferentUncShare,
    DifferentKind,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HoverExpandAction {
    None,
    Pending,
    Expand { target: usize },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct HoverExpandPending {
    target: usize,
    started_at_ms: u64,
    expanded: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct HoverExpandState {
    pending: Option<HoverExpandPending>,
}

impl HoverExpandState {
    pub fn update(
        &mut self,
        target: Option<usize>,
        now_ms: u64,
        threshold_ms: u64,
    ) -> HoverExpandAction {
        let Some(target) = target else {
            self.clear();
            return HoverExpandAction::None;
        };

        let mut pending = match self.pending {
            Some(pending) if pending.target == target => pending,
            _ => HoverExpandPending {
                target,
                started_at_ms: now_ms,
                expanded: false,
            },
        };

        if pending.expanded {
            self.pending = Some(pending);
            return HoverExpandAction::None;
        }

        if now_ms.saturating_sub(pending.started_at_ms) >= threshold_ms {
            pending.expanded = true;
            self.pending = Some(pending);
            HoverExpandAction::Expand { target }
        } else {
            self.pending = Some(pending);
            HoverExpandAction::Pending
        }
    }

    pub fn clear(&mut self) {
        self.pending = None;
    }

    pub fn pending_target(&self) -> Option<usize> {
        self.pending.map(|pending| pending.target)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AutoScrollDirection {
    Up,
    Down,
}

pub fn decide_vertical_auto_scroll_direction(
    pointer_y: i32,
    viewport_height: i32,
    edge_threshold: i32,
) -> Option<AutoScrollDirection> {
    if viewport_height <= 0 || edge_threshold <= 0 {
        return None;
    }

    let edge_threshold = edge_threshold.min((viewport_height / 2).max(1));
    if pointer_y < edge_threshold {
        Some(AutoScrollDirection::Up)
    } else if pointer_y >= viewport_height - edge_threshold {
        Some(AutoScrollDirection::Down)
    } else {
        None
    }
}

pub fn decide_drop_operation(
    sources: &[NavigationLocation],
    destination: &NavigationLocation,
    source_kind: DropSourceKind,
    modifiers: DropModifierKeys,
) -> DropOperation {
    if modifiers.control {
        return DropOperation::Copy;
    }
    if modifiers.shift {
        return DropOperation::Move;
    }

    match source_kind {
        DropSourceKind::Internal => default_internal_drop_operation(sources, destination),
        DropSourceKind::External { default_operation } => {
            default_operation.unwrap_or(DropOperation::Copy)
        }
    }
}

pub fn default_external_drop_operation(
    allowed: DropAllowedOperations,
    preferred: Option<DropOperation>,
) -> Option<DropOperation> {
    match preferred {
        Some(DropOperation::Move) if allowed.move_ => Some(DropOperation::Move),
        Some(DropOperation::Copy) if allowed.copy => Some(DropOperation::Copy),
        _ => match (allowed.copy, allowed.move_) {
            (false, true) => Some(DropOperation::Move),
            (true, false) => Some(DropOperation::Copy),
            _ => None,
        },
    }
}

pub fn validate_move_drop(
    sources: &[NavigationLocation],
    destination: &NavigationLocation,
) -> ExplorerResult<()> {
    let destination_key = destination.normalized_path_key();
    if sources.iter().any(|source| {
        let source_key = source.normalized_path_key();
        source_key.contains(&destination_key)
    }) {
        return Err(ExplorerError::invalid_input(
            "이동 대상이 원본과 같거나 원본의 하위 폴더입니다.",
        ));
    }

    Ok(())
}

pub fn snapshot_drag_source_locations(
    items: &[FileItem],
    selected_indices: &[usize],
    drag_index: usize,
) -> Vec<NavigationLocation> {
    let Some(dragged_item) = items.get(drag_index) else {
        return Vec::new();
    };

    if !selected_indices.contains(&drag_index) {
        return vec![dragged_item.location.clone()];
    }

    unique_location_refs(
        selected_indices
            .iter()
            .filter_map(|index| items.get(*index))
            .map(|item| &item.location),
        selected_indices.len(),
    )
}

pub fn unique_drag_sources(sources: Vec<NavigationLocation>) -> Vec<NavigationLocation> {
    unique_locations(sources)
}

pub fn drag_source_refresh_locations(
    sources: &[NavigationLocation],
    completion: DragSourceCompletion,
) -> ExplorerResult<Vec<NavigationLocation>> {
    if completion != DragSourceCompletion::Move {
        return Ok(Vec::new());
    }

    source_parent_locations(sources)
}

pub fn file_transfer_refresh_locations(
    sources: &[NavigationLocation],
    destination: &NavigationLocation,
    operation: DropOperation,
) -> ExplorerResult<Vec<NavigationLocation>> {
    match operation {
        DropOperation::Copy => Ok(unique_locations(vec![destination.clone()])),
        DropOperation::Move => {
            let mut locations = source_parent_locations(sources)?;
            locations.push(destination.clone());
            Ok(unique_locations(locations))
        }
    }
}

fn default_internal_drop_operation(
    sources: &[NavigationLocation],
    destination: &NavigationLocation,
) -> DropOperation {
    if sources.is_empty() {
        return DropOperation::Copy;
    }

    let destination_root = storage_root(destination.as_path());
    if sources.iter().all(|source| {
        let source_root = storage_root(source.as_path());
        same_storage_root_value(source_root.as_ref(), destination_root.as_ref())
    }) {
        DropOperation::Move
    } else {
        DropOperation::Copy
    }
}

pub fn same_storage_root(left: &Path, right: &Path) -> bool {
    matches!(
        compare_storage_roots(left, right),
        StorageRootRelation::SameDrive | StorageRootRelation::SameUncShare
    )
}

pub fn compare_storage_roots(left: &Path, right: &Path) -> StorageRootRelation {
    let left = storage_root(left);
    let right = storage_root(right);

    compare_storage_root_values(left.as_ref(), right.as_ref())
}

fn compare_storage_root_values(
    left: Option<&StorageRoot>,
    right: Option<&StorageRoot>,
) -> StorageRootRelation {
    match (left, right) {
        (Some(StorageRoot::Drive(left)), Some(StorageRoot::Drive(right))) if left == right => {
            StorageRootRelation::SameDrive
        }
        (Some(StorageRoot::Drive(_)), Some(StorageRoot::Drive(_))) => {
            StorageRootRelation::DifferentDrive
        }
        (
            Some(StorageRoot::Unc {
                server: left_server,
                share: left_share,
            }),
            Some(StorageRoot::Unc {
                server: right_server,
                share: right_share,
            }),
        ) if left_server == right_server && left_share == right_share => {
            StorageRootRelation::SameUncShare
        }
        (Some(StorageRoot::Unc { .. }), Some(StorageRoot::Unc { .. })) => {
            StorageRootRelation::DifferentUncShare
        }
        (Some(_), Some(_)) => StorageRootRelation::DifferentKind,
        _ => StorageRootRelation::Unknown,
    }
}

fn same_storage_root_value(left: Option<&StorageRoot>, right: Option<&StorageRoot>) -> bool {
    matches!(
        compare_storage_root_values(left, right),
        StorageRootRelation::SameDrive | StorageRootRelation::SameUncShare
    )
}

fn storage_root(path: &Path) -> Option<StorageRoot> {
    let mut components = path.components();
    let prefix = match components.next()? {
        Component::Prefix(prefix) => prefix,
        _ => return None,
    };

    match prefix.kind() {
        Prefix::Disk(letter) | Prefix::VerbatimDisk(letter) => {
            Some(StorageRoot::Drive(letter.to_ascii_lowercase()))
        }
        Prefix::UNC(server, share) | Prefix::VerbatimUNC(server, share) => Some(StorageRoot::Unc {
            server: case_fold_os(server),
            share: case_fold_os(share),
        }),
        _ => None,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum StorageRoot {
    Drive(u8),
    Unc { server: Vec<u16>, share: Vec<u16> },
}

pub fn source_parent_locations(
    selected_items: &[NavigationLocation],
) -> ExplorerResult<Vec<NavigationLocation>> {
    let mut parents = Vec::new();
    for item in selected_items {
        if let Some(parent) = item.parent()? {
            parents.push(parent);
        }
    }
    Ok(unique_locations(parents))
}

fn unique_locations(locations: Vec<NavigationLocation>) -> Vec<NavigationLocation> {
    let mut seen = HashSet::with_capacity(locations.len());
    let mut unique = Vec::with_capacity(locations.len());
    for location in locations {
        if seen.insert(location.normalized_path_key()) {
            unique.push(location);
        }
    }
    unique
}

fn unique_location_refs<'a>(
    locations: impl IntoIterator<Item = &'a NavigationLocation>,
    capacity: usize,
) -> Vec<NavigationLocation> {
    let mut seen = HashSet::with_capacity(capacity);
    let mut unique = Vec::with_capacity(capacity);
    for location in locations {
        if seen.insert(location.normalized_path_key()) {
            unique.push(location.clone());
        }
    }
    unique
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{FileAttributes, FileItemKind};
    use std::ffi::OsString;
    use std::path::PathBuf;

    fn location(path: &str) -> ExplorerResult<NavigationLocation> {
        NavigationLocation::from_path(path)
    }

    fn file_item(path: &str) -> ExplorerResult<FileItem> {
        let location = location(path)?;
        let display_name = location
            .as_path()
            .file_name()
            .map(OsString::from)
            .unwrap_or_else(|| location.as_path().as_os_str().to_os_string());
        Ok(FileItem {
            location,
            display_name,
            kind: FileItemKind::File,
            type_name: OsString::from("file"),
            size: None,
            updated_at: None,
            attributes: FileAttributes::default(),
        })
    }

    #[test]
    fn modifiers_override_drop_defaults() -> ExplorerResult<()> {
        let sources = vec![location(r"C:\from\a.txt")?];
        let destination = location(r"C:\to")?;

        assert_eq!(
            decide_drop_operation(
                &sources,
                &destination,
                DropSourceKind::Internal,
                DropModifierKeys {
                    control: true,
                    shift: true,
                },
            ),
            DropOperation::Copy
        );
        assert_eq!(
            decide_drop_operation(
                &sources,
                &destination,
                DropSourceKind::Internal,
                DropModifierKeys {
                    control: false,
                    shift: true,
                },
            ),
            DropOperation::Move
        );

        Ok(())
    }

    #[test]
    fn internal_drop_defaults_to_move_on_same_drive() -> ExplorerResult<()> {
        let sources = vec![location(r"C:\from\a.txt")?, location(r"c:\from\b.txt")?];
        let destination = location(r"C:\to")?;

        assert_eq!(
            decide_drop_operation(
                &sources,
                &destination,
                DropSourceKind::Internal,
                DropModifierKeys::default(),
            ),
            DropOperation::Move
        );

        Ok(())
    }

    #[test]
    fn internal_drop_defaults_to_move_for_unicode_paths_with_spaces() -> ExplorerResult<()> {
        let sources = vec![location(r"C:\작업 폴더\보고서 1.txt")?];
        let destination = location(r"c:\작업 폴더\대상")?;

        assert_eq!(
            decide_drop_operation(
                &sources,
                &destination,
                DropSourceKind::Internal,
                DropModifierKeys::default(),
            ),
            DropOperation::Move
        );

        Ok(())
    }

    #[test]
    fn internal_drop_defaults_to_copy_across_drives() -> ExplorerResult<()> {
        let sources = vec![location(r"C:\from\a.txt")?];
        let destination = location(r"D:\to")?;

        assert_eq!(
            decide_drop_operation(
                &sources,
                &destination,
                DropSourceKind::Internal,
                DropModifierKeys::default(),
            ),
            DropOperation::Copy
        );

        Ok(())
    }

    #[test]
    fn internal_drop_defaults_to_copy_when_any_source_crosses_root() -> ExplorerResult<()> {
        let sources = vec![location(r"C:\from\a.txt")?, location(r"D:\from\b.txt")?];
        let destination = location(r"C:\to")?;

        assert_eq!(
            decide_drop_operation(
                &sources,
                &destination,
                DropSourceKind::Internal,
                DropModifierKeys::default(),
            ),
            DropOperation::Copy
        );

        Ok(())
    }

    #[test]
    fn prepared_internal_drop_sources_match_validation_and_defaults() -> ExplorerResult<()> {
        let sources = vec![location(r"C:\from\a.txt")?, location(r"C:\from\b.txt")?];
        let operation_for_destination = DropSourceKind::internal_operation_resolver(&sources);

        assert_eq!(
            operation_for_destination(&location(r"C:\to")?, DropModifierKeys::default(),)?,
            DropOperation::Move
        );
        assert_eq!(
            operation_for_destination(&location(r"D:\to")?, DropModifierKeys::default(),)?,
            DropOperation::Copy
        );

        let invalid = operation_for_destination(
            &location(r"C:\from\a.txt\child")?,
            DropModifierKeys::default(),
        );
        assert!(matches!(
            invalid,
            Err(ExplorerError::InvalidInput { message })
                if message.contains("원본의 하위 폴더")
        ));

        Ok(())
    }

    #[test]
    fn internal_drop_defaults_to_move_on_same_unc_share() -> ExplorerResult<()> {
        let sources = vec![location(r"\\Server\Share\from\a.txt")?];
        let destination = location(r"\\server\share\to")?;

        assert_eq!(
            decide_drop_operation(
                &sources,
                &destination,
                DropSourceKind::Internal,
                DropModifierKeys::default(),
            ),
            DropOperation::Move
        );

        Ok(())
    }

    #[test]
    fn internal_drop_defaults_to_move_on_same_verbatim_unc_share() -> ExplorerResult<()> {
        let sources = vec![location(r"\\?\UNC\Server\Share\from\a.txt")?];
        let destination = location(r"\\?\UNC\server\share\to")?;

        assert_eq!(
            decide_drop_operation(
                &sources,
                &destination,
                DropSourceKind::Internal,
                DropModifierKeys::default(),
            ),
            DropOperation::Move
        );

        Ok(())
    }

    #[test]
    fn internal_drop_defaults_to_copy_across_unc_shares() -> ExplorerResult<()> {
        let sources = vec![location(r"\\Server\Share\from\a.txt")?];
        let destination = location(r"\\server\other\to")?;

        assert_eq!(
            decide_drop_operation(
                &sources,
                &destination,
                DropSourceKind::Internal,
                DropModifierKeys::default(),
            ),
            DropOperation::Copy
        );

        Ok(())
    }

    #[test]
    fn storage_root_comparison_distinguishes_drive_and_unc_roots() {
        assert_eq!(
            compare_storage_roots(Path::new(r"C:\from"), Path::new(r"c:\to")),
            StorageRootRelation::SameDrive
        );
        assert_eq!(
            compare_storage_roots(Path::new(r"C:\from"), Path::new(r"D:\to")),
            StorageRootRelation::DifferentDrive
        );
        assert_eq!(
            compare_storage_roots(
                Path::new(r"\\Server\Share\from"),
                Path::new(r"\\server\share\to")
            ),
            StorageRootRelation::SameUncShare
        );
        assert_eq!(
            compare_storage_roots(
                Path::new(r"\\Server\Share\from"),
                Path::new(r"\\server\other\to")
            ),
            StorageRootRelation::DifferentUncShare
        );
        assert_eq!(
            compare_storage_roots(Path::new(r"\\?\C:\from"), Path::new(r"\\?\c:\to")),
            StorageRootRelation::SameDrive
        );
        assert_eq!(
            compare_storage_roots(
                Path::new(r"\\?\UNC\Server\Share\from"),
                Path::new(r"\\?\UNC\server\share\to")
            ),
            StorageRootRelation::SameUncShare
        );
    }

    #[test]
    fn storage_root_comparison_marks_mixed_or_unknown_roots() {
        assert_eq!(
            compare_storage_roots(Path::new(r"C:\from"), Path::new(r"\\server\share\to")),
            StorageRootRelation::DifferentKind
        );
        assert_eq!(
            compare_storage_roots(Path::new(r"relative\from"), Path::new(r"C:\to")),
            StorageRootRelation::Unknown
        );
    }

    #[test]
    fn external_drop_uses_shell_default_without_modifiers() -> ExplorerResult<()> {
        let sources = vec![location(r"C:\from\a.txt")?];
        let destination = location(r"D:\to")?;

        assert_eq!(
            decide_drop_operation(
                &sources,
                &destination,
                DropSourceKind::External {
                    default_operation: Some(DropOperation::Move),
                },
                DropModifierKeys::default(),
            ),
            DropOperation::Move
        );

        Ok(())
    }

    #[test]
    fn external_drop_without_shell_default_falls_back_to_copy() -> ExplorerResult<()> {
        let sources = vec![location(r"C:\from\a.txt")?];
        let destination = location(r"C:\to")?;

        assert_eq!(
            decide_drop_operation(
                &sources,
                &destination,
                DropSourceKind::External {
                    default_operation: None,
                },
                DropModifierKeys::default(),
            ),
            DropOperation::Copy
        );

        Ok(())
    }

    #[test]
    fn external_drop_default_prefers_allowed_shell_preference() {
        let allowed = DropAllowedOperations {
            copy: true,
            move_: true,
        };

        assert_eq!(
            default_external_drop_operation(allowed, Some(DropOperation::Move)),
            Some(DropOperation::Move)
        );
        assert_eq!(
            default_external_drop_operation(allowed, Some(DropOperation::Copy)),
            Some(DropOperation::Copy)
        );
    }

    #[test]
    fn external_drop_default_ignores_disallowed_shell_preference() {
        assert_eq!(
            default_external_drop_operation(
                DropAllowedOperations {
                    copy: true,
                    move_: false,
                },
                Some(DropOperation::Move),
            ),
            Some(DropOperation::Copy)
        );
    }

    #[test]
    fn external_drop_default_keeps_ambiguous_copy_move_for_safe_copy_fallback() {
        assert_eq!(
            default_external_drop_operation(
                DropAllowedOperations {
                    copy: true,
                    move_: true,
                },
                None,
            ),
            None
        );
    }

    #[test]
    fn hover_expand_pending_target_updates_when_hover_target_changes() {
        let mut state = HoverExpandState::default();

        assert_eq!(state.update(Some(1), 100, 700), HoverExpandAction::Pending);
        assert_eq!(state.pending_target(), Some(1));

        assert_eq!(state.update(Some(2), 250, 700), HoverExpandAction::Pending);
        assert_eq!(state.pending_target(), Some(2));
    }

    #[test]
    fn hover_expand_waits_until_threshold_is_reached() {
        let mut state = HoverExpandState::default();

        assert_eq!(state.update(Some(7), 0, 700), HoverExpandAction::Pending);
        assert_eq!(state.update(Some(7), 699, 700), HoverExpandAction::Pending);
        assert_eq!(
            state.update(Some(7), 700, 700),
            HoverExpandAction::Expand { target: 7 }
        );
        assert_eq!(state.update(Some(7), 900, 700), HoverExpandAction::None);
    }

    #[test]
    fn hover_expand_clear_removes_pending_state() {
        let mut state = HoverExpandState::default();

        assert_eq!(state.update(Some(4), 10, 700), HoverExpandAction::Pending);
        state.clear();

        assert_eq!(state.pending_target(), None);
        assert_eq!(state.update(Some(4), 709, 700), HoverExpandAction::Pending);
    }

    #[test]
    fn hover_expand_clears_when_target_disappears() {
        let mut state = HoverExpandState::default();

        assert_eq!(state.update(Some(3), 0, 700), HoverExpandAction::Pending);
        assert_eq!(state.update(None, 500, 700), HoverExpandAction::None);

        assert_eq!(state.pending_target(), None);
    }

    #[test]
    fn vertical_auto_scroll_direction_uses_top_and_bottom_edges() {
        assert_eq!(
            decide_vertical_auto_scroll_direction(0, 100, 16),
            Some(AutoScrollDirection::Up)
        );
        assert_eq!(
            decide_vertical_auto_scroll_direction(15, 100, 16),
            Some(AutoScrollDirection::Up)
        );
        assert_eq!(decide_vertical_auto_scroll_direction(16, 100, 16), None);
        assert_eq!(decide_vertical_auto_scroll_direction(83, 100, 16), None);
        assert_eq!(
            decide_vertical_auto_scroll_direction(84, 100, 16),
            Some(AutoScrollDirection::Down)
        );
    }

    #[test]
    fn vertical_auto_scroll_direction_ignores_invalid_geometry() {
        assert_eq!(decide_vertical_auto_scroll_direction(0, 0, 16), None);
        assert_eq!(decide_vertical_auto_scroll_direction(0, 100, 0), None);
    }

    #[test]
    fn move_drop_rejects_self_and_descendant_targets() -> ExplorerResult<()> {
        let source = location(r"C:\root\folder")?;

        let self_target = validate_move_drop(std::slice::from_ref(&source), &source)
            .expect_err("moving a folder onto itself must fail");
        assert_eq!(
            self_target.user_message(),
            "이동 대상이 원본과 같거나 원본의 하위 폴더입니다."
        );

        let child = location(r"C:\root\folder\child")?;
        let child_target = validate_move_drop(&[source], &child)
            .expect_err("moving a folder into its child must fail");
        assert_eq!(
            child_target.user_message(),
            "이동 대상이 원본과 같거나 원본의 하위 폴더입니다."
        );

        Ok(())
    }

    #[test]
    fn move_drop_rejects_case_variant_descendant_targets() -> ExplorerResult<()> {
        let source = location(r"C:\Root\Folder")?;
        let child = location(r"c:\root\folder\하위 폴더")?;

        let error = validate_move_drop(&[source], &child)
            .expect_err("case-insensitive descendant move must fail");

        assert_eq!(
            error.user_message(),
            "이동 대상이 원본과 같거나 원본의 하위 폴더입니다."
        );
        Ok(())
    }

    #[test]
    fn move_drop_rejects_parent_dir_disguised_self_and_descendant_targets() -> ExplorerResult<()> {
        let source = location(r"C:\root\folder")?;
        let sources = [source];
        let disguised_self =
            NavigationLocation::LocalPath(PathBuf::from(r"C:\root\folder\child\.."));
        let self_error = validate_move_drop(&sources, &disguised_self)
            .expect_err("parent-dir disguised self target must fail");
        assert_eq!(
            self_error.user_message(),
            "이동 대상이 원본과 같거나 원본의 하위 폴더입니다."
        );

        let disguised_child =
            NavigationLocation::LocalPath(PathBuf::from(r"C:\root\folder\child\..\nested"));
        let child_error = validate_move_drop(&sources, &disguised_child)
            .expect_err("parent-dir disguised descendant target must fail");
        assert_eq!(
            child_error.user_message(),
            "이동 대상이 원본과 같거나 원본의 하위 폴더입니다."
        );

        Ok(())
    }

    #[test]
    fn drag_source_snapshot_uses_full_selection_when_dragged_item_is_selected() -> ExplorerResult<()>
    {
        let items = vec![
            file_item(r"C:\root\a.txt")?,
            file_item(r"C:\root\b.txt")?,
            file_item(r"C:\root\c.txt")?,
        ];

        let snapshot = snapshot_drag_source_locations(&items, &[0, 2], 2);

        assert_eq!(
            snapshot,
            vec![location(r"C:\root\a.txt")?, location(r"C:\root\c.txt")?]
        );
        Ok(())
    }

    #[test]
    fn drag_source_snapshot_deduplicates_selected_sources() -> ExplorerResult<()> {
        let items = vec![
            file_item(r"C:\Root\Report.txt")?,
            file_item(r"c:\root\REPORT.txt")?,
            file_item(r"C:\root\other.txt")?,
        ];

        let snapshot = snapshot_drag_source_locations(&items, &[0, 1, 2], 1);

        assert_eq!(
            snapshot,
            vec![
                location(r"C:\Root\Report.txt")?,
                location(r"C:\root\other.txt")?
            ]
        );
        Ok(())
    }

    #[test]
    fn drag_source_snapshot_uses_dragged_item_when_it_is_not_selected() -> ExplorerResult<()> {
        let items = vec![
            file_item(r"C:\root\a.txt")?,
            file_item(r"C:\root\b.txt")?,
            file_item(r"C:\root\c.txt")?,
        ];

        let snapshot = snapshot_drag_source_locations(&items, &[0, 2], 1);

        assert_eq!(snapshot, vec![location(r"C:\root\b.txt")?]);
        Ok(())
    }

    #[test]
    fn drag_sources_deduplicate_normalized_paths_preserving_first_entry() -> ExplorerResult<()> {
        let sources = vec![
            location(r"C:\Root\Report.txt")?,
            location(r"c:\root\REPORT.txt")?,
            location(r"\\Server\Share\자료.txt")?,
            location(r"\\server\share\자료.txt")?,
            location(r"D:\Other\Report.txt")?,
        ];

        let unique = unique_drag_sources(sources);

        assert_eq!(
            unique,
            vec![
                location(r"C:\Root\Report.txt")?,
                location(r"\\Server\Share\자료.txt")?,
                location(r"D:\Other\Report.txt")?,
            ]
        );
        Ok(())
    }

    #[test]
    fn drag_source_move_completion_refreshes_unique_source_parents() -> ExplorerResult<()> {
        let sources = vec![
            location(r"C:\root\a.txt")?,
            location(r"C:\root\nested\b.txt")?,
            location(r"C:\root\c.txt")?,
        ];

        let moved = drag_source_refresh_locations(&sources, DragSourceCompletion::Move)?;
        let copied = drag_source_refresh_locations(&sources, DragSourceCompletion::Copy)?;

        assert_eq!(
            moved,
            vec![location(r"C:\root")?, location(r"C:\root\nested")?]
        );
        assert!(copied.is_empty());
        Ok(())
    }

    #[test]
    fn file_transfer_refresh_locations_track_copy_destination() -> ExplorerResult<()> {
        let sources = vec![location(r"C:\from\a.txt")?, location(r"C:\from\b.txt")?];
        let destination = location(r"D:\to")?;

        assert_eq!(
            file_transfer_refresh_locations(&sources, &destination, DropOperation::Copy)?,
            vec![destination]
        );

        Ok(())
    }

    #[test]
    fn file_transfer_refresh_locations_track_move_source_parents_and_destination(
    ) -> ExplorerResult<()> {
        let sources = vec![
            location(r"C:\root\a.txt")?,
            location(r"C:\root\nested\b.txt")?,
            location(r"C:\root\c.txt")?,
        ];
        let destination = location(r"D:\to")?;

        assert_eq!(
            file_transfer_refresh_locations(&sources, &destination, DropOperation::Move)?,
            vec![
                location(r"C:\root")?,
                location(r"C:\root\nested")?,
                destination
            ]
        );

        Ok(())
    }
}
