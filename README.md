<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="brand/logo/skiff-horizontal-dark.svg">
    <img src="brand/logo/skiff-horizontal-light.svg" alt="Skiff" width="240">
  </picture>
</p>

<p align="center"><strong>Your containers. Your infrastructure. Your links.</strong></p>

<p align="center"><a href="https://runskiff.com">runskiff.com</a> · <a href="docs/development.md">Build Skiff</a> · <a href="docs/cloudflare-sharing.md">Sharing guide</a></p>

Skiff is a lightweight container runtime and desktop app for macOS, Linux,
and Windows. Run containers with familiar Docker commands, manage them in a
native window, and share them through your own Cloudflare account and domains.

<p align="center"><img src="brand/icon/png/skiff-app-icon-256.png" alt="Skiff application icon" width="160"></p>

## Why Skiff

- **Control your sharing:** temporary Cloudflare links or stable addresses on
  your domain, with browser authorization and automatic tunnel/DNS setup.
- **Use familiar tools:** Docker CLI, Compose, buildx and a desktop interface.
- **Stay lightweight:** Rust runtime, direct Linux container execution,
  Apple Virtualization.framework on macOS, and WSL2 on Windows.
- **Keep ownership:** separate local data and credentials, explicit relay
  configuration, and no default dependency on upstream hosted services.
- **Build natively:** Linux and Windows x64/ARM64 CI builds, sharing unit tests,
  and the same local lifecycle suite on all four targets.

## Why this fork exists

Skiff began after upstream declined an optional sharing provider that would
let end users control the hosting and base domains of their public endpoints.
The maintainer explained that “sharing already goes through Kiri Share” and
“Cloudflare Tunnel sharing isn't planned.” Skiff continues that work as an
independent application with user-controlled sharing infrastructure as a core
feature. [Read the proposal, response, and code attribution](UPSTREAM.md).

## Try it

Skiff is currently a development build. [GitHub Actions](https://github.com/shebashio/skiff/actions/workflows/native-builds.yml)
provides unsigned Linux and Windows binaries; complete signed installers are
not yet published. Windows container workloads need the WSL2 bundle, and the
Linux desktop needs GTK 3 and WebKitGTK 4.1. See [build instructions](docs/development.md).

```sh
# Build the CLI on the current platform
cargo build --release --locked -p skiff

# Share a local HTTP service; install cloudflared first
skiff share 3000

# Connect your account and create a stable public address
skiff share domain connect example.com
skiff share domain create example.com demo
skiff share 3000 --name demo
```

On macOS, `cargo xtask app-dev` builds `target/dev/Skiff Dev.app` after the
kernel, guest and disk assets have been built. On Linux, add
`--features skiff/window` for the desktop. On Windows, build `-p skiff-win`
for the desktop and use `cargo xtask windows-dev` for the complete bundle.

Once the engine is running, use the `skiff` Docker context:

```sh
docker --context skiff run --rm alpine echo hello
skiff compose up -d
```

[Cloudflare sharing](docs/cloudflare-sharing.md) ·
[Moving from Kiri](docs/skiff-transition.md) ·
[Development](docs/development.md) ·
[Contributing](CONTRIBUTING.md)

## License and provenance

Skiff is maintained by SHE BASH and derives from Kiri under
[Apache-2.0](LICENSE). Its independent direction stems from the upstream
sharing-hosting decision described in [UPSTREAM.md](UPSTREAM.md).
Original and third-party notices are retained in [NOTICE](NOTICE).
The guest Linux kernel is GPL-2.0; see its [source offer](guest/kernel/SOURCE_OFFER.md).
Skiff uses its own name, marks, application identities, and installation paths.
