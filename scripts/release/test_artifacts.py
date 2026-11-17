import json
import tempfile
import unittest
from pathlib import Path
from artifacts import collect, digest, expected, stamp, version_of


class ReleaseTests(unittest.TestCase):
    def test_versions_reject_injection_invalid_semver_and_msi_overflow(self):
        for tag in ("v1.2.3", "v0.1.0-rc.2", "v255.255.65535"):
            self.assertEqual(version_of(tag), tag[1:])
        for tag in ("1.2.3", "v01.2.3", "v1.2.3\n", "v1.2.3-rc..1", "v1.2.3-01", "v256.0.0", "v1.2.65536", "v1.2.3;echo bad", "v1.2.3+build"):
            with self.subTest(tag=tag), self.assertRaises(ValueError):
                version_of(tag)

    def test_stamping_keeps_dependency_versions_pinned(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "crates/app").mkdir(parents=True)
            (root / "Cargo.toml").write_text('[workspace]\nmembers = ["crates/*"]\n[workspace.package]\n# comment\nversion = "0.0.0"\n')
            (root / "crates/app/Cargo.toml").write_text('[package]\nname = "app"\nversion.workspace = true\n')
            (root / "Cargo.lock").write_text('version = 4\n[[package]]\nname = "app"\nversion = "0.0.0"\n[[package]]\nname = "dependency"\nversion = "2.0.0"\nsource = "registry+https://example.com"\n')
            stamp(root, "1.2.3-rc.1")
            self.assertIn('version = "1.2.3-rc.1"', (root / "Cargo.toml").read_text())
            self.assertIn('version = "1.2.3-rc.1"', (root / "Cargo.lock").read_text())
            self.assertIn('version = "2.0.0"', (root / "Cargo.lock").read_text())

    def fixture(self, directory):
        version = "1.2.3"
        for name in expected(version):
            (directory / name).write_bytes(b"test release artifact\n")
        for platform, ext in (("mac", "tar.gz"), ("windows", "msi")):
            for arch in ("arm64", "x86_64"):
                asset = directory / f"Skiff-{version}-{arch}.{ext}"
                (directory / f"latest-{platform}-{arch}.json").write_text(json.dumps({
                    "version": version, "url": f"https://github.com/shebashio/skiff/releases/download/v{version}/{asset.name}",
                    "sha256": digest(asset), "size": asset.stat().st_size
                }))

    def test_complete_set_produces_checksums_and_explicit_signing_status(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            self.fixture(root)
            collect(root, "1.2.3", "shebashio/skiff", "a" * 40, False)
            manifest = json.loads((root / "release-manifest.json").read_text())
            self.assertFalse(manifest["macos_notarized"])
            self.assertFalse(manifest["windows_authenticode_signed"])
            self.assertEqual(len(manifest["artifacts"]), len(expected("1.2.3")))
            for line in (root / "SHA256SUMS").read_text().splitlines():
                value, name = line.split("  ", 1)
                self.assertEqual(value, digest(root / name))

    def test_missing_source_or_platform_blocks_release(self):
        for missing in ("Skiff-1.2.3-kernel-source.tar.xz", "Skiff-1.2.3-arm64.msi", "skiff_1.2.3_arm64.deb"):
            with self.subTest(missing=missing), tempfile.TemporaryDirectory() as tmp:
                root = Path(tmp)
                self.fixture(root)
                (root / missing).unlink()
                with self.assertRaisesRegex(ValueError, "missing="):
                    collect(root, "1.2.3", "shebashio/skiff", "a" * 40, True)

    def test_feed_must_match_exact_versioned_artifact(self):
        for field, value in (("version", "1.2.2"), ("sha256", "0" * 64), ("url", "https://example.com/download"), ("size", 0)):
            with self.subTest(field=field), tempfile.TemporaryDirectory() as tmp:
                root = Path(tmp)
                self.fixture(root)
                feed = root / "latest-windows-arm64.json"
                data = json.loads(feed.read_text())
                data[field] = value
                feed.write_text(json.dumps(data))
                with self.assertRaises(ValueError):
                    collect(root, "1.2.3", "shebashio/skiff", "a" * 40, True)

    def test_extra_files_and_symlinks_block_release(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            self.fixture(root)
            (root / "private-key.p12").write_text("unexpected")
            with self.assertRaisesRegex(ValueError, "extra="):
                collect(root, "1.2.3", "shebashio/skiff", "a" * 40, True)
            (root / "private-key.p12").unlink()
            (root / "NOTICE").unlink()
            (root / "NOTICE").symlink_to(root / "LICENSE")
            with self.assertRaisesRegex(ValueError, "invalid or empty"):
                collect(root, "1.2.3", "shebashio/skiff", "a" * 40, True)


if __name__ == "__main__":
    unittest.main()
