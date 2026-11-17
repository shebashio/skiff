export const repository = "https://github.com/shebashio/skiff";
export const previewBuilds = `${repository}/actions/workflows/native-builds.yml?query=branch%3Amain+event%3Apush`;

export function normalizeDomain(input) {
  const domain = input.trim().toLowerCase().replace(/\.$/, "");
  if (
    domain.length > 245 ||
    !domain.includes(".") ||
    /^\d+(?:\.\d+){3}$/.test(domain)
  )
    return null;
  const labels = domain.split(".");
  if (
    !labels.every((label) =>
      /^[a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?$/.test(label),
    )
  )
    return null;
  if (!/^[a-z]{2,63}$/.test(labels.at(-1))) return null;
  return domain;
}

export function releaseDownload(releases, platform, arch) {
  if (!Array.isArray(releases) || !["arm64", "x86_64"].includes(arch))
    return null;
  const release = releases.find(
    (item) =>
      item &&
      !item.draft &&
      !item.prerelease &&
      /^v\d+\.\d+\.\d+$/.test(item.tag_name),
  );
  if (!release) return null;
  const version = release.tag_name.slice(1);
  const name =
    platform === "mac"
      ? `Skiff-${version}-${arch}.dmg`
      : platform === "windows"
        ? `Skiff-${version}-${arch}.msi`
        : platform === "linux"
          ? `skiff_${version}_${arch === "x86_64" ? "amd64" : "arm64"}.deb`
          : null;
  if (!Array.isArray(release.assets)) return null;
  const asset = release.assets.find(
    (item) => item.name === name && item.state === "uploaded",
  );
  const expected = `${repository}/releases/download/${release.tag_name}/${name}`;
  if (!asset || asset.browser_download_url !== expected) return null;
  return { url: expected, version, name };
}

export function recipes(domain = "example.com") {
  return {
    run: {
      comment: "# With Skiff installed and the engine running",
      code: "docker --context skiff run -d \\\n  --name web -p 8080:80 nginx:alpine\n\nskiff share 8080",
      result: "Your local web server, ready for a public preview.",
    },
    compose: {
      comment: "# From a project with a compose.yaml file",
      code: "docker context use skiff\ndocker compose up -d\ndocker compose logs -f",
      result: "Your stack, your Compose file, a different engine.",
    },
    share: {
      comment: "# With cloudflared installed and port 3000 running",
      code: `skiff share domain connect ${domain}\nskiff share domain create ${domain} preview\nskiff share 3000 --name preview`,
      result: `Your local service → https://preview.${domain}`,
    },
  };
}
