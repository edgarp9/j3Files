use std::ffi::{OsStr, OsString};
use std::fs;
use std::io;
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::ptr::{null, null_mut};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use j3files::app::{
    ExplorerApp, ItemListingGateway, NeverCancelSearch, NoopSearchProgressReporter,
    SearchCancellation, SearchFileSystemGateway, ShellOpenGateway,
};
use j3files::domain::{
    DisplayOptions, ExplorerResult, FileItem, NavigationLocation, SearchCriteria, SearchScope,
    SortDirection, SortKey,
};
use j3files::infra::NativeFileSystemGateway;
use windows_sys::Win32::Foundation::{
    CloseHandle, GetLastError, FILETIME, HANDLE, INVALID_HANDLE_VALUE,
};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, SetFileAttributesW, SetFileTime, FILE_ATTRIBUTE_HIDDEN, FILE_ATTRIBUTE_NORMAL,
    FILE_ATTRIBUTE_SYSTEM, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE,
    FILE_WRITE_ATTRIBUTES, OPEN_EXISTING,
};

#[test]
fn real_windows_listing_filters_sorting_and_navigation() -> Result<(), Box<dyn std::error::Error>> {
    let mut temp_dir = TempDirectory::new()?;
    let root_location = NavigationLocation::from_path(temp_dir.path().to_path_buf())?;
    let folder_path = temp_dir.path().join("폴더-유니코드-🙂");
    let small_path = temp_dir.path().join("a-small.txt");
    let large_path = temp_dir.path().join("z-large.bin");
    let hidden_path = temp_dir.path().join("숨김.txt");
    let system_path = temp_dir.path().join("시스템.dat");

    fs::create_dir(&folder_path)?;
    fs::write(&small_path, b"a")?;
    fs::write(&large_path, vec![0_u8; 4096])?;
    fs::write(&hidden_path, b"hidden")?;
    fs::write(&system_path, b"system")?;

    set_updated_time(&small_path, UNIX_EPOCH + Duration::from_secs(946_684_800))?;
    set_updated_time(&large_path, UNIX_EPOCH + Duration::from_secs(1_577_836_800))?;
    set_file_attributes(&hidden_path, FILE_ATTRIBUTE_HIDDEN)?;
    temp_dir.track_attribute_path(hidden_path.clone());
    let system_attribute_set = match set_file_attributes(&system_path, FILE_ATTRIBUTE_SYSTEM) {
        Ok(()) => {
            temp_dir.track_attribute_path(system_path.clone());
            true
        }
        Err(error) => {
            eprintln!(
                "system attribute setup skipped for {:?}: {error}",
                system_path
            );
            false
        }
    };

    let mut app = ExplorerApp::new(
        root_location.clone(),
        NativeFileSystemGateway::new(),
        NoopShellGateway,
    );

    let visible_items = app.list_active_items()?;
    let visible_names = display_names(&visible_items);
    assert_eq!(visible_names[0], OsString::from("폴더-유니코드-🙂"));
    assert!(visible_names.contains(&OsString::from("a-small.txt")));
    assert!(visible_names.contains(&OsString::from("z-large.bin")));
    assert!(!visible_names.contains(&OsString::from("숨김.txt")));
    if system_attribute_set {
        assert!(!visible_names.contains(&OsString::from("시스템.dat")));
    }

    let folder_item = item_named(&visible_items, OsStr::new("폴더-유니코드-🙂"))?;
    app.activate_item_in_active(&folder_item)?;
    assert_eq!(
        app.active_tab()?.current_location().as_path(),
        folder_path.as_path()
    );

    app.go_back()?;
    assert_eq!(
        app.active_tab()?.current_location().as_path(),
        temp_dir.path()
    );
    app.go_forward()?;
    assert_eq!(
        app.active_tab()?.current_location().as_path(),
        folder_path.as_path()
    );
    app.go_up()?;
    assert_eq!(
        app.active_tab()?.current_location().as_path(),
        temp_dir.path()
    );

    app.navigate_active_path(folder_path.clone())?;
    assert_eq!(
        app.active_tab()?.current_location().as_path(),
        folder_path.as_path()
    );
    app.go_back()?;

    let missing_path = temp_dir.path().join("missing-folder");
    let before_failed_navigation = app.active_tab()?.current_location().clone();
    let missing_error = app
        .navigate_active_path(missing_path)
        .expect_err("missing folder navigation must fail");
    assert_eq!(missing_error.user_message(), "위치를 찾을 수 없습니다.");
    assert_eq!(
        app.active_tab()?.current_location(),
        &before_failed_navigation
    );

    app.set_show_hidden(true);
    let hidden_names = display_names(&app.list_active_items()?);
    assert!(hidden_names.contains(&OsString::from("숨김.txt")));
    if system_attribute_set {
        assert!(!hidden_names.contains(&OsString::from("시스템.dat")));
    }

    app.set_show_system(true);
    let all_names = display_names(&app.list_active_items()?);
    if system_attribute_set {
        assert!(all_names.contains(&OsString::from("시스템.dat")));
    }

    app.set_show_hidden(false);
    app.set_show_system(false);
    app.set_active_sort_key(SortKey::Size)?;
    app.set_active_sort_direction(SortDirection::Descending)?;
    let size_sorted_names = display_names(&app.list_active_items()?);
    assert_order(&size_sorted_names, "z-large.bin", "a-small.txt");

    app.set_active_sort_key(SortKey::UpdatedAt)?;
    app.set_active_sort_direction(SortDirection::Ascending)?;
    let updated_sorted_names = display_names(&app.list_active_items()?);
    assert_order(&updated_sorted_names, "a-small.txt", "z-large.bin");

    Ok(())
}

#[test]
fn file_path_is_not_accepted_as_navigation_location() -> Result<(), Box<dyn std::error::Error>> {
    let temp_dir = TempDirectory::new()?;
    let root_location = NavigationLocation::from_path(temp_dir.path().to_path_buf())?;
    let file_path = temp_dir.path().join("not-a-folder.txt");
    fs::write(&file_path, b"file")?;

    let mut app = ExplorerApp::new(
        root_location,
        NativeFileSystemGateway::new(),
        NoopShellGateway,
    );
    let error = app
        .navigate_active_path(file_path)
        .expect_err("file paths cannot be opened as folder navigation targets");

    assert_eq!(
        error.user_message(),
        "위치를 열 수 없습니다. 경로가 올바른지 확인해 주세요."
    );

    Ok(())
}

#[test]
fn navigation_to_unlistable_folder_preserves_current_location(
) -> Result<(), Box<dyn std::error::Error>> {
    let temp_dir = TempDirectory::new()?;
    let root_location = NavigationLocation::from_path(temp_dir.path().to_path_buf())?;
    let locked_dir = temp_dir.path().join("locked");
    fs::create_dir(&locked_dir)?;
    let _locked_dir = ExclusiveDirectoryLock::new(&locked_dir)?;

    let mut app = ExplorerApp::new(
        root_location.clone(),
        NativeFileSystemGateway::new(),
        NoopShellGateway,
    );
    let error = app
        .navigate_active_path(locked_dir)
        .expect_err("unlistable folders must not become the current navigation location");

    assert_eq!(
        app.active_tab()?.current_location().as_path(),
        root_location.as_path()
    );
    assert_eq!(
        error.user_message(),
        "파일 또는 폴더가 사용 중이라 작업을 완료할 수 없습니다."
    );
    assert!(error.to_string().contains("CreateFileW"));

    Ok(())
}

#[test]
fn real_windows_search_matches_name_and_scope_on_safe_temp_tree(
) -> Result<(), Box<dyn std::error::Error>> {
    let temp_dir = TempDirectory::new()?;
    let root_location = NavigationLocation::from_path(temp_dir.path().to_path_buf())?;
    let nested_dir = temp_dir.path().join("nested");
    fs::create_dir(&nested_dir)?;

    fs::write(temp_dir.path().join("report-final.TXT"), b"root txt")?;
    fs::write(temp_dir.path().join("report.md"), b"root md")?;
    fs::write(temp_dir.path().join("notes.txt"), b"notes")?;
    fs::write(nested_dir.join("nested-report.txt"), b"nested")?;

    let gateway = NativeFileSystemGateway::new();
    let mut criteria = SearchCriteria {
        query: "REPORT".to_string(),
        scope: SearchScope::CurrentFolder,
    };

    let current_folder = gateway.search_items(
        &root_location,
        &criteria,
        DisplayOptions::default(),
        Default::default(),
        &NeverCancelSearch,
        &NoopSearchProgressReporter,
    )?;

    let current_names = display_names(&current_folder.items);
    assert_eq!(current_names.len(), 2);
    assert!(current_names.contains(&OsString::from("report-final.TXT")));
    assert!(current_names.contains(&OsString::from("report.md")));
    assert!(!current_names.contains(&OsString::from("notes.txt")));
    assert_eq!(current_folder.progress.matched_items, 2);
    assert!(!current_folder.cancelled);

    criteria.scope = SearchScope::IncludeSubfolders;
    let recursive = gateway.search_items(
        &root_location,
        &criteria,
        DisplayOptions::default(),
        Default::default(),
        &NeverCancelSearch,
        &NoopSearchProgressReporter,
    )?;

    let recursive_paths = recursive
        .items
        .iter()
        .map(|item| item.location.as_path().to_path_buf())
        .collect::<Vec<_>>();
    assert_eq!(recursive.items.len(), 3);
    assert!(recursive_paths.contains(&temp_dir.path().join("report-final.TXT")));
    assert!(recursive_paths.contains(&temp_dir.path().join("report.md")));
    assert!(recursive_paths.contains(&nested_dir.join("nested-report.txt")));
    assert_eq!(recursive.progress.matched_items, 3);
    assert!(recursive.progress.visited_folders >= 2);
    assert!(!recursive.cancelled);

    Ok(())
}

#[test]
fn real_windows_search_skips_locked_subfolder_on_safe_temp_tree(
) -> Result<(), Box<dyn std::error::Error>> {
    let temp_dir = TempDirectory::new()?;
    let root_location = NavigationLocation::from_path(temp_dir.path().to_path_buf())?;
    let locked_dir = temp_dir.path().join("locked");
    fs::create_dir(&locked_dir)?;
    fs::write(temp_dir.path().join("visible-report.txt"), b"visible")?;
    fs::write(locked_dir.join("blocked-report.txt"), b"blocked")?;
    let _locked_dir = ExclusiveDirectoryLock::new(&locked_dir)?;

    let gateway = NativeFileSystemGateway::new();
    let criteria = SearchCriteria {
        query: "report".to_string(),
        scope: SearchScope::IncludeSubfolders,
    };

    let outcome = gateway.search_items(
        &root_location,
        &criteria,
        DisplayOptions::default(),
        Default::default(),
        &NeverCancelSearch,
        &NoopSearchProgressReporter,
    )?;

    let paths = outcome
        .items
        .iter()
        .map(|item| item.location.as_path().to_path_buf())
        .collect::<Vec<_>>();
    assert!(paths.contains(&temp_dir.path().join("visible-report.txt")));
    assert!(!paths.contains(&locked_dir.join("blocked-report.txt")));
    assert_eq!(outcome.progress.skipped_folders, 1);
    assert_eq!(outcome.diagnostics.len(), 1);
    assert_eq!(outcome.diagnostics[0].path, locked_dir);
    assert!(!outcome.cancelled);

    Ok(())
}

#[test]
fn real_windows_search_cancel_stops_before_scanning_full_safe_tree(
) -> Result<(), Box<dyn std::error::Error>> {
    const FILE_COUNT: usize = 200;

    let temp_dir = TempDirectory::new()?;
    let root_location = NavigationLocation::from_path(temp_dir.path().to_path_buf())?;
    for index in 0..FILE_COUNT {
        fs::write(
            temp_dir.path().join(format!("report-{index:03}.txt")),
            b"match",
        )?;
    }

    let gateway = NativeFileSystemGateway::new();
    let criteria = SearchCriteria {
        query: "report".to_string(),
        scope: SearchScope::CurrentFolder,
    };
    let cancellation = CancelAfterChecks::new(12);

    let outcome = gateway.search_items(
        &root_location,
        &criteria,
        DisplayOptions::default(),
        Default::default(),
        &cancellation,
        &NoopSearchProgressReporter,
    )?;

    assert!(outcome.cancelled);
    assert!(outcome.progress.scanned_items < FILE_COUNT);
    assert!(outcome.items.len() < FILE_COUNT);

    Ok(())
}

#[test]
fn restricted_path_returns_recoverable_error_when_windows_denies_listing(
) -> Result<(), Box<dyn std::error::Error>> {
    let Some(system_drive) = std::env::var_os("SystemDrive") else {
        return Ok(());
    };
    let protected_path = PathBuf::from(system_drive).join("System Volume Information");
    if !protected_path.exists() {
        return Ok(());
    }

    let gateway = NativeFileSystemGateway::new();
    let location = NavigationLocation::from_path(protected_path.clone())?;
    match gateway.list_items(&location, DisplayOptions::default(), Default::default()) {
        Ok(_) => {
            eprintln!(
                "restricted path probe was accessible in this environment: {:?}",
                protected_path
            );
            Ok(())
        }
        Err(error) => {
            assert!(
                error.user_message() == "권한이 없어 작업을 완료할 수 없습니다."
                    || error.user_message() == "위치를 찾을 수 없습니다."
            );
            assert!(
                error.to_string().contains("GetFileAttributesW")
                    || error.to_string().contains("FindFirstFileW")
            );
            Ok(())
        }
    }
}

#[test]
fn unc_navigation_location_preserves_unicode_path_without_network_access(
) -> Result<(), Box<dyn std::error::Error>> {
    let raw = PathBuf::from(r"\\server\share\폴더-🙂");
    let location = NavigationLocation::from_path(raw.clone())?;

    assert!(matches!(location, NavigationLocation::NetworkShare(_)));
    assert_eq!(location.as_path(), raw.as_path());

    Ok(())
}

#[derive(Debug, Clone, Copy)]
struct NoopShellGateway;

impl ShellOpenGateway for NoopShellGateway {
    fn open_path(&self, _location: &NavigationLocation) -> ExplorerResult<()> {
        Ok(())
    }
}

struct TempDirectory {
    path: PathBuf,
    attribute_paths: Vec<PathBuf>,
}

impl TempDirectory {
    fn new() -> io::Result<Self> {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or(0);
        let path = std::env::temp_dir().join(format!(
            "j3files-real-fs-test-{}-{unique}",
            std::process::id()
        ));
        fs::create_dir(&path)?;
        Ok(Self {
            path,
            attribute_paths: Vec::new(),
        })
    }

    fn path(&self) -> &Path {
        &self.path
    }

    fn track_attribute_path(&mut self, path: PathBuf) {
        self.attribute_paths.push(path);
    }
}

impl Drop for TempDirectory {
    fn drop(&mut self) {
        for path in &self.attribute_paths {
            let _ = set_file_attributes(path, FILE_ATTRIBUTE_NORMAL);
        }
        let _ = fs::remove_dir_all(&self.path);
    }
}

struct CancelAfterChecks {
    checks: AtomicUsize,
    allowed_false_checks: usize,
}

impl CancelAfterChecks {
    fn new(allowed_false_checks: usize) -> Self {
        Self {
            checks: AtomicUsize::new(0),
            allowed_false_checks,
        }
    }
}

impl SearchCancellation for CancelAfterChecks {
    fn is_cancel_requested(&self) -> bool {
        self.checks.fetch_add(1, Ordering::Relaxed) >= self.allowed_false_checks
    }
}

struct ExclusiveDirectoryLock {
    handle: HANDLE,
}

impl ExclusiveDirectoryLock {
    fn new(path: &Path) -> io::Result<Self> {
        const GENERIC_READ_ACCESS: u32 = 0x8000_0000;
        const FILE_FLAG_BACKUP_SEMANTICS_VALUE: u32 = 0x0200_0000;

        let wide_path = path_to_wide_null(path);
        // SAFETY: wide_path is null-terminated. Share mode 0 intentionally keeps this test
        // directory from being enumerated until the handle is dropped.
        let handle = unsafe {
            CreateFileW(
                wide_path.as_ptr(),
                GENERIC_READ_ACCESS,
                0,
                null(),
                OPEN_EXISTING,
                FILE_FLAG_BACKUP_SEMANTICS_VALUE,
                null_mut(),
            )
        };
        if handle == INVALID_HANDLE_VALUE {
            Err(last_os_error())
        } else {
            Ok(Self { handle })
        }
    }
}

impl Drop for ExclusiveDirectoryLock {
    fn drop(&mut self) {
        // SAFETY: handle was returned by CreateFileW and is closed exactly once here.
        unsafe {
            CloseHandle(self.handle);
        }
    }
}

fn display_names(items: &[FileItem]) -> Vec<OsString> {
    items.iter().map(|item| item.display_name.clone()).collect()
}

fn item_named(items: &[FileItem], name: &OsStr) -> Result<FileItem, Box<dyn std::error::Error>> {
    items
        .iter()
        .find(|item| item.display_name.as_os_str() == name)
        .cloned()
        .ok_or_else(|| format!("missing listed item {:?}", name).into())
}

fn assert_order(names: &[OsString], earlier: &str, later: &str) {
    let earlier_index = names
        .iter()
        .position(|name| name.as_os_str() == OsStr::new(earlier))
        .unwrap_or_else(|| panic!("missing {earlier} in {names:?}"));
    let later_index = names
        .iter()
        .position(|name| name.as_os_str() == OsStr::new(later))
        .unwrap_or_else(|| panic!("missing {later} in {names:?}"));
    assert!(
        earlier_index < later_index,
        "expected {earlier} before {later} in {names:?}"
    );
}

fn set_file_attributes(path: &Path, attributes: u32) -> io::Result<()> {
    let wide_path = path_to_wide_null(path);
    // SAFETY: wide_path is null-terminated and remains alive for the call.
    let succeeded = unsafe { SetFileAttributesW(wide_path.as_ptr(), attributes) };
    if succeeded == 0 {
        Err(last_os_error())
    } else {
        Ok(())
    }
}

fn set_updated_time(path: &Path, updated_at: SystemTime) -> io::Result<()> {
    let wide_path = path_to_wide_null(path);
    // SAFETY: wide_path is null-terminated. The handle is closed before returning.
    let handle = unsafe {
        CreateFileW(
            wide_path.as_ptr(),
            FILE_WRITE_ATTRIBUTES,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            null(),
            OPEN_EXISTING,
            FILE_ATTRIBUTE_NORMAL,
            null_mut(),
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        return Err(last_os_error());
    }

    let filetime = system_time_to_filetime(updated_at)?;
    // SAFETY: handle is valid, and null creation/access time pointers leave those fields unchanged.
    let succeeded = unsafe { SetFileTime(handle, null(), null(), &filetime) };
    let result = if succeeded == 0 {
        Err(last_os_error())
    } else {
        Ok(())
    };
    // SAFETY: handle was returned by CreateFileW and is closed exactly once here.
    unsafe {
        CloseHandle(handle);
    }
    result
}

fn system_time_to_filetime(value: SystemTime) -> io::Result<FILETIME> {
    const UNIX_EPOCH_FILETIME_TICKS: u64 = 116_444_736_000_000_000;
    const WINDOWS_TICKS_PER_SECOND: u64 = 10_000_000;

    let duration = value.duration_since(UNIX_EPOCH).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "test file time must be after the Unix epoch",
        )
    })?;
    let ticks = UNIX_EPOCH_FILETIME_TICKS
        + duration.as_secs() * WINDOWS_TICKS_PER_SECOND
        + u64::from(duration.subsec_nanos() / 100);

    Ok(FILETIME {
        dwLowDateTime: ticks as u32,
        dwHighDateTime: (ticks >> 32) as u32,
    })
}

fn path_to_wide_null(path: &Path) -> Vec<u16> {
    let mut value = path.as_os_str().encode_wide().collect::<Vec<_>>();
    value.push(0);
    value
}

fn last_os_error() -> io::Error {
    // SAFETY: GetLastError reads the calling thread's Windows error state.
    io::Error::from_raw_os_error(unsafe { GetLastError() } as i32)
}
