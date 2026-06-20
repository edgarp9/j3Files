use std::cmp::{Ordering, Reverse};
use std::ffi::{OsStr, OsString};
use std::fmt;
use std::time::SystemTime;

use super::text::{case_fold_os_key, CaseFoldedOsKey};
use super::NavigationLocation;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileItemKind {
    File,
    Folder,
    Drive,
    NetworkShare,
    Other,
}

impl fmt::Display for FileItemKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let label = match self {
            Self::File => "file",
            Self::Folder => "folder",
            Self::Drive => "drive",
            Self::NetworkShare => "network",
            Self::Other => "other",
        };
        formatter.write_str(label)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct FileAttributes {
    pub hidden: bool,
    pub system: bool,
    pub read_only: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileItem {
    pub location: NavigationLocation,
    pub display_name: OsString,
    pub kind: FileItemKind,
    pub type_name: OsString,
    pub size: Option<u64>,
    pub updated_at: Option<SystemTime>,
    pub attributes: FileAttributes,
}

impl FileItem {
    pub fn is_folder(&self) -> bool {
        matches!(
            self.kind,
            FileItemKind::Folder | FileItemKind::Drive | FileItemKind::NetworkShare
        )
    }

    pub fn is_hidden(&self) -> bool {
        self.attributes.hidden
    }

    pub fn is_system(&self) -> bool {
        self.attributes.system
    }

    pub fn extension(&self) -> Option<&OsStr> {
        self.location.as_path().extension()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortKey {
    Name,
    Size,
    UpdatedAt,
    Kind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortDirection {
    Ascending,
    Descending,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SortState {
    pub key: SortKey,
    pub direction: SortDirection,
}

impl Default for SortState {
    fn default() -> Self {
        Self {
            key: SortKey::Name,
            direction: SortDirection::Ascending,
        }
    }
}

impl SortState {
    /// Compares file items with the same ordering used by `sort_file_items`.
    pub fn compare_file_items(self, left: &FileItem, right: &FileItem) -> Ordering {
        compare_file_items_for_sort(left, right, self)
    }

    /// Sorts file items unless cancellation is requested.
    ///
    /// Returns `false` and leaves `items` in its original order when cancellation is
    /// observed before the sorted order is applied.
    pub fn sort_file_items_unless_cancelled(
        self,
        items: &mut [FileItem],
        mut is_cancel_requested: impl FnMut() -> bool,
    ) -> bool {
        sort_file_items_unless_cancelled(items, self, &mut is_cancel_requested)
    }

    /// Merges already-sorted existing items with new insertions using the same
    /// stable ordering as `sort_file_items`.
    pub fn merge_file_items_with_payload<T>(
        self,
        existing_items: Vec<FileItem>,
        insertions: Vec<(FileItem, T)>,
    ) -> (Vec<FileItem>, Vec<(usize, T)>) {
        merge_file_items_with_payload(existing_items, insertions, self)
    }

    /// Inserts new items into an already-sorted item list using the same stable
    /// ordering as `sort_file_items`.
    pub fn insert_file_items_with_payload<T>(
        self,
        items: &mut Vec<FileItem>,
        insertions: Vec<(FileItem, T)>,
    ) -> Vec<(usize, T)> {
        insert_file_items_with_payload(items, insertions, self)
    }
}

fn compare_file_items_for_sort(left: &FileItem, right: &FileItem, sort: SortState) -> Ordering {
    folder_sort_rank(left)
        .cmp(&folder_sort_rank(right))
        .then_with(|| {
            let ordering = match sort.key {
                SortKey::Name => normalized_name(left.display_name.as_os_str())
                    .cmp(&normalized_name(right.display_name.as_os_str())),
                SortKey::Kind => normalized_name(left.type_name.as_os_str())
                    .cmp(&normalized_name(right.type_name.as_os_str()))
                    .then_with(|| file_kind_rank(left.kind).cmp(&file_kind_rank(right.kind))),
                SortKey::Size => left.size.cmp(&right.size),
                SortKey::UpdatedAt => left.updated_at.cmp(&right.updated_at),
            };
            apply_sort_direction(ordering, sort.direction)
        })
}

fn apply_sort_direction(ordering: Ordering, direction: SortDirection) -> Ordering {
    match direction {
        SortDirection::Ascending => ordering,
        SortDirection::Descending => ordering.reverse(),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DisplayOptions {
    pub show_hidden: bool,
    pub show_system: bool,
}

impl DisplayOptions {
    pub fn allows(&self, item: &FileItem) -> bool {
        (self.show_hidden || !item.is_hidden()) && (self.show_system || !item.is_system())
    }
}

pub fn sort_file_items(items: &mut [FileItem], sort: SortState) {
    sort_file_item_entries(items, sort, |item| item);
}

pub fn sort_file_items_with_payload<T>(items: &mut [(FileItem, T)], sort: SortState) {
    sort_file_item_entries(items, sort, |(item, _)| item);
}

fn merge_file_items_with_payload<T>(
    existing_items: Vec<FileItem>,
    insertions: Vec<(FileItem, T)>,
    sort: SortState,
) -> (Vec<FileItem>, Vec<(usize, T)>) {
    match sort.key {
        SortKey::Name => match sort.direction {
            SortDirection::Ascending => {
                merge_file_items_by_cached_key(existing_items, insertions, |item| {
                    (
                        folder_sort_rank(item),
                        normalized_name(item.display_name.as_os_str()),
                    )
                })
            }
            SortDirection::Descending => {
                merge_file_items_by_cached_key(existing_items, insertions, |item| {
                    (
                        folder_sort_rank(item),
                        Reverse(normalized_name(item.display_name.as_os_str())),
                    )
                })
            }
        },
        SortKey::Kind => match sort.direction {
            SortDirection::Ascending => {
                merge_file_items_by_cached_key(existing_items, insertions, |item| {
                    (
                        folder_sort_rank(item),
                        normalized_name(item.type_name.as_os_str()),
                        file_kind_rank(item.kind),
                    )
                })
            }
            SortDirection::Descending => {
                merge_file_items_by_cached_key(existing_items, insertions, |item| {
                    (
                        folder_sort_rank(item),
                        Reverse(normalized_name(item.type_name.as_os_str())),
                        Reverse(file_kind_rank(item.kind)),
                    )
                })
            }
        },
        SortKey::Size => match sort.direction {
            SortDirection::Ascending => {
                merge_file_items_by_cached_key(existing_items, insertions, |item| {
                    (folder_sort_rank(item), item.size)
                })
            }
            SortDirection::Descending => {
                merge_file_items_by_cached_key(existing_items, insertions, |item| {
                    (folder_sort_rank(item), Reverse(item.size))
                })
            }
        },
        SortKey::UpdatedAt => match sort.direction {
            SortDirection::Ascending => {
                merge_file_items_by_cached_key(existing_items, insertions, |item| {
                    (folder_sort_rank(item), item.updated_at)
                })
            }
            SortDirection::Descending => {
                merge_file_items_by_cached_key(existing_items, insertions, |item| {
                    (folder_sort_rank(item), Reverse(item.updated_at))
                })
            }
        },
    }
}

fn merge_file_items_by_cached_key<T, K: Ord>(
    existing_items: Vec<FileItem>,
    insertions: Vec<(FileItem, T)>,
    mut sort_key: impl FnMut(&FileItem) -> K,
) -> (Vec<FileItem>, Vec<(usize, T)>) {
    if insertions.is_empty() {
        return (existing_items, Vec::new());
    }

    let mut insertion_entries = Vec::with_capacity(insertions.len());
    for (item, payload) in insertions {
        let key = sort_key(&item);
        insertion_entries.push((item, payload, key));
    }
    insertion_entries.sort_by(|left, right| left.2.cmp(&right.2));

    let mut merged_items = Vec::with_capacity(existing_items.len() + insertion_entries.len());
    let mut inserted_payloads = Vec::with_capacity(insertion_entries.len());
    let mut pending_insertions = insertion_entries.into_iter();
    let mut pending_insertion = pending_insertions.next();

    for existing_item in existing_items {
        let existing_key = sort_key(&existing_item);
        while pending_insertion
            .as_ref()
            .is_some_and(|(_, _, insertion_key)| existing_key.cmp(insertion_key).is_gt())
        {
            let Some((insertion, payload, _)) = pending_insertion.take() else {
                break;
            };
            let inserted_index = merged_items.len();
            merged_items.push(insertion);
            inserted_payloads.push((inserted_index, payload));
            pending_insertion = pending_insertions.next();
        }
        merged_items.push(existing_item);
    }

    while let Some((insertion, payload, _)) = pending_insertion {
        let inserted_index = merged_items.len();
        merged_items.push(insertion);
        inserted_payloads.push((inserted_index, payload));
        pending_insertion = pending_insertions.next();
    }

    (merged_items, inserted_payloads)
}

fn insert_file_items_with_payload<T>(
    items: &mut Vec<FileItem>,
    mut insertions: Vec<(FileItem, T)>,
    sort: SortState,
) -> Vec<(usize, T)> {
    if insertions.is_empty() {
        return Vec::new();
    }

    if insertions.len() > 1 {
        let existing_items = std::mem::take(items);
        let (merged_items, inserted_payloads) =
            merge_file_items_with_payload(existing_items, insertions, sort);
        *items = merged_items;
        return inserted_payloads;
    }

    insertions.sort_by(|(left, _), (right, _)| compare_file_items_for_sort(left, right, sort));

    let mut inserted_payloads = Vec::with_capacity(insertions.len());
    items.reserve(insertions.len());
    for (insertion, payload) in insertions {
        let insert_index = items.partition_point(|existing| {
            compare_file_items_for_sort(existing, &insertion, sort) != Ordering::Greater
        });
        items.insert(insert_index, insertion);
        inserted_payloads.push((insert_index, payload));
    }

    inserted_payloads
}

fn sort_file_items_unless_cancelled(
    items: &mut [FileItem],
    sort: SortState,
    is_cancel_requested: &mut impl FnMut() -> bool,
) -> bool {
    match sort.key {
        SortKey::Name => match sort.direction {
            SortDirection::Ascending => sort_file_items_by_key_unless_cancelled(
                items,
                |item| {
                    (
                        folder_sort_rank(item),
                        normalized_name(item.display_name.as_os_str()),
                    )
                },
                is_cancel_requested,
            ),
            SortDirection::Descending => sort_file_items_by_key_unless_cancelled(
                items,
                |item| {
                    (
                        folder_sort_rank(item),
                        Reverse(normalized_name(item.display_name.as_os_str())),
                    )
                },
                is_cancel_requested,
            ),
        },
        SortKey::Kind => match sort.direction {
            SortDirection::Ascending => sort_file_items_by_key_unless_cancelled(
                items,
                |item| {
                    (
                        folder_sort_rank(item),
                        normalized_name(item.type_name.as_os_str()),
                        file_kind_rank(item.kind),
                    )
                },
                is_cancel_requested,
            ),
            SortDirection::Descending => sort_file_items_by_key_unless_cancelled(
                items,
                |item| {
                    (
                        folder_sort_rank(item),
                        Reverse(normalized_name(item.type_name.as_os_str())),
                        Reverse(file_kind_rank(item.kind)),
                    )
                },
                is_cancel_requested,
            ),
        },
        SortKey::Size => match sort.direction {
            SortDirection::Ascending => sort_file_items_by_key_unless_cancelled(
                items,
                |item| (folder_sort_rank(item), item.size),
                is_cancel_requested,
            ),
            SortDirection::Descending => sort_file_items_by_key_unless_cancelled(
                items,
                |item| (folder_sort_rank(item), Reverse(item.size)),
                is_cancel_requested,
            ),
        },
        SortKey::UpdatedAt => match sort.direction {
            SortDirection::Ascending => sort_file_items_by_key_unless_cancelled(
                items,
                |item| (folder_sort_rank(item), item.updated_at),
                is_cancel_requested,
            ),
            SortDirection::Descending => sort_file_items_by_key_unless_cancelled(
                items,
                |item| (folder_sort_rank(item), Reverse(item.updated_at)),
                is_cancel_requested,
            ),
        },
    }
}

fn sort_file_items_by_key_unless_cancelled<K: Ord>(
    items: &mut [FileItem],
    mut sort_key: impl FnMut(&FileItem) -> K,
    is_cancel_requested: &mut impl FnMut() -> bool,
) -> bool {
    if is_cancel_requested() {
        return false;
    }
    if items.len() < 2 {
        return true;
    }

    let Some(mut entries) =
        prepare_file_item_sort_entries(items, &mut sort_key, is_cancel_requested)
    else {
        return false;
    };
    if is_cancel_requested() {
        return false;
    }

    if !sort_file_item_entries_unless_cancelled(&mut entries, is_cancel_requested) {
        return false;
    }
    if is_cancel_requested() {
        return false;
    }

    apply_sorted_file_item_entries(items, &mut entries)
}

struct FileItemSortEntry<K> {
    key: K,
    original_index: usize,
}

fn prepare_file_item_sort_entries<K>(
    items: &[FileItem],
    sort_key: &mut impl FnMut(&FileItem) -> K,
    is_cancel_requested: &mut impl FnMut() -> bool,
) -> Option<Vec<FileItemSortEntry<K>>> {
    let mut entries = Vec::with_capacity(items.len());
    for (original_index, item) in items.iter().enumerate() {
        if is_cancel_requested() {
            return None;
        }
        entries.push(FileItemSortEntry {
            key: sort_key(item),
            original_index,
        });
    }
    Some(entries)
}

fn sort_file_item_entries_unless_cancelled<K: Ord>(
    entries: &mut [FileItemSortEntry<K>],
    is_cancel_requested: &mut impl FnMut() -> bool,
) -> bool {
    if is_cancel_requested() {
        return false;
    }

    let mut cancelled = false;
    entries.sort_unstable_by(|left, right| {
        if !cancelled && is_cancel_requested() {
            cancelled = true;
        }
        left.key
            .cmp(&right.key)
            .then_with(|| left.original_index.cmp(&right.original_index))
    });
    !cancelled
}

fn apply_sorted_file_item_entries<T, K>(
    items: &mut [T],
    sorted_entries: &mut [FileItemSortEntry<K>],
) -> bool {
    if items.len() != sorted_entries.len() {
        return false;
    }

    for start in 0..items.len() {
        if sorted_entries[start].original_index == usize::MAX {
            continue;
        }

        let mut current = start;
        loop {
            let source = sorted_entries[current].original_index;
            if source == usize::MAX || source >= items.len() {
                return false;
            }
            sorted_entries[current].original_index = usize::MAX;
            if source == start {
                break;
            }
            items.swap(current, source);
            current = source;
        }
    }
    true
}

fn sort_file_item_entries<T>(
    items: &mut [T],
    sort: SortState,
    item: impl Fn(&T) -> &FileItem + Copy,
) {
    match sort.key {
        SortKey::Name => sort_file_items_by_name(items, sort.direction, item),
        SortKey::Kind => sort_file_items_by_kind(items, sort.direction, item),
        SortKey::Size => {
            sort_file_items_by_comparison(items, sort.direction, item, |left, right| {
                left.size.cmp(&right.size)
            })
        }
        SortKey::UpdatedAt => {
            sort_file_items_by_comparison(items, sort.direction, item, |left, right| {
                left.updated_at.cmp(&right.updated_at)
            })
        }
    }
}

fn sort_file_items_by_name<T>(
    items: &mut [T],
    direction: SortDirection,
    item: impl Fn(&T) -> &FileItem + Copy,
) {
    match direction {
        SortDirection::Ascending => items.sort_by_cached_key(|entry| {
            let item = item(entry);
            (
                folder_sort_rank(item),
                normalized_name(item.display_name.as_os_str()),
            )
        }),
        SortDirection::Descending => items.sort_by_cached_key(|entry| {
            let item = item(entry);
            (
                folder_sort_rank(item),
                Reverse(normalized_name(item.display_name.as_os_str())),
            )
        }),
    }
}

fn sort_file_items_by_kind<T>(
    items: &mut [T],
    direction: SortDirection,
    item: impl Fn(&T) -> &FileItem + Copy,
) {
    match direction {
        SortDirection::Ascending => items.sort_by_cached_key(|entry| {
            let item = item(entry);
            (
                folder_sort_rank(item),
                normalized_name(item.type_name.as_os_str()),
                file_kind_rank(item.kind),
            )
        }),
        SortDirection::Descending => items.sort_by_cached_key(|entry| {
            let item = item(entry);
            (
                folder_sort_rank(item),
                Reverse(normalized_name(item.type_name.as_os_str())),
                Reverse(file_kind_rank(item.kind)),
            )
        }),
    }
}

fn sort_file_items_by_comparison<T>(
    items: &mut [T],
    direction: SortDirection,
    item: impl Fn(&T) -> &FileItem + Copy,
    mut compare_key: impl FnMut(&FileItem, &FileItem) -> Ordering,
) {
    items.sort_by(|left, right| {
        let left = item(left);
        let right = item(right);
        folder_sort_rank(left)
            .cmp(&folder_sort_rank(right))
            .then_with(|| {
                let key_order = compare_key(left, right);
                match direction {
                    SortDirection::Ascending => key_order,
                    SortDirection::Descending => key_order.reverse(),
                }
            })
    });
}

fn normalized_name(value: &OsStr) -> CaseFoldedOsKey {
    case_fold_os_key(value)
}

fn folder_sort_rank(item: &FileItem) -> u8 {
    if item.is_folder() {
        0
    } else {
        1
    }
}

fn file_kind_rank(kind: FileItemKind) -> u8 {
    match kind {
        FileItemKind::Drive => 0,
        FileItemKind::NetworkShare => 1,
        FileItemKind::Folder => 2,
        FileItemKind::File => 3,
        FileItemKind::Other => 4,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use std::path::PathBuf;

    struct CancelAfterChecks {
        remaining: Cell<usize>,
    }

    impl CancelAfterChecks {
        fn new(remaining: usize) -> Self {
            Self {
                remaining: Cell::new(remaining),
            }
        }

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

    fn item(name: &str, kind: FileItemKind, type_name: &str, size: Option<u64>) -> FileItem {
        FileItem {
            location: NavigationLocation::LocalPath(PathBuf::from(name)),
            display_name: OsString::from(name),
            kind,
            type_name: OsString::from(type_name),
            size,
            updated_at: None,
            attributes: FileAttributes::default(),
        }
    }

    fn item_with_updated_at(
        name: &str,
        kind: FileItemKind,
        type_name: &str,
        size: Option<u64>,
        updated_secs: u64,
    ) -> FileItem {
        let mut item = item(name, kind, type_name, size);
        item.updated_at =
            Some(std::time::UNIX_EPOCH + std::time::Duration::from_secs(updated_secs));
        item
    }

    fn item_names(items: &[FileItem]) -> Vec<OsString> {
        items.iter().map(|item| item.display_name.clone()).collect()
    }

    #[test]
    fn display_options_filter_hidden_and_system_items() {
        let mut hidden = item("hidden.txt", FileItemKind::File, "txt file", Some(1));
        hidden.attributes.hidden = true;
        let mut system = item("system.txt", FileItemKind::File, "txt file", Some(1));
        system.attributes.system = true;
        let visible = item("visible.txt", FileItemKind::File, "txt file", Some(1));

        let default_options = DisplayOptions::default();
        assert!(!default_options.allows(&hidden));
        assert!(!default_options.allows(&system));
        assert!(default_options.allows(&visible));

        let show_all = DisplayOptions {
            show_hidden: true,
            show_system: true,
        };
        assert!(show_all.allows(&hidden));
        assert!(show_all.allows(&system));
    }

    #[test]
    fn hidden_and_system_display_options_are_independent() {
        let mut hidden = item("hidden.txt", FileItemKind::File, "txt file", Some(1));
        hidden.attributes.hidden = true;
        let mut system = item("system.txt", FileItemKind::File, "txt file", Some(1));
        system.attributes.system = true;

        let show_hidden_only = DisplayOptions {
            show_hidden: true,
            show_system: false,
        };
        assert!(show_hidden_only.allows(&hidden));
        assert!(!show_hidden_only.allows(&system));

        let show_system_only = DisplayOptions {
            show_hidden: false,
            show_system: true,
        };
        assert!(!show_system_only.allows(&hidden));
        assert!(show_system_only.allows(&system));
    }

    #[test]
    fn sort_keeps_folders_first_and_uses_type_name_for_kind() {
        let mut items = vec![
            item("b.txt", FileItemKind::File, "txt file", Some(1)),
            item("images", FileItemKind::Folder, "file folder", None),
            item("a.png", FileItemKind::File, "png file", Some(1)),
        ];

        sort_file_items(
            &mut items,
            SortState {
                key: SortKey::Kind,
                direction: SortDirection::Ascending,
            },
        );

        let names = items
            .into_iter()
            .map(|item| item.display_name)
            .collect::<Vec<_>>();
        assert_eq!(
            names,
            vec![
                OsString::from("images"),
                OsString::from("a.png"),
                OsString::from("b.txt")
            ]
        );
    }

    #[test]
    fn sort_is_stable_for_equal_selected_keys() {
        let mut items = vec![
            item("c.txt", FileItemKind::File, "txt file", Some(10)),
            item("a.txt", FileItemKind::File, "txt file", Some(10)),
            item("b.txt", FileItemKind::File, "txt file", Some(10)),
        ];

        sort_file_items(
            &mut items,
            SortState {
                key: SortKey::Size,
                direction: SortDirection::Ascending,
            },
        );

        let names = items
            .into_iter()
            .map(|item| item.display_name)
            .collect::<Vec<_>>();
        assert_eq!(
            names,
            vec![
                OsString::from("c.txt"),
                OsString::from("a.txt"),
                OsString::from("b.txt")
            ]
        );
    }

    #[test]
    fn sort_state_comparison_matches_regular_pair_sort() {
        let mut left = item("alpha.txt", FileItemKind::File, "txt file", Some(10));
        let left_updated_at = std::time::UNIX_EPOCH + std::time::Duration::from_secs(10);
        left.updated_at = Some(left_updated_at);
        let mut right = item("beta.bin", FileItemKind::File, "bin file", Some(20));
        let right_updated_at = std::time::UNIX_EPOCH + std::time::Duration::from_secs(20);
        right.updated_at = Some(right_updated_at);
        let sort_states = [
            SortState {
                key: SortKey::Name,
                direction: SortDirection::Ascending,
            },
            SortState {
                key: SortKey::Name,
                direction: SortDirection::Descending,
            },
            SortState {
                key: SortKey::Kind,
                direction: SortDirection::Ascending,
            },
            SortState {
                key: SortKey::Kind,
                direction: SortDirection::Descending,
            },
            SortState {
                key: SortKey::Size,
                direction: SortDirection::Ascending,
            },
            SortState {
                key: SortKey::Size,
                direction: SortDirection::Descending,
            },
            SortState {
                key: SortKey::UpdatedAt,
                direction: SortDirection::Ascending,
            },
            SortState {
                key: SortKey::UpdatedAt,
                direction: SortDirection::Descending,
            },
        ];

        for sort in sort_states {
            let mut ordered = vec![left.clone(), right.clone()];
            sort_file_items(&mut ordered, sort);

            assert_eq!(
                sort.compare_file_items(&left, &right) != Ordering::Greater,
                ordered.first().is_some_and(|item| item == &left),
                "{sort:?}"
            );
        }
    }

    #[test]
    fn sort_with_payload_keeps_payload_attached_to_item() {
        let mut items = vec![
            (
                item("b.txt", FileItemKind::File, "txt file", Some(2)),
                "row-b",
            ),
            (
                item("docs", FileItemKind::Folder, "file folder", None),
                "row-docs",
            ),
            (
                item("a.txt", FileItemKind::File, "txt file", Some(1)),
                "row-a",
            ),
        ];

        sort_file_items_with_payload(
            &mut items,
            SortState {
                key: SortKey::Name,
                direction: SortDirection::Ascending,
            },
        );

        let names_and_rows = items
            .into_iter()
            .map(|(item, row)| (item.display_name, row))
            .collect::<Vec<_>>();
        assert_eq!(
            names_and_rows,
            vec![
                (OsString::from("docs"), "row-docs"),
                (OsString::from("a.txt"), "row-a"),
                (OsString::from("b.txt"), "row-b"),
            ]
        );
    }

    #[test]
    fn merge_file_items_with_payload_matches_regular_sort() {
        let sort_states = [
            SortState {
                key: SortKey::Name,
                direction: SortDirection::Ascending,
            },
            SortState {
                key: SortKey::Name,
                direction: SortDirection::Descending,
            },
            SortState {
                key: SortKey::Kind,
                direction: SortDirection::Ascending,
            },
            SortState {
                key: SortKey::Kind,
                direction: SortDirection::Descending,
            },
            SortState {
                key: SortKey::Size,
                direction: SortDirection::Ascending,
            },
            SortState {
                key: SortKey::Size,
                direction: SortDirection::Descending,
            },
            SortState {
                key: SortKey::UpdatedAt,
                direction: SortDirection::Ascending,
            },
            SortState {
                key: SortKey::UpdatedAt,
                direction: SortDirection::Descending,
            },
        ];

        for sort in sort_states {
            let mut existing = vec![
                item_with_updated_at("docs", FileItemKind::Folder, "file folder", None, 30),
                item_with_updated_at("alpha.txt", FileItemKind::File, "txt file", Some(10), 10),
                item_with_updated_at("omega.bin", FileItemKind::File, "bin file", Some(30), 50),
            ];
            sort_file_items(&mut existing, sort);

            let insertions = vec![
                (
                    item_with_updated_at("beta.bin", FileItemKind::File, "bin file", Some(20), 40),
                    "beta.bin",
                ),
                (
                    item_with_updated_at("images", FileItemKind::Folder, "file folder", None, 20),
                    "images",
                ),
                (
                    item_with_updated_at(
                        "same-size.txt",
                        FileItemKind::File,
                        "txt file",
                        Some(30),
                        60,
                    ),
                    "same-size.txt",
                ),
            ];
            let mut expected = existing.clone();
            expected.extend(insertions.iter().map(|(item, _)| item.clone()));
            sort_file_items(&mut expected, sort);

            let (merged, inserted_payloads) =
                sort.merge_file_items_with_payload(existing, insertions);

            assert_eq!(item_names(&merged), item_names(&expected), "{sort:?}");
            for (index, payload) in inserted_payloads {
                assert_eq!(
                    merged[index].display_name,
                    OsString::from(payload),
                    "{sort:?}"
                );
            }
        }
    }

    #[test]
    fn insert_file_items_with_payload_matches_regular_sort() {
        let sort_states = [
            SortState {
                key: SortKey::Name,
                direction: SortDirection::Ascending,
            },
            SortState {
                key: SortKey::Name,
                direction: SortDirection::Descending,
            },
            SortState {
                key: SortKey::Kind,
                direction: SortDirection::Ascending,
            },
            SortState {
                key: SortKey::Kind,
                direction: SortDirection::Descending,
            },
            SortState {
                key: SortKey::Size,
                direction: SortDirection::Ascending,
            },
            SortState {
                key: SortKey::Size,
                direction: SortDirection::Descending,
            },
            SortState {
                key: SortKey::UpdatedAt,
                direction: SortDirection::Ascending,
            },
            SortState {
                key: SortKey::UpdatedAt,
                direction: SortDirection::Descending,
            },
        ];

        for sort in sort_states {
            let mut items = vec![
                item_with_updated_at("docs", FileItemKind::Folder, "file folder", None, 30),
                item_with_updated_at("alpha.txt", FileItemKind::File, "txt file", Some(10), 10),
                item_with_updated_at("omega.bin", FileItemKind::File, "bin file", Some(30), 50),
            ];
            sort_file_items(&mut items, sort);

            let insertions = vec![
                (
                    item_with_updated_at("beta.bin", FileItemKind::File, "bin file", Some(20), 40),
                    "beta.bin",
                ),
                (
                    item_with_updated_at("images", FileItemKind::Folder, "file folder", None, 20),
                    "images",
                ),
                (
                    item_with_updated_at(
                        "same-size.txt",
                        FileItemKind::File,
                        "txt file",
                        Some(30),
                        60,
                    ),
                    "same-size.txt",
                ),
            ];
            let mut expected = items.clone();
            expected.extend(insertions.iter().map(|(item, _)| item.clone()));
            sort_file_items(&mut expected, sort);

            let inserted_payloads = sort.insert_file_items_with_payload(&mut items, insertions);

            assert_eq!(item_names(&items), item_names(&expected), "{sort:?}");
            for (index, payload) in inserted_payloads {
                assert_eq!(
                    items[index].display_name,
                    OsString::from(payload),
                    "{sort:?}"
                );
            }
        }
    }

    #[test]
    fn descending_sort_keeps_folders_first_and_reverses_each_group() {
        let mut items = vec![
            item("a.txt", FileItemKind::File, "txt file", Some(1)),
            item("b-folder", FileItemKind::Folder, "file folder", None),
            item("b.txt", FileItemKind::File, "txt file", Some(1)),
            item("a-folder", FileItemKind::Folder, "file folder", None),
        ];

        sort_file_items(
            &mut items,
            SortState {
                key: SortKey::Name,
                direction: SortDirection::Descending,
            },
        );

        let names = items
            .into_iter()
            .map(|item| item.display_name)
            .collect::<Vec<_>>();
        assert_eq!(
            names,
            vec![
                OsString::from("b-folder"),
                OsString::from("a-folder"),
                OsString::from("b.txt"),
                OsString::from("a.txt")
            ]
        );
    }

    #[test]
    fn cancellable_sort_matches_regular_sort_when_not_cancelled() {
        let sort_states = [
            SortState {
                key: SortKey::Name,
                direction: SortDirection::Ascending,
            },
            SortState {
                key: SortKey::Name,
                direction: SortDirection::Descending,
            },
            SortState {
                key: SortKey::Kind,
                direction: SortDirection::Ascending,
            },
            SortState {
                key: SortKey::Kind,
                direction: SortDirection::Descending,
            },
            SortState {
                key: SortKey::Size,
                direction: SortDirection::Ascending,
            },
            SortState {
                key: SortKey::Size,
                direction: SortDirection::Descending,
            },
            SortState {
                key: SortKey::UpdatedAt,
                direction: SortDirection::Ascending,
            },
            SortState {
                key: SortKey::UpdatedAt,
                direction: SortDirection::Descending,
            },
        ];

        for sort in sort_states {
            let mut regular_items = vec![
                item_with_updated_at("notes.txt", FileItemKind::File, "txt file", Some(10), 40),
                item_with_updated_at("archive.zip", FileItemKind::File, "zip file", Some(30), 20),
                item_with_updated_at("docs", FileItemKind::Folder, "file folder", None, 30),
                item_with_updated_at("readme", FileItemKind::File, "File", Some(20), 10),
                item_with_updated_at("images", FileItemKind::Folder, "file folder", None, 50),
            ];
            let mut cancellable_items = regular_items.clone();

            sort_file_items(&mut regular_items, sort);
            assert!(
                sort.sort_file_items_unless_cancelled(&mut cancellable_items, || false),
                "{sort:?}"
            );

            assert_eq!(
                item_names(&cancellable_items),
                item_names(&regular_items),
                "{sort:?}"
            );
        }
    }

    #[test]
    fn cancellable_sort_leaves_items_unchanged_when_cancelled_during_sort() {
        let mut items = vec![
            item("d.txt", FileItemKind::File, "txt file", Some(1)),
            item("c.txt", FileItemKind::File, "txt file", Some(1)),
            item("b.txt", FileItemKind::File, "txt file", Some(1)),
            item("a.txt", FileItemKind::File, "txt file", Some(1)),
        ];
        let original_names = item_names(&items);
        let cancellation = CancelAfterChecks::new(items.len() + 2);
        let sort = SortState::default();

        assert!(!sort.sort_file_items_unless_cancelled(&mut items, || {
            cancellation.is_cancel_requested()
        }));

        assert_eq!(item_names(&items), original_names);
    }
}
