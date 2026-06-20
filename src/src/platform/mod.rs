pub mod win32_ui;

mod clipboard;
mod hdrop;
mod ole_drag_drop;
mod shell_context_menu;
mod shell_execute;
mod shell_icon;
mod shell_operation;
mod win32;

pub use clipboard::{
    clipboard_file_items, set_clipboard_file_items, ClipboardFileItems, ClipboardFileOperation,
};
pub use ole_drag_drop::{
    register_file_drop_target, start_internal_file_drag, start_shell_file_drag,
    validate_shell_file_drag_paths, OleDragSourceOutcome, OleDropData, OleDropEffectHint,
    OleDropEffects, OleDropEvent, OleDropEventQueue, OleDropFeedback, OleDropFeedbackTimerConfig,
    OleDropKeyState, OleDropPreferredEffect, OleDropTargetKind, OleDropTargetRegistration,
};
pub use shell_context_menu::{
    shell_show_context_menu, shell_show_folder_background_context_menu, ShellContextMenuOutcome,
    ShellContextMenuPoint,
};
pub use shell_execute::{
    shell_execute, shell_execute_with_owner, shell_open_path, shell_open_path_with_owner,
    shell_open_with, shell_open_with_owner, shell_show_properties,
    shell_show_properties_with_owner,
};
pub use shell_icon::{
    shell_file_icon, ShellIconIndex, ShellIconLookup, ShellIconQuery, ShellImageListHandle,
};
pub use shell_operation::{
    shell_copy_items, shell_copy_items_with_owner, shell_delete_permanently,
    shell_delete_permanently_with_owner, shell_delete_to_recycle_bin,
    shell_delete_to_recycle_bin_with_owner, shell_move_items, shell_move_items_with_owner,
    shell_rename_item, shell_rename_item_with_owner,
};
pub use win32::{
    create_directory, directory_entries, directory_entry, ensure_directory_listable,
    file_attributes, known_folder_path, logical_drive_roots, replace_file, visit_directory_entries,
    visit_directory_entries_until, watch_directory_changes, DirectoryChange, DirectoryChangeBatch,
    DirectoryChangeCancellation, DirectoryChangeKind, DirectoryVisit, SynchronousIoCancellation,
    Win32DirectoryEntry, Win32FileAttributes, Win32KnownFolder,
};
