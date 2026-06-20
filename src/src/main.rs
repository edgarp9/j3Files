#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

#[cfg(not(windows))]
compile_error!("j3Files is a Windows-only file explorer application.");

#[cfg(windows)]
mod windows_main;

#[cfg(windows)]
fn main() {
    windows_main::main();
}

#[cfg(not(windows))]
fn main() {}
