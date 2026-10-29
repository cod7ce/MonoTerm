#!/usr/bin/env python3
"""Validate MonoTerm's update metadata and print its Cargo package version."""

import argparse
import base64
import binascii
from pathlib import Path
import plistlib
import re
import tomllib


ROOT = Path(__file__).resolve().parent.parent
FEED_URL = "https://github.com/cod7ce/MonoTerm/releases/latest/download/appcast.xml"
BUNDLE_ID = "local.monoterm.app"


def fail(message):
    raise SystemExit(message)


def read_plist(path):
    try:
        with path.open("rb") as source:
            info = plistlib.load(source)
    except (OSError, ValueError, plistlib.InvalidFileException) as error:
        fail(f"Cannot read {path}: {error}")
    if not isinstance(info, dict):
        fail(f"{path}: expected a plist dictionary.")
    return info


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--release", action="store_true", help="require a configured update public key")
    parser.add_argument("--bundle", type=Path, help="also validate an existing .app")
    args = parser.parse_args()
    with (ROOT / "Cargo.toml").open("rb") as source:
        manifest = tomllib.load(source)
    version = manifest.get("package", {}).get("version", "")
    if not isinstance(version, str) or not re.fullmatch(r"0\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)", version):
        fail("Cargo.toml must declare a stable 0.x.x package version.")

    source_path = ROOT / "scripts/Info.plist"
    source_info = read_plist(source_path)
    plists = [(source_path, source_info)]
    if args.bundle:
        path = args.bundle / "Contents/Info.plist"
        plists.append((path, read_plist(path)))
    expected = {
        "CFBundleShortVersionString": version,
        "CFBundleVersion": version,
        "CFBundleIdentifier": BUNDLE_ID,
        "CFBundleExecutable": "MonoTerm",
        "SUFeedURL": FEED_URL,
        "LSMinimumSystemVersion": "14.0",
        "SUEnableAutomaticChecks": False,
        "SUAutomaticallyUpdate": False,
        "SUSendProfileInfo": False,
        "SUEnableInstallerLauncherService": True,
    }
    for path, info in plists:
        for key, value in expected.items():
            if info.get(key) != value or type(info.get(key)) is not type(value):
                fail(f"{path}: {key} must equal {value!r}.")
        if args.release:
            encoded = info.get("SUPublicEDKey", "")
            try:
                key = base64.b64decode(encoded, validate=True)
            except (ValueError, TypeError, binascii.Error):
                key = b""
            if len(key) != 32 or not any(key) or base64.b64encode(key).decode("ascii") != encoded:
                fail(f"{path}: SUPublicEDKey must contain a base64-encoded 32-byte Ed25519 public key.")
        if info.get("SUPublicEDKey", "") != source_info.get("SUPublicEDKey", ""):
            fail(f"{path}: SUPublicEDKey differs from scripts/Info.plist; rebuild the bundle.")
    if args.bundle and args.release and plists[-1][1].get("MonoTermBuildProfile") != "release":
        fail(f"{args.bundle}: expected a bundle created by scripts/build-app.sh release.")
    print(version)


if __name__ == "__main__":
    main()
