#!/usr/bin/env python3
"""Version stamping and the release artifact contract (standard library only)."""
import argparse
import hashlib
import json
import re
import tomllib
from pathlib import Path


def version_of(tag):
    match = re.fullmatch(r"v(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)(?:-([0-9A-Za-z.-]+))?", tag)
    if not match:
        raise ValueError("expected a release tag such as v0.1.0 or v0.1.0-rc.1")
    major, minor, patch = map(int, match.group(1, 2, 3))
    if major > 255 or minor > 255 or patch > 65535:
        raise ValueError("version exceeds Windows Installer's 255.255.65535 limit")
    if match[4]:
        for part in match[4].split("."):
            if not part or (part.isdigit() and len(part) > 1 and part.startswith("0")):
                raise ValueError("invalid SemVer prerelease identifier")
    return tag[1:]


def stamp(root, version):
    version_of("v" + version)
    manifest = root / "Cargo.toml"
    text = manifest.read_text()
    data = tomllib.loads(text)
    members = set()
    for pattern in data["workspace"]["members"]:
        for member in root.glob(pattern):
            package = tomllib.loads((member / "Cargo.toml").read_text())["package"]
            if package.get("version") == {"workspace": True}:
                members.add(package["name"])
    text, count = re.subn(r'(\[workspace\.package\][\s\S]*?^version = ")[^"]+("$)', rf'\g<1>{version}\2', text, count=1, flags=re.M)
    if count != 1:
        raise ValueError("workspace version not found")
    lock = root / "Cargo.lock"
    blocks = lock.read_text().split("[[package]]")
    changed = set()
    for i, block in enumerate(blocks[1:], 1):
        package = tomllib.loads(block)
        if package.get("name") in members and "source" not in package:
            blocks[i], count = re.subn(r'^version = "[^"]+"$', f'version = "{version}"', block, count=1, flags=re.M)
            if count != 1:
                raise ValueError("missing package version in Cargo.lock")
            changed.add(package["name"])
    if changed != members:
        raise ValueError("Cargo.lock does not contain every workspace package")
    manifest.write_text(text)
    lock.write_text("[[package]]".join(blocks))


def digest(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def expected(version):
    files = {"LICENSE", "NOTICE", f"Skiff-{version}-kernel-source.tar.xz"}
    for arch in ("arm64", "x86_64"):
        files.update(f"Skiff-{version}-{arch}.{ext}" for ext in ("dmg", "tar.gz", "msi"))
        files.add(f"Skiff-{version}-windows-{arch}.zip")
        files.update(f"latest-{platform}-{arch}.json" for platform in ("mac", "windows"))
    linux_version = version.replace("-", "~", 1)
    for name in ("skiff", "skiff-lite"):
        files.update(f"{name}_{linux_version}_{arch}.deb" for arch in ("amd64", "arm64"))
        files.update(f"{name}-{linux_version}-1.{arch}.rpm" for arch in ("x86_64", "aarch64"))
    return files


def collect(directory, version, repository, commit, signed):
    version_of("v" + version)
    if not re.fullmatch(r"[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+", repository):
        raise ValueError("invalid repository")
    if not re.fullmatch(r"[a-f0-9]{40}", commit):
        raise ValueError("expected a full source commit SHA")
    required = expected(version)
    actual = {p.name for p in directory.iterdir() if p.name not in {"SHA256SUMS", "release-manifest.json", "release-notes.md"}}
    if actual != required:
        raise ValueError(f"release artifacts differ: missing={sorted(required - actual)}, extra={sorted(actual - required)}")
    for name in required:
        path = directory / name
        if path.is_symlink() or not path.is_file() or not path.stat().st_size:
            raise ValueError(f"invalid or empty artifact: {name}")
    hashes = {name: digest(directory / name) for name in sorted(required)}
    base = f"https://github.com/{repository}/releases/download/v{version}"
    for platform, ext in (("mac", "tar.gz"), ("windows", "msi")):
        for arch in ("arm64", "x86_64"):
            name = f"Skiff-{version}-{arch}.{ext}"
            feed = json.loads((directory / f"latest-{platform}-{arch}.json").read_text())
            if feed.get("version") != version or feed.get("url") != f"{base}/{name}" or feed.get("sha256") != hashes[name]:
                raise ValueError(f"incorrect {platform}/{arch} update feed")
            if platform == "windows" and feed.get("size") != (directory / name).stat().st_size:
                raise ValueError(f"incorrect Windows/{arch} download size")
    manifest = {"version": version, "commit": commit, "repository": repository,
                "macos_notarized": signed, "windows_authenticode_signed": False,
                "artifacts": [{"name": name, "sha256": value, "size": (directory / name).stat().st_size} for name, value in hashes.items()]}
    (directory / "release-manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")
    hashes["release-manifest.json"] = digest(directory / "release-manifest.json")
    (directory / "SHA256SUMS").write_text("".join(f"{value}  {name}\n" for name, value in sorted(hashes.items())))
    signing = "Developer ID signed and notarized" if signed else "ad hoc signed; test build only"
    (directory / "release-notes.md").write_text(
        f"Skiff {version}\n\nBuilt from `{commit}`.\n\n"
        f"- macOS: Apple Silicon and Intel DMG installers and update archives; {signing}.\n"
        "- Windows: x64 and ARM64 MSI installers and portable bundles. These are currently unsigned (no Authenticode certificate is configured).\n"
        "- Linux: x64 and ARM64 desktop and engine-only packages in DEB and RPM formats.\n"
        "- SHA256SUMS, build manifest, license notices, and corresponding guest-kernel source are included.\n\n"
        "Verify installation, upgrades, and removal before publishing this draft.\n")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="command", required=True)
    sub.add_parser("version").add_argument("tag")
    stamping = sub.add_parser("stamp")
    stamping.add_argument("version")
    stamping.add_argument("--root", type=Path, default=Path.cwd())
    gathering = sub.add_parser("collect")
    gathering.add_argument("directory", type=Path)
    gathering.add_argument("version")
    gathering.add_argument("repository")
    gathering.add_argument("commit")
    gathering.add_argument("--signed", action="store_true")
    args = parser.parse_args()
    if args.command == "version":
        print(version_of(args.tag))
    elif args.command == "stamp":
        stamp(args.root, args.version)
    else:
        collect(args.directory, args.version, args.repository, args.commit, args.signed)


if __name__ == "__main__":
    main()
