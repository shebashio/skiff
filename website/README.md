# runskiff.com

Skiff’s product site, deployed as Cloudflare Workers Static Assets in SHE BASH LLC’s account. The source lives alongside the runtime so claims, docs, and downloads can evolve together.

## Development

```sh
cd website
npm ci
npm run check
npm run dev
```

Open http://127.0.0.1:8788. There is no frontend build step or framework runtime: HTML, CSS, and two small JavaScript modules are served directly. Fonts and artwork are local. The product window and domain form are labeled illustrations, not connections to a running engine or Cloudflare account.

`npm run check` verifies local assets, anchors and ARIA references, social image dimensions, domain validation, and trusted release artifact selection. It does not replace browser testing. Check the product tabs, domain validation, command copy, platform selectors, mobile menu, FAQ, and privacy dialog when making changes. Test widths of 320, 390, 768, and 1280 pixels and keyboard-only navigation.

## Deployment

```sh
npx wrangler login
npx wrangler whoami
npm run check
npm run deploy:check
npm run deploy
```

Verify the account is SHE BASH LLC before deploying. `wrangler.jsonc` pins the account, Worker name, and custom domain. Cloudflare manages DNS and TLS for the Worker custom domain. No Cloudflare credentials are stored in this repository or passed to browser code. The website validation workflow does not deploy or access account credentials.

For a change after launch, validate and review it before running the deploy command. Use Cloudflare’s deployment history to roll back to a known version if needed.

## Downloads and privacy

The download section checks the public GitHub releases API once when the section approaches the viewport. It only accepts a non-draft, non-prerelease `vX.Y.Z` release and an uploaded asset with an exact filename and GitHub URL in `shebashio/skiff`. It never constructs a download link for an asset that does not exist. Expected names follow `scripts/release/artifacts.py` in the release pipeline: DMG for macOS, MSI for Windows, and DEB for Linux (RPM remains available through release notes).

When the API is unavailable or a matching installer is absent, the page links to the checked-in `native-builds.yml` workflow, source instructions, and all releases. The native workflow publishes Linux and Windows previews; macOS can be built from source until its release is available. GitHub Actions artifacts require login. Validation checks that the static and dynamic preview links agree and that the referenced workflow exists. Keep release status and signing claims consistent with the release workflow.

There are no analytics scripts, cookies, forms that send user input, or remote fonts. Only the optional public GitHub release lookup makes a third-party request. The privacy dialog explains normal host and GitHub connection metadata. `_headers` restricts scripts, styles, fonts, images, and connections with a Content Security Policy.

## Copy and attribution

Keep product claims grounded in the code and documentation. Do not publish unmeasured performance comparisons or infer private monetization plans. The independent-project explanation links to `UPSTREAM.md`, including the maintainer’s documented decision not to accept user-controlled Cloudflare sharing. Retain the repository’s Apache-2.0 attribution.

Skiff artwork comes from `brand/`. Manrope and IBM Plex Mono font licenses are included under `public/assets/`. The 1200 × 630 social image source is `scripts/social.svg`; regenerate it with `rsvg-convert scripts/social.svg -o public/assets/social.png`.
