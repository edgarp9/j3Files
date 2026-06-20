use std::cell::Cell;
use std::path::PathBuf;
use std::rc::Rc;

use crate::app::{
    ContextMenuOutcome, ContextMenuPosition, ShellContextMenuGateway, ShellDeleteGateway,
    ShellFileOperationGateway, ShellGateway, ShellOpenGateway, ShellOpenWithGateway,
    ShellPropertiesGateway, ShellRenameGateway, ShellTransferGateway,
};
use crate::domain::{ExplorerResult, NavigationLocation, RenameItemName};
use crate::platform;

#[derive(Debug, Default, Clone)]
pub struct WindowsShellGateway {
    owner_window: Rc<Cell<isize>>,
}

impl WindowsShellGateway {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set_owner_window(&self, owner_window: isize) {
        self.owner_window.set(owner_window);
    }
}

impl ShellOpenGateway for WindowsShellGateway {
    fn open_path(&self, location: &NavigationLocation) -> ExplorerResult<()> {
        platform::shell_open_path_with_owner(self.owner_window.get(), location.as_path())
    }
}

impl ShellOpenWithGateway for WindowsShellGateway {
    fn open_with(&self, location: &NavigationLocation) -> ExplorerResult<()> {
        platform::shell_open_with_owner(self.owner_window.get(), location.as_path())
    }
}

impl ShellPropertiesGateway for WindowsShellGateway {
    fn show_properties(&self, location: &NavigationLocation) -> ExplorerResult<()> {
        platform::shell_show_properties_with_owner(self.owner_window.get(), location.as_path())
    }
}

impl ShellContextMenuGateway for WindowsShellGateway {
    fn show_context_menu(
        &self,
        targets: &[NavigationLocation],
        position: ContextMenuPosition,
    ) -> ExplorerResult<ContextMenuOutcome> {
        let outcome = platform::shell_show_context_menu(
            self.owner_window.get(),
            &paths_from_locations(targets),
            platform::ShellContextMenuPoint {
                x: position.x,
                y: position.y,
            },
        )?;
        Ok(ContextMenuOutcome {
            command_invoked: outcome.command_invoked,
            refresh_current_folder: outcome.refresh_current_folder,
        })
    }

    fn show_folder_background_context_menu(
        &self,
        folder: &NavigationLocation,
        position: ContextMenuPosition,
    ) -> ExplorerResult<ContextMenuOutcome> {
        let outcome = platform::shell_show_folder_background_context_menu(
            self.owner_window.get(),
            folder.as_path(),
            platform::ShellContextMenuPoint {
                x: position.x,
                y: position.y,
            },
        )?;
        Ok(ContextMenuOutcome {
            command_invoked: outcome.command_invoked,
            refresh_current_folder: outcome.refresh_current_folder,
        })
    }
}

impl ShellTransferGateway for WindowsShellGateway {
    fn copy_items(
        &self,
        sources: &[NavigationLocation],
        destination: &NavigationLocation,
    ) -> ExplorerResult<()> {
        platform::shell_copy_items_with_owner(
            self.owner_window.get(),
            paths_from_locations(sources),
            destination.as_path(),
        )
    }

    fn move_items(
        &self,
        sources: &[NavigationLocation],
        destination: &NavigationLocation,
    ) -> ExplorerResult<()> {
        platform::shell_move_items_with_owner(
            self.owner_window.get(),
            paths_from_locations(sources),
            destination.as_path(),
        )
    }
}

impl ShellDeleteGateway for WindowsShellGateway {
    fn delete_to_recycle_bin(&self, targets: &[NavigationLocation]) -> ExplorerResult<()> {
        platform::shell_delete_to_recycle_bin_with_owner(
            self.owner_window.get(),
            paths_from_locations(targets),
        )
    }

    fn delete_permanently(&self, targets: &[NavigationLocation]) -> ExplorerResult<()> {
        platform::shell_delete_permanently_with_owner(
            self.owner_window.get(),
            paths_from_locations(targets),
        )
    }
}

impl ShellRenameGateway for WindowsShellGateway {
    fn rename_item(
        &self,
        target: &NavigationLocation,
        new_name: &RenameItemName,
    ) -> ExplorerResult<()> {
        platform::shell_rename_item_with_owner(
            self.owner_window.get(),
            target.as_path(),
            new_name.as_os_str(),
        )
    }
}

impl ShellFileOperationGateway for WindowsShellGateway {}

impl ShellGateway for WindowsShellGateway {}

fn paths_from_locations(locations: &[NavigationLocation]) -> Vec<PathBuf> {
    locations
        .iter()
        .map(|location| location.as_path().to_path_buf())
        .collect()
}
