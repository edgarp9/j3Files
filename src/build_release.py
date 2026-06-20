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


DEFAULT_LINUX_WINDOWS_TARGET = "x86_64-pc-windows-gnu"


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


def selected_target(requested_target: str | None) -> str | None:
    if requested_target:
        return requested_target
    if platform.system() == "Windows":
        return None
    return DEFAULT_LINUX_WINDOWS_TARGET


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

    print(f"Release build completed: {binary_dir}")
    if not args.no_open:
        open_folder(binary_dir)
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
