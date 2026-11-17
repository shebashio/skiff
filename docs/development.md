# Development

Building Skiff from source, running it, and how it fits together.

For a distributable macOS build, follow [Apple signing and notarization](macos-signing.md).
`scripts/macos-signing.sh check` verifies the local certificate and Keychain
profile before signing or building a release.

## Build

Needs Rust and, for the kernel and disk template, a Docker engine. Skiff can
build its own kernel: `DOCKER_CONTEXT=skiff cargo xtask kernel`.

```sh
cargo xtask kernel    # guest/out/Image
cargo xtask disk      # guest/out/data.img (sparse 64 GiB ext4 template)
cargo xtask guest     # guest/out/initramfs.cpio
cargo xtask host      # target/release/skiff, signed with the virtualization entitlement
```

For Intel Macs add `--arch x86_64` to `kernel`, `guest`, `host`, `app`,
`dmg` and `release`: an x86-64 guest in `guest/out/x86_64` (a `bzImage`,
cross-compiled on Apple Silicon), the host for `x86_64-apple-darwin`
(`rustup target add x86_64-apple-darwin x86_64-unknown-linux-musl`), and
`target/intel/Skiff.app` with Intel builds of the docker CLI, Compose and
buildx, released as `Skiff-<version>-x86_64.dmg`/`.tar.gz` with its own feed,
`latest-mac-x86_64.json`. Each Mac gets its own app rather than one universal
app, which would carry both builds of the docker tools. On Intel there is
no Rosetta for Linux: amd64 images run natively, and images with no amd64
build are refused.

On Windows, `cargo xtask windows-dev` builds the Linux engine, Skiff Linux and
Docker tools into `target/skiff-windows-x86_64` with a debug `skiff-win.exe`.
`cargo run -p skiff-win` uses that bundle automatically. Use `cargo xtask
windows` for the release bundle and `cargo xtask msi` for the installer.
Add `--arch arm64` to any of the three for Windows on Arm
(`target/skiff-windows-aarch64`, `Skiff-<version>-arm64.msi`); testing it in
a VM on a Mac is in [docs/testing-windows-arm.md](docs/testing-windows-arm.md).

### Native Linux and Windows builds

The `Native builds` GitHub Actions workflow compiles on Linux and Windows,
on both x64 and ARM64 hosts. Each job builds release binaries, runs the
sharing unit tests, and exercises the real CLI and subprocess supervisor
against a local cloudflared fixture. No Cloudflare account is needed.

On Ubuntu 24.04, install Rust and the desktop development libraries:

```sh
sudo apt-get install build-essential pkg-config libwebkit2gtk-4.1-dev
cargo build --release --locked -p skiff -p skiff-init --features skiff/window
```

This produces `target/release/skiff` (including `skiff ui`) and `skiff-init`.
The desktop binary needs GTK 3 and WebKitGTK 4.1 at runtime. For an engine
without desktop dependencies, omit `--features skiff/window`; Linux release
packaging uses a separate static musl engine build.

On Windows, install Rust's native MSVC toolchain and Visual Studio's C++
build tools, including the Windows SDK. Windows on Arm also needs LLVM
(ring's assembly build uses clang). Then run:

```powershell
cargo build --release --locked -p skiff -p skiff-win
```

This produces `target/release/skiff.exe` and `skiff-win.exe`. Cloudflare sharing
runs on Windows itself; the container engine still needs the WSL2 bundle
described above. `cloudflared` is a separate runtime prerequisite on both
platforms; see [Cloudflare sharing](cloudflare-sharing.md).

Successful workflow runs attach `skiff-linux-x64`, `skiff-linux-arm64`,
`skiff-windows-x64`, and `skiff-windows-arm64` artifacts. They contain unsigned
development binaries and license files, not installers or complete Windows
bundles. These checks validate native compilation and sharing lifecycle;
interactive desktop, WSL2/container, and installer tests are separate.

## Use

```sh
./target/release/skiff daemon &      # serves ~/.skiff/run/docker.sock; the VM boots on first use
docker context create skiff --docker host=unix://$HOME/.skiff/run/docker.sock
docker --context skiff run --rm alpine echo hello
docker buildx create --name skiff-build --driver docker-container \
  --driver-opt default-load=true --use skiff    # for docker build
./target/release/skiff bench         # Skiff vs OrbStack vs Docker Desktop
SOAK=600 tests/e2e.sh               # every check, then 600 runs watching for leaks
```

Docker's commands also work as `skiff` commands, always on Skiff whatever
docker context is selected: `skiff run`, `skiff ps`, `skiff exec`, `skiff logs`,
`skiff build`, `skiff compose up` and the rest run the docker CLI against
Skiff's socket. (`skiff import` is Skiff's own: it brings images, volumes and
containers over from OrbStack or Docker Desktop.) The daemon-free tools
that boot their own guest are `skiff raw run|pull|images|rmi`.

`skiff` keeps its data in `~/Library/Application Support/Skiff` (override with
`SKIFF_DATA_DIR`). It does not touch `/usr/local/bin/docker`.
`/var/run/docker.sock` changes only when you ask: `skiff setup --docker-socket`
(or the password prompt for `*.skiff.local` names) links it to Skiff's socket
when no other engine has it, after macOS asks for an administrator password,
and installs `/Library/LaunchDaemons/io.shebash.skiff.docker-socket.plist` to put the
link back after a restart. A socket another engine owns is never replaced.

## Architecture

```
docker CLI ─┐
compose ────┼─ ~/.skiff/run/docker.sock ─► skiff daemon ── vsock ──► guest agent (PID 1)
buildx ─────┤     Docker Engine API          owns the VM            clone3 → container
skiff UI ────┘
```

- **VM**: Apple's Virtualization.framework, driven from Rust through objc2.
  A custom 10 MB kernel (`guest/kernel/`) built from allnoconfig, with one
  patch that cuts virtio-pci reset waits from a scheduler tick to
  microseconds, one that hands big TCP to the host as GSO, one that keeps
  newly created FUSE files owner-readable, and an upstream GRO fix that keeps
  forwarded packets segmentable. Rosetta is shared in over virtiofs and registered with
  binfmt_misc.
- **Guest agent**: a ~900 KB static Rust binary that is PID 1. One `poll()`
  loop serves the host, reaps children, forwards container output, relays
  ports, answers DNS and runs DHCP. `docker cp` and friends run in forked
  children so a large copy never stalls it.
- **Containers**: one `clone3` straight into new namespaces and the
  container's cgroup (`CLONE_INTO_CGROUP`), overlayfs over layers unpacked
  on an ext4 data disk, `pivot_root`, capabilities and a seccomp filter.
  Networking is set up with hand-written rtnetlink and nf_tables messages.
  No runc, no shim, no dockerd.
- **Images**: pulled and pushed on the Mac (`crates/skiff-image`), streamed
  into the guest once and unpacked there.

## Layout

| Path | What |
|------|------|
| `crates/skiff` | the `skiff` binary: daemon, Docker API, CLI, UI, bench |
| `crates/skiff-vmm` | Virtualization.framework VM |
| `crates/skiff-init` | guest PID 1, agent and container runtime |
| `crates/skiff-proto` | host ↔ guest wire protocol (no_std, no allocation per message) |
| `crates/skiff-image` | image references, registry client, blob store, archives |
| `guest/kernel` | kernel config and patches |
| `tests/e2e.sh` | ~350 checks through the real `docker` CLI, plus a leak soak (`K8S=1` adds Kubernetes) |
| `tests/linux-host.sh` | the Linux host build, driven in a privileged container |
| `tests/finder-mount.sh` | the Mac's Finder mount at `~/Skiff`: covers only an empty folder or its own old mount, never your files or another mount |
| `contrib/` | systemd unit, WSL2 and Windows install scripts |
| `installer/windows` | the Windows MSI (WiX 5): `skiff.wxs`, licence, dialog bitmaps |
| `packaging/` | Linux `.deb` and `.rpm` (`skiff` with the docker CLI, `skiff-lite` engine only), the Homebrew plan |
| `xtask` | build tasks |
| `bench/results.json` | measured results; the website keeps a copy |
| `brand/` | brand (the website and raw benchmark output live outside this repository) |

## Unexpected VM stops

The daemon keeps the guest's serial console, kernel oopses included, in
`~/.skiff/logs/guest-console.log` (Skiff Dev: in its own state folder). The file and two
rotations (`.1`, `.2`) hold at most 1 MiB each. Files are private, credential
patterns are redacted, and lines over 16 KiB are omitted. The unexpected-stop
log line names this file. The console stays local; crash reports do not send it.

After a guest crash, Skiff automatically boots a new VM when containers were
running with `always` or `unless-stopped`, then restarts them and increments
their `RestartCount`. Crashes in a row back off from 100 ms to one minute;
a VM that ran ten seconds starts the backoff over. A manual stop, kill or
removal wins, including during that wait, and so does Skiff's Stop (Start
brings them back); `no`, `on-failure` and containers that never ran stay
down. If the new VM does not boot, recovery tries again, waiting longer
each time up to a minute.

Before a replacement opens the data disk, Skiff confirms that the previous
VM has stopped and releases its resources, even if relay threads still
hold old links. It allows ten seconds for shutdown, then up to ten seconds
for a confirmed forced stop. Errors and timeouts keep the previous VM
owned and block replacement; recovery retries with backoff. This wait does
not hold container lifecycle locks, so stop, kill and removal can cancel
pending starts. Linux gives each guest process its own socket directory,
so late destruction of an old link cannot remove its successor's sockets.

The real-VM regression is an opt-in ignored test:
`vm_loss_live_recovers_without_a_request`. Run its signed test binary with
an isolated `HOME` and `SKIFF_DATA_DIR` containing `alpine:latest` and no
containers, `SKIFF_GUEST_DIR` pointing to matching guest assets, and
`SKIFF_LIVE_VM_RECOVERY=1`. It force-stops only the VM it creates.
