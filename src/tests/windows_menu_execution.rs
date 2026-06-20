#![cfg(windows)]

use std::collections::HashSet;
use std::error::Error;
use std::ffi::OsStr;
use std::fs;
use std::io;
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::ptr::null_mut;
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use windows_sys::Win32::Foundation::{HWND, LPARAM, RECT, WPARAM};
use windows_sys::Win32::System::DataExchange::IsClipboardFormatAvailable;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetClassNameW, GetDlgItem, GetMenu, GetMenuItemCount, GetMenuItemID, GetMenuState,
    GetMenuStringW, GetSubMenu, GetWindowRect, GetWindowTextLengthW, GetWindowTextW,
    GetWindowThreadProcessId, IsWindow, IsWindowVisible, PostMessageW, SendMessageTimeoutW,
    SendMessageW, SetWindowPos, BM_CLICK, HMENU, MF_BYCOMMAND, MF_BYPOSITION, MF_CHECKED,
    SMTO_ABORTIFHUNG, SMTO_BLOCK, SWP_NOMOVE, SWP_NOZORDER, WM_COMMAND,
};

const WINDOW_TITLE: &str = "j3Files";

const ID_NAV_BACK: u16 = 1005;
const ID_NAV_FORWARD: u16 = 1006;
const ID_NAV_UP: u16 = 1001;
const ID_REFRESH: u16 = 1002;
const ID_GO: u16 = 1003;
const ID_EXIT: u16 = 1004;
const ID_FILE_NEW_FOLDER: u16 = 1007;
const ID_FILE_OPEN: u16 = 1008;
const ID_FILE_OPEN_WITH: u16 = 1009;
const ID_FILE_PROPERTIES: u16 = 1010;
const ID_FILE_COPY: u16 = 1011;
const ID_FILE_CUT: u16 = 1012;
const ID_FILE_PASTE: u16 = 1013;
const ID_FILE_UNDO: u16 = 1014;
const ID_FILE_DELETE: u16 = 1015;
const ID_FILE_DELETE_PERMANENTLY: u16 = 1016;
const ID_FILE_RENAME: u16 = 1017;
const ID_FILE_SELECT_ALL: u16 = 1018;
const ID_TAB_CONTROL: i32 = 1100;
const ID_ADDRESS: i32 = 1101;
const ID_FILE_LIST: i32 = 1102;
const ID_FOLDER_TREE: i32 = 1103;
const ID_ADDRESS_FOCUS: u16 = 1104;
const ID_KNOWN_HOME: u16 = 1201;
const ID_KNOWN_DESKTOP: u16 = 1202;
const ID_KNOWN_DOWNLOADS: u16 = 1203;
const ID_KNOWN_DOCUMENTS: u16 = 1204;
const ID_DRIVE_BASE: u16 = 1300;
const ID_TAB_NEW: u16 = 1401;
const ID_TAB_CLOSE: u16 = 1402;
const ID_TAB_REOPEN: u16 = 1403;
const ID_TAB_MOVE_LEFT: u16 = 1404;
const ID_TAB_MOVE_RIGHT: u16 = 1405;
const ID_TAB_OPEN_SELECTED_FOLDER: u16 = 1406;
const ID_TAB_RESTORE_ON_STARTUP: u16 = 1407;
const ID_TAB_NEXT: u16 = 1408;
const ID_TAB_SET_STARTUP_FOLDER: u16 = 1409;
const ID_TAB_CLEAR_STARTUP_FOLDER: u16 = 1410;
const ID_BOOKMARK_ADD_CURRENT: u16 = 1501;
const ID_BOOKMARK_ADD_SELECTED_FOLDER: u16 = 1502;
const ID_BOOKMARK_REMOVE_CURRENT: u16 = 1503;
const ID_BOOKMARK_BASE: u16 = 1600;
const ID_VIEW_SHOW_HIDDEN: u16 = 1801;
const ID_VIEW_SHOW_SYSTEM: u16 = 1802;
const ID_SORT_NAME: u16 = 1811;
const ID_SORT_SIZE: u16 = 1812;
const ID_SORT_UPDATED: u16 = 1813;
const ID_SORT_KIND: u16 = 1814;
const ID_SORT_ASCENDING: u16 = 1821;
const ID_SORT_DESCENDING: u16 = 1822;
const ID_THEME_LIGHT: u16 = 1831;
const ID_THEME_CLASSIC_DARK: u16 = 1832;
const ID_THEME_SEPIA_TEAL: u16 = 1833;
const ID_THEME_GRAPHITE: u16 = 1834;
const ID_THEME_FOREST: u16 = 1835;
const ID_THEME_STEEL_BLUE: u16 = 1836;
const ID_VIEW_FONT: u16 = 1841;
const ID_VIEW_FONT_RESET: u16 = 1842;
const ID_SEARCH_QUERY_LABEL: i32 = 1901;
const ID_SEARCH_QUERY: i32 = 1902;
const ID_SEARCH_FIND: u16 = 1903;
const ID_SEARCH_SUBFOLDERS: i32 = 1904;
const ID_SEARCH_CANCEL: u16 = 1905;
const ID_SEARCH_FOCUS: u16 = 1906;
const ID_SEARCH_CLOSE: u16 = 1907;
const ID_SEARCH_INCLUDE_SUBFOLDERS: u16 = 1908;
const ID_FILE_OPERATION_STATUS: i32 = 1909;
const ID_ABOUT: u16 = 2001;

const CF_HDROP_FORMAT: u32 = 15;
const IDCANCEL_COMMAND: u16 = 2;
const MAX_BOOKMARK_MENU_ITEMS: u16 = 128;
const MAX_DRIVE_MENU_ITEMS: u16 = 64;

#[test]
#[ignore = "opens the Win32 GUI and dispatches real menu commands"]
fn menu_commands_execute_on_real_window() -> Result<(), Box<dyn Error>> {
    let start_dir = TempDirectory::new("j3files-menu-start")?;
    fs::write(start_dir.path().join("sample.txt"), b"sample")?;
    fs::create_dir(start_dir.path().join("child-folder"))?;

    let mut app = RunningApp::launch(start_dir.path())?;
    wait_for_menu_labels(
        app.hwnd,
        &[
            "File",
            "Edit",
            "View",
            "Go",
            "Bookmarks",
            "Tabs",
            "Search",
            "About",
        ],
    )?;
    assert_required_menu_commands(app.hwnd)?;
    resize_and_assert_layout(app.hwnd)?;

    for command in SAFE_NO_SELECTION_COMMANDS {
        send_command(&mut app, command.id, command.label)?;
    }

    send_command(&mut app, ID_SEARCH_FIND, "Search > Find")?;
    assert_search_controls_visible(app.hwnd)?;
    click_search_subfolder_checkbox(app.hwnd)?;
    assert_menu_item_checked(app.hwnd, ID_SEARCH_INCLUDE_SUBFOLDERS, true)?;
    send_command(
        &mut app,
        ID_SEARCH_INCLUDE_SUBFOLDERS,
        "Search > Include Subfolders",
    )?;
    assert_menu_item_checked(app.hwnd, ID_SEARCH_INCLUDE_SUBFOLDERS, false)?;
    send_command(&mut app, ID_SEARCH_CANCEL, "Search > Cancel Search")?;
    send_command(&mut app, ID_SEARCH_CLOSE, "Search > Close Search")?;

    send_command(&mut app, ID_BOOKMARK_ADD_CURRENT, "Bookmarks > Add Current")?;
    execute_first_dynamic_bookmark(&mut app)?;
    send_command(
        &mut app,
        ID_BOOKMARK_REMOVE_CURRENT,
        "Bookmarks > Remove Current",
    )?;

    send_command(&mut app, ID_FILE_NEW_FOLDER, "File > New Folder")?;
    send_command(&mut app, ID_FILE_SELECT_ALL, "Edit > Select All")?;
    send_command(
        &mut app,
        ID_BOOKMARK_ADD_SELECTED_FOLDER,
        "Bookmarks > Add Selected Folder",
    )?;
    execute_first_dynamic_bookmark(&mut app)?;
    send_command(
        &mut app,
        ID_BOOKMARK_REMOVE_CURRENT,
        "Bookmarks > Remove Current",
    )?;

    send_command(
        &mut app,
        ID_TAB_OPEN_SELECTED_FOLDER,
        "Tabs > Open Selected Folder in New Tab",
    )?;
    send_command(&mut app, ID_TAB_NEW, "Tabs > New Tab")?;
    send_command(&mut app, ID_TAB_NEXT, "Tabs > Next Tab")?;
    send_command(&mut app, ID_TAB_MOVE_LEFT, "Tabs > Move Tab Left")?;
    send_command(&mut app, ID_TAB_MOVE_RIGHT, "Tabs > Move Tab Right")?;
    send_command(
        &mut app,
        ID_TAB_SET_STARTUP_FOLDER,
        "Tabs > Startup > Use Current",
    )?;
    send_command(
        &mut app,
        ID_TAB_CLEAR_STARTUP_FOLDER,
        "Tabs > Startup > Clear",
    )?;
    send_command(
        &mut app,
        ID_TAB_RESTORE_ON_STARTUP,
        "Tabs > Startup > Restore Previous",
    )?;
    send_command(&mut app, ID_TAB_CLOSE, "Tabs > Close Tab")?;
    send_command(&mut app, ID_TAB_REOPEN, "Tabs > Reopen Closed Tab")?;

    run_font_dialog_cancel_smoke(&mut app)?;

    if unsafe { IsClipboardFormatAvailable(CF_HDROP_FORMAT) } == 0 {
        send_command(&mut app, ID_FILE_PASTE, "Edit > Paste")?;
    } else {
        eprintln!("[menu-smoke] skipped Paste because CF_HDROP is present on the clipboard");
    }

    for command in NAVIGATION_COMMANDS {
        send_command(&mut app, command.id, command.label)?;
    }
    execute_drive_menu_items(&mut app)?;

    send_command(&mut app, ID_EXIT, "File > Exit")?;
    let output = app.wait_for_exit(Duration::from_secs(5))?;
    assert!(
        output.status.success(),
        "j3Files exited unsuccessfully: {:?}",
        output.status
    );
    assert!(
        output.stderr.trim().is_empty(),
        "j3Files wrote to stderr during menu smoke:\n{}",
        output.stderr
    );
    assert!(
        output.stdout.trim().is_empty(),
        "j3Files wrote to stdout during menu smoke:\n{}",
        output.stdout
    );

    Ok(())
}

#[derive(Debug, Clone, Copy)]
struct MenuCommand {
    id: u16,
    label: &'static str,
}

const SAFE_NO_SELECTION_COMMANDS: &[MenuCommand] = &[
    MenuCommand {
        id: ID_NAV_BACK,
        label: "Go > Back",
    },
    MenuCommand {
        id: ID_NAV_FORWARD,
        label: "Go > Forward",
    },
    MenuCommand {
        id: ID_REFRESH,
        label: "View > Refresh",
    },
    MenuCommand {
        id: ID_GO,
        label: "Address Go",
    },
    MenuCommand {
        id: ID_ADDRESS_FOCUS,
        label: "Address Focus",
    },
    MenuCommand {
        id: ID_SEARCH_FOCUS,
        label: "Search Focus",
    },
    MenuCommand {
        id: ID_FILE_OPEN,
        label: "File > Open",
    },
    MenuCommand {
        id: ID_FILE_OPEN_WITH,
        label: "File > Open With",
    },
    MenuCommand {
        id: ID_FILE_RENAME,
        label: "File > Rename",
    },
    MenuCommand {
        id: ID_FILE_DELETE,
        label: "File > Move to Recycle Bin",
    },
    MenuCommand {
        id: ID_FILE_DELETE_PERMANENTLY,
        label: "File > Delete Permanently",
    },
    MenuCommand {
        id: ID_FILE_PROPERTIES,
        label: "File > Properties",
    },
    MenuCommand {
        id: ID_FILE_COPY,
        label: "Edit > Copy",
    },
    MenuCommand {
        id: ID_FILE_CUT,
        label: "Edit > Cut",
    },
    MenuCommand {
        id: ID_FILE_UNDO,
        label: "Edit > Undo",
    },
    MenuCommand {
        id: ID_SORT_NAME,
        label: "View > Sort By > Name",
    },
    MenuCommand {
        id: ID_SORT_SIZE,
        label: "View > Sort By > Size",
    },
    MenuCommand {
        id: ID_SORT_UPDATED,
        label: "View > Sort By > Updated",
    },
    MenuCommand {
        id: ID_SORT_KIND,
        label: "View > Sort By > Type",
    },
    MenuCommand {
        id: ID_SORT_ASCENDING,
        label: "View > Sort By > Ascending",
    },
    MenuCommand {
        id: ID_SORT_DESCENDING,
        label: "View > Sort By > Descending",
    },
    MenuCommand {
        id: ID_VIEW_SHOW_HIDDEN,
        label: "View > Show Hidden Files",
    },
    MenuCommand {
        id: ID_VIEW_SHOW_SYSTEM,
        label: "View > Show System Files",
    },
    MenuCommand {
        id: ID_THEME_LIGHT,
        label: "View > Appearance > Light",
    },
    MenuCommand {
        id: ID_THEME_CLASSIC_DARK,
        label: "View > Appearance > Classic Dark",
    },
    MenuCommand {
        id: ID_THEME_SEPIA_TEAL,
        label: "View > Appearance > Sepia Teal",
    },
    MenuCommand {
        id: ID_THEME_GRAPHITE,
        label: "View > Appearance > Graphite",
    },
    MenuCommand {
        id: ID_THEME_FOREST,
        label: "View > Appearance > Forest",
    },
    MenuCommand {
        id: ID_THEME_STEEL_BLUE,
        label: "View > Appearance > Steel Blue",
    },
    MenuCommand {
        id: ID_VIEW_FONT_RESET,
        label: "View > Appearance > Reset Font",
    },
];

const NAVIGATION_COMMANDS: &[MenuCommand] = &[
    MenuCommand {
        id: ID_NAV_UP,
        label: "Go > Up One Level",
    },
    MenuCommand {
        id: ID_KNOWN_HOME,
        label: "Go > Home",
    },
    MenuCommand {
        id: ID_KNOWN_DESKTOP,
        label: "Go > Desktop",
    },
    MenuCommand {
        id: ID_KNOWN_DOWNLOADS,
        label: "Go > Downloads",
    },
    MenuCommand {
        id: ID_KNOWN_DOCUMENTS,
        label: "Go > Documents",
    },
];

struct RunningApp {
    child: Option<Child>,
    hwnd: HWND,
}

impl RunningApp {
    fn launch(start_dir: &Path) -> Result<Self, Box<dyn Error>> {
        let exe = smoke_exe_path()?;
        let mut child = Command::new(&exe)
            .arg(start_dir)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|error| format!("failed to launch {:?}: {error}", exe))?;
        let pid = child.id();
        let hwnd = match wait_for_main_window(pid, Duration::from_secs(10)) {
            Ok(hwnd) => hwnd,
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(error);
            }
        };
        thread::sleep(Duration::from_millis(250));
        Ok(Self {
            child: Some(child),
            hwnd,
        })
    }

    fn ensure_running(&mut self, label: &str) -> Result<(), Box<dyn Error>> {
        let Some(child) = self.child.as_mut() else {
            return Err("j3Files process is no longer owned by the test".into());
        };
        if let Some(status) = child.try_wait()? {
            return Err(format!("j3Files exited while executing {label}: {status:?}").into());
        }
        if unsafe { IsWindow(self.hwnd) } == 0 {
            return Err(format!("j3Files window disappeared while executing {label}").into());
        }
        Ok(())
    }

    fn wait_for_exit(&mut self, timeout: Duration) -> Result<AppOutput, Box<dyn Error>> {
        let Some(mut child) = self.child.take() else {
            return Err("j3Files process was already taken".into());
        };
        let deadline = Instant::now() + timeout;
        loop {
            if child.try_wait()?.is_some() {
                let output = child.wait_with_output()?;
                return Ok(AppOutput {
                    status: output.status,
                    stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
                    stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
                });
            }
            if Instant::now() >= deadline {
                let _ = child.kill();
                let output = child.wait_with_output()?;
                return Err(format!(
                    "j3Files did not exit in time; stdout:\n{}\nstderr:\n{}",
                    String::from_utf8_lossy(&output.stdout),
                    String::from_utf8_lossy(&output.stderr)
                )
                .into());
            }
            thread::sleep(Duration::from_millis(50));
        }
    }
}

impl Drop for RunningApp {
    fn drop(&mut self) {
        if let Some(child) = self.child.as_mut() {
            if matches!(child.try_wait(), Ok(None)) {
                let _ = child.kill();
                let _ = child.wait();
            }
        }
    }
}

struct AppOutput {
    status: std::process::ExitStatus,
    stdout: String,
    stderr: String,
}

fn smoke_exe_path() -> Result<PathBuf, Box<dyn Error>> {
    if let Some(path) = std::env::var_os("J3FILES_SMOKE_EXE") {
        return Ok(PathBuf::from(path));
    }
    Ok(PathBuf::from(env!("CARGO_BIN_EXE_j3files")))
}

fn send_command(app: &mut RunningApp, id: u16, label: &str) -> Result<(), Box<dyn Error>> {
    app.ensure_running(label)?;
    let mut result = 0;
    let succeeded = unsafe {
        SendMessageTimeoutW(
            app.hwnd,
            WM_COMMAND,
            WPARAM::from(id),
            0,
            SMTO_ABORTIFHUNG | SMTO_BLOCK,
            3_000,
            &mut result,
        )
    };
    if succeeded == 0 {
        return Err(format!(
            "{label} timed out or failed: {}",
            io::Error::last_os_error()
        )
        .into());
    }
    if id == ID_EXIT {
        return Ok(());
    }
    thread::sleep(Duration::from_millis(75));
    app.ensure_running(label)
}

fn post_command(hwnd: HWND, id: u16) -> Result<(), Box<dyn Error>> {
    let succeeded = unsafe { PostMessageW(hwnd, WM_COMMAND, WPARAM::from(id), 0) };
    if succeeded == 0 {
        return Err(format!(
            "failed to post command {id}: {}",
            io::Error::last_os_error()
        )
        .into());
    }
    Ok(())
}

fn run_font_dialog_cancel_smoke(app: &mut RunningApp) -> Result<(), Box<dyn Error>> {
    post_command(app.hwnd, ID_VIEW_FONT)?;
    let dialog = wait_for_modal_dialog(
        app.child.as_ref().ok_or("missing child process")?.id(),
        app.hwnd,
        Duration::from_secs(5),
    )?;
    let mut result = 0;
    let succeeded = unsafe {
        SendMessageTimeoutW(
            dialog,
            WM_COMMAND,
            WPARAM::from(IDCANCEL_COMMAND),
            0,
            SMTO_ABORTIFHUNG | SMTO_BLOCK,
            3_000,
            &mut result,
        )
    };
    if succeeded == 0 {
        return Err(format!(
            "failed to cancel font dialog: {}",
            io::Error::last_os_error()
        )
        .into());
    }
    wait_until_no_modal_dialog(
        app.child.as_ref().ok_or("missing child process")?.id(),
        app.hwnd,
        Duration::from_secs(5),
    )?;
    app.ensure_running("View > Appearance > Font")
}

fn execute_first_dynamic_bookmark(app: &mut RunningApp) -> Result<(), Box<dyn Error>> {
    thread::sleep(Duration::from_millis(100));
    let ids = menu_command_ids(app.hwnd)?;
    let Some(id) =
        (ID_BOOKMARK_BASE..ID_BOOKMARK_BASE + MAX_BOOKMARK_MENU_ITEMS).find(|id| ids.contains(id))
    else {
        return Err("expected a dynamic bookmark menu item".into());
    };
    send_command(app, id, "Bookmarks > dynamic bookmark")
}

fn execute_drive_menu_items(app: &mut RunningApp) -> Result<(), Box<dyn Error>> {
    let ids = menu_command_ids(app.hwnd)?;
    for id in (ID_DRIVE_BASE..ID_DRIVE_BASE + MAX_DRIVE_MENU_ITEMS).filter(|id| ids.contains(id)) {
        send_command(app, id, "Go > Drives > dynamic drive")?;
    }
    Ok(())
}

fn click_search_subfolder_checkbox(hwnd: HWND) -> Result<(), Box<dyn Error>> {
    let checkbox = control(hwnd, ID_SEARCH_SUBFOLDERS)?;
    unsafe {
        SendMessageW(checkbox, BM_CLICK, 0, 0);
    }
    thread::sleep(Duration::from_millis(75));
    Ok(())
}

fn resize_and_assert_layout(hwnd: HWND) -> Result<(), Box<dyn Error>> {
    let succeeded =
        unsafe { SetWindowPos(hwnd, null_mut(), 0, 0, 980, 620, SWP_NOMOVE | SWP_NOZORDER) };
    if succeeded == 0 {
        return Err(format!("failed to resize j3Files: {}", io::Error::last_os_error()).into());
    }
    thread::sleep(Duration::from_millis(100));
    for (id, label) in [
        (ID_FOLDER_TREE, "folder tree"),
        (ID_TAB_CONTROL, "tab control"),
        (ID_ADDRESS, "address edit"),
        (ID_FILE_LIST, "file list"),
        (ID_FILE_OPERATION_STATUS, "file operation status"),
    ] {
        let child = control(hwnd, id)?;
        let rect = window_rect(child, label)?;
        assert!(
            rect.right > rect.left && rect.bottom > rect.top,
            "{label} has non-positive bounds: left={}, top={}, right={}, bottom={}",
            rect.left,
            rect.top,
            rect.right,
            rect.bottom
        );
    }
    Ok(())
}

fn assert_search_controls_visible(hwnd: HWND) -> Result<(), Box<dyn Error>> {
    for (id, label) in [
        (ID_SEARCH_QUERY_LABEL, "search query label"),
        (ID_SEARCH_QUERY, "search query edit"),
        (i32::from(ID_SEARCH_FIND), "search find button"),
        (ID_SEARCH_SUBFOLDERS, "search subfolder checkbox"),
        (i32::from(ID_SEARCH_CANCEL), "search cancel button"),
    ] {
        let child = control(hwnd, id)?;
        assert!(
            unsafe { IsWindowVisible(child) } != 0,
            "{label} should be visible after Search > Find"
        );
        let rect = window_rect(child, label)?;
        assert!(
            rect.right > rect.left && rect.bottom > rect.top,
            "{label} has non-positive bounds: left={}, top={}, right={}, bottom={}",
            rect.left,
            rect.top,
            rect.right,
            rect.bottom
        );
    }
    Ok(())
}

fn control(hwnd: HWND, id: i32) -> Result<HWND, Box<dyn Error>> {
    let child = unsafe { GetDlgItem(hwnd, id) };
    if child.is_null() {
        return Err(format!("missing child control id {id}").into());
    }
    Ok(child)
}

fn window_rect(hwnd: HWND, label: &str) -> Result<RECT, Box<dyn Error>> {
    let mut rect = RECT::default();
    let succeeded = unsafe { GetWindowRect(hwnd, &mut rect) };
    if succeeded == 0 {
        return Err(format!(
            "failed to read {label} rect: {}",
            io::Error::last_os_error()
        )
        .into());
    }
    Ok(rect)
}

fn assert_required_menu_commands(hwnd: HWND) -> Result<(), Box<dyn Error>> {
    let ids = menu_command_ids(hwnd)?;
    for command in REQUIRED_MENU_COMMANDS {
        assert!(
            ids.contains(&command.id),
            "missing menu command {} ({})",
            command.id,
            command.label
        );
    }
    Ok(())
}

const REQUIRED_MENU_COMMANDS: &[MenuCommand] = &[
    MenuCommand {
        id: ID_FILE_NEW_FOLDER,
        label: "New Folder",
    },
    MenuCommand {
        id: ID_FILE_OPEN,
        label: "Open",
    },
    MenuCommand {
        id: ID_FILE_OPEN_WITH,
        label: "Open With",
    },
    MenuCommand {
        id: ID_FILE_RENAME,
        label: "Rename",
    },
    MenuCommand {
        id: ID_FILE_DELETE,
        label: "Move to Recycle Bin",
    },
    MenuCommand {
        id: ID_FILE_DELETE_PERMANENTLY,
        label: "Delete Permanently",
    },
    MenuCommand {
        id: ID_FILE_PROPERTIES,
        label: "Properties",
    },
    MenuCommand {
        id: ID_EXIT,
        label: "Exit",
    },
    MenuCommand {
        id: ID_FILE_UNDO,
        label: "Undo",
    },
    MenuCommand {
        id: ID_FILE_CUT,
        label: "Cut",
    },
    MenuCommand {
        id: ID_FILE_COPY,
        label: "Copy",
    },
    MenuCommand {
        id: ID_FILE_PASTE,
        label: "Paste",
    },
    MenuCommand {
        id: ID_FILE_SELECT_ALL,
        label: "Select All",
    },
    MenuCommand {
        id: ID_REFRESH,
        label: "Refresh",
    },
    MenuCommand {
        id: ID_SORT_NAME,
        label: "Sort Name",
    },
    MenuCommand {
        id: ID_SORT_SIZE,
        label: "Sort Size",
    },
    MenuCommand {
        id: ID_SORT_UPDATED,
        label: "Sort Updated",
    },
    MenuCommand {
        id: ID_SORT_KIND,
        label: "Sort Type",
    },
    MenuCommand {
        id: ID_SORT_ASCENDING,
        label: "Sort Ascending",
    },
    MenuCommand {
        id: ID_SORT_DESCENDING,
        label: "Sort Descending",
    },
    MenuCommand {
        id: ID_VIEW_SHOW_HIDDEN,
        label: "Show Hidden Files",
    },
    MenuCommand {
        id: ID_VIEW_SHOW_SYSTEM,
        label: "Show System Files",
    },
    MenuCommand {
        id: ID_THEME_LIGHT,
        label: "Theme Light",
    },
    MenuCommand {
        id: ID_THEME_CLASSIC_DARK,
        label: "Theme Classic Dark",
    },
    MenuCommand {
        id: ID_THEME_SEPIA_TEAL,
        label: "Theme Sepia Teal",
    },
    MenuCommand {
        id: ID_THEME_GRAPHITE,
        label: "Theme Graphite",
    },
    MenuCommand {
        id: ID_THEME_FOREST,
        label: "Theme Forest",
    },
    MenuCommand {
        id: ID_THEME_STEEL_BLUE,
        label: "Theme Steel Blue",
    },
    MenuCommand {
        id: ID_VIEW_FONT,
        label: "Font",
    },
    MenuCommand {
        id: ID_VIEW_FONT_RESET,
        label: "Reset Font",
    },
    MenuCommand {
        id: ID_NAV_BACK,
        label: "Back",
    },
    MenuCommand {
        id: ID_NAV_FORWARD,
        label: "Forward",
    },
    MenuCommand {
        id: ID_NAV_UP,
        label: "Up",
    },
    MenuCommand {
        id: ID_KNOWN_HOME,
        label: "Home",
    },
    MenuCommand {
        id: ID_KNOWN_DESKTOP,
        label: "Desktop",
    },
    MenuCommand {
        id: ID_KNOWN_DOWNLOADS,
        label: "Downloads",
    },
    MenuCommand {
        id: ID_KNOWN_DOCUMENTS,
        label: "Documents",
    },
    MenuCommand {
        id: ID_BOOKMARK_ADD_CURRENT,
        label: "Add Current Location",
    },
    MenuCommand {
        id: ID_BOOKMARK_ADD_SELECTED_FOLDER,
        label: "Add Selected Folder",
    },
    MenuCommand {
        id: ID_BOOKMARK_REMOVE_CURRENT,
        label: "Remove Current Bookmark",
    },
    MenuCommand {
        id: ID_TAB_NEW,
        label: "New Tab",
    },
    MenuCommand {
        id: ID_TAB_OPEN_SELECTED_FOLDER,
        label: "Open Selected Folder in New Tab",
    },
    MenuCommand {
        id: ID_TAB_CLOSE,
        label: "Close Tab",
    },
    MenuCommand {
        id: ID_TAB_NEXT,
        label: "Next Tab",
    },
    MenuCommand {
        id: ID_TAB_REOPEN,
        label: "Reopen Closed Tab",
    },
    MenuCommand {
        id: ID_TAB_MOVE_LEFT,
        label: "Move Tab Left",
    },
    MenuCommand {
        id: ID_TAB_MOVE_RIGHT,
        label: "Move Tab Right",
    },
    MenuCommand {
        id: ID_TAB_SET_STARTUP_FOLDER,
        label: "Use Current Folder on Startup",
    },
    MenuCommand {
        id: ID_TAB_CLEAR_STARTUP_FOLDER,
        label: "Clear Startup Folder",
    },
    MenuCommand {
        id: ID_TAB_RESTORE_ON_STARTUP,
        label: "Restore Previous Tabs on Startup",
    },
    MenuCommand {
        id: ID_SEARCH_FIND,
        label: "Find",
    },
    MenuCommand {
        id: ID_SEARCH_INCLUDE_SUBFOLDERS,
        label: "Include Subfolders",
    },
    MenuCommand {
        id: ID_SEARCH_CANCEL,
        label: "Cancel Search",
    },
    MenuCommand {
        id: ID_SEARCH_CLOSE,
        label: "Close Search",
    },
    MenuCommand {
        id: ID_ABOUT,
        label: "About j3Files",
    },
];

fn wait_for_menu_labels(hwnd: HWND, expected: &[&str]) -> Result<(), Box<dyn Error>> {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let labels = top_menu_labels(hwnd)?;
        if labels == expected {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(format!("unexpected top menu labels: {labels:?}").into());
        }
        thread::sleep(Duration::from_millis(50));
    }
}

fn top_menu_labels(hwnd: HWND) -> Result<Vec<String>, Box<dyn Error>> {
    let menu = window_menu(hwnd)?;
    let count = unsafe { GetMenuItemCount(menu) };
    if count < 0 {
        return Err(format!("failed to count menu items: {}", io::Error::last_os_error()).into());
    }
    let mut labels = Vec::new();
    for index in 0..count {
        labels.push(menu_text(menu, index as u32)?);
    }
    Ok(labels)
}

fn window_menu(hwnd: HWND) -> Result<HMENU, Box<dyn Error>> {
    let menu = unsafe { GetMenu(hwnd) };
    if menu.is_null() {
        return Err("main window has no menu".into());
    }
    Ok(menu)
}

fn menu_command_ids(hwnd: HWND) -> Result<HashSet<u16>, Box<dyn Error>> {
    let menu = window_menu(hwnd)?;
    let mut ids = HashSet::new();
    collect_menu_command_ids(menu, &mut ids)?;
    Ok(ids)
}

fn collect_menu_command_ids(menu: HMENU, ids: &mut HashSet<u16>) -> Result<(), Box<dyn Error>> {
    let count = unsafe { GetMenuItemCount(menu) };
    if count < 0 {
        return Err(format!(
            "failed to count submenu items: {}",
            io::Error::last_os_error()
        )
        .into());
    }
    for index in 0..count {
        let submenu = unsafe { GetSubMenu(menu, index) };
        if !submenu.is_null() {
            collect_menu_command_ids(submenu, ids)?;
            continue;
        }

        let id = unsafe { GetMenuItemID(menu, index) };
        if id != u32::MAX && id != 0 {
            if let Ok(id) = u16::try_from(id) {
                ids.insert(id);
            }
        }
    }
    Ok(())
}

fn assert_menu_item_checked(hwnd: HWND, id: u16, expected: bool) -> Result<(), Box<dyn Error>> {
    let menu = window_menu(hwnd)?;
    let state = unsafe { GetMenuState(menu, u32::from(id), MF_BYCOMMAND) };
    if state == u32::MAX {
        return Err(format!("menu item {id} was not found").into());
    }
    assert_eq!(
        state & MF_CHECKED != 0,
        expected,
        "unexpected checked state for menu item {id}"
    );
    Ok(())
}

fn menu_text(menu: HMENU, index: u32) -> Result<String, Box<dyn Error>> {
    let mut buffer = vec![0_u16; 256];
    let len = unsafe {
        GetMenuStringW(
            menu,
            index,
            buffer.as_mut_ptr(),
            buffer.len() as i32,
            MF_BYPOSITION,
        )
    };
    if len == 0 {
        return Ok(String::new());
    }
    buffer.truncate(len as usize);
    Ok(String::from_utf16_lossy(&buffer).replace('&', ""))
}

fn wait_for_main_window(pid: u32, timeout: Duration) -> Result<HWND, Box<dyn Error>> {
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(hwnd) = find_main_window(pid) {
            return Ok(hwnd);
        }
        if Instant::now() >= deadline {
            return Err(format!("timed out waiting for j3Files window for pid {pid}").into());
        }
        thread::sleep(Duration::from_millis(50));
    }
}

fn find_main_window(pid: u32) -> Option<HWND> {
    let mut data = FindWindowData {
        pid,
        hwnd: null_mut(),
    };
    unsafe {
        EnumWindows(
            Some(enum_main_window),
            &mut data as *mut FindWindowData as LPARAM,
        );
    }
    if data.hwnd.is_null() {
        None
    } else {
        Some(data.hwnd)
    }
}

struct FindWindowData {
    pid: u32,
    hwnd: HWND,
}

unsafe extern "system" fn enum_main_window(hwnd: HWND, lparam: LPARAM) -> i32 {
    let data = &mut *(lparam as *mut FindWindowData);
    let mut window_pid = 0;
    GetWindowThreadProcessId(hwnd, &mut window_pid);
    if window_pid == data.pid && IsWindowVisible(hwnd) != 0 && window_text(hwnd) == WINDOW_TITLE {
        data.hwnd = hwnd;
        return 0;
    }
    1
}

fn wait_for_modal_dialog(pid: u32, owner: HWND, timeout: Duration) -> Result<HWND, Box<dyn Error>> {
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(hwnd) = find_modal_dialog(pid, owner) {
            return Ok(hwnd);
        }
        if Instant::now() >= deadline {
            return Err("timed out waiting for font dialog".into());
        }
        thread::sleep(Duration::from_millis(50));
    }
}

fn wait_until_no_modal_dialog(
    pid: u32,
    owner: HWND,
    timeout: Duration,
) -> Result<(), Box<dyn Error>> {
    let deadline = Instant::now() + timeout;
    loop {
        if find_modal_dialog(pid, owner).is_none() {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err("font dialog did not close".into());
        }
        thread::sleep(Duration::from_millis(50));
    }
}

fn find_modal_dialog(pid: u32, owner: HWND) -> Option<HWND> {
    let mut data = FindDialogData {
        pid,
        owner,
        hwnd: null_mut(),
    };
    unsafe {
        EnumWindows(
            Some(enum_modal_dialog),
            &mut data as *mut FindDialogData as LPARAM,
        );
    }
    if data.hwnd.is_null() {
        None
    } else {
        Some(data.hwnd)
    }
}

struct FindDialogData {
    pid: u32,
    owner: HWND,
    hwnd: HWND,
}

unsafe extern "system" fn enum_modal_dialog(hwnd: HWND, lparam: LPARAM) -> i32 {
    let data = &mut *(lparam as *mut FindDialogData);
    if hwnd == data.owner || IsWindowVisible(hwnd) == 0 {
        return 1;
    }

    let mut window_pid = 0;
    GetWindowThreadProcessId(hwnd, &mut window_pid);
    if window_pid == data.pid && window_class(hwnd) == "#32770" {
        data.hwnd = hwnd;
        return 0;
    }
    1
}

unsafe fn window_text(hwnd: HWND) -> String {
    let len = GetWindowTextLengthW(hwnd);
    if len <= 0 {
        return String::new();
    }
    let mut buffer = vec![0_u16; len as usize + 1];
    let copied = GetWindowTextW(hwnd, buffer.as_mut_ptr(), buffer.len() as i32);
    buffer.truncate(copied.max(0) as usize);
    String::from_utf16_lossy(&buffer)
}

unsafe fn window_class(hwnd: HWND) -> String {
    let mut buffer = vec![0_u16; 128];
    let copied = GetClassNameW(hwnd, buffer.as_mut_ptr(), buffer.len() as i32);
    buffer.truncate(copied.max(0) as usize);
    String::from_utf16_lossy(&buffer)
}

struct TempDirectory {
    path: PathBuf,
}

impl TempDirectory {
    fn new(prefix: &str) -> io::Result<Self> {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or(0);
        let path = std::env::temp_dir().join(format!("{prefix}-{}-{unique}", std::process::id()));
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

#[allow(dead_code)]
fn os_to_wide_null(value: &OsStr) -> Vec<u16> {
    let mut wide = value.encode_wide().collect::<Vec<_>>();
    wide.push(0);
    wide
}
