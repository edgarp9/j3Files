use std::ffi::{OsStr, OsString};
use std::fs::{self, OpenOptions};
use std::io::{self, Read, Write};
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::app::{UserSession, UserSettings, UserSettingsGateway};
use crate::domain::{
    dark_theme_storage_value, AppearanceFont, AppearanceTheme, BookmarkAccessibility, BookmarkItem,
    BookmarkList, ExplorerError, ExplorerResult, KnownFolderKind, NavigationLocation,
    SortDirection, SortKey, SortState, TabId, TabState,
};
use crate::platform;

const SETTINGS_FILE_EXTENSION: &str = "json";
const SETTINGS_HEADER: &str = "j3files-settings\t1";
const SETTING_APPEARANCE_THEME: &str = "appearance_theme";
const SETTING_APPEARANCE_DARK_THEME: &str = "dark_theme";
const SETTING_APPEARANCE_THEME_COMPAT: &str = "appearance.theme";
const SETTING_APPEARANCE_DARK_THEME_COMPAT: &str = "appearance.dark_theme";
const SETTING_APPEARANCE_FONT_FAMILY: &str = "appearance_font_family";
const SETTING_APPEARANCE_FONT_SIZE: &str = "appearance_font_size";
const RECORD_STARTUP_FOLDER: &str = "startup_folder";
const NO_VALUE: &str = "-";
const MAX_USER_SETTINGS_BYTES: usize = 4 * 1024 * 1024;
const MAX_OS_STRING_UTF16_UNITS: usize = 32 * 1024;
const MAX_ENCODED_OS_HEX_CHARS: usize = MAX_OS_STRING_UTF16_UNITS * 4;
static TEMP_FILE_COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StoredTabList {
    Open,
    Closed,
}

#[derive(Debug)]
struct StoredTabBuilder {
    id: TabId,
    current_location: NavigationLocation,
    back_history: Vec<NavigationLocation>,
    forward_history: Vec<NavigationLocation>,
    sort: SortState,
}

impl StoredTabBuilder {
    fn new(id: TabId, current_location: NavigationLocation, sort: SortState) -> Self {
        Self {
            id,
            current_location,
            back_history: Vec::new(),
            forward_history: Vec::new(),
            sort,
        }
    }

    fn into_tab(self) -> TabState {
        TabState::from_parts(
            self.id,
            self.current_location,
            self.back_history,
            self.forward_history,
            self.sort,
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeUserSettingsStore {
    path: PathBuf,
}

#[derive(Debug)]
pub struct UserSettingsLoadOutcome {
    pub settings: UserSettings,
    pub warning: Option<ExplorerError>,
    pub save_allowed: bool,
}

impl NativeUserSettingsStore {
    pub fn new() -> ExplorerResult<Self> {
        Ok(Self::at_path(default_user_settings_path()?))
    }

    pub fn at_path(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn load_user_settings_with_recovery(&self) -> UserSettingsLoadOutcome {
        match <Self as UserSettingsGateway>::load_user_settings(self) {
            Ok(settings) => UserSettingsLoadOutcome {
                settings,
                warning: None,
                save_allowed: true,
            },
            Err(error) => UserSettingsLoadOutcome {
                settings: UserSettings::default(),
                warning: Some(error),
                save_allowed: false,
            },
        }
    }
}

impl UserSettingsGateway for NativeUserSettingsStore {
    fn load_user_settings(&self) -> ExplorerResult<UserSettings> {
        let content = match read_user_settings_file(&self.path)? {
            Some(content) => content,
            None => return Ok(UserSettings::default()),
        };

        parse_user_settings(&content)
    }

    fn save_user_settings(&self, settings: &UserSettings) -> ExplorerResult<()> {
        if let Some(parent) = self
            .path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            fs::create_dir_all(parent).map_err(|source| {
                ExplorerError::io(
                    "create user settings directory",
                    Some(parent.to_path_buf()),
                    source,
                )
            })?;
        }

        let content = serialize_user_settings(settings)?;
        if content.len() > MAX_USER_SETTINGS_BYTES {
            return Err(settings_file_size_error());
        }

        let temporary_path = temporary_settings_path(&self.path)?;
        if let Err(error) = write_settings_file(&temporary_path, &content) {
            let _ = fs::remove_file(&temporary_path);
            return Err(error);
        }

        match platform::replace_file(&temporary_path, &self.path) {
            Ok(()) => Ok(()),
            Err(error) => {
                let _ = fs::remove_file(&temporary_path);
                Err(error)
            }
        }
    }
}

fn read_user_settings_file(path: &Path) -> ExplorerResult<Option<String>> {
    let file = match fs::File::open(path) {
        Ok(file) => file,
        Err(source) if source.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(source) => {
            return Err(ExplorerError::io(
                "read user settings",
                Some(path.to_path_buf()),
                source,
            ));
        }
    };

    let metadata = file.metadata().map_err(|source| {
        ExplorerError::io("inspect user settings", Some(path.to_path_buf()), source)
    })?;
    if metadata.len() > MAX_USER_SETTINGS_BYTES as u64 {
        return Err(settings_file_size_error());
    }

    let mut content = String::new();
    let mut reader = file.take(MAX_USER_SETTINGS_BYTES as u64 + 1);
    let bytes_read = reader.read_to_string(&mut content).map_err(|source| {
        ExplorerError::io("read user settings", Some(path.to_path_buf()), source)
    })?;
    if bytes_read > MAX_USER_SETTINGS_BYTES {
        return Err(settings_file_size_error());
    }

    Ok(Some(content))
}

fn settings_file_size_error() -> ExplorerError {
    ExplorerError::invalid_input(format!(
        "사용자 설정 파일이 너무 큽니다. 최대 {MAX_USER_SETTINGS_BYTES}바이트까지 읽을 수 있습니다."
    ))
}

pub fn default_user_settings_path() -> ExplorerResult<PathBuf> {
    let executable_path = std::env::current_exe()
        .map_err(|source| ExplorerError::io("read current executable path", None, source))?;
    let executable_dir = executable_path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .ok_or_else(|| {
            ExplorerError::invalid_input(
                "사용자 설정 위치를 찾을 수 없습니다. 실행 파일 폴더를 확인할 수 없습니다.",
            )
        })?;
    let executable_stem = executable_path
        .file_stem()
        .filter(|stem| !stem.is_empty())
        .ok_or_else(|| {
            ExplorerError::invalid_input(
                "사용자 설정 파일 이름을 만들 수 없습니다. 실행 파일 이름을 확인할 수 없습니다.",
            )
        })?;
    let mut settings_file_name = executable_stem.to_os_string();
    settings_file_name.push(".");
    settings_file_name.push(SETTINGS_FILE_EXTENSION);

    Ok(executable_dir.join(settings_file_name))
}

fn write_settings_file(path: &Path, content: &str) -> ExplorerResult<()> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|source| {
            ExplorerError::io(
                "create temporary user settings",
                Some(path.to_path_buf()),
                source,
            )
        })?;
    file.write_all(content.as_bytes()).map_err(|source| {
        ExplorerError::io(
            "write temporary user settings",
            Some(path.to_path_buf()),
            source,
        )
    })?;
    file.sync_all().map_err(|source| {
        ExplorerError::io(
            "flush temporary user settings",
            Some(path.to_path_buf()),
            source,
        )
    })?;
    Ok(())
}

fn temporary_settings_path(path: &Path) -> ExplorerResult<PathBuf> {
    let file_name = path
        .file_name()
        .ok_or_else(|| ExplorerError::invalid_input("사용자 설정 파일 이름을 만들 수 없습니다."))?;
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let counter = TEMP_FILE_COUNTER.fetch_add(1, Ordering::Relaxed);
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    let mut temporary_name = file_name.to_os_string();
    temporary_name.push(format!(".{}.{}.{}.tmp", std::process::id(), counter, nonce));
    Ok(parent.join(temporary_name))
}

fn serialize_user_settings(settings: &UserSettings) -> ExplorerResult<String> {
    let mut content = String::new();
    content.push_str(SETTINGS_HEADER);
    content.push('\n');
    push_setting_bool(
        &mut content,
        "show_hidden",
        settings.display_options.show_hidden,
    );
    push_setting_bool(
        &mut content,
        "show_system",
        settings.display_options.show_system,
    );
    push_setting_bool(
        &mut content,
        "restore_tabs_on_startup",
        settings.restore_tabs_on_startup,
    );
    push_setting_value(
        &mut content,
        SETTING_APPEARANCE_THEME,
        settings.appearance_theme.storage_value(),
    );
    push_setting_value(
        &mut content,
        SETTING_APPEARANCE_DARK_THEME,
        dark_theme_storage_value(settings.appearance_theme.uses_dark_mode()),
    );
    let font_family = settings
        .appearance_font
        .family_name()
        .map(encode_os)
        .unwrap_or_else(|| NO_VALUE.to_string());
    push_setting_value(&mut content, SETTING_APPEARANCE_FONT_FAMILY, &font_family);
    push_setting_value(
        &mut content,
        SETTING_APPEARANCE_FONT_SIZE,
        &settings.appearance_font.point_size().to_string(),
    );
    if let Some(startup_folder) = &settings.startup_folder {
        let (target_kind, known_kind) = location_storage_kind(startup_folder);
        content.push_str(RECORD_STARTUP_FOLDER);
        content.push('\t');
        content.push_str(target_kind);
        content.push('\t');
        content.push_str(known_kind);
        content.push('\t');
        content.push_str(&encode_os(startup_folder.as_path().as_os_str()));
        content.push('\n');
    }
    if let Some(active_tab_id) = settings.session.active_tab_id {
        content.push_str("active_tab");
        content.push('\t');
        content.push_str(&active_tab_id.0.to_string());
        content.push('\n');
    }

    for tab in &settings.session.tabs {
        push_tab(&mut content, StoredTabList::Open, tab);
    }

    for tab in &settings.session.closed_tabs {
        push_tab(&mut content, StoredTabList::Closed, tab);
    }

    for item in settings.bookmarks.items() {
        let (target_kind, known_kind) = location_storage_kind(&item.target);
        content.push_str("bookmark");
        content.push('\t');
        content.push_str(target_kind);
        content.push('\t');
        content.push_str(known_kind);
        content.push('\t');
        content.push_str(&encode_os(item.target.as_path().as_os_str()));
        content.push('\t');
        content.push_str(&encode_os(item.display_name.as_os_str()));
        content.push('\t');
        content.push_str(&item.sort_order.to_string());
        content.push('\t');
        content.push_str(&encode_system_time(item.created_at)?);
        content.push('\t');
        content.push_str(&encode_optional_system_time(item.last_used_at)?);
        content.push('\t');
        content.push_str(encode_accessibility(item.accessibility));
        content.push('\n');
    }

    Ok(content)
}

fn parse_user_settings(content: &str) -> ExplorerResult<UserSettings> {
    let mut settings = UserSettings::default();
    let mut bookmarks = Vec::new();
    let mut open_tabs = Vec::new();
    let mut closed_tabs = Vec::new();
    let mut active_tab_id = None;
    let mut saw_header = false;
    let mut appearance_theme_seen = false;

    for (line_index, line) in content.lines().enumerate() {
        let line_number = line_index + 1;
        if line.is_empty() {
            continue;
        }

        let parts = line.split('\t').collect::<Vec<_>>();
        match parts.as_slice() {
            ["j3files-settings", "1"] => saw_header = true,
            ["setting", key, value] => apply_setting(
                &mut settings,
                key,
                value,
                line_number,
                &mut appearance_theme_seen,
            )?,
            ["active_tab", tab_id] => {
                active_tab_id = Some(parse_tab_id(tab_id, line_number)?);
            }
            [RECORD_STARTUP_FOLDER, target_kind, known_kind, path] => {
                let path = PathBuf::from(decode_os(path, line_number)?);
                settings.startup_folder = Some(parse_navigation_location(
                    target_kind,
                    known_kind,
                    path,
                    line_number,
                )?);
            }
            ["tab", list, tab_id, target_kind, known_kind, path, sort_key, sort_direction] => {
                let list = parse_stored_tab_list(list, line_number)?;
                let tab = parse_tab(
                    tab_id,
                    target_kind,
                    known_kind,
                    path,
                    sort_key,
                    sort_direction,
                    line_number,
                )?;
                add_tab_builder(
                    tab_builders_mut(list, &mut open_tabs, &mut closed_tabs),
                    tab,
                    line_number,
                )?;
            }
            ["tab_history", list, tab_id, history_kind, target_kind, known_kind, path] => {
                let list = parse_stored_tab_list(list, line_number)?;
                let tab_id = parse_tab_id(tab_id, line_number)?;
                let path = PathBuf::from(decode_os(path, line_number)?);
                let location =
                    parse_navigation_location(target_kind, known_kind, path, line_number)?;
                add_tab_history(
                    tab_builders_mut(list, &mut open_tabs, &mut closed_tabs),
                    tab_id,
                    history_kind,
                    location,
                    line_number,
                )?;
            }
            ["bookmark", target_kind, known_kind, path, display_name, sort_order, created_at, last_used_at, accessibility] => {
                bookmarks.push(parse_bookmark(StoredBookmarkFields {
                    target_kind,
                    known_kind,
                    path,
                    display_name,
                    sort_order,
                    created_at,
                    last_used_at,
                    accessibility,
                    line_number,
                })?)
            }
            _ => {
                return Err(settings_parse_error(
                    line_number,
                    "알 수 없는 설정 항목입니다.",
                ));
            }
        }
    }

    if !saw_header && !content.trim().is_empty() {
        return Err(settings_parse_error(
            1,
            "지원하는 j3Files 설정 파일이 아닙니다.",
        ));
    }

    settings.bookmarks = BookmarkList::from_items(bookmarks);
    settings.session = UserSession {
        tabs: open_tabs
            .into_iter()
            .map(StoredTabBuilder::into_tab)
            .collect(),
        active_tab_id,
        closed_tabs: closed_tabs
            .into_iter()
            .map(StoredTabBuilder::into_tab)
            .collect(),
    };
    Ok(settings)
}

fn push_setting_bool(content: &mut String, key: &str, value: bool) {
    push_setting_value(content, key, if value { "1" } else { "0" });
}

fn push_setting_value(content: &mut String, key: &str, value: &str) {
    content.push_str("setting");
    content.push('\t');
    content.push_str(key);
    content.push('\t');
    content.push_str(value);
    content.push('\n');
}

fn push_tab(content: &mut String, list: StoredTabList, tab: &TabState) {
    let (target_kind, known_kind) = location_storage_kind(tab.current_location());
    content.push_str("tab");
    content.push('\t');
    content.push_str(encode_stored_tab_list(list));
    content.push('\t');
    content.push_str(&tab.id.0.to_string());
    content.push('\t');
    content.push_str(target_kind);
    content.push('\t');
    content.push_str(known_kind);
    content.push('\t');
    content.push_str(&encode_os(tab.current_location().as_path().as_os_str()));
    content.push('\t');
    content.push_str(encode_sort_key(tab.sort.key));
    content.push('\t');
    content.push_str(encode_sort_direction(tab.sort.direction));
    content.push('\n');

    for location in tab.back_history() {
        push_tab_history(content, list, tab.id, "back", location);
    }

    for location in tab.forward_history() {
        push_tab_history(content, list, tab.id, "forward", location);
    }
}

fn push_tab_history(
    content: &mut String,
    list: StoredTabList,
    tab_id: TabId,
    history_kind: &str,
    location: &NavigationLocation,
) {
    let (target_kind, known_kind) = location_storage_kind(location);
    content.push_str("tab_history");
    content.push('\t');
    content.push_str(encode_stored_tab_list(list));
    content.push('\t');
    content.push_str(&tab_id.0.to_string());
    content.push('\t');
    content.push_str(history_kind);
    content.push('\t');
    content.push_str(target_kind);
    content.push('\t');
    content.push_str(known_kind);
    content.push('\t');
    content.push_str(&encode_os(location.as_path().as_os_str()));
    content.push('\n');
}

fn apply_setting(
    settings: &mut UserSettings,
    key: &str,
    value: &str,
    line_number: usize,
    appearance_theme_seen: &mut bool,
) -> ExplorerResult<()> {
    match key {
        "show_hidden" => settings.display_options.show_hidden = parse_bool(value, line_number)?,
        "show_system" => settings.display_options.show_system = parse_bool(value, line_number)?,
        "restore_tabs_on_startup" => {
            settings.restore_tabs_on_startup = parse_bool(value, line_number)?;
        }
        SETTING_APPEARANCE_THEME | SETTING_APPEARANCE_THEME_COMPAT => {
            *appearance_theme_seen = true;
            settings.appearance_theme =
                AppearanceTheme::from_storage_value(value).unwrap_or_default();
        }
        SETTING_APPEARANCE_DARK_THEME | SETTING_APPEARANCE_DARK_THEME_COMPAT => {
            if !*appearance_theme_seen {
                settings.appearance_theme =
                    AppearanceTheme::from_legacy_dark_theme(parse_bool(value, line_number)?);
            }
        }
        SETTING_APPEARANCE_FONT_FAMILY => {
            let family_name = parse_optional_font_family(value, line_number)?;
            let point_size = settings.appearance_font.point_size();
            settings.appearance_font = AppearanceFont::from_storage(family_name, point_size);
        }
        SETTING_APPEARANCE_FONT_SIZE => {
            let family_name = settings.appearance_font.family_name().map(OsString::from);
            settings.appearance_font = match value.trim().parse::<u16>() {
                Ok(point_size) => AppearanceFont::from_storage(family_name, point_size),
                Err(_) => AppearanceFont::default(),
            };
        }
        _ => {
            return Err(settings_parse_error(
                line_number,
                "알 수 없는 설정 이름입니다.",
            ))
        }
    }
    Ok(())
}

struct StoredBookmarkFields<'a> {
    target_kind: &'a str,
    known_kind: &'a str,
    path: &'a str,
    display_name: &'a str,
    sort_order: &'a str,
    created_at: &'a str,
    last_used_at: &'a str,
    accessibility: &'a str,
    line_number: usize,
}

fn parse_bookmark(fields: StoredBookmarkFields<'_>) -> ExplorerResult<BookmarkItem> {
    let path = PathBuf::from(decode_os(fields.path, fields.line_number)?);
    let target = parse_navigation_location(
        fields.target_kind,
        fields.known_kind,
        path,
        fields.line_number,
    )?;
    let display_name = decode_os(fields.display_name, fields.line_number)?;
    let sort_order = parse_u32(
        fields.sort_order,
        fields.line_number,
        "북마크 정렬 순서가 올바르지 않습니다.",
    )?;
    let created_at = parse_system_time(fields.created_at, fields.line_number)?;
    let last_used_at = parse_optional_system_time(fields.last_used_at, fields.line_number)?;
    let accessibility = parse_accessibility(fields.accessibility, fields.line_number)?;

    Ok(BookmarkItem::from_parts(
        target,
        display_name,
        sort_order,
        created_at,
        last_used_at,
        accessibility,
    ))
}

fn parse_tab(
    tab_id: &str,
    target_kind: &str,
    known_kind: &str,
    path: &str,
    sort_key: &str,
    sort_direction: &str,
    line_number: usize,
) -> ExplorerResult<StoredTabBuilder> {
    let id = parse_tab_id(tab_id, line_number)?;
    let path = PathBuf::from(decode_os(path, line_number)?);
    let current_location = parse_navigation_location(target_kind, known_kind, path, line_number)?;
    let sort = SortState {
        key: parse_sort_key(sort_key, line_number)?,
        direction: parse_sort_direction(sort_direction, line_number)?,
    };

    Ok(StoredTabBuilder::new(id, current_location, sort))
}

fn add_tab_builder(
    tabs: &mut Vec<StoredTabBuilder>,
    tab: StoredTabBuilder,
    line_number: usize,
) -> ExplorerResult<()> {
    if tabs.iter().any(|existing| existing.id == tab.id) {
        return Err(settings_parse_error(
            line_number,
            "중복된 탭 식별자가 있습니다.",
        ));
    }
    tabs.push(tab);
    Ok(())
}

fn add_tab_history(
    tabs: &mut [StoredTabBuilder],
    tab_id: TabId,
    history_kind: &str,
    location: NavigationLocation,
    line_number: usize,
) -> ExplorerResult<()> {
    let tab = tabs
        .iter_mut()
        .find(|tab| tab.id == tab_id)
        .ok_or_else(|| settings_parse_error(line_number, "탭 기록의 대상 탭이 없습니다."))?;

    match history_kind {
        "back" => tab.back_history.push(location),
        "forward" => tab.forward_history.push(location),
        _ => {
            return Err(settings_parse_error(
                line_number,
                "탭 기록 종류가 올바르지 않습니다.",
            ))
        }
    }
    Ok(())
}

fn tab_builders_mut<'a>(
    list: StoredTabList,
    open_tabs: &'a mut Vec<StoredTabBuilder>,
    closed_tabs: &'a mut Vec<StoredTabBuilder>,
) -> &'a mut Vec<StoredTabBuilder> {
    match list {
        StoredTabList::Open => open_tabs,
        StoredTabList::Closed => closed_tabs,
    }
}

fn parse_navigation_location(
    target_kind: &str,
    known_kind: &str,
    path: PathBuf,
    line_number: usize,
) -> ExplorerResult<NavigationLocation> {
    match target_kind {
        "local" | "drive" | "network" => {
            if known_kind != NO_VALUE {
                return Err(settings_parse_error(
                    line_number,
                    "일반 경로에 Windows 기본 폴더 종류가 지정되어 있습니다.",
                ));
            }
            NavigationLocation::from_path(path)
        }
        "known" => {
            NavigationLocation::known_folder(parse_known_folder(known_kind, line_number)?, path)
        }
        _ => Err(settings_parse_error(
            line_number,
            "저장된 경로 종류가 올바르지 않습니다.",
        )),
    }
}

fn location_storage_kind(location: &NavigationLocation) -> (&'static str, &'static str) {
    match location {
        NavigationLocation::LocalPath(_) => ("local", NO_VALUE),
        NavigationLocation::DriveRoot(_) => ("drive", NO_VALUE),
        NavigationLocation::NetworkShare(_) => ("network", NO_VALUE),
        NavigationLocation::KnownFolder { kind, .. } => ("known", encode_known_folder(*kind)),
    }
}

fn encode_stored_tab_list(list: StoredTabList) -> &'static str {
    match list {
        StoredTabList::Open => "open",
        StoredTabList::Closed => "closed",
    }
}

fn parse_stored_tab_list(value: &str, line_number: usize) -> ExplorerResult<StoredTabList> {
    match value {
        "open" => Ok(StoredTabList::Open),
        "closed" => Ok(StoredTabList::Closed),
        _ => Err(settings_parse_error(
            line_number,
            "탭 목록 종류가 올바르지 않습니다.",
        )),
    }
}

fn encode_known_folder(kind: KnownFolderKind) -> &'static str {
    match kind {
        KnownFolderKind::Desktop => "desktop",
        KnownFolderKind::Downloads => "downloads",
        KnownFolderKind::Documents => "documents",
        KnownFolderKind::Home => "home",
    }
}

fn parse_known_folder(value: &str, line_number: usize) -> ExplorerResult<KnownFolderKind> {
    match value {
        "desktop" => Ok(KnownFolderKind::Desktop),
        "downloads" => Ok(KnownFolderKind::Downloads),
        "documents" => Ok(KnownFolderKind::Documents),
        "home" => Ok(KnownFolderKind::Home),
        _ => Err(settings_parse_error(
            line_number,
            "Windows 기본 폴더 종류가 올바르지 않습니다.",
        )),
    }
}

fn encode_sort_key(key: SortKey) -> &'static str {
    match key {
        SortKey::Name => "name",
        SortKey::Size => "size",
        SortKey::UpdatedAt => "updated",
        SortKey::Kind => "kind",
    }
}

fn parse_sort_key(value: &str, line_number: usize) -> ExplorerResult<SortKey> {
    match value {
        "name" => Ok(SortKey::Name),
        "size" => Ok(SortKey::Size),
        "modified" => Ok(SortKey::UpdatedAt),
        "updated" => Ok(SortKey::UpdatedAt),
        "kind" => Ok(SortKey::Kind),
        _ => Err(settings_parse_error(
            line_number,
            "탭 정렬 기준이 올바르지 않습니다.",
        )),
    }
}

fn encode_sort_direction(direction: SortDirection) -> &'static str {
    match direction {
        SortDirection::Ascending => "asc",
        SortDirection::Descending => "desc",
    }
}

fn parse_sort_direction(value: &str, line_number: usize) -> ExplorerResult<SortDirection> {
    match value {
        "asc" => Ok(SortDirection::Ascending),
        "desc" => Ok(SortDirection::Descending),
        _ => Err(settings_parse_error(
            line_number,
            "탭 정렬 방향이 올바르지 않습니다.",
        )),
    }
}

fn encode_accessibility(accessibility: BookmarkAccessibility) -> &'static str {
    match accessibility {
        BookmarkAccessibility::Unknown => "unknown",
        BookmarkAccessibility::Accessible => "accessible",
        BookmarkAccessibility::Inaccessible => "inaccessible",
    }
}

fn parse_accessibility(value: &str, line_number: usize) -> ExplorerResult<BookmarkAccessibility> {
    match value {
        "unknown" => Ok(BookmarkAccessibility::Unknown),
        "accessible" => Ok(BookmarkAccessibility::Accessible),
        "inaccessible" => Ok(BookmarkAccessibility::Inaccessible),
        _ => Err(settings_parse_error(
            line_number,
            "북마크 접근 가능 상태가 올바르지 않습니다.",
        )),
    }
}

fn encode_system_time(value: SystemTime) -> ExplorerResult<String> {
    let duration = value.duration_since(UNIX_EPOCH).map_err(|_| {
        ExplorerError::invalid_input("1970년 이전 시각은 사용자 설정에 저장할 수 없습니다.")
    })?;
    Ok(duration.as_secs().to_string())
}

fn encode_optional_system_time(value: Option<SystemTime>) -> ExplorerResult<String> {
    match value {
        Some(value) => encode_system_time(value),
        None => Ok(NO_VALUE.to_string()),
    }
}

fn parse_system_time(value: &str, line_number: usize) -> ExplorerResult<SystemTime> {
    let seconds = parse_u64(
        value,
        line_number,
        "설정 파일의 시각 값이 올바르지 않습니다.",
    )?;
    UNIX_EPOCH
        .checked_add(Duration::from_secs(seconds))
        .ok_or_else(|| settings_parse_error(line_number, "설정 파일의 시각 값이 너무 큽니다."))
}

fn parse_optional_system_time(
    value: &str,
    line_number: usize,
) -> ExplorerResult<Option<SystemTime>> {
    if value == NO_VALUE {
        Ok(None)
    } else {
        parse_system_time(value, line_number).map(Some)
    }
}

fn parse_optional_font_family(value: &str, line_number: usize) -> ExplorerResult<Option<OsString>> {
    if value == NO_VALUE {
        Ok(None)
    } else {
        decode_os(value, line_number).map(Some)
    }
}

fn parse_bool(value: &str, line_number: usize) -> ExplorerResult<bool> {
    match value.trim().to_ascii_lowercase().as_str() {
        "1" | "true" => Ok(true),
        "0" | "false" => Ok(false),
        _ => Err(settings_parse_error(
            line_number,
            "설정 파일의 불리언 값이 올바르지 않습니다.",
        )),
    }
}

fn parse_u32(value: &str, line_number: usize, message: &str) -> ExplorerResult<u32> {
    value
        .parse::<u32>()
        .map_err(|_| settings_parse_error(line_number, message))
}

fn parse_u64(value: &str, line_number: usize, message: &str) -> ExplorerResult<u64> {
    value
        .parse::<u64>()
        .map_err(|_| settings_parse_error(line_number, message))
}

fn parse_tab_id(value: &str, line_number: usize) -> ExplorerResult<TabId> {
    let id = parse_u64(value, line_number, "탭 식별자 값이 올바르지 않습니다.")?;
    if id == 0 {
        return Err(settings_parse_error(
            line_number,
            "탭 식별자 값이 올바르지 않습니다.",
        ));
    }
    Ok(TabId(id))
}

fn encode_os(value: &OsStr) -> String {
    let mut encoded = String::new();
    for unit in value.encode_wide() {
        push_hex_u16(&mut encoded, unit);
    }
    encoded
}

fn decode_os(value: &str, line_number: usize) -> ExplorerResult<OsString> {
    let bytes = value.as_bytes();
    if bytes.len() > MAX_ENCODED_OS_HEX_CHARS {
        return Err(settings_parse_error(
            line_number,
            "UTF-16 hex 문자열이 너무 깁니다.",
        ));
    }
    if !bytes.len().is_multiple_of(4) {
        return Err(settings_parse_error(
            line_number,
            "UTF-16 hex 문자열 길이가 올바르지 않습니다.",
        ));
    }

    let mut units = Vec::with_capacity(bytes.len() / 4);
    for chunk in bytes.chunks(4) {
        let mut unit = 0_u16;
        for byte in chunk {
            unit = (unit << 4) | u16::from(hex_value(*byte, line_number)?);
        }
        units.push(unit);
    }

    Ok(OsString::from_wide(&units))
}

fn push_hex_u16(output: &mut String, value: u16) {
    for shift in [12, 8, 4, 0] {
        let digit = ((value >> shift) & 0x0f) as u8;
        output.push(hex_digit(digit));
    }
}

fn hex_digit(value: u8) -> char {
    let byte = if value < 10 {
        b'0' + value
    } else {
        b'A' + (value - 10)
    };
    char::from(byte)
}

fn hex_value(value: u8, line_number: usize) -> ExplorerResult<u8> {
    match value {
        b'0'..=b'9' => Ok(value - b'0'),
        b'a'..=b'f' => Ok(value - b'a' + 10),
        b'A'..=b'F' => Ok(value - b'A' + 10),
        _ => Err(settings_parse_error(
            line_number,
            "UTF-16 hex 문자열에 잘못된 문자가 있습니다.",
        )),
    }
}

fn settings_parse_error(line_number: usize, message: &str) -> ExplorerError {
    ExplorerError::invalid_input(format!(
        "사용자 설정 파일 {line_number}행을 읽을 수 없습니다. {message}"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{AppearanceFont, AppearanceTheme, DisplayOptions};

    fn location(path: &str) -> ExplorerResult<NavigationLocation> {
        NavigationLocation::from_path(PathBuf::from(path))
    }

    fn temp_settings_path(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "j3files-settings-test-{}-{name}.v1",
            std::process::id()
        ))
    }

    #[test]
    fn default_settings_path_uses_current_executable_directory_and_name() -> ExplorerResult<()> {
        let executable_path = std::env::current_exe()
            .map_err(|source| ExplorerError::io("read test executable path", None, source))?;
        let executable_dir = executable_path
            .parent()
            .ok_or_else(|| ExplorerError::state_conflict("expected test executable directory"))?;
        let executable_stem = executable_path
            .file_stem()
            .ok_or_else(|| ExplorerError::state_conflict("expected test executable name"))?;
        let mut expected_file_name = executable_stem.to_os_string();
        expected_file_name.push(".json");

        let settings_path = default_user_settings_path()?;

        assert_eq!(settings_path.parent(), Some(executable_dir));
        assert_eq!(
            settings_path.file_name(),
            Some(expected_file_name.as_os_str())
        );
        Ok(())
    }

    #[test]
    fn missing_settings_file_loads_defaults() -> ExplorerResult<()> {
        let path = temp_settings_path("missing");
        let _ = fs::remove_file(&path);
        let store = NativeUserSettingsStore::at_path(path);

        let settings = store.load_user_settings()?;

        assert_eq!(settings, UserSettings::default());
        Ok(())
    }

    #[test]
    fn settings_round_trip_preserves_bookmark_paths_and_options() -> ExplorerResult<()> {
        let path = temp_settings_path("round-trip");
        let _ = fs::remove_file(&path);
        let store = NativeUserSettingsStore::at_path(&path);
        let created_at = UNIX_EPOCH + Duration::from_secs(100);
        let last_used_at = UNIX_EPOCH + Duration::from_secs(200);
        let open_tab = TabState::from_parts(
            TabId(7),
            location(r"C:\work\child")?,
            vec![location(r"C:\work")?],
            vec![location(r"C:\work\next")?],
            SortState {
                key: SortKey::UpdatedAt,
                direction: SortDirection::Descending,
            },
        );
        let second_open_tab = TabState::new(TabId(9), location(r"C:\work\second")?);
        let first_closed_tab = TabState::new(TabId(8), location(r"C:\closed")?);
        let second_closed_tab = TabState::new(TabId(10), location(r"\\offline\share")?);
        let settings = UserSettings {
            bookmarks: BookmarkList::from_items(vec![
                BookmarkItem::from_parts(
                    location(r"\\server\share\한글")?,
                    OsString::from("네트워크"),
                    0,
                    created_at,
                    Some(last_used_at),
                    BookmarkAccessibility::Accessible,
                ),
                BookmarkItem::from_parts(
                    NavigationLocation::known_folder(
                        KnownFolderKind::Downloads,
                        PathBuf::from(r"C:\Users\Test\Downloads"),
                    )?,
                    OsString::from("Downloads"),
                    1,
                    created_at,
                    None,
                    BookmarkAccessibility::Unknown,
                ),
            ]),
            display_options: DisplayOptions {
                show_hidden: true,
                show_system: false,
            },
            appearance_theme: AppearanceTheme::Forest,
            appearance_font: AppearanceFont::custom(OsString::from("맑은 고딕"), 13)
                .ok_or_else(|| ExplorerError::state_conflict("expected valid font"))?,
            startup_folder: Some(location(r"C:\startup")?),
            restore_tabs_on_startup: true,
            session: UserSession {
                tabs: vec![open_tab, second_open_tab],
                active_tab_id: Some(TabId(9)),
                closed_tabs: vec![first_closed_tab, second_closed_tab],
            },
        };

        store.save_user_settings(&settings)?;
        let loaded = store.load_user_settings()?;
        let _ = fs::remove_file(&path);

        assert_eq!(loaded, settings);
        assert_eq!(loaded.appearance_theme, AppearanceTheme::Forest);
        assert_eq!(
            loaded.appearance_font.family_name(),
            Some(OsStr::new("맑은 고딕"))
        );
        assert_eq!(loaded.appearance_font.point_size(), 13);
        assert_eq!(
            loaded
                .startup_folder
                .as_ref()
                .map(NavigationLocation::as_path),
            Some(Path::new(r"C:\startup"))
        );
        assert_eq!(loaded.session.active_tab_id, Some(TabId(9)));
        assert_eq!(
            loaded.session.tabs[0].current_location().as_path(),
            Path::new(r"C:\work\child")
        );
        assert_eq!(
            loaded.session.tabs[1].current_location().as_path(),
            Path::new(r"C:\work\second")
        );
        assert_eq!(
            loaded.session.closed_tabs[1].current_location().as_path(),
            Path::new(r"\\offline\share")
        );
        Ok(())
    }

    #[test]
    fn parse_sort_key_accepts_legacy_sort_value() -> ExplorerResult<()> {
        assert_eq!(parse_sort_key("modified", 1)?, SortKey::UpdatedAt);
        assert_eq!(parse_sort_key("updated", 1)?, SortKey::UpdatedAt);
        Ok(())
    }

    #[test]
    fn corrupt_settings_file_loads_defaults_with_warning() -> ExplorerResult<()> {
        let path = temp_settings_path("corrupt");
        fs::write(
            &path,
            format!("{SETTINGS_HEADER}\nsetting\tshow_hidden\tmaybe\n"),
        )
        .map_err(|source| {
            ExplorerError::io("write corrupt settings fixture", Some(path.clone()), source)
        })?;
        let store = NativeUserSettingsStore::at_path(&path);

        let outcome = store.load_user_settings_with_recovery();
        let _ = fs::remove_file(&path);

        assert_eq!(outcome.settings, UserSettings::default());
        assert!(!outcome.save_allowed);
        let warning = outcome
            .warning
            .ok_or_else(|| ExplorerError::state_conflict("expected settings warning"))?;
        assert!(warning.user_message().contains("사용자 설정 파일 2행"));
        Ok(())
    }

    #[test]
    fn oversized_settings_file_loads_defaults_with_warning() -> ExplorerResult<()> {
        let path = temp_settings_path("oversized");
        let _ = fs::remove_file(&path);
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .map_err(|source| {
                ExplorerError::io(
                    "create oversized settings fixture",
                    Some(path.clone()),
                    source,
                )
            })?;
        file.set_len(MAX_USER_SETTINGS_BYTES as u64 + 1)
            .map_err(|source| {
                ExplorerError::io(
                    "size oversized settings fixture",
                    Some(path.clone()),
                    source,
                )
            })?;
        drop(file);
        let store = NativeUserSettingsStore::at_path(&path);

        let outcome = store.load_user_settings_with_recovery();
        let _ = fs::remove_file(&path);

        assert_eq!(outcome.settings, UserSettings::default());
        assert!(!outcome.save_allowed);
        let warning = outcome
            .warning
            .ok_or_else(|| ExplorerError::state_conflict("expected settings warning"))?;
        assert!(warning
            .user_message()
            .contains("사용자 설정 파일이 너무 큽니다."));
        Ok(())
    }

    #[test]
    fn oversized_settings_are_rejected_before_writing() -> ExplorerResult<()> {
        let path = temp_settings_path("oversized-save");
        let _ = fs::remove_file(&path);
        let store = NativeUserSettingsStore::at_path(&path);
        let oversized_name = "x".repeat((MAX_USER_SETTINGS_BYTES / 4) + 1);
        let settings = UserSettings {
            bookmarks: BookmarkList::from_items(vec![BookmarkItem::from_parts(
                location(r"C:\large")?,
                OsString::from(oversized_name),
                0,
                UNIX_EPOCH,
                None,
                BookmarkAccessibility::Unknown,
            )]),
            ..UserSettings::default()
        };
        assert!(serialize_user_settings(&settings)?.len() > MAX_USER_SETTINGS_BYTES);

        let error = match store.save_user_settings(&settings) {
            Ok(()) => {
                return Err(ExplorerError::state_conflict(
                    "expected oversized settings to be rejected",
                ));
            }
            Err(error) => error,
        };

        let _ = fs::remove_file(&path);
        assert!(error
            .user_message()
            .contains("사용자 설정 파일이 너무 큽니다."));
        assert!(!path.exists());
        Ok(())
    }

    #[test]
    fn settings_rejects_tab_path_with_embedded_nul() -> ExplorerResult<()> {
        let path = "0043003A005C00730061006600650000007400610069006C";
        let content = format!("{SETTINGS_HEADER}\ntab\topen\t1\tlocal\t-\t{path}\tname\tasc\n");

        assert!(parse_user_settings(&content).is_err());
        Ok(())
    }

    #[test]
    fn decode_os_rejects_oversized_hex_string() -> ExplorerResult<()> {
        let value = "0".repeat(MAX_ENCODED_OS_HEX_CHARS + 4);

        let error = match decode_os(&value, 7) {
            Ok(_) => {
                return Err(ExplorerError::state_conflict(
                    "expected oversized hex string to be rejected",
                ));
            }
            Err(error) => error,
        };

        assert!(error
            .user_message()
            .contains("UTF-16 hex 문자열이 너무 깁니다."));
        Ok(())
    }

    #[test]
    fn save_replaces_existing_settings_file() -> ExplorerResult<()> {
        let path = temp_settings_path("replace");
        let _ = fs::remove_file(&path);
        let store = NativeUserSettingsStore::at_path(&path);
        let mut settings = UserSettings {
            display_options: DisplayOptions {
                show_hidden: true,
                show_system: false,
            },
            ..UserSettings::default()
        };
        store.save_user_settings(&settings)?;

        settings.display_options.show_hidden = false;
        settings.display_options.show_system = true;
        settings.restore_tabs_on_startup = true;
        store.save_user_settings(&settings)?;
        let loaded = store.load_user_settings()?;
        let _ = fs::remove_file(&path);

        assert_eq!(loaded.display_options, settings.display_options);
        assert!(loaded.restore_tabs_on_startup);
        Ok(())
    }

    #[test]
    fn appearance_theme_storage_falls_back_to_light_when_invalid() -> ExplorerResult<()> {
        let content = format!("{SETTINGS_HEADER}\nsetting\tappearance_theme\tunknown\n");

        let loaded = parse_user_settings(&content)?;

        assert_eq!(loaded.appearance_theme, AppearanceTheme::Light);
        Ok(())
    }

    #[test]
    fn appearance_font_storage_falls_back_to_default_when_invalid() -> ExplorerResult<()> {
        let name = encode_os(OsStr::new("Segoe UI"));
        let content =
            format!("{SETTINGS_HEADER}\nsetting\tappearance_font_family\t{name}\nsetting\tappearance_font_size\t200\n");

        let loaded = parse_user_settings(&content)?;

        assert_eq!(loaded.appearance_font, AppearanceFont::default());
        Ok(())
    }

    #[test]
    fn appearance_font_size_can_be_loaded_without_custom_family() -> ExplorerResult<()> {
        let content = format!("{SETTINGS_HEADER}\nsetting\tappearance_font_size\t14\n");

        let loaded = parse_user_settings(&content)?;

        assert_eq!(loaded.appearance_font.family_name(), None);
        assert_eq!(loaded.appearance_font.point_size(), 14);
        assert!(loaded.appearance_font.is_custom());
        Ok(())
    }

    #[test]
    fn legacy_dark_theme_storage_is_read_when_theme_is_missing() -> ExplorerResult<()> {
        let content = format!("{SETTINGS_HEADER}\nsetting\tdark_theme\t1\n");

        let loaded = parse_user_settings(&content)?;

        assert_eq!(loaded.appearance_theme, AppearanceTheme::ClassicDark);
        Ok(())
    }

    #[test]
    fn parsed_bookmarks_are_deduplicated_by_target_path() -> ExplorerResult<()> {
        let path = encode_os(std::ffi::OsStr::new(r"C:\one"));
        let first_name = encode_os(std::ffi::OsStr::new("One"));
        let duplicate_name = encode_os(std::ffi::OsStr::new("Duplicate"));
        let content = format!(
            "{SETTINGS_HEADER}\nbookmark\tlocal\t-\t{path}\t{first_name}\t0\t0\t-\tunknown\nbookmark\tlocal\t-\t{path}\t{duplicate_name}\t1\t0\t-\tunknown\n"
        );

        let loaded = parse_user_settings(&content)?;

        assert_eq!(loaded.bookmarks.items().len(), 1);
        assert_eq!(
            loaded.bookmarks.items()[0].target.as_path(),
            Path::new(r"C:\one")
        );
        Ok(())
    }
}
