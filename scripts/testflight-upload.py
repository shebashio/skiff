#!/usr/bin/env python3
"""Validate and upload a signed TestFlight .pkg without password arguments."""
import argparse
import getpass
import os
from pathlib import Path
import subprocess
import sys


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("package", type=Path)
    parser.add_argument("--username", default=os.getenv("SKIFF_APPLE_ID"))
    parser.add_argument("--keychain-item", help="Existing altool password item (not a notarytool profile)")
    parser.add_argument("--validate-only", action="store_true")
    args = parser.parse_args()
    package = args.package.resolve()
    if not package.is_file() or package.suffix != ".pkg":
        parser.error("provide the signed Skiff .pkg created by scripts/testflight.py package")
    if not args.username:
        parser.error("provide --username or set SKIFF_APPLE_ID")
    subprocess.run(["pkgutil", "--check-signature", str(package)], check=True)
    env = os.environ.copy()
    if args.keychain_item:
        password = f"@keychain:{args.keychain_item}"
    else:
        if not sys.stdin.isatty():
            parser.error("run in your own Terminal for the hidden password prompt, or use an existing --keychain-item")
        env["SKIFF_TESTFLIGHT_UPLOAD_PASSWORD"] = getpass.getpass("Apple app-specific password (hidden; not saved): ")
        if not env["SKIFF_TESTFLIGHT_UPLOAD_PASSWORD"]:
            parser.error("password was empty")
        password = "@env:SKIFF_TESTFLIGHT_UPLOAD_PASSWORD"
    auth = ["--username", args.username, "--password", password]
    try:
        subprocess.run(["xcrun", "altool", "--validate-app", str(package), *auth], env=env, check=True)
        if not args.validate_only:
            subprocess.run(["xcrun", "altool", "--upload-package", str(package), "--wait", *auth], env=env, check=True)
    finally:
        env.pop("SKIFF_TESTFLIGHT_UPLOAD_PASSWORD", None)


if __name__ == "__main__":
    main()
