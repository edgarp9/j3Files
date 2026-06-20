use std::char::decode_utf16;
use std::cmp::Ordering;
use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;

const CASE_FOLD_INLINE_UNITS: usize = 32;

#[derive(Debug, Clone, Eq, PartialEq)]
pub(crate) struct CaseFoldedOsKey {
    inline: [u16; CASE_FOLD_INLINE_UNITS],
    len: usize,
    overflow: Vec<u16>,
}

impl CaseFoldedOsKey {
    fn new() -> Self {
        Self {
            inline: [0; CASE_FOLD_INLINE_UNITS],
            len: 0,
            overflow: Vec::new(),
        }
    }

    fn as_slice(&self) -> &[u16] {
        if self.overflow.is_empty() {
            &self.inline[..self.len]
        } else {
            &self.overflow
        }
    }

    fn push(&mut self, unit: u16) {
        if self.overflow.is_empty() && self.len < CASE_FOLD_INLINE_UNITS {
            self.inline[self.len] = unit;
        } else {
            if self.overflow.is_empty() {
                self.overflow.reserve(self.len + 1);
                self.overflow.extend_from_slice(&self.inline[..self.len]);
            }
            self.overflow.push(unit);
        }
        self.len += 1;
    }

    #[cfg(test)]
    fn is_inline(&self) -> bool {
        self.overflow.is_empty()
    }
}

impl Ord for CaseFoldedOsKey {
    fn cmp(&self, other: &Self) -> Ordering {
        self.as_slice().cmp(other.as_slice())
    }
}

impl PartialOrd for CaseFoldedOsKey {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

trait CaseFoldOutput {
    fn push_unit(&mut self, unit: u16);
}

impl CaseFoldOutput for Vec<u16> {
    fn push_unit(&mut self, unit: u16) {
        self.push(unit);
    }
}

impl CaseFoldOutput for CaseFoldedOsKey {
    fn push_unit(&mut self, unit: u16) {
        self.push(unit);
    }
}

pub(crate) fn case_fold_os(value: &OsStr) -> Vec<u16> {
    let mut folded = Vec::new();
    push_case_folded_wide(value.encode_wide(), &mut folded);
    folded
}

pub(crate) fn case_fold_os_key(value: &OsStr) -> CaseFoldedOsKey {
    let mut folded = CaseFoldedOsKey::new();
    push_case_folded_wide(value.encode_wide(), &mut folded);
    folded
}

pub(crate) fn case_fold_str(value: &str) -> Vec<u16> {
    let mut folded = Vec::new();
    push_case_folded_chars(value.chars(), &mut folded);
    folded
}

pub(crate) fn contains_case_folded_os_with_query(value: &OsStr, query: &[u16]) -> bool {
    if query.is_empty() {
        return true;
    }

    let mut folded = Vec::new();
    contains_case_folded_os_with_query_buffered(value, query, &mut folded)
}

pub(crate) fn contains_case_folded_os_with_query_buffered(
    value: &OsStr,
    query: &[u16],
    folded: &mut Vec<u16>,
) -> bool {
    if query.is_empty() {
        return true;
    }

    folded.clear();
    push_case_folded_wide(value.encode_wide(), folded);
    folded.windows(query.len()).any(|window| window == query)
}

fn push_case_folded_wide(units: impl IntoIterator<Item = u16>, folded: &mut impl CaseFoldOutput) {
    for decoded in decode_utf16(units) {
        match decoded {
            Ok(character) => push_case_folded_chars(std::iter::once(character), folded),
            Err(error) => folded.push_unit(error.unpaired_surrogate()),
        }
    }
}

fn push_case_folded_chars(
    characters: impl IntoIterator<Item = char>,
    output: &mut impl CaseFoldOutput,
) {
    for character in characters {
        for folded in character.to_lowercase() {
            let mut buffer = [0_u16; 2];
            for unit in folded.encode_utf16(&mut buffer) {
                output.push_unit(*unit);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;
    use std::os::windows::ffi::OsStringExt;

    #[test]
    fn case_fold_os_preserves_unpaired_surrogates() {
        let value = OsString::from_wide(&[b'A' as u16, 0xd800, b'B' as u16]);

        assert_eq!(
            case_fold_os(value.as_os_str()),
            vec![b'a' as u16, 0xd800, b'b' as u16]
        );
    }

    #[test]
    fn case_fold_os_key_matches_vec_ordering_without_heap_for_short_names() {
        let upper = OsString::from("Report.TXT");
        let lower = OsString::from("report.txt");
        let next = OsString::from("summary.txt");

        let upper_key = case_fold_os_key(upper.as_os_str());
        let lower_key = case_fold_os_key(lower.as_os_str());
        let next_key = case_fold_os_key(next.as_os_str());

        assert!(upper_key.is_inline());
        assert_eq!(upper_key.as_slice(), case_fold_os(upper.as_os_str()));
        assert_eq!(upper_key, lower_key);
        assert!(lower_key < next_key);
    }

    #[test]
    fn case_fold_os_key_spills_only_after_inline_capacity() {
        let inline = OsString::from("a".repeat(CASE_FOLD_INLINE_UNITS));
        let spilled = OsString::from("a".repeat(CASE_FOLD_INLINE_UNITS + 1));

        assert!(case_fold_os_key(inline.as_os_str()).is_inline());
        assert!(!case_fold_os_key(spilled.as_os_str()).is_inline());
    }

    #[test]
    fn contains_case_folded_os_matches_unicode_without_lossy_conversion() {
        let value = OsString::from("Straße.txt");
        let query = case_fold_str("straße");
        let missing = case_fold_str("missing");

        assert!(contains_case_folded_os_with_query(
            value.as_os_str(),
            &query
        ));
        assert!(!contains_case_folded_os_with_query(
            value.as_os_str(),
            &missing
        ));
    }

    #[test]
    fn buffered_contains_clears_previous_folded_name() {
        let query = case_fold_str("report");
        let mut folded = case_fold_str("previous-report");

        assert!(!contains_case_folded_os_with_query_buffered(
            OsString::from("notes.txt").as_os_str(),
            &query,
            &mut folded
        ));
        assert!(contains_case_folded_os_with_query_buffered(
            OsString::from("REPORT.txt").as_os_str(),
            &query,
            &mut folded
        ));
    }
}
