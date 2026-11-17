# Moving to Skiff

Skiff is an independent application with its own installation and data. Its
CLI is `skiff`, environment variables use `SKIFF_`, Docker and Kubernetes
contexts use `skiff`, and local hostnames end in `.skiff.local` or
`.skiff.localhost`. State is stored under `~/.skiff`; the macOS data folder is
`~/Library/Application Support/Skiff`. Windows installs under
`%LOCALAPPDATA%\Skiff` and uses a new MSI UpgradeCode. Linux installs the
`skiff` systemd service and Skiff desktop entry.

Existing Kiri installations, settings, credentials, images, containers, and
volumes are not automatically moved or deleted. Start with an empty Skiff
installation, then use explicit image/volume export and import as needed.
Run one engine's local port router at a time: both may need ports 80 and 443.

Cloudflare is the default sharing provider. Install `cloudflared`, then use
Settings → Sharing or `skiff share domain connect` to authorize the account
you want Skiff to use. The new app does not copy Kiri's account credentials.
An optional compatible custom relay can be configured explicitly using
`SKIFF_SHARE_SERVER` or the `server` field in `~/.skiff/share.json`; select it
with `skiff share provider relay`.

Skiff has no default remote update feed. Updates require a source rebuild or
an explicitly configured `SKIFF_UPDATE_URL`; release packaging uses
`SKIFF_RELEASE_BASE` for a separately hosted release channel. Diagnostics
stay local unless a compatible relay has been configured and reporting is
explicitly requested or enabled.

See [Why Skiff exists](../UPSTREAM.md) for the sharing-hosting decision that
led to the fork and the upstream maintainer's response.
