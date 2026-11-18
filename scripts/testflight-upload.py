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
    parser.add_argument("--provider-public-id", help="App Store Connect provider UUID (altool --list-providers)")
    parser.add_argument("--apple-id", help="Numeric App Store Connect app ID")
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
        # Xcode may fail to infer the provider even for an existing bundle ID.
        # Check an explicit provider against the authenticated account first.
        provider = []
        if args.provider_public_id:
            result = subprocess.run(["xcrun", "altool", "--list-providers", "--output-format", "json", *auth], env=env, capture_output=True, text=True, check=True)
            if args.provider_public_id.lower() not in result.stdout.lower():
                print(result.stdout)
                parser.error("the requested provider is absent from this Apple account; check the Apple ID and provider UUID")
            provider = ["--provider-public-id", args.provider_public_id]
        app_id = ["--apple-id", args.apple_id] if args.apple_id else []
        options = [*auth, *provider, *app_id, "--platform", "macos"]
        subprocess.run(["xcrun", "altool", "--validate-app", str(package), *options], env=env, check=True)
        if not args.validate_only:
            subprocess.run(["xcrun", "altool", "--upload-package", str(package), "--wait", *options], env=env, check=True)
    except subprocess.CalledProcessError as error:
        if error.stdout:
            print(error.stdout)
        if error.stderr:
            print(error.stderr, file=sys.stderr)
        print("Apple did not complete this step; see its error above.", file=sys.stderr)
        return error.returncode
    finally:
        env.pop("SKIFF_TESTFLIGHT_UPLOAD_PASSWORD", None)


if __name__ == "__main__":
    sys.exit(main())
