use std::ffi::{OsStr, OsString};
use std::os::windows::ffi::OsStrExt;

use super::{ExplorerError, ExplorerResult, FileNameErrorKind};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct NewFolderName {
    value: OsString,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RenameItemName {
    value: OsString,
}

impl NewFolderName {
    pub fn new(value: impl AsRef<OsStr>) -> ExplorerResult<Self> {
        let value = value.as_ref();
        validate_windows_file_name(value)?;
        Ok(Self {
            value: value.to_os_string(),
        })
    }

    pub fn as_os_str(&self) -> &OsStr {
        self.value.as_os_str()
    }
}

impl RenameItemName {
    pub fn new(value: impl AsRef<OsStr>) -> ExplorerResult<Self> {
        let value = value.as_ref();
        validate_windows_file_name(value)?;
        Ok(Self {
            value: value.to_os_string(),
        })
    }

    pub fn as_os_str(&self) -> &OsStr {
        self.value.as_os_str()
    }
}

fn validate_windows_file_name(value: &OsStr) -> ExplorerResult<()> {
    let units = value.encode_wide().collect::<Vec<_>>();
    if units.is_empty() {
        return Err(invalid_name(value, FileNameErrorKind::Empty));
    }

    if units == [b'.' as u16] || units == [b'.' as u16, b'.' as u16] {
        return Err(invalid_name(value, FileNameErrorKind::ReservedName));
    }

    if units
        .last()
        .is_some_and(|unit| *unit == b' ' as u16 || *unit == b'.' as u16)
    {
        return Err(invalid_name(
            value,
            FileNameErrorKind::EndsWithSpaceOrPeriod,
        ));
    }

    for &unit in &units {
        if unit <= 0x1f {
            return Err(invalid_name(value, FileNameErrorKind::HasControlCharacter));
        }

        if unit == b'\\' as u16 || unit == b'/' as u16 {
            return Err(invalid_name(value, FileNameErrorKind::HasPathSeparator));
        }

        if is_invalid_windows_file_name_unit(unit) {
            return Err(invalid_name(value, FileNameErrorKind::HasInvalidCharacter));
        }
    }

    let base_name = units
        .split(|unit| *unit == b'.' as u16)
        .next()
        .unwrap_or_default();
    if is_reserved_device_name(base_name) {
        return Err(invalid_name(value, FileNameErrorKind::ReservedName));
    }

    Ok(())
}

fn invalid_name(value: &OsStr, reason: FileNameErrorKind) -> ExplorerError {
    ExplorerError::invalid_file_name(value.to_os_string(), reason)
}

fn is_invalid_windows_file_name_unit(unit: u16) -> bool {
    matches!(
        unit,
        value if value == b'<' as u16
            || value == b'>' as u16
            || value == b':' as u16
            || value == b'"' as u16
            || value == b'|' as u16
            || value == b'?' as u16
            || value == b'*' as u16
    )
}

fn is_reserved_device_name(value: &[u16]) -> bool {
    matches_ascii_ci(value, b"CON")
        || matches_ascii_ci(value, b"PRN")
        || matches_ascii_ci(value, b"AUX")
        || matches_ascii_ci(value, b"NUL")
        || is_numbered_reserved_device_name(value, b"COM")
        || is_numbered_reserved_device_name(value, b"LPT")
}

fn is_numbered_reserved_device_name(value: &[u16], prefix: &[u8]) -> bool {
    if value.len() != prefix.len() + 1 || !starts_with_ascii_ci(value, prefix) {
        return false;
    }

    matches!(value[prefix.len()], unit if unit >= b'1' as u16 && unit <= b'9' as u16)
}

fn matches_ascii_ci(value: &[u16], expected: &[u8]) -> bool {
    value.len() == expected.len() && starts_with_ascii_ci(value, expected)
}

fn starts_with_ascii_ci(value: &[u16], expected: &[u8]) -> bool {
    value
        .iter()
        .zip(expected.iter())
        .all(|(&actual, &expected)| ascii_upper_unit(actual) == Some(expected))
}

fn ascii_upper_unit(unit: u16) -> Option<u8> {
    let value = u8::try_from(unit).ok()?;
    Some(value.to_ascii_uppercase())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::windows::ffi::OsStringExt;

    #[test]
    fn accepts_unicode_folder_names() -> ExplorerResult<()> {
        let name = NewFolderName::new(OsStr::new("한글 자료"))?;

        assert_eq!(name.as_os_str(), OsStr::new("한글 자료"));
        Ok(())
    }

    #[test]
    fn validation_preserves_non_utf8_windows_name_units() -> ExplorerResult<()> {
        let raw = OsString::from_wide(&[b'a' as u16, 0xd800, b'b' as u16]);
        let name = NewFolderName::new(raw.as_os_str())?;

        assert_eq!(name.as_os_str(), raw.as_os_str());
        Ok(())
    }

    #[test]
    fn rejects_empty_folder_names() {
        let error =
            NewFolderName::new(OsStr::new("")).expect_err("empty folder names should be rejected");

        assert!(matches!(
            error,
            ExplorerError::InvalidFileName {
                reason: FileNameErrorKind::Empty,
                ..
            }
        ));
    }

    #[test]
    fn rejects_path_separators() {
        let error = NewFolderName::new(OsStr::new("parent\\child"))
            .expect_err("folder names cannot contain path separators");

        assert!(matches!(
            error,
            ExplorerError::InvalidFileName {
                reason: FileNameErrorKind::HasPathSeparator,
                ..
            }
        ));
    }

    #[test]
    fn rejects_invalid_windows_characters() {
        let error = NewFolderName::new(OsStr::new("bad:name"))
            .expect_err("folder names cannot contain invalid Windows characters");

        assert!(matches!(
            error,
            ExplorerError::InvalidFileName {
                reason: FileNameErrorKind::HasInvalidCharacter,
                ..
            }
        ));
    }

    #[test]
    fn rejects_reserved_device_names_with_extensions() {
        let error = NewFolderName::new(OsStr::new("COM1.txt"))
            .expect_err("reserved device names should be rejected even with extensions");

        assert!(matches!(
            error,
            ExplorerError::InvalidFileName {
                reason: FileNameErrorKind::ReservedName,
                ..
            }
        ));
    }

    #[test]
    fn rejects_names_ending_in_space_or_period() {
        let error = NewFolderName::new(OsStr::new("Reports."))
            .expect_err("Windows folder names cannot end in a period");

        assert!(matches!(
            error,
            ExplorerError::InvalidFileName {
                reason: FileNameErrorKind::EndsWithSpaceOrPeriod,
                ..
            }
        ));
    }

    #[test]
    fn rename_item_names_reuse_windows_file_name_validation() {
        let embedded_nul = OsString::from_wide(&[b'a' as u16, 0, b'b' as u16]);

        for (name, reason) in [
            (OsStr::new(""), FileNameErrorKind::Empty),
            (
                embedded_nul.as_os_str(),
                FileNameErrorKind::HasControlCharacter,
            ),
            (
                OsStr::new("parent/child"),
                FileNameErrorKind::HasPathSeparator,
            ),
            (
                OsStr::new("bad:name"),
                FileNameErrorKind::HasInvalidCharacter,
            ),
            (OsStr::new("NUL.txt"), FileNameErrorKind::ReservedName),
        ] {
            let error = RenameItemName::new(name)
                .expect_err("rename item names should reject invalid Windows file names");

            assert!(matches!(
                error,
                ExplorerError::InvalidFileName {
                    reason: actual,
                    ..
                } if actual == reason
            ));
        }
    }
}
