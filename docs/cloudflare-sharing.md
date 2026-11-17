# Share through Cloudflare Tunnel

Cloudflare is the default provider for both `skiff share` and the app's Share buttons. Skiff
starts the installed `cloudflared` for each share, shows its HTTPS URL, and
closes it on Disconnect, Stop, or when its owning Skiff process exits.
It does not require signing in to a hosted Skiff service.

## Start before choosing a domain

Install [cloudflared](https://developers.cloudflare.com/tunnel/downloads/)
on the computer running Skiff, then:

```sh
skiff share provider cloudflare
skiff share 3000
```

On macOS, Skiff also discovers Homebrew installations in `/opt/homebrew/bin`
and `/usr/local/bin` when launched from Finder. An explicitly configured
`cloudflare.executable` path takes precedence.

With no base domain configured, each share gets a temporary
`https://<random>.trycloudflare.com` URL. A requested name is ignored in this
mode. A Cloudflare account and domain are not required. These are
[Quick Tunnels](https://developers.cloudflare.com/tunnel/get-started/quick-tunnels/),
intended for development, subject to Cloudflare's limitations, and with a
new address on each restart.

The selection is saved in `~/.skiff/share.json` (`%USERPROFILE%\.skiff\share.json`
on Windows). `SKIFF_SHARE_CONFIG` selects a different file. Skiff Dev uses its
own config. An app opened from Finder may need an absolute executable path:

```json
{
  "provider": "cloudflare",
  "cloudflare": {
    "executable": "/opt/homebrew/bin/cloudflared"
  }
}
```

Merge these fields into an existing config, keeping its other fields.
On Windows use an escaped absolute path, for example
`"C:\\Program Files\\cloudflared\\cloudflared.exe"`.

## Use your domain later

Choose a domain already managed in your Cloudflare account. In the app:

1. Open **Share → Custom domain** and select **Custom domain** as the link type.
2. Enter your base domain and a share name. A container named `web` uses
   `web.example.com` when the base domain is `example.com`.
3. Select **Connect Cloudflare**. Approve the domain in the Cloudflare browser
   page that opens; Skiff receives the authorization automatically. If the
   browser did not open, use **Open Cloudflare** in the dialog.
4. Select **Create public address**. Skiff creates a locally managed tunnel,
   stores its credentials privately, creates its proxied DNS record, and saves
   the route. There is no JSON file to copy or DNS record to enter by hand.
5. Press **Share** on the container to start serving that address.

**Create public address** also starts browser authorization if needed, so
the separate Connect step is optional. Reconnect to authorize the domain
again if permission has expired or you selected the wrong zone. Signing in
uses Cloudflare's [browser authorization](https://developers.cloudflare.com/tunnel/features/locally-managed-tunnels/create-local-tunnel/).
Skiff does not receive your password. It runs cloudflared with a separate
per-domain home under `cloudflare-managed`, beside its sharing config, so
your existing cloudflared login and configuration are not overwritten.

The equivalent CLI flow is:

```sh
skiff share domain connect example.com
skiff share domain create example.com web
skiff share 3000 --name web
```

`connect` and `create` accept `--json` for progress and completion events.
Closing the dialog or pressing **Cancel setup** stops the active operation.
If DNS creation fails, Skiff keeps the tunnel credentials and reuses that
tunnel on retry. It does not overwrite a conflicting DNS record. Any tunnel
created before a cancellation remains in your Cloudflare account; retrying
the same domain and share name reuses it when its credentials were saved.
The Cloudflare dashboard can remove resources you no longer want.

The preview app uses its own settings. For that app's CLI, use
`~/.skiff-cloudflare/bin/skiff`; the normal `skiff` command configures the
installed app instead.

### Advanced: import an existing tunnel

Manual import remains available for tunnels you manage yourself. Provision one **locally managed** tunnel
for each share name. For example, after replacing `example.com` with your
domain:

```sh
cloudflared tunnel login
cloudflared tunnel create skiff-web
cloudflared tunnel route dns skiff-web web.example.com
```

The create command prints a tunnel UUID and creates its credentials JSON.
Skiff can import that file without editing `share.json`:

```sh
skiff share domain add example.com web ~/.cloudflared/<TUNNEL-UUID>.json
skiff share 3000 --name web
```

Replace `<TUNNEL-UUID>` with the ID printed by `cloudflared`. The command
selects Cloudflare, reads the tunnel ID from its credentials, and prints the
hostname and proxied CNAME target to configure in Cloudflare DNS. It validates
the domain, share name, credential file and duplicate tunnel IDs before
saving. Existing settings, including the cloudflared executable path, are
preserved. Each additional share needs its own tunnel and DNS record:

```sh
skiff share domain add example.com api ~/.cloudflared/<OTHER-TUNNEL-UUID>.json
skiff share domain list --json
skiff share domain remove api
skiff share domain clear   # return to temporary links
```

`remove` removes one local mapping; it keeps the base domain, so a missing
mapping cannot silently create a temporary public URL. `clear` removes all
local domain mappings and returns to temporary links. Both keep credentials
and Cloudflare resources. Existing links retain their routing until stopped;
saved settings apply to new shares. To change the base domain in the CLI,
clear it first, then add the new routes.

### Import in the app

In **Share → Custom domain**, enter the base domain, expand
**Advanced: import or edit tunnel routes**, and select **Add tunnel**.
Enter the share name (the container's name for the
app's Share button) and the path to its credentials JSON. Absolute paths and
`~/.cloudflared/...` paths are supported. Add each tunnel, then select
**Save imported routes**. Advanced shows the proxied CNAME records to
create in Cloudflare. It also lets you edit/remove routes or return to
temporary links. Validation errors leave your edits visible and the saved
configuration intact.

### Configure the file directly

You can also add the UUID and absolute credentials path to Skiff's config:

```json
{
  "provider": "cloudflare",
  "cloudflare": {
    "executable": "/opt/homebrew/bin/cloudflared",
    "base_domain": "example.com",
    "tunnels": {
      "web": {
        "id": "01234567-89ab-cdef-0123-456789abcdef",
        "credentials_file": "/absolute/path/to/.cloudflared/01234567-89ab-cdef-0123-456789abcdef.json"
      }
    }
  }
}
```

Then `skiff share 3000 --name web` serves `https://web.example.com`. Sharing
a container named `web` from the app uses that entry automatically. For a
different container, port or Kubernetes service, configure its name or
specify an explicit name in the CLI. Missing entries fail with an explanation;
Skiff does not fall back to Custom relay or a temporary URL when a base domain
is configured.

Skiff generates an isolated ingress configuration for the selected origin,
with that exact hostname followed by a 404 catch-all. Automatic setup creates
the tunnel and DNS record; importing an existing tunnel leaves those steps
to you. Stopping a share or removing a local mapping does not delete the
Cloudflare tunnel or DNS record. Follow Cloudflare's
[local tunnel setup](https://developers.cloudflare.com/tunnel/features/locally-managed-tunnels/create-local-tunnel/)
for the account and DNS steps. If you choose a nested base such as
`apps.example.com`, ensure Cloudflare has an edge certificate covering
`web.apps.example.com`; a certificate for `*.example.com` does not cover it.

Each named share needs a distinct UUID. Do not run the same tunnel elsewhere
or as a background service: Cloudflare treats those processes as replicas
and may route a visitor to either. Skiff locks beside the credentials file to
prevent concurrent use of that file on this computer; that folder must be
writable. Do not copy the credentials to bypass the lock. Remotely managed
tunnel tokens are not supported by this integration.

## Manage shares

```sh
skiff share status --json
skiff share list --json
skiff share stop <ID-from-list>
skiff share provider relay          # use your explicitly configured custom relay
```

The app shows Cloudflare shares opened by the CLI and app using the same
config directory, and its Stop action closes those shares on this computer.
Switching providers only changes new shares. Existing links from either
provider remain listed and can be stopped; Cloudflare IDs in the list use
a `cloudflare:` prefix. Local Cloudflare links remain available even when
signed out of Custom relay or when its API fails. With Cloudflare selected,
an expired relay login or an unreachable relay does not produce an error;
reachable relay links are still included for cleanup. With the custom relay selected,
authentication and network failures report a warning. Older edges without
tunnel discovery disable remote polling without an error, while local
Cloudflare discovery continues. Other computers' Cloudflare tunnels are not listed. Cloudflare's
own reconnect logic handles transient network failures; Skiff observes its readiness endpoint. Startup times out
after 90 seconds. No background tunnel is installed as a system service.
Terminal Ctrl+C closes the tunnel and removes its private runtime directory.

Cloudflare forwards HTTP and WebSocket traffic. This integration does not
provide Skiff's per-request logs, traffic counters, Cloudflare account quotas,
or Custom relay name reservation. Skiff's UI marks request statistics unavailable.
The selected provider affects sharing; Skiff's update and diagnostic-report
settings remain separate.

## Development checks

```sh
cargo build -p skiff
cargo test -p skiff share::
cargo build --locked -p skiff --example fake-cloudflared
python3 tests/cloudflare-share.py target/debug/skiff target/debug/examples/fake-cloudflared
python3 scripts/ui-test.py
```

The lifecycle integration test uses a local fake cloudflared; it creates no
public tunnel and needs no account. It covers browser authorization, tunnel
and DNS creation, retries, cancellation, sharing, provider switches,
environment-token isolation, executable discovery, and process termination
(including terminal Ctrl+C on Unix). On Windows use `python` and append
`.exe` to both binary paths. The native fixture uses the same subprocess
supervisor as real cloudflared on each operating system.
Settings changes preserve the saved Skiff login; `SKIFF_SHARE_TOKEN` overrides
authentication only for the current process and is never saved by those
changes. A real custom-domain test requires authorizing a domain in your
Cloudflare account.

## Custom relays

A compatible relay is optional and must be configured explicitly with
`SKIFF_SHARE_SERVER` or the `server` field in `share.json`. Skiff supplies no
default hosted relay, billing service, report endpoint, or notice page.
Relay features depend on the operator's implementation. Cloudflare is the
default and needs no relay configuration.
