# Skiff releases

The [Release workflow](.github/workflows/release.yml) builds complete installers
on GitHub-hosted runners. A version tag creates a **draft** GitHub release only
after every platform, update feed, checksum, and source archive passes validation.
Publishing the draft is a separate maintainer action. No production release has
been published yet.

## Downloads

| Platform | Architectures | Artifacts |
| --- | --- | --- |
| macOS | Apple Silicon, Intel | Developer ID signed, notarized DMG; stapled app archive; update feed |
| Windows | x64, ARM64 | MSI, portable bundle, update feed |
| Linux | x64, ARM64 | Desktop and engine-only DEB/RPM packages |
| Source | Both guest kernels | Patched corresponding kernel source, build recipes, license notices |

Windows artifacts are currently **unsigned**: an Authenticode certificate has
not been configured. The build manifest and draft release notes state this.
macOS release jobs require Developer ID and notarization; they never fall back
to an ad hoc signature. Linux desktop packages include the native window on
both architectures and are tested for installation, container execution, and removal.

Every release includes `SHA256SUMS` and `release-manifest.json`, with the exact
source commit, file sizes, hashes, and signing status. Update feeds point at
immutable version-tag URLs and are checked against the actual downloads.
The updater still requires an operator-configured `SKIFF_UPDATE_URL`.

## One-time GitHub signing setup

Local signing remains available through [the Keychain helper](docs/macos-signing.md).
Unattended GitHub releases need an explicitly exported copy of the Developer ID
identity and notarization credentials in the repository's `release` environment.
The signing private key is never committed or included in build artifacts.

1. Create an Actions environment named `release`. Under deployment branches and
   tags, choose **Selected branches and tags**, add the **tag** pattern `v*`, and
   allow no branches. Pull requests and test builds cannot use this environment.
   Under **Required reviewers**, add the release maintainers and tick
   **Prevent self-review**. Untick **Allow administrators to bypass configured
   protection rules**: it is on by default, and then any admin can start a
   signing job with no review.
2. Under **Settings → Rules → Rulesets**, add an active **tag** ruleset that
   targets `refs/tags/v*` with **Restrict creations**, **Restrict updates** and
   **Restrict deletions**, and only the release maintainers in its bypass list:
   the Admin or Maintain role, organization admins, or named teams, people or
   apps. Never the Write role, a custom role, or deploy keys.

   Both are required. The workflow's "tag must be on main" check runs from the
   tagged commit, so it can't stop someone who can push a tag from also
   changing that check. Only these settings decide who can start a signing job.
   `python3 scripts/release/configure-secrets.py --check` checks both, and the
   helper below refuses to upload secrets until they pass. It can't protect
   secrets uploaded before: they are open to anyone who could start a signing
   job until both settings are in place. Add the settings first, and consider
   replacing secrets that were uploaded without them.
3. In Keychain Access, find **Developer ID Application: SHE BASH LLC** under
   **My Certificates**. Export only that identity and its private key as an
   encrypted `.p12`, protected by a strong export password. Keep it outside
   the repository.
4. From your own terminal, run:

   ```sh
   python3 scripts/release/configure-secrets.py /absolute/path/Skiff-Developer-ID.p12
   ```

   The helper prompts privately for the P12 password and Apple app-specific
   password, then uploads five environment secrets through `gh` stdin:
   `APPLE_CERTIFICATE_P12_BASE64`, `APPLE_CERTIFICATE_PASSWORD`, `APPLE_ID`,
   `APPLE_TEAM_ID`, and `APPLE_APP_SPECIFIC_PASSWORD`. It checks the certificate
   type, team, and environment policy first. Never paste credentials into chat.
5. Each macOS signing job first compiles, downloads and installs everything the
   release needs (`cargo xtask release-prep`), then imports the identity into a
   temporary Keychain with a non-exportable private key that `codesign` and
   Apple's own tools may use, and validates the notarization profile. dmgbuild and its
   dependencies are installed from hash-pinned wheels
   (`scripts/dmgbuild-requirements.txt`). The Keychain and certificate file are
   removed at job completion, including failure. Secrets are never provided to
   unsigned test jobs. No self-hosted runner is needed on a developer's Mac.

Apple can take longer to process a new notarization submission. A signing job
waits up to the GitHub-hosted job limit; no release is created if it cannot
validate the final notarization tickets. Read the submission ID in the job log
and check its status before retrying a timed-out run.

## Test the pipeline

Run **Actions → Release → Run workflow** on the branch you want to test. Changes
to release tooling on `build/**` branches also run it automatically. These runs
use `0.0.0-ci.<run-number>`, have no signing credentials, and produce an Actions
artifact named `skiff-release-<version>`. They do not create tags or releases.
The macOS test installers are ad hoc signed, and the manifest says they are
not notarized. All packaging, source, feed, and checksum checks still run.

The workflow must be present on the default branch before GitHub exposes its
manual **Run workflow** button. Branch pushes exercise it before that merge.

## Create a release candidate

1. Merge and validate the intended source on `main`, including the release
   workflow and signing setup. Choose a new SemVer tag, such as `v0.1.0` or
   `v0.1.0-rc.1`; versions must fit Windows Installer's `255.255.65535` limit.
2. Tag that exact commit and push the tag:

   ```sh
   git tag -a v0.1.0 <validated-main-commit> -m 'Skiff 0.1.0'
   git push origin v0.1.0
   ```

3. The workflow rejects tags outside `main` history, stamps both Cargo manifests
   and lockfile without updating dependency versions, builds each architecture
   natively, notarizes macOS, and uploads all downloads to a draft. Prerelease
   tags mark the draft as a prerelease. A failed platform blocks the draft.
4. Download and validate installation, upgrades, and uninstall on supported
   operating systems. Verify `SHA256SUMS`, then publish the draft in GitHub.
   Every download also has a build provenance attestation, which the draft
   job verified before uploading. Anyone can check one; name the release
   workflow and tag, or an attestation from any workflow or branch of the
   repository would pass:

   ```sh
   gh attestation verify <file> --repo shebashio/skiff \
     --signer-workflow shebashio/skiff/.github/workflows/release.yml \
     --source-ref refs/tags/v<version>
   ```

Re-running an interrupted draft upload replaces its assets; the pipeline refuses
to modify an already published release. Windows Installer and the macOS bundle
use the numeric version core, so test prereleases with the same core in clean
test environments rather than treating them as distinct installer upgrades.

Guest payloads are built on Linux and transferred as sparse archives; macOS
runners do not need Docker, and Windows runners do not need WSL. Windows bundle
hashes are embedded after all payload files have been assembled.

Never use Kiri's feeds, signing identity, installer identity, or domain for a
Skiff release. See [UPSTREAM.md](UPSTREAM.md) for the hosting-control decision
behind the fork. Preserve all license notices and the matching kernel source.
