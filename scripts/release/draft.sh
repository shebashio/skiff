#!/bin/bash
set -euo pipefail
tag="v${VERSION:?}"
test "$GITHUB_REF" = "refs/tags/$tag"
cd dist
sha256sum --check SHA256SUMS
if gh release view "$tag" --repo "$GITHUB_REPOSITORY" --json isDraft > "$RUNNER_TEMP/skiff-release-state.json" 2>/dev/null; then
  python3 -c 'import json,sys; sys.exit(0 if json.load(open(sys.argv[1]))["isDraft"] else "Refusing to modify a published release")' "$RUNNER_TEMP/skiff-release-state.json"
else
  flags=()
  if [[ "$VERSION" == *-* ]]; then flags+=(--prerelease); fi
  gh release create "$tag" --repo "$GITHUB_REPOSITORY" --verify-tag --draft --title "Skiff $VERSION" --notes-file release-notes.md "${flags[@]}"
fi
# Re-running a failed draft upload is safe; published releases are immutable here.
gh release upload "$tag" --repo "$GITHUB_REPOSITORY" --clobber -- *.dmg *.tar.gz *.msi *.zip *.deb *.rpm *.tar.xz *.json LICENSE NOTICE SHA256SUMS
