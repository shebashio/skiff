# Why Skiff exists

Skiff gives end users control over where their shared container endpoints
are hosted and which domains those endpoints use. Public sharing should work
through infrastructure and accounts selected by the person running the app.

We proposed an optional Cloudflare Tunnel provider to Kiri, including browser
authorization, automated tunnel and DNS provisioning, and custom domains:
[proposal #488](https://github.com/libracoder/kiri/issues/488) and
[PR #486](https://github.com/libracoder/kiri/pull/486).

The upstream maintainer declined that direction, explaining that
“sharing already goes through Kiri Share” and “Cloudflare Tunnel sharing
isn't planned.” The [maintainer's response](https://github.com/libracoder/kiri/pull/486#issuecomment-6955898130)
also states that Kiri is focusing on its container engine.

That decision left our requirement for user-controlled sharing infrastructure
outside upstream's plans. Skiff continues the work as an independent
application, with Cloudflare sharing enabled by default and no dependency on
Kiri's hosted sharing, update, or diagnostics services.

## Code provenance

Skiff derives from [Kiri](https://github.com/libracoder/kiri), including upstream
commit `1bfdaa22cd1b9ae0ad7fda8c9fd7eb2d9cc5c8ad`, plus the Cloudflare and native
build work in the SHE BASH fork. Git history is retained. The upstream code
is licensed under Apache-2.0; original notices are retained in [NOTICE](NOTICE)
and [LICENSE](LICENSE). The guest Linux kernel has its own GPL-2.0 source offer.

Skiff has a new name, icon, application identity, and installation paths.
Historical release notes under `docs/releases` and the saved benchmark data
describe upstream Kiri releases and measurements, not published Skiff releases.
