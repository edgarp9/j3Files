use std::ffi::OsStr;
use std::path::PathBuf;

use super::text::{
    case_fold_str, contains_case_folded_os_with_query, contains_case_folded_os_with_query_buffered,
};
use super::FileItem;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SearchRunId(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchScope {
    CurrentFolder,
    IncludeSubfolders,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SearchProgress {
    pub visited_folders: usize,
    pub scanned_items: usize,
    pub matched_items: usize,
    pub skipped_folders: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchDiagnostic {
    pub path: PathBuf,
    pub detail: String,
}

impl SearchDiagnostic {
    pub const MAX_RECORDED_DETAILS: usize = 64;

    pub fn new(path: impl Into<PathBuf>, detail: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            detail: detail.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchCriteria {
    pub query: String,
    pub scope: SearchScope,
}

impl SearchCriteria {
    pub fn matches(&self, item: &FileItem) -> bool {
        matches_search_criteria(self, item)
    }
}

impl Default for SearchCriteria {
    fn default() -> Self {
        Self {
            query: String::new(),
            scope: SearchScope::CurrentFolder,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedSearchCriteria {
    query: Vec<u16>,
}

impl PreparedSearchCriteria {
    pub fn new(criteria: &SearchCriteria) -> Self {
        Self {
            query: case_fold_str(&criteria.query),
        }
    }

    pub fn matches(&self, item: &FileItem) -> bool {
        file_name_matches(&self.query, item)
    }

    pub fn matches_display_name(&self, display_name: &OsStr, folded_name: &mut Vec<u16>) -> bool {
        contains_case_folded_os_with_query_buffered(display_name, &self.query, folded_name)
    }
}

pub fn matches_search_criteria(criteria: &SearchCriteria, item: &FileItem) -> bool {
    PreparedSearchCriteria::new(criteria).matches(item)
}

fn file_name_matches(query: &[u16], item: &FileItem) -> bool {
    contains_case_folded_os_with_query(item.display_name.as_os_str(), query)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{FileAttributes, FileItemKind, NavigationLocation};
    use std::ffi::{OsStr, OsString};
    use std::path::PathBuf;

    fn item(name: &str) -> FileItem {
        FileItem {
            location: NavigationLocation::LocalPath(PathBuf::from(name)),
            display_name: OsString::from(name),
            kind: FileItemKind::File,
            type_name: OsString::from("test file"),
            size: None,
            updated_at: None,
            attributes: FileAttributes::default(),
        }
    }

    #[test]
    fn criteria_matches_file_name_case_insensitively() {
        let criteria = SearchCriteria {
            query: "REPORT".to_string(),
            ..SearchCriteria::default()
        };

        assert!(matches_search_criteria(
            &criteria,
            &item("monthly-report.txt")
        ));
        assert!(!matches_search_criteria(&criteria, &item("notes.txt")));
    }

    #[test]
    fn prepared_criteria_matches_display_name_with_reused_buffer() {
        let criteria = SearchCriteria {
            query: "REPORT".to_string(),
            ..SearchCriteria::default()
        };
        let prepared = PreparedSearchCriteria::new(&criteria);
        let mut folded_name = Vec::new();

        assert!(prepared.matches_display_name(OsStr::new("monthly-report.txt"), &mut folded_name));
        assert!(!prepared.matches_display_name(OsStr::new("notes.txt"), &mut folded_name));
    }

    #[test]
    fn default_search_scope_is_current_folder() {
        assert_eq!(SearchCriteria::default().scope, SearchScope::CurrentFolder);
    }
}
