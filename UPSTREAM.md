# Why Skiff exists

Skiff gives end users control over where their shared container endpoints
are hosted and which domains those endpoints use. Public sharing should work
through infrastructure and accounts selected by the person running the app.

We proposed an optional Cloudflare Tunnel provider to Kiri, including browser
authorization, automated tunnel and DNS provisioning, and custom domains:
[proposal #488](https://github.com/libracoder/kiri/issues/488) and
[PR #486](https://github.com/libracoder/kiri/pull/486).

The upstream maintainer initially declined that direction, explaining that
“sharing already goes through Kiri Share” and “Cloudflare Tunnel sharing
isn't planned.” The [maintainer's response](https://github.com/libracoder/kiri/pull/486#issuecomment-6955898130)
also states that Kiri is focusing on its container engine.

That initial decision prompted this fork. The maintainer later
[reconsidered](https://github.com/libracoder/kiri/pull/486#issuecomment-6968656134),
writing, “On second thought, this is a nice feature.” As of November 18, 2026,
the proposal remains closed and unmerged. We welcome the change in tone;
our reason for continuing independently is to make user-controlled sharing
infrastructure a core product commitment.

Skiff continues the work as an independent
application, with Cloudflare sharing enabled by default and no dependency on
Kiri's hosted sharing, update, or diagnostics services.

## The sharing comparison

As documented on November 18, 2026, [Kiri Share](https://docs.runkiri.com/share/)
requires a Kiri Cloud account, serves addresses under `runkiri.cloud`, and
routes public traffic through Kiri's edge. Sharing plans set concurrent-link,
speed, monthly-transfer, and reserved-name limits; some plans show a Kiri Cloud
notice before the shared app. The [Kiri Cloud documentation](https://docs.runkiri.com/kiri-cloud/)
also describes a forthcoming browser app for managing that account.

[Skiff's default sharing](docs/cloudflare-sharing.md) uses Cloudflare directly:
temporary Quick Tunnels without an account, or tunnels and DNS in the user's
Cloudflare account under their own domain. No hosted Skiff account or Skiff
sharing plan is required. Cloudflare's limits and terms still apply. Both
projects support local containers without a cloud account.

This comparison concerns the documented sharing products, not Kiri's license
or its maintainer's private intentions. It does not assert that Kiri currently
charges for sharing or that its roadmap cannot change.

## Code provenance

Skiff derives from [Kiri](https://github.com/libracoder/kiri), including upstream
commit `1bfdaa22cd1b9ae0ad7fda8c9fd7eb2d9cc5c8ad`, plus the Cloudflare and native
build work in the SHE BASH fork. Git history is retained. The upstream code
is licensed under Apache-2.0; original notices are retained in [NOTICE](NOTICE)
and [LICENSE](LICENSE). The guest Linux kernel has its own GPL-2.0 source offer.

Skiff has a new name, icon, application identity, and installation paths.
Historical release notes under `docs/releases` and the saved benchmark data
describe upstream Kiri releases and measurements, not published Skiff releases.
