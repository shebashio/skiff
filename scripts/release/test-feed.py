#!/usr/bin/env python3
"""Write the unsigned macOS test feed without bypassing xtask's release guard."""
import json
import os
import sys
from pathlib import Path
from artifacts import digest, version_of

version, arch = sys.argv[1:]
version_of("v" + version)
if arch not in {"arm64", "x86_64"}:
    raise ValueError("invalid architecture")
archive = Path(f"target/Skiff-{version}-{arch}.tar.gz")
Path(f"target/latest-mac-{arch}.json").write_text(json.dumps({
    "version": version, "url": f"{os.environ['SKIFF_RELEASE_BASE']}/{archive.name}", "sha256": digest(archive)
}) + "\n")
