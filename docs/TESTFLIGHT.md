# Skiff for TestFlight

The `app-store` Cargo feature builds a macOS App Sandbox edition. The first beta is for Apple silicon and macOS 14 or newer. Direct-download builds keep their existing system integrations.

## Using the beta

- Start Skiff to run its engine. Quitting stops the engine and Cloudflare links.
- Pull an image in Images, then run it. Container logs and the container Terminal tab work inside the sandbox.
- Add project directories in Settings → Shared folders. Quit and reopen Skiff to apply additions or removals. Bind mounts outside these folders are rejected, including symlinks that escape them.
- Settings → Shared folders shows the bundled CLI path and the engine socket. An existing Docker CLI can connect explicitly with `docker --host unix://<socket>`. Use that external CLI for build contexts and `docker cp`; the sandboxed bundled tools cannot freely read your Mac.
- Published container ports use localhost URLs. Cloudflare sharing supports temporary links and the existing browser-based custom-domain setup, using a bundled, pinned cloudflared.
- Vulnerability scanning, the host terminal, USB passthrough, Finder filesystem mounts, system DNS/CA installation, global Docker socket/PATH setup, login agents, and self-updates are unavailable. TestFlight manages updates.
- This edition uses its own container data and configuration under `~/Library/Containers/io.shebash.skiff.app/Data`. It does not migrate the direct-download app’s data automatically. Both editions use the same bundle ID; install and run one edition at a time.

## Build and sign

Prepare the normal guest assets first (`guest/out` and the CLI cache), following the development build instructions. Then, from a dedicated checkout with no concurrent Cargo builds:

```sh
python3 scripts/testflight.py build --version 0.1.0 --build 1
```

This stamps the Rust binaries as well as Info.plist, restores the checkout’s Cargo version and lockfile afterwards, and creates an ad hoc sandbox test app at `target/testflight/Skiff.app`. Test this app before distribution signing; an App Store distribution profile cannot be used for ordinary local installation.

For TestFlight distribution, install a Mac App Distribution identity and a Mac Installer Distribution identity in your Keychain. Create a **Mac App Store Connect** provisioning profile for `io.shebash.skiff.app` with Virtualization enabled. Developer ID identities and notarization profiles do not replace these credentials.

```sh
python3 scripts/testflight.py package target/testflight/Skiff.app \
  --version 0.1.0 --build 1 \
  --profile /private/path/Skiff.provisionprofile \
  --identity '3rd Party Mac Developer Application: SHE BASH LLC (U42F5DDR4B)' \
  --installer '3rd Party Mac Developer Installer: SHE BASH LLC (U42F5DDR4B)'
```

The script validates the profile’s app, team, expiry, platform and distribution type; checks the signing certificate against the profile; signs nested executables before the app; embeds the profile; verifies the signature; and creates `target/testflight/Skiff-0.1.0-1.pkg`.

Upload the package using Apple’s Transporter or the interactive wrapper:

```sh
python3 scripts/testflight-upload.py target/testflight/Skiff-0.1.0-1.pkg --username YOUR_APPLE_ID
```

It checks the package signature, prompts for an app-specific password without saving it, validates the package through Apple, then uploads and waits for processing. Use `--validate-only` to stop after validation or `--keychain-item` for an existing altool credential.

If altool cannot determine the provider from the bundle ID, pass `--provider-public-id YOUR_PROVIDER_UUID --apple-id YOUR_NUMERIC_APP_ID`. The wrapper verifies that the provider appears in the authenticated account before validation. Obtain the provider UUID from `altool --list-providers`; the app ID is shown in App Store Connect.

Alternatively use `xcrun altool` directly. Keep passwords out of command arguments and source control: use altool’s Keychain/environment reference or its interactive prompt. Increase the build number for every upload. App Store Connect processing and any required beta review must finish before testers can install it.

Certificates, profiles, private keys and passwords are not repository assets. The existing Developer ID GitHub release environment does not contain the App Store identities and must not be reused for this workflow.

## Architecture and checks

The app launches `skiff-helper` with inherited sandbox permissions plus the Virtualization entitlement. Its stdin pipe controls engine lifetime, including parent termination. Security-scoped bookmarks retain native folder-picker grants in the app; temporary bookmarks transfer those grants to the helper. VirtioFS exports only selected directories and Skiff’s internal data directory, rather than the host root.

```sh
cargo check --locked -p skiff --features app-store
cargo test --locked -p skiff --features app-store app_store::tests
python3 -m unittest discover -s scripts -p test_testflight.py
```

Smoke tests should cover container execution, HTTP port forwarding, persistent folder grants, unapproved paths/symlinks, removing grants after restart, Cloudflare sharing, and closing both the engine and links on quit. Full Rust tests need local socket access; the unrelated AppleScript UI integration test is excluded from headless sandbox validation.
