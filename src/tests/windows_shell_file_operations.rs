use std::ffi::OsStr;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use j3files::app::ExplorerApp;
use j3files::domain::{ExplorerResult, NavigationLocation};
use j3files::infra::NativeFileSystemGateway;
use j3files::platform;

#[test]
#[ignore = "uses Windows Shell IFileOperation and moves one temp file to the Recycle Bin"]
fn shell_file_operations_work_on_safe_temp_tree() -> Result<(), Box<dyn std::error::Error>> {
    let temp_dir = TempDirectory::new()?;
    let root = temp_dir.path();

    let root_location = NavigationLocation::from_path(root.to_path_buf())?;
    let mut app = ExplorerApp::new(
        root_location,
        NativeFileSystemGateway::new(),
        NoopShellGateway,
    );
    let new_folder = app.create_folder_in_active(OsStr::new("new-folder-from-app"), false)?;
    assert!(new_folder.created_folder.as_path().is_dir());
    eprintln!(
        "[shell-smoke] create folder: ok; path={:?}",
        new_folder.created_folder.as_path()
    );

    let source_folder = root.join("source-한글");
    let source_nested_file = source_folder.join("nested.txt");
    let copy_destination = root.join("copy-destination");
    fs::create_dir(&source_folder)?;
    fs::write(&source_nested_file, b"nested")?;
    fs::create_dir(&copy_destination)?;

    log_shell_result(
        "copy folder",
        platform::shell_copy_items(std::slice::from_ref(&source_folder), &copy_destination),
    )?;
    let copied_nested_file = copy_destination
        .join(Path::new(
            source_folder
                .file_name()
                .ok_or("missing source folder name")?,
        ))
        .join("nested.txt");
    assert!(copied_nested_file.is_file());

    let move_destination = root.join("move-destination");
    fs::create_dir(&move_destination)?;
    log_shell_result(
        "move file",
        platform::shell_move_items(std::slice::from_ref(&copied_nested_file), &move_destination),
    )?;
    let moved_file = move_destination.join("nested.txt");
    assert!(moved_file.is_file());
    assert!(!copied_nested_file.exists());

    log_shell_result(
        "rename file",
        platform::shell_rename_item(&moved_file, OsStr::new("renamed.txt")),
    )?;
    let renamed_file = move_destination.join("renamed.txt");
    assert!(renamed_file.is_file());
    assert!(!moved_file.exists());

    let recycle_target = root.join("recycle-me.txt");
    fs::write(&recycle_target, b"trash")?;
    log_shell_result(
        "delete to recycle bin",
        platform::shell_delete_to_recycle_bin(std::slice::from_ref(&recycle_target)),
    )?;
    assert!(!recycle_target.exists());

    Ok(())
}

fn log_shell_result(result_name: &str, result: ExplorerResult<()>) -> ExplorerResult<()> {
    match &result {
        Ok(()) => eprintln!("[shell-smoke] {result_name}: ok"),
        Err(error) => eprintln!(
            "[shell-smoke] {result_name}: error; cancelled={}; elevation_required={}; diagnostic={error}",
            error.is_cancelled(),
            error.requires_elevation()
        ),
    }
    result
}

#[derive(Debug, Clone, Copy)]
struct NoopShellGateway;

struct TempDirectory {
    path: PathBuf,
}

impl TempDirectory {
    fn new() -> io::Result<Self> {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or(0);
        let path = std::env::temp_dir().join(format!(
            "j3files-shell-operation-test-{}-{unique}",
            std::process::id()
        ));
        fs::create_dir(&path)?;
        Ok(Self { path })
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}
