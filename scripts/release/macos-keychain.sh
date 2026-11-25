#!/bin/bash
# Only run on an ephemeral GitHub-hosted runner. Never source local credentials.
set -euo pipefail
test "${GITHUB_ACTIONS:-}" = true
keychain="$RUNNER_TEMP/skiff-signing.keychain-db"
certificate="$RUNNER_TEMP/skiff-signing.p12"
if [[ ${1:-} == cleanup ]]; then
  [[ ! -f "$keychain" ]] || security delete-keychain "$keychain"
  rm -f "$certificate"
  exit 0
fi
for name in APPLE_CERTIFICATE_P12_BASE64 APPLE_CERTIFICATE_PASSWORD APPLE_ID APPLE_TEAM_ID APPLE_APP_SPECIFIC_PASSWORD; do
  [[ -n ${!name:-} ]] || { echo "Missing release environment secret: $name" >&2; exit 1; }
done
umask 077
printf '%s' "$APPLE_CERTIFICATE_P12_BASE64" | base64 --decode > "$certificate"
keychain_password=$(openssl rand -hex 32)
echo "::add-mask::$keychain_password"
security create-keychain -p "$keychain_password" "$keychain"
security set-keychain-settings -lut 21600 "$keychain"
security unlock-keychain -p "$keychain_password" "$keychain"
# -x: the private key can sign but can never be exported again, even by a
# process that runs while this keychain is unlocked. Build steps run before
# this script (`cargo xtask release-prep`), so little else runs meanwhile.
security import "$certificate" -k "$keychain" -P "$APPLE_CERTIFICATE_PASSWORD" -x -T /usr/bin/codesign
rm -f "$certificate"
security set-key-partition-list -S apple-tool:,apple:,codesign: -s -k "$keychain_password" "$keychain" >/dev/null
security list-keychains -d user -s "$keychain"
security default-keychain -d user -s "$keychain"
# notarytool's password option is masked by Actions; it is never written to disk.
xcrun notarytool store-credentials skiff-release --apple-id "$APPLE_ID" --team-id "$APPLE_TEAM_ID" --password "$APPLE_APP_SPECIFIC_PASSWORD" --keychain "$keychain"
echo 'SKIFF_NOTARY_PROFILE=skiff-release' >> "$GITHUB_ENV"
