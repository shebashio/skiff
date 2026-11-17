# Skiff releases

Skiff is an independent development application. No signed production release
channel has been published yet. Native CI uploads unsigned Linux and Windows
binaries for x64 and ARM64; these are not complete installers.

For macOS, follow [Apple signing and notarization](docs/macos-signing.md).
`scripts/macos-signing.sh release` checks the Developer ID identity and
Keychain notarization profile before building. Signing keys stay local.

Release builds take their version from a tag, update the workspace version,
and run the platform packaging tasks described in docs/development.md.
Publish the matching kernel source archive alongside every distributed guest
kernel, and retain all required license notices.

The updater has no default feed. An operator can configure SKIFF_UPDATE_URL
for their own feed and SKIFF_RELEASE_BASE when building release metadata.
Never use Kiri's feeds, signing identity, installer identity, or domain for a
Skiff release. See UPSTREAM.md for the hosting-control decision behind the fork.

Before the first production release, validate installation, upgrade and
uninstall on each supported OS, sign/notarize the relevant platform artifacts,
and publish checksums and source archives with the release.
