# j3Files
Windows-only file explorer built with Rust and the Win32 Shell, focused on native file operations, tabs, bookmarks, search, and drag-and-drop.

## Features

- Native Win32 desktop UI built with Rust and `windows-sys`.
- Folder navigation with an address bar, history, toolbar actions, and a folder tree.
- Tabbed browsing with per-tab navigation history.
- Bookmarks, quick access locations, drives, known folders, and UNC network paths.
- Detailed file list with sorting, file type labels, sizes, modified times, and shell icons.
- Shell-backed copy, move, recycle-bin delete, rename, open, open with, properties, and context menu flows.
- Explorer/Desktop-compatible drag and drop using Windows Shell data formats.
- Filename search with optional subfolder search and cancellation.
- User settings for tabs, bookmarks, hidden/system file visibility, theme, font, and startup folder.
- Portable settings stored next to the executable.

## Project Status

This project was created with AI assistance through an in-house tool.

The current test coverage is not sufficient yet. Some domain rules and file-operation
paths are tested, but Windows Shell integration still requires more automated coverage
and manual verification, especially for drag and drop, permissions, UAC elevation,
network paths, long paths, locked files, and Shell conflict dialogs.

Treat j3Files as early-stage software and verify important file operations carefully.

## Requirements

- Windows
- Rust stable toolchain
- Windows build tools capable of compiling Win32 resources

## Build

From the repository root:

```powershell
cd src
cargo build
```

Run a debug build:

```powershell
cd src
cargo run
```

Run tests:

```powershell
cd src
cargo test
```

Build a release binary:

```powershell
cd src
python build_release.py --no-open
```

The release executable is written under Cargo's release target directory.

## Repository Layout

- `src/src/domain` - core file explorer types, validation, sorting, search, and pure rules.
- `src/src/app` - application use cases and state coordination.
- `src/src/infra` - file system, settings, startup, and shell gateway code.
- `src/src/platform` - Win32, Shell, OLE drag-and-drop, clipboard, and UI wrappers.
- `src/docs` - domain notes, release notes, and verification notes.

## License

This project is licensed under the GNU General Public License v3.0. See
[`LICENSE`](LICENSE) for details.

## Icon Notice and Thanks

Toolbar and search icons use Google Fonts Material Symbols Outlined assets from
[Google Fonts Icons](https://fonts.google.com/icons). Google makes Material icons
available under the [Apache License Version 2.0](https://www.apache.org/licenses/LICENSE-2.0).

Thank you to Google and the Material Symbols contributors for making these icons
available.
