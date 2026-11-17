# Apple signing and notarization

Skiff distributed outside the Mac App Store needs a **Developer ID Application**
certificate and Apple's notarization ticket. An Apple Development certificate
is useful for development but cannot replace this distribution certificate.
Skiff's bundle identifier is `io.shebash.skiff.app`.

## Prepare this Mac

1. Install Xcode and finish its required component setup. The macOS SDK is
   sufficient; Skiff does not need the mobile simulator downloads.
2. In Xcode → Settings → Apple Accounts, select the intended developer team,
   then **Manage Certificates**. Create **Developer ID Application**, or import
   an existing certificate **with its private key**. A `.cer` file alone does
   not contain the private key. Apple may require the Account Holder to create
   this certificate. If Apple reports **PLA Update available**, the account
   holder must review and accept the current agreement in the developer portal.
3. Run `security find-identity -v -p codesigning`. The list must contain a valid
   Developer ID Application identity. Keep its private key in Keychain.
4. Run `scripts/macos-signing.sh init`. The ignored `.env.signing.local` contains
   only a certificate selector and notarization profile name. When more than
   one Developer ID identity is available, set `SKIFF_SIGN_IDENTITY` to the
   intended certificate's SHA-1 fingerprint from the previous command. A full
   unique certificate name also works; quote it because this file is sourced by
   the shell, for example:
   `SKIFF_SIGN_IDENTITY='Developer ID Application: Example LLC (ABCDE12345)'`.
   Empty means auto-select the sole Developer ID Application identity.

## Store notarization credentials

Run this yourself in a local terminal:

```sh
scripts/macos-signing.sh credentials
```

Apple's `notarytool` prompts for credentials and stores them in Keychain under
`skiff-notary`, validating them with Apple. For Apple Account authentication,
use the **developer Team ID** and an **app-specific password**, not your normal
account password. The Team ID is in your Apple Developer membership details;
it is not necessarily the identifier printed in an Apple Development
certificate's name. Create the app-specific password in your Apple Account's
sign-in/security settings. Alternatively, `notarytool store-credentials`
supports an App Store Connect API key.

The optional `SKIFF_APPLE_ID` and `SKIFF_APPLE_TEAM_ID` values in the local
configuration prefill the account and team. The password is always entered
at Apple's secure terminal prompt, never stored in that file.

Keep credentials out of chat, shell command arguments, environment files,
Git history, and CI logs. The local configuration stores the **profile name**,
not its credentials. No signing keys are uploaded to GitHub by this setup.

## Check, sign, and release

```sh
scripts/macos-signing.sh check
scripts/macos-signing.sh sign       # sign the existing target/Skiff.app
scripts/macos-signing.sh release    # build, sign, notarize, staple, and package
# For a previously built Intel app:
scripts/macos-signing.sh sign --arch x86_64
```

The check requires a valid Developer ID Application identity and validates
the selected notarytool profile with a read-only history request before any
build or upload. Signing covers Docker, Compose, buildx, the Skiff executable,
and the app bundle, inside out, with hardened runtime and secure timestamps.
Skiff retains its virtualization entitlement.

The release task submits the signed DMG to Apple, staples the ticket to the
DMG and app, validates both tickets, and verifies the app with Gatekeeper.
`cargo xtask release` now requires the certificate and notarization profile;
it cannot silently produce an ad hoc release. `cargo xtask sign` requires a
Developer ID identity; it also notarizes when `SKIFF_NOTARY_PROFILE` is set.
`cargo xtask pack` packages an already signed, stapled, Gatekeeper-accepted app
without needing the private key on the packaging machine.

Source builds with `cargo xtask app` can still use an ad hoc signature when
no identity is configured or available, including when Keychain lookup fails.
An explicit Apple Development or other valid code-signing identity is also
supported for local builds. `SKIFF_SIGN_IDENTITY=-` explicitly
selects ad hoc signing for local builds, and is rejected for distribution.
`cargo xtask app-dev` never submits a notarization request, even if a profile
is present in the environment. A missing explicitly selected certificate or
ambiguous selection fails instead of choosing a different identity.

The helper loads `.env.signing.local`; direct `cargo xtask` commands use the
current shell's environment. To use the latter with the local configuration:

```sh
set -a
source .env.signing.local
set +a
```

Signing and notarization produce local artifacts; they do not publish a release.
Check the version, platform installation behavior, kernel source offer, and
checksums before publishing as described in [RELEASES.md](../RELEASES.md).

## References

- [Apple: Developer ID certificates](https://developer.apple.com/help/account/certificates/create-developer-id-certificates/)
- [Apple: notarizing macOS software](https://developer.apple.com/documentation/security/notarizing-macos-software-before-distribution)
- [Apple: customizing the notarization workflow](https://developer.apple.com/documentation/security/customizing-the-notarization-workflow)
