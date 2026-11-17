import assert from "node:assert/strict";
import { readFile, access, stat } from "node:fs/promises";
import {
  normalizeDomain,
  releaseDownload,
  recipes,
  repository,
} from "../public/model.js";

const html = await readFile(
  new URL("../public/index.html", import.meta.url),
  "utf8",
);
const ids = [...html.matchAll(/\bid="([^"]+)"/g)].map((match) => match[1]);
assert.equal(ids.length, new Set(ids).size, "IDs must be unique");
for (const [, id] of html.matchAll(/href="#([^"]+)"/g))
  assert(ids.includes(id), `Missing anchor: ${id}`);
for (const [, attribute] of html.matchAll(
  /aria-(?:controls|labelledby|describedby)="([^"]+)"/g,
)) {
  for (const id of attribute.split(" "))
    assert(ids.includes(id), `Missing ARIA target: ${id}`);
}
for (const path of new Set(
  [...html.matchAll(/(?:src|href)="(\/[^"#]+)"/g)].map((match) => match[1]),
)) {
  await access(new URL(`../public${path}`, import.meta.url));
}
const social = await readFile(
  new URL("../public/assets/social.png", import.meta.url),
);
assert.equal(social.readUInt32BE(16), 1200);
assert.equal(social.readUInt32BE(20), 630);
assert(html.includes("https://runskiff.com/"), "Canonical domain is required");
assert(
  html.includes("Interactive example only"),
  "Domain demo must be clearly labeled",
);
assert(html.includes("UPSTREAM.md"), "Keep upstream attribution discoverable");
assert(
  (await stat(new URL("../public/site.js", import.meta.url))).size < 15000,
  "Keep the client script lightweight",
);
for (const [input, expected] of [
  ["Example.COM", "example.com"],
  ["demo.example.co.uk", "demo.example.co.uk"],
  [" example.com. ", "example.com"],
])
  assert.equal(normalizeDomain(input), expected);
for (const input of [
  "https://example.com",
  "example.com/path",
  "example.com:443",
  "-example.com",
  "x..com",
  "localhost",
  "127.0.0.1",
  "x.com; echo nope",
  "<script>.com",
  `${"a".repeat(64)}.com`,
])
  assert.equal(normalizeDomain(input), null, input);
const release = {
  tag_name: "v0.1.0",
  draft: false,
  prerelease: false,
  assets: [],
};
for (const platform of ["mac", "windows", "linux"]) {
  for (const arch of ["arm64", "x86_64"]) {
    const name =
      platform === "linux"
        ? `skiff_0.1.0_${arch === "arm64" ? "arm64" : "amd64"}.deb`
        : `Skiff-0.1.0-${arch}.${platform === "mac" ? "dmg" : "msi"}`;
    const asset = {
      name,
      state: "uploaded",
      browser_download_url: `${repository}/releases/download/v0.1.0/${name}`,
    };
    const fixture = { ...release, assets: [asset] };
    assert.equal(releaseDownload([fixture], platform, arch)?.name, name);
    assert.equal(
      releaseDownload([{ ...fixture, draft: true }], platform, arch),
      null,
    );
    assert.equal(
      releaseDownload([{ ...fixture, prerelease: true }], platform, arch),
      null,
    );
    assert.equal(
      releaseDownload(
        [
          {
            ...fixture,
            assets: [
              {
                ...asset,
                browser_download_url: "https://example.com/untrusted",
              },
            ],
          },
        ],
        platform,
        arch,
      ),
      null,
    );
  }
}
assert.equal(releaseDownload([], "mac", "arm64"), null);
assert.equal(releaseDownload(null, "mac", "arm64"), null);
assert.equal(releaseDownload([release], "windows", "x86_64"), null);
assert(
  recipes("example.com").share.code.includes(
    "skiff share domain create example.com preview",
  ),
);
console.log(
  "Passed: assets, anchors, accessibility references, metadata, domain validation, and release asset selection.",
);
