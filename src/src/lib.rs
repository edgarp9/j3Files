#[cfg(not(windows))]
compile_error!("j3Files is a Windows-only file explorer application.");

#[cfg(windows)]
pub mod app;
#[cfg(windows)]
pub mod domain;
#[cfg(windows)]
pub mod infra;
#[cfg(windows)]
pub mod platform;
