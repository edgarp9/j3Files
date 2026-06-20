use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MainWindowCommand {
    Back,
    Forward,
    Up,
    Refresh,
    AddressGo,
    NewFolder,
    Open,
    OpenWith,
    Properties,
    Copy,
    Cut,
    Paste,
    Undo,
    DeleteToRecycleBin,
    DeletePermanently,
    Rename,
    SelectAll,
    NewTab,
    CloseTab,
    NextTab,
    ReopenTab,
    MoveTabLeft,
    MoveTabRight,
    OpenSelectedFolderInNewTab,
    ToggleRestoreTabsOnStartup,
    SetCurrentFolderAsStartupFolder,
    ClearStartupFolder,
    AddCurrentBookmark,
    AddSelectedFolderBookmark,
    RemoveCurrentBookmark,
    ToggleShowHidden,
    ToggleShowSystem,
    SortBy(SortKey),
    SortDirection(SortDirection),
    Theme(AppearanceTheme),
    ChooseFont,
    ResetFont,
    SearchFind,
    SearchFocus,
    SearchSubfolders,
    SearchSubfoldersCheckboxChanged,
    SearchCancel,
    SearchClose,
    About,
    KnownFolder(KnownFolderKind),
    DriveMenu(u16),
    BookmarkMenu(u16),
    AddressKillFocus,
    AddressFocus,
    Exit,
    Ignore,
}

struct MainWindowCommandRouter;

impl MainWindowCommandRouter {
    fn route(id: u16, notification: u16) -> MainWindowCommand {
        match id {
            ID_NAV_BACK => MainWindowCommand::Back,
            ID_NAV_FORWARD => MainWindowCommand::Forward,
            ID_NAV_UP => MainWindowCommand::Up,
            ID_REFRESH => MainWindowCommand::Refresh,
            ID_GO => MainWindowCommand::AddressGo,
            ID_FILE_NEW_FOLDER => MainWindowCommand::NewFolder,
            ID_FILE_OPEN => MainWindowCommand::Open,
            ID_FILE_OPEN_WITH => MainWindowCommand::OpenWith,
            ID_FILE_PROPERTIES => MainWindowCommand::Properties,
            ID_FILE_COPY => MainWindowCommand::Copy,
            ID_FILE_CUT => MainWindowCommand::Cut,
            ID_FILE_PASTE => MainWindowCommand::Paste,
            ID_FILE_UNDO => MainWindowCommand::Undo,
            ID_FILE_DELETE => MainWindowCommand::DeleteToRecycleBin,
            ID_FILE_DELETE_PERMANENTLY => MainWindowCommand::DeletePermanently,
            ID_FILE_RENAME => MainWindowCommand::Rename,
            ID_FILE_SELECT_ALL => MainWindowCommand::SelectAll,
            ID_TAB_NEW => MainWindowCommand::NewTab,
            ID_TAB_CLOSE => MainWindowCommand::CloseTab,
            ID_TAB_NEXT => MainWindowCommand::NextTab,
            ID_TAB_REOPEN => MainWindowCommand::ReopenTab,
            ID_TAB_MOVE_LEFT => MainWindowCommand::MoveTabLeft,
            ID_TAB_MOVE_RIGHT => MainWindowCommand::MoveTabRight,
            ID_TAB_OPEN_SELECTED_FOLDER => MainWindowCommand::OpenSelectedFolderInNewTab,
            ID_TAB_RESTORE_ON_STARTUP => MainWindowCommand::ToggleRestoreTabsOnStartup,
            ID_TAB_SET_STARTUP_FOLDER => MainWindowCommand::SetCurrentFolderAsStartupFolder,
            ID_TAB_CLEAR_STARTUP_FOLDER => MainWindowCommand::ClearStartupFolder,
            ID_BOOKMARK_ADD_CURRENT => MainWindowCommand::AddCurrentBookmark,
            ID_BOOKMARK_ADD_SELECTED_FOLDER => MainWindowCommand::AddSelectedFolderBookmark,
            ID_BOOKMARK_REMOVE_CURRENT => MainWindowCommand::RemoveCurrentBookmark,
            ID_VIEW_SHOW_HIDDEN => MainWindowCommand::ToggleShowHidden,
            ID_VIEW_SHOW_SYSTEM => MainWindowCommand::ToggleShowSystem,
            ID_SORT_NAME => MainWindowCommand::SortBy(SortKey::Name),
            ID_SORT_SIZE => MainWindowCommand::SortBy(SortKey::Size),
            ID_SORT_UPDATED => MainWindowCommand::SortBy(SortKey::UpdatedAt),
            ID_SORT_KIND => MainWindowCommand::SortBy(SortKey::Kind),
            ID_SORT_ASCENDING => MainWindowCommand::SortDirection(SortDirection::Ascending),
            ID_SORT_DESCENDING => MainWindowCommand::SortDirection(SortDirection::Descending),
            ID_VIEW_FONT => MainWindowCommand::ChooseFont,
            ID_VIEW_FONT_RESET => MainWindowCommand::ResetFont,
            ID_SEARCH_FIND => MainWindowCommand::SearchFind,
            ID_SEARCH_FOCUS => MainWindowCommand::SearchFocus,
            ID_SEARCH_SUBFOLDERS => MainWindowCommand::SearchSubfoldersCheckboxChanged,
            ID_SEARCH_INCLUDE_SUBFOLDERS => MainWindowCommand::SearchSubfolders,
            ID_SEARCH_CANCEL => MainWindowCommand::SearchCancel,
            ID_SEARCH_CLOSE => MainWindowCommand::SearchClose,
            ID_ABOUT => MainWindowCommand::About,
            ID_KNOWN_HOME => MainWindowCommand::KnownFolder(KnownFolderKind::Home),
            ID_KNOWN_DESKTOP => MainWindowCommand::KnownFolder(KnownFolderKind::Desktop),
            ID_KNOWN_DOWNLOADS => MainWindowCommand::KnownFolder(KnownFolderKind::Downloads),
            ID_KNOWN_DOCUMENTS => MainWindowCommand::KnownFolder(KnownFolderKind::Documents),
            ID_ADDRESS if notification == ui::EDIT_KILL_FOCUS => {
                MainWindowCommand::AddressKillFocus
            }
            ID_ADDRESS_FOCUS => MainWindowCommand::AddressFocus,
            ID_EXIT => MainWindowCommand::Exit,
            id => {
                if let Some(theme) = appearance_theme_for_command(id) {
                    MainWindowCommand::Theme(theme)
                } else if is_drive_menu_id(id) {
                    MainWindowCommand::DriveMenu(id)
                } else if is_bookmark_menu_id(id) {
                    MainWindowCommand::BookmarkMenu(id)
                } else {
                    MainWindowCommand::Ignore
                }
            }
        }
    }
}

impl MainWindowCommand {
    fn execute(self, window: &mut MainWindow) -> ExplorerResult<()> {
        match self {
            Self::Back => window.go_back(),
            Self::Forward => window.go_forward(),
            Self::Up => window.go_up(),
            Self::Refresh => window.refresh_active_view(),
            Self::AddressGo | Self::AddressKillFocus => window.navigate_to_address_if_changed(),
            Self::NewFolder => window.create_new_folder(),
            Self::Open => window.activate_selected_item(),
            Self::OpenWith => window.open_selected_item_with_picker(),
            Self::Properties => window.show_selected_item_properties(),
            Self::Copy => window.copy_selected_items_to_clipboard(),
            Self::Cut => window.cut_selected_items_to_clipboard(),
            Self::Paste => window.paste_clipboard_items(),
            Self::Undo => window.undo_last_file_operation(),
            Self::DeleteToRecycleBin => window.delete_selected_items_to_recycle_bin(),
            Self::DeletePermanently => window.delete_selected_items_permanently(),
            Self::Rename => window.begin_rename_selected_item(),
            Self::SelectAll => window.select_all_items(),
            Self::NewTab => window.open_new_tab_from_active(),
            Self::CloseTab => window.close_active_tab(),
            Self::NextTab => window.switch_to_next_tab(),
            Self::ReopenTab => window.reopen_closed_tab(),
            Self::MoveTabLeft => window.move_active_tab_left(),
            Self::MoveTabRight => window.move_active_tab_right(),
            Self::OpenSelectedFolderInNewTab => window.open_selected_folder_in_new_tab(),
            Self::ToggleRestoreTabsOnStartup => window.toggle_restore_tabs_on_startup(),
            Self::SetCurrentFolderAsStartupFolder => window.set_current_folder_as_startup_folder(),
            Self::ClearStartupFolder => window.clear_startup_folder(),
            Self::AddCurrentBookmark => window.add_current_location_bookmark(),
            Self::AddSelectedFolderBookmark => window.add_selected_folder_bookmark(),
            Self::RemoveCurrentBookmark => window.remove_current_location_bookmark(),
            Self::ToggleShowHidden => window.toggle_show_hidden_files(),
            Self::ToggleShowSystem => window.toggle_show_system_files(),
            Self::SortBy(key) => window.set_active_sort_key(key),
            Self::SortDirection(direction) => window.set_active_sort_direction(direction),
            Self::Theme(theme) => window.set_appearance_theme(theme),
            Self::ChooseFont => window.choose_appearance_font(),
            Self::ResetFont => window.reset_appearance_font(),
            Self::SearchFind => window.show_or_start_search(),
            Self::SearchFocus => window.show_search_controls(),
            Self::SearchSubfolders => window.toggle_search_subfolders(),
            Self::SearchSubfoldersCheckboxChanged => window.sync_search_subfolders_menu(),
            Self::SearchCancel => window.cancel_active_search(),
            Self::SearchClose => window.close_search_controls(),
            Self::About => window.show_about_dialog(),
            Self::KnownFolder(kind) => window.navigate_to_known_folder(kind),
            Self::DriveMenu(id) => window.navigate_to_drive_menu_item(id),
            Self::BookmarkMenu(id) => window.navigate_to_bookmark_menu_item(id),
            Self::AddressFocus => window.focus_address_bar(),
            Self::Exit => window.request_shutdown(),
            Self::Ignore => Ok(()),
        }
    }
}

impl MainWindow {
    pub(super) fn on_command(&mut self, id: u16, notification: u16) {
        let command = MainWindowCommandRouter::route(id, notification);
        if let Err(error) = command.execute(self) {
            self.recover_after_error(&error);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn routes_dynamic_drive_and_bookmark_commands() {
        assert_eq!(
            MainWindowCommandRouter::route(ID_DRIVE_BASE, 0),
            MainWindowCommand::DriveMenu(ID_DRIVE_BASE)
        );
        assert_eq!(
            MainWindowCommandRouter::route(ID_BOOKMARK_BASE, 0),
            MainWindowCommand::BookmarkMenu(ID_BOOKMARK_BASE)
        );
    }

    #[test]
    fn address_edit_only_routes_on_kill_focus() {
        assert_eq!(
            MainWindowCommandRouter::route(ID_ADDRESS, ui::EDIT_KILL_FOCUS),
            MainWindowCommand::AddressKillFocus
        );
        assert_eq!(
            MainWindowCommandRouter::route(ID_ADDRESS, 0),
            MainWindowCommand::Ignore
        );
    }

    #[test]
    fn search_subfolders_checkbox_and_menu_have_separate_routes() {
        assert_eq!(
            MainWindowCommandRouter::route(ID_SEARCH_SUBFOLDERS, 0),
            MainWindowCommand::SearchSubfoldersCheckboxChanged
        );
        assert_eq!(
            MainWindowCommandRouter::route(ID_SEARCH_INCLUDE_SUBFOLDERS, 0),
            MainWindowCommand::SearchSubfolders
        );
    }

    #[test]
    fn routes_about_command() {
        assert_eq!(
            MainWindowCommandRouter::route(ID_ABOUT, 0),
            MainWindowCommand::About
        );
    }
}
