#!/bin/bash
# Keep credentials in Keychain. This file loads only the local certificate
# selector and notarytool profile name, then runs the existing build tasks.
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$root"
if [[ "$(uname -s)" != Darwin ]]; then
  echo 'Apple signing commands must run on macOS.' >&2
  exit 1
fi
if [[ -f .env.signing.local ]]; then
  set -a
  source .env.signing.local
  set +a
fi
task="${1:-check}"
if (( $# )); then shift; fi
case "$task" in
  init)
    if [[ ! -f .env.signing.local ]]; then
      (umask 077; cp .env.signing.example .env.signing.local)
    fi
    echo 'Local signing configuration: .env.signing.local'
    echo 'Next: install Developer ID Application in Keychain, then run credentials and check.'
    ;;
  credentials)
    # Leave credentials unspecified so Apple's tool prompts in the terminal.
    # Never use --password here or echo the credential values.
    args=(notarytool store-credentials "${SKIFF_NOTARY_PROFILE:-skiff-notary}")
    if [[ -n "${SKIFF_APPLE_ID:-}" ]]; then args+=(--apple-id "$SKIFF_APPLE_ID"); fi
    if [[ -n "${SKIFF_APPLE_TEAM_ID:-}" ]]; then args+=(--team-id "$SKIFF_APPLE_TEAM_ID"); fi
    exec xcrun "${args[@]}"
    ;;
  check) exec cargo xtask signing-check "$@" ;;
  sign|release|pack) exec cargo xtask "$task" "$@" ;;
  *) echo 'Usage: scripts/macos-signing.sh [init|credentials|check|sign|release|pack] [--arch x86_64]' >&2; exit 2 ;;
esac
