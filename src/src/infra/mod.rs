mod file_system;
mod icon;
mod settings;
mod shell;
mod startup;

pub use file_system::NativeFileSystemGateway;
pub use icon::{ShellFileIcon, ShellIconCache, ShellIconLoadCompletion, ShellIconLoadTask};
pub use settings::{default_user_settings_path, NativeUserSettingsStore, UserSettingsLoadOutcome};
pub use shell::WindowsShellGateway;
pub use startup::{
    default_start_locations, startup_plan_from_args, startup_plan_from_configured_folder,
    startup_plan_from_selected_folder, StartupPlan,
};
