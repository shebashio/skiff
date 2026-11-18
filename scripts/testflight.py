#!/usr/bin/env python3
"""Sign sandbox builds locally, or prepare a Mac App Store/TestFlight package.

No password is accepted on the command line. Upload uses an existing altool
Keychain item, or Transporter. Developer ID/notarytool are not used here.
"""
import argparse
import datetime
import os
from pathlib import Path
import plistlib
import re
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parent.parent
BUNDLE = "io.shebash.skiff.app"
TEAM = "U42F5DDR4B"


def run(*args, **kwargs):
    return subprocess.run([str(a) for a in args], check=True, **kwargs)


def read_profile(path):
    data = run("security", "cms", "-D", "-i", path, capture_output=True).stdout
    profile = plistlib.loads(data)
    return validate_profile(profile)


def validate_profile(profile):
    """Preflight only; Apple’s upload service performs authoritative validation."""
    ent = profile["Entitlements"]
    if profile["ExpirationDate"] <= datetime.datetime.now(datetime.timezone.utc).replace(tzinfo=None):
        raise ValueError("provisioning profile has expired")
    if profile.get("TeamIdentifier") != [TEAM]:
        raise ValueError("profile is not for SHE BASH LLC")
    if ent.get("com.apple.application-identifier") != f"{TEAM}.{BUNDLE}":
        raise ValueError("profile must explicitly match Skiff's bundle identifier")
    if ent.get("get-task-allow") or ent.get("com.apple.security.get-task-allow") or "ProvisionedDevices" in profile or profile.get("ProvisionsAllDevices"):
        raise ValueError("use a Mac App Store distribution profile, not development or Developer ID")
    if profile.get("Platform") != ["OSX"] or not ent.get("com.apple.security.virtualization"):
        raise ValueError("use a macOS profile with Skiff’s Virtualization capability")
    return profile


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", choices=["build", "sign-dev", "package"])
    parser.add_argument("app", type=Path, nargs="?", default=ROOT/"target/testflight/Skiff.app")
    parser.add_argument("--version", default="0.1.0")
    parser.add_argument("--build", default="1")
    parser.add_argument("--profile", type=Path)
    parser.add_argument("--identity", default=os.getenv("SKIFF_APP_STORE_IDENTITY"))
    parser.add_argument("--installer", default=os.getenv("SKIFF_APP_STORE_INSTALLER"))
    args = parser.parse_args()
    if not re.fullmatch(r"\d+\.\d+\.\d+", args.version) or not re.fullmatch(r"[1-9]\d{0,8}", args.build):
        parser.error("use a numeric x.y.z version and a positive build number")
    app = args.app.resolve()
    if args.command == "build":
        if app != ROOT/"target/testflight/Skiff.app":
            parser.error("build writes target/testflight/Skiff.app")
        manifest, lock = ROOT/"Cargo.toml", ROOT/"Cargo.lock"
        original, original_lock = manifest.read_bytes(), lock.read_bytes()
        try:
            # Cargo embeds this version in the UI and engine, not just Info.plist.
            manifest.write_text(re.sub(r'^version = "[^"]*"', f'version = "{args.version}"', original.decode(), count=1, flags=re.M))
            run("cargo", "xtask", "app-testflight", cwd=ROOT)
        finally:
            manifest.write_bytes(original)
            lock.write_bytes(original_lock)
    info_path = app / "Contents/Info.plist"
    info = plistlib.loads(info_path.read_bytes())
    if info.get("CFBundleIdentifier") != BUNDLE or info.get("SkiffDistributionChannel") != "testflight":
        parser.error("build a sandboxed app with cargo xtask app-testflight first")
    profile = None
    if args.command == "package":
        if not args.profile or not args.identity or not args.installer:
            parser.error("package requires --profile, --identity (Apple/Mac Distribution), and --installer (Mac Installer Distribution)")
        if not args.identity.startswith(("Apple Distribution:", "3rd Party Mac Developer Application:")):
            parser.error("Developer ID identities cannot sign a TestFlight build")
        if not args.installer.startswith("3rd Party Mac Developer Installer:"):
            parser.error("use a Mac Installer Distribution certificate")
        profile = read_profile(args.profile)
        # Confirm the selected leaf certificate is actually allowed by this profile.
        with tempfile.TemporaryDirectory(prefix="skiff-cert-") as tmp:
            with (Path(tmp)/"cert.pem").open("wb") as certificate:
                run("security", "find-certificate", "-c", args.identity, "-p", stdout=certificate)
            cert = run("openssl", "x509", "-in", Path(tmp)/"cert.pem", "-outform", "DER", capture_output=True).stdout
        if cert not in profile["DeveloperCertificates"]:
            parser.error("the selected distribution certificate is absent from the provisioning profile")
    info.update(CFBundleShortVersionString=args.version, CFBundleVersion=args.build,
                NSHumanReadableCopyright="Copyright © 2026 SHE BASH LLC. Apache-2.0 licensed; see bundled notices.")
    info_path.write_bytes(plistlib.dumps(info, fmt=plistlib.FMT_XML))
    embedded = app / "Contents/embedded.provisionprofile"
    if profile:
        embedded.write_bytes(args.profile.read_bytes())
    else:
        embedded.unlink(missing_ok=True)
    entitlements = plistlib.loads((ROOT / "crates/skiff/skiff-app-store.entitlements").read_bytes())
    if profile:
        for key in ("com.apple.application-identifier", "com.apple.developer.team-identifier", "beta-reports-active"):
            if key in profile["Entitlements"]:
                entitlements[key] = profile["Entitlements"][key]
    identity = args.identity if profile else "-"
    with tempfile.TemporaryDirectory(prefix="skiff-testflight-") as tmp:
        main_ent = Path(tmp)/"app.plist"
        main_ent.write_bytes(plistlib.dumps(entitlements))
        helpers = [app/"Contents/MacOS/skiff-helper"] + sorted((app/"Contents/Resources/bin").rglob("*"))
        for helper in helpers:
            if helper.is_file():
                run("codesign", "--force", "--sign", identity, "--entitlements", ROOT/("crates/skiff/skiff-engine.entitlements" if helper.name == "skiff-helper" else "crates/skiff/skiff-inherit.entitlements"), helper)
        run("codesign", "--force", "--sign", identity, "--entitlements", main_ent, app)
        run("codesign", "--verify", "--deep", "--strict", app)
        signed = run("codesign", "-d", "--entitlements", ":-", app, capture_output=True).stdout
        if not plistlib.loads(signed).get("com.apple.security.app-sandbox"):
            raise ValueError("signed app is missing App Sandbox")
    if profile:
        package = app.parent / f"Skiff-{args.version}-{args.build}.pkg"
        run("productbuild", "--component", app, "/Applications", "--sign", args.installer, package)
        run("pkgutil", "--check-signature", package)
        print(f"TestFlight package: {package}\nValidate and upload using Transporter or xcrun altool; no notarization is needed.")
    else:
        print(f"Local sandbox test build (not uploadable): {app}")


if __name__ == "__main__":
    main()
