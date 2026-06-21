#!/usr/bin/env python3
"""Build the release binary and open the output directory."""

from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import platform
import shutil
import subprocess
import sys
from typing import Iterable
import zipfile


DEFAULT_LINUX_WINDOWS_TARGET = "x86_64-pc-windows-gnu"
THIRD_PARTY_NOTICES_FILE = "THIRD_PARTY_NOTICES.txt"
ABOUT_FILE = "about.txt"
LICENSE_DOCUMENT_FILES = ("LICENSE", "README.md", THIRD_PARTY_NOTICES_FILE, ABOUT_FILE)
LEGACY_LICENSE_DOCUMENT_FILES = ("THIRD_PARTY_NOTICES.md",)
DIST_DIR_NAME = "dist"
SOURCE_PACKAGE_EXCLUDED_DIRS = {
    ".git",
    ".my",
    ".idea",
    ".vscode",
    "__pycache__",
    "target",
    DIST_DIR_NAME,
    "coverage",
    "criterion",
}
SOURCE_PACKAGE_EXCLUDED_FILE_NAMES = {
    ".DS_Store",
    "Thumbs.db",
    "Desktop.ini",
    "tarpaulin-report.html",
    "cargo-tarpaulin-report.xml",
    "flamegraph.svg",
}
SOURCE_PACKAGE_EXCLUDED_SUFFIXES = (
    ".rlib",
    ".rmeta",
    ".profraw",
    ".profdata",
    ".pdb",
    ".ilk",
    ".pyc",
    ".log",
    ".tmp",
    ".bak",
    ".swp",
    ".swo",
)


def parse_args(argv: list[str]) -> tuple[argparse.Namespace, list[str]]:
    parser = argparse.ArgumentParser(
        description="Build j3Files in release mode and open the binary directory.",
    )
    parser.add_argument(
        "--target",
        default=os.environ.get("J3FILES_RELEASE_TARGET"),
        help=(
            "Cargo target triple. Defaults to the host target on Windows and "
            f"{DEFAULT_LINUX_WINDOWS_TARGET} on Linux."
        ),
    )
    parser.add_argument(
        "--no-open",
        action="store_true",
        help="Build only; do not open the binary directory.",
    )

    args, cargo_args = parser.parse_known_args(argv)
    if cargo_args[:1] == ["--"]:
        cargo_args = cargo_args[1:]
    return args, cargo_args


def project_root() -> Path:
    return Path(__file__).resolve().parent


def package_version(root: Path) -> str:
    try:
        import tomllib
    except ModuleNotFoundError:
        return "0.0.0"

    try:
        with (root / "Cargo.toml").open("rb") as cargo_toml:
            metadata = tomllib.load(cargo_toml)
    except (OSError, tomllib.TOMLDecodeError):
        return "0.0.0"

    package = metadata.get("package")
    if not isinstance(package, dict):
        return "0.0.0"

    version = package.get("version")
    if not isinstance(version, str) or not version:
        return "0.0.0"
    return version


def selected_target(requested_target: str | None) -> str | None:
    if requested_target:
        return requested_target
    if platform.system() == "Windows":
        return None
    return DEFAULT_LINUX_WINDOWS_TARGET


def host_target(root: Path) -> str | None:
    rustc = shutil.which("rustc")
    if rustc is None:
        return None

    result = subprocess.run(
        [rustc, "-vV"],
        cwd=root,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )
    if result.returncode != 0:
        return None

    for line in result.stdout.splitlines():
        if line.startswith("host:"):
            value = line.removeprefix("host:").strip()
            if value:
                return value
    return None


def target_dir_from_cargo_metadata(root: Path) -> Path:
    fallback = root / "target"
    cargo = shutil.which("cargo")
    if cargo is None:
        return fallback

    result = subprocess.run(
        [cargo, "metadata", "--format-version", "1", "--no-deps"],
        cwd=root,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )
    if result.returncode != 0:
        return fallback

    try:
        metadata = json.loads(result.stdout)
    except json.JSONDecodeError:
        return fallback

    value = metadata.get("target_directory")
    if not isinstance(value, str) or not value:
        return fallback
    return Path(value)


def option_value(args: Iterable[str], option_name: str) -> str | None:
    args_list = list(args)
    prefix = f"{option_name}="
    for index, item in enumerate(args_list):
        if item.startswith(prefix):
            return item[len(prefix) :]
        if item == option_name and index + 1 < len(args_list):
            return args_list[index + 1]
    return None


def release_binary_dir(root: Path, target: str | None, cargo_args: list[str]) -> Path:
    explicit_target_dir = option_value(cargo_args, "--target-dir")
    if explicit_target_dir:
        target_dir = Path(explicit_target_dir)
        if not target_dir.is_absolute():
            target_dir = root / target_dir
    else:
        target_dir = target_dir_from_cargo_metadata(root)

    if target:
        return target_dir / target / "release"
    return target_dir / "release"


def command_text(command: Iterable[str]) -> str:
    return " ".join(command)


def run_release_build(root: Path, target: str | None, cargo_args: list[str]) -> int:
    cargo = shutil.which("cargo")
    if cargo is None:
        print("error: cargo was not found in PATH.", file=sys.stderr)
        return 127

    command = [cargo, "build", "--release"]
    if target:
        command.extend(["--target", target])
    command.extend(cargo_args)

    print(f"Running: {command_text(command)}")
    result = subprocess.run(command, cwd=root)
    return result.returncode


def copy_license_documents(root: Path, binary_dir: Path) -> bool:
    for file_name in LICENSE_DOCUMENT_FILES:
        source = root / file_name
        if not source.is_file():
            print(
                f"error: required license document was not found: {source}",
                file=sys.stderr,
            )
            return False

    try:
        binary_dir.mkdir(parents=True, exist_ok=True)
        for file_name in LEGACY_LICENSE_DOCUMENT_FILES:
            legacy_path = binary_dir / file_name
            if legacy_path.exists():
                legacy_path.unlink()
                print(f"Removed legacy license document: {legacy_path}")
        for file_name in LICENSE_DOCUMENT_FILES:
            source = root / file_name
            destination = binary_dir / file_name
            shutil.copy2(source, destination)
            print(f"Copied license document: {destination}")
    except OSError as error:
        print(
            f"error: failed to copy license documents to {binary_dir}: {error}",
            file=sys.stderr,
        )
        return False

    return True


def release_binary_path(binary_dir: Path) -> Path | None:
    for file_name in ("j3files.exe", "j3files"):
        candidate = binary_dir / file_name
        if candidate.is_file():
            return candidate
    return None


def package_root_name(version: str, suffix: str) -> str:
    return f"j3files-{version}-{suffix}"


def zip_arcname(package_root: str, relative_path: Path) -> str:
    return f"{package_root}/{relative_path.as_posix()}"


def source_file_is_excluded(path: Path) -> bool:
    name = path.name
    if name in SOURCE_PACKAGE_EXCLUDED_FILE_NAMES:
        return True
    if name.endswith("~"):
        return True
    return any(name.endswith(suffix) for suffix in SOURCE_PACKAGE_EXCLUDED_SUFFIXES)


def source_package_files(root: Path) -> Iterable[Path]:
    for current, dirs, files in os.walk(root):
        dirs[:] = sorted(
            directory
            for directory in dirs
            if directory not in SOURCE_PACKAGE_EXCLUDED_DIRS
        )
        current_path = Path(current)
        for file_name in sorted(files):
            path = current_path / file_name
            if source_file_is_excluded(path):
                continue
            yield path


def create_source_package(root: Path, dist_dir: Path, version: str) -> Path:
    package_root = package_root_name(version, "source")
    archive_path = dist_dir / f"{package_root}.zip"
    dist_dir.mkdir(parents=True, exist_ok=True)

    with zipfile.ZipFile(
        archive_path,
        "w",
        compression=zipfile.ZIP_DEFLATED,
    ) as archive:
        for path in source_package_files(root):
            relative_path = path.relative_to(root)
            archive.write(path, zip_arcname(package_root, relative_path))

    print(f"Created source package: {archive_path}")
    return archive_path


def create_binary_package(
    binary_dir: Path,
    dist_dir: Path,
    version: str,
    target_label: str,
) -> Path | None:
    binary_path = release_binary_path(binary_dir)
    if binary_path is None:
        print(
            f"error: release binary was not found in {binary_dir}",
            file=sys.stderr,
        )
        return None

    package_root = package_root_name(version, f"windows-{target_label}")
    archive_path = dist_dir / f"{package_root}.zip"
    dist_dir.mkdir(parents=True, exist_ok=True)

    with zipfile.ZipFile(
        archive_path,
        "w",
        compression=zipfile.ZIP_DEFLATED,
    ) as archive:
        archive.write(binary_path, zip_arcname(package_root, Path(binary_path.name)))
        for file_name in LICENSE_DOCUMENT_FILES:
            archive.write(
                binary_dir / file_name,
                zip_arcname(package_root, Path(file_name)),
            )

    print(f"Created binary package: {archive_path}")
    return archive_path


def create_distribution_packages(
    root: Path,
    binary_dir: Path,
    target: str | None,
) -> bool:
    version = package_version(root)
    dist_dir = root / DIST_DIR_NAME
    target_label = target or host_target(root) or "host"

    create_source_package(root, dist_dir, version)
    return create_binary_package(binary_dir, dist_dir, version, target_label) is not None


def open_folder(path: Path) -> bool:
    path = path.resolve()
    system = platform.system()

    if system == "Windows":
        os.startfile(str(path))  # type: ignore[attr-defined]
        return True

    if system == "Linux":
        if not (os.environ.get("DISPLAY") or os.environ.get("WAYLAND_DISPLAY")):
            print(f"warning: no desktop session detected. Binary directory: {path}")
            return False

        openers: list[tuple[str, list[str]]] = [
            ("xdg-open", ["xdg-open", str(path)]),
            ("gio", ["gio", "open", str(path)]),
            ("kde-open", ["kde-open", str(path)]),
            ("nautilus", ["nautilus", str(path)]),
        ]
        for executable, command in openers:
            resolved = shutil.which(executable)
            if resolved:
                subprocess.Popen(
                    [resolved, *command[1:]],
                    stdout=subprocess.DEVNULL,
                    stderr=subprocess.DEVNULL,
                )
                return True

        print(f"warning: no file manager opener was found. Binary directory: {path}")
        return False

    if system == "Darwin":
        opener = shutil.which("open")
        if opener:
            subprocess.Popen(
                [opener, str(path)],
                stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL,
            )
            return True

    print(f"warning: unsupported OS for opening folders. Binary directory: {path}")
    return False


def main(argv: list[str]) -> int:
    args, cargo_args = parse_args(argv)
    root = project_root()
    target = selected_target(args.target)
    binary_dir = release_binary_dir(root, target, cargo_args)

    if platform.system() != "Windows" and target == DEFAULT_LINUX_WINDOWS_TARGET:
        print(
            "Non-Windows host detected; building the Windows release target "
            f"{DEFAULT_LINUX_WINDOWS_TARGET}."
        )

    status = run_release_build(root, target, cargo_args)
    if status != 0:
        print("Release build failed.", file=sys.stderr)
        return status

    if not copy_license_documents(root, binary_dir):
        return 1

    if not create_distribution_packages(root, binary_dir, target):
        return 1

    print(f"Release build completed: {binary_dir}")
    if not args.no_open:
        open_folder(binary_dir)
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
