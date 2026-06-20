use std::ffi::{OsStr, OsString};
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};

use crate::domain::{ExplorerError, ExplorerResult, NavigationLocation};
use crate::platform;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StartupPlan {
    locations: Vec<NavigationLocation>,
    selected_item: Option<NavigationLocation>,
    explicit_path: bool,
}

impl StartupPlan {
    fn new(
        locations: Vec<NavigationLocation>,
        selected_item: Option<NavigationLocation>,
        explicit_path: bool,
    ) -> Self {
        Self {
            locations,
            selected_item,
            explicit_path,
        }
    }

    pub fn locations(&self) -> &[NavigationLocation] {
        &self.locations
    }

    pub fn selected_item(&self) -> Option<&NavigationLocation> {
        self.selected_item.as_ref()
    }

    pub fn has_explicit_path(&self) -> bool {
        self.explicit_path
    }

    pub fn into_parts(self) -> (Vec<NavigationLocation>, Option<NavigationLocation>) {
        (self.locations, self.selected_item)
    }
}

pub fn startup_plan_from_args<I>(args: I) -> ExplorerResult<StartupPlan>
where
    I: IntoIterator<Item = OsString>,
{
    let mut select_next = false;

    for argument in args {
        if argument.as_os_str().is_empty() {
            continue;
        }

        if select_next {
            return explicit_startup_plan(PathBuf::from(argument), true);
        }

        let argument_os = argument.as_os_str();
        if let Some(path) = strip_ascii_prefix_ignore_case(argument_os, "/select,")
            .or_else(|| strip_ascii_prefix_ignore_case(argument_os, "-select,"))
        {
            return explicit_startup_plan(PathBuf::from(path), true);
        }

        if ascii_os_eq_ignore_case(argument_os, "/select")
            || ascii_os_eq_ignore_case(argument_os, "-select")
        {
            select_next = true;
            continue;
        }

        if let Some(path) = strip_ascii_prefix_ignore_case(argument_os, "/e,")
            .or_else(|| strip_ascii_prefix_ignore_case(argument_os, "-e,"))
            .or_else(|| strip_ascii_prefix_ignore_case(argument_os, "/root,"))
            .or_else(|| strip_ascii_prefix_ignore_case(argument_os, "-root,"))
        {
            return explicit_startup_plan(PathBuf::from(path), false);
        }

        if is_ignored_explorer_switch(argument_os) {
            continue;
        }

        return explicit_startup_plan(PathBuf::from(argument), false);
    }

    if select_next {
        return Err(ExplorerError::invalid_input(
            "/select 옵션에는 선택할 경로가 필요합니다.",
        ));
    }

    default_startup_plan()
}

pub fn default_startup_plan() -> ExplorerResult<StartupPlan> {
    Ok(StartupPlan::new(default_start_locations()?, None, false))
}

pub fn startup_plan_from_selected_folder(path: PathBuf) -> ExplorerResult<StartupPlan> {
    let path = resolve_argument_path(path)?;
    let attributes = platform::file_attributes(&path)?;
    if !attributes.directory {
        return Err(ExplorerError::invalid_input(
            "시작 폴더는 폴더 경로여야 합니다.",
        ));
    }

    Ok(StartupPlan::new(
        vec![NavigationLocation::from_path(path)?],
        None,
        true,
    ))
}

pub fn startup_plan_from_configured_folder(
    location: NavigationLocation,
) -> ExplorerResult<StartupPlan> {
    let mut locations = vec![location];

    for fallback in default_start_locations()? {
        if locations
            .iter()
            .any(|location| location.as_path() == fallback.as_path())
        {
            continue;
        }
        locations.push(fallback);
    }

    Ok(StartupPlan::new(locations, None, true))
}

pub fn default_start_locations() -> ExplorerResult<Vec<NavigationLocation>> {
    let mut locations = Vec::new();

    if let Some(profile) = std::env::var_os("USERPROFILE") {
        if !profile.as_os_str().is_empty() {
            push_unique_location(&mut locations, PathBuf::from(profile))?;
        }
    }

    match std::env::current_dir() {
        Ok(current_dir) => push_unique_location(&mut locations, current_dir)?,
        Err(source) if locations.is_empty() => {
            return Err(ExplorerError::io("read current directory", None, source));
        }
        Err(_) => {}
    }

    if locations.is_empty() {
        return Err(ExplorerError::invalid_input(
            "시작 위치 후보를 만들 수 없습니다.",
        ));
    }

    Ok(locations)
}

fn push_unique_location(
    locations: &mut Vec<NavigationLocation>,
    path: PathBuf,
) -> ExplorerResult<()> {
    if locations
        .iter()
        .any(|location| location.as_path() == path.as_path())
    {
        return Ok(());
    }

    locations.push(NavigationLocation::from_path(path)?);
    Ok(())
}

fn explicit_startup_plan(path: PathBuf, select_item: bool) -> ExplorerResult<StartupPlan> {
    let path = resolve_argument_path(path)?;
    let attributes = platform::file_attributes(&path)?;

    if attributes.directory && !select_item {
        return Ok(StartupPlan::new(
            vec![NavigationLocation::from_path(path)?],
            None,
            true,
        ));
    }

    let Some(parent) = navigable_parent(&path) else {
        return Ok(StartupPlan::new(
            vec![NavigationLocation::from_path(path)?],
            None,
            true,
        ));
    };

    Ok(StartupPlan::new(
        vec![NavigationLocation::from_path(parent)?],
        Some(NavigationLocation::from_path(path)?),
        true,
    ))
}

fn resolve_argument_path(path: PathBuf) -> ExplorerResult<PathBuf> {
    if path.as_os_str().is_empty() {
        return Err(ExplorerError::invalid_input(
            "시작 경로 인자가 비어 있습니다.",
        ));
    }

    if path.is_absolute() {
        return Ok(path);
    }

    let current_dir = std::env::current_dir()
        .map_err(|source| ExplorerError::io("read current directory", None, source))?;
    Ok(current_dir.join(path))
}

fn navigable_parent(path: &Path) -> Option<PathBuf> {
    path.parent()
        .filter(|parent| !parent.as_os_str().is_empty() && parent.as_os_str() != path.as_os_str())
        .map(Path::to_path_buf)
}

fn is_ignored_explorer_switch(argument: &OsStr) -> bool {
    ["/n", "-n", "/e", "-e", "/root", "-root"]
        .iter()
        .any(|switch| ascii_os_eq_ignore_case(argument, switch))
}

fn strip_ascii_prefix_ignore_case(value: &OsStr, prefix: &str) -> Option<OsString> {
    let value = value.encode_wide().collect::<Vec<_>>();
    let prefix = prefix.encode_utf16().collect::<Vec<_>>();
    if value.len() < prefix.len()
        || !value
            .iter()
            .zip(prefix.iter())
            .all(|(left, right)| ascii_unit_eq_ignore_case(*left, *right))
    {
        return None;
    }

    Some(OsString::from_wide(&value[prefix.len()..]))
}

fn ascii_os_eq_ignore_case(value: &OsStr, expected: &str) -> bool {
    let value = value.encode_wide().collect::<Vec<_>>();
    let expected = expected.encode_utf16().collect::<Vec<_>>();
    value.len() == expected.len()
        && value
            .iter()
            .zip(expected.iter())
            .all(|(left, right)| ascii_unit_eq_ignore_case(*left, *right))
}

fn ascii_unit_eq_ignore_case(left: u16, right: u16) -> bool {
    ascii_unit_to_lower(left) == ascii_unit_to_lower(right)
}

fn ascii_unit_to_lower(value: u16) -> u16 {
    if (b'A' as u16..=b'Z' as u16).contains(&value) {
        value + u16::from(b'a' - b'A')
    } else {
        value
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::error::Error;
    use std::fs;
    use std::io;
    use std::time::{SystemTime, UNIX_EPOCH};

    type TestResult = Result<(), Box<dyn Error>>;

    struct TempDir {
        path: PathBuf,
    }

    impl TempDir {
        fn new() -> io::Result<Self> {
            let timestamp = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_err(io::Error::other)?
                .as_nanos();
            let path = std::env::temp_dir().join(format!(
                "j3files-startup-{}-{timestamp}",
                std::process::id()
            ));
            fs::create_dir(&path)?;
            Ok(Self { path })
        }

        fn path(&self) -> &Path {
            &self.path
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    #[test]
    fn explicit_directory_argument_becomes_start_location() -> TestResult {
        let temp_dir = TempDir::new()?;
        let start_path = temp_dir.path().join("Work Folder");
        fs::create_dir(&start_path)?;

        let plan = startup_plan_from_args([start_path.as_os_str().to_os_string()])?;

        assert!(plan.has_explicit_path());
        assert_eq!(plan.locations().len(), 1);
        assert_eq!(plan.locations()[0].as_path(), start_path.as_path());
        assert!(plan.selected_item().is_none());

        Ok(())
    }

    #[test]
    fn file_argument_opens_parent_and_selects_file() -> TestResult {
        let temp_dir = TempDir::new()?;
        let file_path = temp_dir.path().join("한글 report.txt");
        fs::write(&file_path, b"hello")?;

        let plan = startup_plan_from_args([file_path.as_os_str().to_os_string()])?;

        assert!(plan.has_explicit_path());
        assert_eq!(plan.locations()[0].as_path(), temp_dir.path());
        assert_eq!(
            plan.selected_item().map(NavigationLocation::as_path),
            Some(file_path.as_path())
        );

        Ok(())
    }

    #[test]
    fn select_option_opens_parent_and_selects_directory() -> TestResult {
        let temp_dir = TempDir::new()?;
        let selected_path = temp_dir.path().join("Selected Folder");
        fs::create_dir(&selected_path)?;
        let mut argument = OsString::from("/SeLeCt,");
        argument.push(selected_path.as_os_str());

        let plan = startup_plan_from_args([argument])?;

        assert!(plan.has_explicit_path());
        assert_eq!(plan.locations()[0].as_path(), temp_dir.path());
        assert_eq!(
            plan.selected_item().map(NavigationLocation::as_path),
            Some(selected_path.as_path())
        );

        Ok(())
    }

    #[test]
    fn explorer_switch_before_path_is_ignored() -> TestResult {
        let temp_dir = TempDir::new()?;

        let plan = startup_plan_from_args([
            OsString::from("/e"),
            temp_dir.path().as_os_str().to_os_string(),
        ])?;

        assert!(plan.has_explicit_path());
        assert_eq!(plan.locations()[0].as_path(), temp_dir.path());

        Ok(())
    }

    #[test]
    fn selected_startup_folder_becomes_explicit_start_location() -> TestResult {
        let temp_dir = TempDir::new()?;
        let selected_path = temp_dir.path().join("Selected Start");
        fs::create_dir(&selected_path)?;

        let plan = startup_plan_from_selected_folder(selected_path.clone())?;

        assert!(plan.has_explicit_path());
        assert_eq!(plan.locations().len(), 1);
        assert_eq!(plan.locations()[0].as_path(), selected_path.as_path());
        assert!(plan.selected_item().is_none());

        Ok(())
    }

    #[test]
    fn selected_startup_folder_rejects_file_path() -> TestResult {
        let temp_dir = TempDir::new()?;
        let file_path = temp_dir.path().join("not-folder.txt");
        fs::write(&file_path, b"hello")?;

        let error = startup_plan_from_selected_folder(file_path);

        assert!(matches!(error, Err(ExplorerError::InvalidInput { .. })));
        Ok(())
    }

    #[test]
    fn configured_startup_folder_is_first_start_location() -> TestResult {
        let configured = NavigationLocation::from_path(PathBuf::from(r"C:\configured"))?;

        let plan = startup_plan_from_configured_folder(configured.clone())?;

        assert!(plan.has_explicit_path());
        assert_eq!(plan.locations().first(), Some(&configured));
        assert!(plan.selected_item().is_none());
        Ok(())
    }

    #[test]
    fn select_option_without_path_is_invalid() {
        let error = startup_plan_from_args([OsString::from("/select")]);

        assert!(matches!(error, Err(ExplorerError::InvalidInput { .. })));
    }
}
