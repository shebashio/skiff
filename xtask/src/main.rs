//! Build tasks for Skiff. Run with `cargo xtask <task>`.
//!
//!   guest    build skiff-init for the guest and pack guest/out/initramfs.cpio
//!   kernel   build the guest kernel into guest/out/Image (needs a Docker engine)
//!   disk     make the empty data disk template guest/out/data.img (needs a Docker engine)
//!   host     build the `skiff` binary and sign it with the virtualization entitlement
//!   app      host + target/Skiff.app: the guest, the icon and a bundled docker CLI,
//!            Compose and buildx
//!   app-testflight  sandboxed app + bundled Cloudflare helper, in target/testflight/Skiff.app
//!   app-dev  app + target/dev/Skiff Dev.app: the same app as a second Skiff, with
//!            its own engine, containers and settings (crates/skiff/src/flavor.rs)
//!            and a DEV band on its icon; never notarized. `app-dev builds`
//!            makes Skiff Builds.app instead: one more, for one feature
//!   release  app + target/Skiff-<version>-arm64.tar.gz and its update feed
//!            (latest-mac-arm64.json)
//!   linux    `skiff` and `skiff-init` for a Linux host, in target/linux/release
//!            (built in a Linux container: needs a Docker engine, Skiff works)
//!   windows  target/skiff-windows-x86_64: the x86-64 Linux engine for WSL2,
//!            skiff-win.exe, the docker CLI, Compose and buildx, and the
//!            scripts (contrib/windows: skiff.ps1, setup-tools.ps1, skiff-admin.ps1)
//!   windows-dev  the same bundle with a debug skiff-win.exe for development
//!   windows-pack  assemble the release bundle using prebuilt Linux payloads
//!            in target/linux-<arch>-musl (CI can build these on Linux)
//!   msi      windows + target/Skiff-<version>-x86_64.msi, the per-user installer
//!            (installer/windows/skiff.wxs; needs the wix dotnet tool)
//!   msi-pack the MSI from the bundle already in target/skiff-windows-x86_64
//!            The four Windows tasks take `--arch arm64` for Windows on Arm:
//!            target/skiff-windows-aarch64 and Skiff-<version>-arm64.msi
//!   docker-win-arm64  only the arm64 docker.exe, built from docker/cli's
//!            source (Docker publishes none), checked against its pin
//!   all      guest + host
//!
//! `--arch x86_64` (or `intel`) makes guest, kernel, host, app, app-lite,
//! dmg, release and pack build Skiff for Intel Macs instead: an x86-64 guest
//! in guest/out/x86_64, the host binary for x86_64-apple-darwin, and
//! target/intel/Skiff.app with Intel builds of the docker CLI, Compose and
//! buildx; its DMG, tarball and feed carry `-x86_64` in their names
//! (latest-mac-x86_64.json), as Apple silicon's carry `-arm64`.

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

mod signing;

/// Which Mac a build is for. Skiff ships one app per architecture rather
/// than a universal one: the bundled docker CLI, Compose and buildx are most
/// of the download, and a universal app would carry both builds of each.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Arch {
    Arm64,
    X86_64,
}

impl Arch {
    /// `--arch x86_64` (`amd64`, `intel`) anywhere after the task; arm64 otherwise.
    fn from_args() -> Arch {
        let args = std::env::args().skip(2).collect::<Vec<_>>();
        let intel = args.windows(2).any(|w| w[0] == "--arch" && matches!(w[1].as_str(), "x86_64" | "amd64" | "intel"));
        if intel { Arch::X86_64 } else { Arch::Arm64 }
    }

    fn guest_target(self) -> &'static str {
        match self {
            Arch::Arm64 => "aarch64-unknown-linux-musl",
            Arch::X86_64 => "x86_64-unknown-linux-musl",
        }
    }

    fn host_target(self) -> &'static str {
        match self {
            Arch::Arm64 => "aarch64-apple-darwin",
            Arch::X86_64 => "x86_64-apple-darwin",
        }
    }

    /// The kernel file in guest/out and in the app (crates/skiff/src/guest.rs).
    fn kernel(self) -> &'static str {
        match self {
            Arch::Arm64 => "Image",
            Arch::X86_64 => "bzImage",
        }
    }

    /// guest/out for Apple Silicon, guest/out/x86_64 for Intel.
    fn guest_out(self, root: &Path) -> io::Result<PathBuf> {
        let out = match self {
            Arch::Arm64 => root.join("guest/out"),
            Arch::X86_64 => root.join("guest/out/x86_64"),
        };
        fs::create_dir_all(&out)?;
        Ok(out)
    }

    /// Where the host binary lands: target/release when it is this Mac's own
    /// architecture (so `cargo run` and the Apple Silicon build stay as they
    /// were), target/<triple>/release when cross-built.
    fn host_bin(self, root: &Path) -> PathBuf {
        if self.native() { root.join("target/release/skiff") } else { root.join("target").join(self.host_target()).join("release/skiff") }
    }

    fn native(self) -> bool {
        (self == Arch::X86_64) == cfg!(target_arch = "x86_64")
    }

    /// In release file names: every one says which Mac it's for.
    fn suffix(self) -> &'static str {
        match self {
            Arch::Arm64 => "-arm64",
            Arch::X86_64 => "-x86_64",
        }
    }

    /// The app's folder under target/.
    fn app_dir(self, lite: bool) -> &'static str {
        match (self, lite) {
            (Arch::Arm64, false) => "target/Skiff.app",
            (Arch::Arm64, true) => "target/lite/Skiff.app",
            (Arch::X86_64, false) => "target/intel/Skiff.app",
            (Arch::X86_64, true) => "target/intel/lite/Skiff.app",
        }
    }

    fn cli(self) -> cli_pins::MacArch {
        match self {
            Arch::Arm64 => cli_pins::MacArch::Arm64,
            Arch::X86_64 => cli_pins::MacArch::X86_64,
        }
    }
}

fn main() -> ExitCode {
    let task = std::env::args().nth(1).unwrap_or_default();
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap().to_path_buf();
    let arch = Arch::from_args();
    let preflight = match task.as_str() {
        "release" => signing::check(true),
        "sign" => signing::check(signing::profile().is_some()),
        "pack" => signing::verify_distribution(&root.join(arch.app_dir(false))),
        _ => Ok(()),
    };
    let result = preflight.and_then(|()| match task.as_str() {
        "signing-check" => signing::check(true),
        "guest" => guest(&root, arch),
        "kernel" => kernel(&root, arch),
        "disk" => disk(&root),
        "host" => host(&root, arch),
        "linux" => linux(&root),
        "windows" | "windows-dev" | "windows-pack" | "msi" | "msi-pack" => match WinArch::from_args() {
            Ok(win) => match task.as_str() {
                "windows" => windows(&root, false, win),
                "windows-dev" => windows(&root, true, win),
                "windows-pack" => windows_bundle(&root, false, win),
                "msi" => windows(&root, false, win).and_then(|()| msi(&root, win)),
                _ => msi(&root, win),
            },
            Err(e) => Err(e),
        },
        "docker-win-arm64" => docker_cli_arm64(&root.join("target/cli-cache")).map(|p| println!("{}", p.display())),
        "app-testflight" => app_mode(&root, arch, false, false, true).map(drop),
        "app" => app(&root, arch, false, true).map(drop),
        "app-lite" => app(&root, arch, true, true).map(drop),
        // `app-dev builds`: Skiff Builds.app, a Skiff Dev of its own for one feature.
        "app-dev" => match std::env::args().nth(2) {
            Some(f) if f.is_empty() || !f.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit()) || f == "dev" => {
                Err(io::Error::other("a feature name is lower-case letters and digits, not `dev`"))
            }
            feature => app(&root, Arch::Arm64, false, false).and_then(|a| app_dev(&root, &a, feature.as_deref())),
        },
        // The disk image of the app already built, signed but not notarized (to look at it).
        "dmg" => dmg(&root, &root.join(arch.app_dir(false)), arch, false).map(|d| println!("{}", d.display())),
        "release" => app(&root, arch, false, true).and_then(|a| release(&root, &a, arch)),
        // What `release` compiles, downloads and installs, without signing:
        // CI runs it before the signing keychain is imported.
        "release-prep" => release_prep(&root, arch),
        // Signs the app already built (on a Mac without the identity).
        "sign" => sign_app(&root, &root.join(arch.app_dir(false)), arch, false, true, true),
        // Packs the app already built, without rebuilding it.
        "pack" => release(&root, &root.join(arch.app_dir(false)), arch),
        "all" => guest(&root, arch).and_then(|()| host(&root, arch)),
        _ => {
            Err(io::Error::other("usage: cargo xtask <guest|kernel|disk|host|linux|windows|windows-dev|windows-pack|msi|msi-pack|docker-win-arm64|app|app-lite|app-dev|release|release-prep|sign|signing-check|pack|all> [--arch arm64]"))
        }
    });
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("xtask {task}: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run(cmd: &mut Command) -> io::Result<()> {
    let status = cmd.status()?;
    if !status.success() {
        return Err(io::Error::other(format!("{cmd:?} failed: {status}")));
    }
    Ok(())
}

fn guest(root: &Path, arch: Arch) -> io::Result<()> {
    let target = arch.guest_target();
    // skiff-init is pure Rust: rust-lld links it against Rust's own musl, as
    // .cargo/config.toml does for arm64. Set here, not there, so the Linux
    // and Windows engine builds (which link C) keep their own linker.
    run(Command::new(env!("CARGO"))
        .current_dir(root)
        .env("CARGO_TARGET_X86_64_UNKNOWN_LINUX_MUSL_LINKER", "rust-lld")
        .args(["build", "--release", "-p", "skiff-init", "--target", target]))?;
    let init = fs::read(root.join("target").join(target).join("release/skiff-init"))?;

    // Uncompressed on purpose: it is small, and the kernel skips decompression.
    let mut cpio = Cpio::default();
    for dir in ["dev", "proc", "sys", "run", "tmp", "etc"] {
        cpio.dir(dir);
    }
    // The kernel opens /dev/console for PID 1 before any filesystem is mounted.
    cpio.char_dev("dev/console", 0o600, 5, 1);
    cpio.file("init", 0o755, &init);
    let path = arch.guest_out(root)?.join("initramfs.cpio");
    fs::write(&path, cpio.finish())?;
    println!("{} ({} KiB)", path.display(), fs::metadata(&path)?.len() / 1024);
    Ok(())
}

/// The arm64 kernel builds in the cached tree in the `skiff-kbuild` volume;
/// the x86-64 one in its own, `skiff-kbuild-x86_64`, since both build in the
/// source tree. On Apple Silicon the x86-64 kernel is cross-compiled.
fn kernel(root: &Path, arch: Arch) -> io::Result<()> {
    let src = root.join("guest/kernel");
    let out = arch.guest_out(root)?;
    let (karch, cache) = match arch {
        Arch::Arm64 => ("KARCH=arm64", "skiff-kbuild:/build"),
        Arch::X86_64 => ("KARCH=x86_64", "skiff-kbuild-x86_64:/build"),
    };
    run(Command::new("docker").args([
        "run",
        "--rm",
        "-e",
        "DEBIAN_FRONTEND=noninteractive",
        "-e",
        karch,
        "-v",
        cache,
        "-v",
        &format!("{}:/src:ro", src.display()),
        "-v",
        &format!("{}:/out", out.display()),
        "debian:trixie",
        "sh",
        "/src/build.sh",
    ]))
}

/// A sparse 64 GiB ext4 image with lazily initialised inode tables: it uses a
/// few MB on the Mac until data is written. Each install copies it (an APFS
/// clone, so instant) into its data directory on first boot.
/// The same ext4 image serves both architectures.
fn disk(root: &Path) -> io::Result<()> {
    let out = Arch::Arm64.guest_out(root)?;
    run(Command::new("docker").args([
        "run",
        "--rm",
        "-v",
        &format!("{}:/out", out.display()),
        "debian:trixie",
        "sh",
        "-c",
        "set -e; apt-get update -qq >/dev/null && apt-get install -y -qq e2fsprogs >/dev/null; \
         rm -f /out/data.img; truncate -s 64G /out/data.img; \
         mkfs.ext4 -q -F -m 0 -L skiff-data -E lazy_itable_init=1,lazy_journal_init=1 /out/data.img",
    ]))?;
    let meta = fs::metadata(out.join("data.img"))?;
    println!("{} ({} GiB sparse)", out.join("data.img").display(), meta.len() >> 30);
    Ok(())
}

fn host(root: &Path, arch: Arch) -> io::Result<()> { host_mode(root, arch, false) }
fn host_mode(root: &Path, arch: Arch, sandboxed: bool) -> io::Result<()> {
    let mut build = Command::new(env!("CARGO"));
    build.current_dir(root).args(["build", "--release", "-p", "skiff"]);
    if sandboxed { build.args(["--features", "app-store"]); }
    if !arch.native() {
        build.args(["--target", arch.host_target()]);
    }
    run(&mut build)?;
    let bin = arch.host_bin(root);
    if cfg!(target_os = "macos") {
        // Virtualization.framework refuses to start a VM without this entitlement.
        run(Command::new("codesign").args(["--force", "--sign", "-", "--entitlements"])
            .arg(root.join("crates/skiff/skiff.entitlements"))
            .arg(&bin))?;
    }
    println!("{} ({} KiB)", bin.display(), fs::metadata(&bin)?.len() / 1024);
    Ok(())
}

/// The Linux host build: native, in a Rust container, so C dependencies
/// (ring) find a compiler. The cargo registry is cached in a volume.
/// `--arch amd64` builds x86-64 binaries (on Apple silicon the container
/// runs under Rosetta), for Linux PCs and WSL2 on Windows.
fn linux(root: &Path) -> io::Result<()> {
    let amd64 = std::env::args().skip(2).collect::<Vec<_>>().windows(2).any(|w| w[0] == "--arch" && (w[1] == "amd64" || w[1] == "x86_64"));
    linux_build(root, amd64)
}

fn linux_build(root: &Path, amd64: bool) -> io::Result<()> {
    let (platform, out, cache) = if amd64 { ("linux/amd64", "linux-x86_64", "skiff-cargo-amd64") } else { ("linux/arm64", "linux", "skiff-cargo") };
    run(Command::new("docker").args(["run", "--rm", "--platform", platform, "-v"]).arg(format!("{}:/src", root.display())).args([
        "-v",
        &format!("{cache}:/usr/local/cargo/registry"),
        "-w",
        "/src",
        "-e",
        &format!("CARGO_TARGET_DIR=/src/target/{out}"),
        // The repo's cross-linking setting is for the guest build from macOS.
        "-e",
        "CARGO_TARGET_AARCH64_UNKNOWN_LINUX_MUSL_LINKER=cc",
        "rust:1-alpine",
        "sh",
        "-c",
        "apk add -q musl-dev && cargo build --release -p skiff -p skiff-init",
    ]))?;
    for b in ["skiff", "skiff-init"] {
        let p = root.join("target").join(out).join("release").join(b);
        println!("{} ({} KiB)", p.display(), fs::metadata(&p)?.len() / 1024);
    }
    Ok(())
}

/// Which Windows a bundle is for: x86-64 PCs, or Windows on Arm (Snapdragon
/// laptops, a Windows 11 VM on a Mac), where WSL2's Linux is arm64 too.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum WinArch {
    X64,
    Arm64,
}

impl WinArch {
    /// `--arch arm64` (or aarch64) after the task; x86-64 without one.
    fn from_args() -> io::Result<WinArch> {
        let args: Vec<String> = std::env::args().skip(2).collect();
        match args.windows(2).find(|w| w[0] == "--arch").map(|w| w[1].as_str()) {
            None | Some("x86_64" | "x64" | "amd64") => Ok(WinArch::X64),
            Some("arm64" | "aarch64") => Ok(WinArch::Arm64),
            Some(a) => Err(io::Error::other(format!("--arch {a}: Windows builds are x86_64 or arm64"))),
        }
    }

    /// The machine this xtask runs on, which builds skiff-win without --target.
    fn host() -> WinArch {
        if cfg!(target_arch = "aarch64") { WinArch::Arm64 } else { WinArch::X64 }
    }

    /// The Linux engine's architecture: target/skiff-windows-<this>, the musl
    /// target and Alpine's name for it.
    fn linux(self) -> &'static str {
        match self {
            WinArch::X64 => "x86_64",
            WinArch::Arm64 => "aarch64",
        }
    }

    /// The MSI's name, as Windows says it (Skiff-<version>-arm64.msi).
    fn msi(self) -> &'static str {
        match self {
            WinArch::X64 => "x86_64",
            WinArch::Arm64 => "arm64",
        }
    }

    /// `wix build -arch`: the package's Platform.
    fn wix(self) -> &'static str {
        match self {
            WinArch::X64 => "x64",
            WinArch::Arm64 => "arm64",
        }
    }

    fn msvc(self) -> &'static str {
        match self {
            WinArch::X64 => "x86_64-pc-windows-msvc",
            WinArch::Arm64 => "aarch64-pc-windows-msvc",
        }
    }

    /// The update feed skiff-win of this architecture reads (crates/skiff-win/src/update.rs).
    fn feed(self) -> &'static str {
        match self {
            WinArch::X64 => "latest-windows-x86_64.json",
            WinArch::Arm64 => "latest-windows-arm64.json",
        }
    }
}

/// The Windows bundle, in target/skiff-windows-x86_64 (or -aarch64): what
/// contrib/windows/skiff.ps1 installs. On Windows the Linux half is built
/// with cargo in WSL (the default distro, with Rust installed); elsewhere
/// as `linux --arch amd64` is, and skiff-win.exe needs the
/// x86_64-pc-windows-gnu target and MinGW.
fn windows(root: &Path, debug: bool, arch: WinArch) -> io::Result<()> {
    // The static (musl) engine, which runs in Skiff Linux and in Ubuntu, and
    // Skiff Linux itself (scripts/skiff-linux.sh), built in WSL: on Windows,
    // the default distro, with Rust installed. zig cross-compiles, so
    // either architecture builds in either WSL.
    let linux = root.join(format!("target/linux-{}-musl", arch.linux()));
    if !cfg!(windows) {
        return Err(io::Error::other("the Windows bundle is built on Windows, in WSL"));
    }
    fs::create_dir_all(&linux)?;
    let wsl = |path: &Path| -> io::Result<String> {
        let out = Command::new("wsl.exe").args(["wslpath", "-a"]).arg(path.to_string_lossy().replace('\\', "/")).output()?;
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
    };
    let (src, out_dir) = (wsl(root)?, wsl(&linux)?);
    run(Command::new("wsl.exe").args(["--", "bash", &format!("{src}/scripts/windows-engine.sh"), &src, &out_dir, arch.linux()]))?;
    run(Command::new("wsl.exe").args(["--", "sh", &format!("{src}/scripts/skiff-linux.sh"), &src, &out_dir, &format!("{out_dir}/skiff-linux.tar.gz"), arch.linux()]))?;
    windows_bundle(root, debug, arch)
}

/// Assemble from Linux payloads built separately on a Linux CI runner, or
/// by windows() in WSL on a developer's machine.
fn windows_bundle(root: &Path, debug: bool, arch: WinArch) -> io::Result<()> {
    if !cfg!(windows) {
        return Err(io::Error::other("the Windows bundle must be assembled on Windows"));
    }
    let linux = root.join(format!("target/linux-{}-musl", arch.linux()));
    for file in ["skiff", "skiff-init", "skiff-linux.tar.gz"] {
        if !linux.join(file).is_file() {
            return Err(io::Error::other(format!("missing Linux payload {}", linux.join(file).display())));
        }
    }
    // Assemble the bundle first: the engine, Skiff Linux, the docker CLI,
    // Compose and buildx (pinned, checked, as Skiff.app bundles them) and the
    // install scripts. skiff-win is built last, with the SHA-256 of each of
    // these embedded, so setup can refuse a bundle file that was tampered
    // with in the unzipped folder.
    let out = root.join(format!("target/skiff-windows-{}", arch.linux()));
    fs::create_dir_all(out.join("bin/cli-plugins"))?;
    for b in ["skiff", "skiff-init", "skiff-linux.tar.gz"] {
        fs::copy(linux.join(b), out.join(b))?;
    }
    let cache = root.join("target/cli-cache");
    if arch == WinArch::Arm64 {
        windows_arm64_cli(&cache, &out)?;
    } else {
        let (docker_v, docker_sum) = WIN_DOCKER_CLI;
        let zip = fetch(&cache, &format!("docker-{docker_v}-windows.zip"), &cli_pins::win_docker_url(docker_v))?;
        verify(&zip, docker_sum)?;
        let unzip_dir = cache.join("docker-windows");
        fs::create_dir_all(&unzip_dir)?;
        // bsdtar reads zips (Windows and macOS have it as tar); GNU tar does not.
        if cfg!(target_os = "linux") {
            run(Command::new("unzip").args(["-o", "-q"]).arg(&zip).arg("docker/docker.exe").arg("-d").arg(&unzip_dir))?;
        } else {
            run(Command::new("tar").args(["-xf"]).arg(&zip).arg("-C").arg(&unzip_dir).arg("docker/docker.exe"))?;
        }
        fs::copy(unzip_dir.join("docker/docker.exe"), out.join("bin/docker.exe"))?;
        let (compose_v, compose_sum) = WIN_COMPOSE;
        let compose = fetch(&cache, &format!("docker-compose-windows-x86_64-{compose_v}.exe"), &cli_pins::win_compose_url(compose_v))?;
        verify(&compose, compose_sum)?;
        fs::copy(&compose, out.join("bin/cli-plugins/docker-compose.exe"))?;
        let (buildx_v, buildx_sum) = WIN_BUILDX;
        let buildx = fetch(&cache, &format!("buildx-{buildx_v}.windows-amd64.exe"), &cli_pins::win_buildx_url(buildx_v))?;
        verify(&buildx, buildx_sum)?;
        fs::copy(&buildx, out.join("bin/cli-plugins/docker-buildx.exe"))?;
    }
    fs::copy(root.join("contrib/wsl/install.sh"), out.join("install.sh"))?;
    // The PowerShell scripts skiff-win runs, from their files beside it with
    // -File: skiff.ps1 (update, uninstall), setup-tools.ps1 (setup's tools
    // step) and skiff-admin.ps1 (the elevated steps: WSL, the DNS rule). Never
    // script text inside skiff-win.exe, which Defender's heuristics take for
    // a dropper (crates/skiff-win/src/win.rs).
    for s in ["skiff.ps1", "setup-tools.ps1", "skiff-admin.ps1"] {
        fs::copy(root.join("contrib/windows").join(s), out.join(s))?;
    }
    fs::copy(root.join("LICENSE"), out.join("LICENSE"))?;
    fs::copy(root.join("NOTICE"), out.join("NOTICE"))?;
    // bin\skiff.exe, the `skiff` command: Share on Windows itself, docker's
    // commands through docker.exe, the engine's own by the `skiff` in WSL
    // (crates/skiff/src/wsl.rs). Built before skiff-win, so its hash goes in
    // the manifest too.
    fs::copy(windows_exe(root, debug, arch, "skiff", None)?, out.join("bin/skiff.exe"))?;

    // `name=sha256;…` for every file setup uses or skiff-win runs, embedded in skiff-win.exe.
    let hashed = ["skiff", "skiff-init", "skiff-linux.tar.gz", "install.sh", "skiff.ps1", "setup-tools.ps1", "skiff-admin.ps1", "bin/docker.exe", "bin/skiff.exe", "bin/cli-plugins/docker-compose.exe", "bin/cli-plugins/docker-buildx.exe"];
    let mut manifest = Vec::new();
    for f in hashed {
        manifest.push(format!("{f}={}", sha256(&out.join(f))?));
    }
    let manifest = manifest.join(";");
    let exe = windows_exe(root, debug, arch, "skiff-win", Some(&manifest))?;
    fs::copy(&exe, out.join("skiff-win.exe"))?;
    println!("{}", out.display());
    Ok(())
}

/// The arm64 docker CLI, Compose and buildx into `out`/bin, pinned as the
/// x86-64 ones are; docker.exe is built, as Docker publishes none.
fn windows_arm64_cli(cache: &Path, out: &Path) -> io::Result<()> {
    let docker = docker_cli_arm64(cache)?;
    fs::copy(&docker, out.join("bin/docker.exe"))?;
    let (compose_v, compose_sum) = WIN_ARM64_COMPOSE;
    let compose = fetch(cache, &format!("docker-compose-windows-aarch64-{compose_v}.exe"), &cli_pins::win_arm64_compose_url(compose_v))?;
    verify(&compose, compose_sum)?;
    fs::copy(&compose, out.join("bin/cli-plugins/docker-compose.exe"))?;
    let (buildx_v, buildx_sum) = WIN_ARM64_BUILDX;
    let buildx = fetch(cache, &format!("buildx-{buildx_v}.windows-arm64.exe"), &cli_pins::win_arm64_buildx_url(buildx_v))?;
    verify(&buildx, buildx_sum)?;
    fs::copy(&buildx, out.join("bin/cli-plugins/docker-buildx.exe"))?;
    Ok(())
}

/// docker.exe for Windows on Arm, from docker/cli's source at the pinned
/// commit, as its own scripts/build/binary builds it but with cgo off (the
/// Windows CLI needs none) and every input fixed: the toolchain is the Go
/// docker/cli's Dockerfile names, fetched by `go` itself (GOTOOLCHAIN,
/// checked against sum.golang.org), the dependencies are the vendored ones
/// in the commit, the paths trimmed and the build time the commit's. Go
/// builds the same bytes that way on any host, so the result must match
/// WIN_ARM64_DOCKER_SRC's SHA-256 like any download. Needs git and a Go of
/// 1.21 or later on PATH; cached in target/cli-cache.
fn docker_cli_arm64(cache: &Path) -> io::Result<PathBuf> {
    let pin = &WIN_ARM64_DOCKER_SRC;
    let exe = cache.join(format!("docker-{}-windows-arm64.exe", pin.version));
    if exe.is_file() && sha256(&exe)? == pin.sha256 {
        return Ok(exe);
    }
    let src = cache.join(format!("docker-cli-{}", pin.version));
    // Checked out byte for byte: Git for Windows' core.autocrlf would turn the
    // sources' LF into CRLF, and Go builds other bytes from them. A clone made
    // without this setting is made again.
    let lf = Command::new("git").arg("-C").arg(&src).args(["config", "--local", "core.autocrlf"]).output().map(|o| String::from_utf8_lossy(&o.stdout).trim() == "false").unwrap_or(false);
    if !src.join(".git").is_dir() || !lf {
        let _ = fs::remove_dir_all(&src);
        fs::create_dir_all(cache)?;
        run(Command::new("git").args(["-c", "advice.detachedHead=false", "clone", "-q", "--config", "core.autocrlf=false", "--config", "core.eol=lf", "--depth", "1", "--branch", &format!("v{}", pin.version), cli_pins::DOCKER_CLI_GIT]).arg(&src))?;
    }
    let git = |args: &[&str]| -> io::Result<String> {
        let o = Command::new("git").arg("-C").arg(&src).args(args).output()?;
        if !o.status.success() {
            return Err(io::Error::other(format!("git {}: {}", args.join(" "), String::from_utf8_lossy(&o.stderr).trim())));
        }
        Ok(String::from_utf8_lossy(&o.stdout).into_owned())
    };
    let head = git(&["rev-parse", "HEAD"])?;
    if head.trim() != pin.commit {
        return Err(io::Error::other(format!("{}: at {}, not docker/cli v{}'s {}; delete it and run again", src.display(), head.trim(), pin.version, pin.commit)));
    }
    // docker/cli is not a Go module yet: its scripts link go.mod and go.sum
    // to vendor.mod and vendor.sum. Copies here (a symlink on Windows needs
    // Developer Mode). Nothing else may differ from the commit, since an
    // untracked .go file would be compiled in.
    fs::copy(src.join("vendor.mod"), src.join("go.mod"))?;
    fs::copy(src.join("vendor.sum"), src.join("go.sum"))?;
    let status = git(&["status", "--porcelain", "--ignored"])?;
    if let Some(l) = status.lines().find(|l| *l != "?? go.mod" && *l != "?? go.sum" && *l != "!! go.mod" && *l != "!! go.sum") {
        return Err(io::Error::other(format!("{}: `{l}` is not docker/cli's commit; delete the folder and run again", src.display())));
    }
    let short = &pin.commit[..7];
    let ldflags = format!(
        "-buildid= -X \"github.com/docker/cli/cli/version.GitCommit={short}\" -X \"github.com/docker/cli/cli/version.BuildTime={}\" -X \"github.com/docker/cli/cli/version.Version={}\"",
        pin.time, pin.version
    );
    let part = cache.join(format!("docker-{}-windows-arm64.exe.part", pin.version));
    let mut go = Command::new("go");
    go.current_dir(&src)
        .env("GOTOOLCHAIN", pin.go)
        .env("GOOS", "windows")
        .env("GOARCH", "arm64")
        .env("GOARM64", "v8.0")
        .env("CGO_ENABLED", "0")
        .env("GO111MODULE", "on")
        .env("GOFLAGS", "-mod=vendor")
        .env("GOWORK", "off")
        .env("GOEXPERIMENT", "")
        .args(["build", "-trimpath", "-buildvcs=false", "-tags", "grpcnotrace", "-ldflags", &ldflags, "-o"])
        .arg(&part)
        .arg("./cmd/docker");
    run(&mut go)?;
    let got = sha256(&part)?;
    if got != pin.sha256 {
        let _ = fs::remove_file(&part);
        return Err(io::Error::other(format!("docker.exe for arm64 built from {} has SHA-256 {got}, expected {}", pin.commit, pin.sha256)));
    }
    fs::rename(&part, &exe)?;
    Ok(exe)
}

/// The bytes of every file under a folder.
fn dir_size(dir: &Path) -> io::Result<u64> {
    let mut total = 0;
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let meta = entry.metadata()?;
        total += if meta.is_dir() { dir_size(&entry.path())? } else { meta.len() };
    }
    Ok(total)
}

/// target/Skiff-<version>-x86_64.msi (or -arm64.msi): the per-user Windows
/// installer (installer/windows/skiff.wxs) around the bundle in
/// target/skiff-windows-x86_64 (or -aarch64). Both share one UpgradeCode, so
/// either replaces the other (skiff.wxs says why); each has its own feed.
/// Needs WiX 5 (`dotnet tool install -g wix`, with the UI and Util
/// extensions: `wix extension add -g WixToolset.UI.wixext/5.0.2`,
/// `WixToolset.Util.wixext/5.0.2`).
/// Builds `package` (skiff-win, or skiff for bin\skiff.exe) for Windows on
/// `arch` and returns its exe: natively on Windows, or the x86-64 one with
/// MinGW elsewhere. `manifest` is the bundle's hashes, embedded in a
/// release skiff-win.exe.
fn windows_exe(root: &Path, debug: bool, arch: WinArch, package: &str, manifest: Option<&str>) -> io::Result<PathBuf> {
    let mut build = Command::new(env!("CARGO"));
    build.current_dir(root).args(["build", "-p", package, "--bin", package]);
    if let Some(m) = manifest.filter(|_| !debug) {
        build.env("SKIFF_BUNDLE_SHA256", m);
    }
    if !debug {
        build.arg("--release");
    }
    let mut dir = root.join("target");
    if cfg!(windows) {
        // The other architecture's skiff-win (arm64 from an x64 PC, or the
        // reverse) needs its rustup target and Visual Studio's MSVC build
        // tools for it ("MSVC ... ARM64 build tools").
        if arch != WinArch::host() {
            build.args(["--target", arch.msvc()]);
            dir = dir.join(arch.msvc());
        }
    } else if arch == WinArch::Arm64 {
        return Err(io::Error::other("the arm64 skiff-win.exe is built on Windows (or checked from here with cargo xwin)"));
    } else {
        build.args(["--target", "x86_64-pc-windows-gnu"]);
        dir = dir.join("x86_64-pc-windows-gnu");
    }
    run(&mut build)?;
    Ok(dir.join(if debug { "debug" } else { "release" }).join(format!("{package}.exe")))
}

fn msi(root: &Path, arch: WinArch) -> io::Result<()> {
    let bundle = root.join(format!("target/skiff-windows-{}", arch.linux()));
    if !bundle.join("skiff-win.exe").is_file() {
        return Err(io::Error::other(format!("no bundle in {}: run cargo xtask windows (or msi) first", bundle.display())));
    }
    let version = env!("CARGO_PKG_VERSION");
    let name = format!("Skiff-{version}-{}.msi", arch.msi());
    let out = root.join("target").join(&name);
    // The dialog art (brand/installer/windows, committed) gets this build's
    // version stamped onto a copy in target/wix-assets, with System.Drawing
    // only, so CI needs nothing beyond the checkout; if that fails the
    // committed, unstamped bitmaps serve.
    let assets = root.join("target/wix-assets");
    let drawn = Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-File"])
        .arg(root.join("brand/installer/windows/make-bitmaps.ps1"))
        .arg("-Stamp")
        .arg("-Version")
        .arg(version)
        .arg("-Out")
        .arg(&assets)
        .status()
        .is_ok_and(|s| s.success());
    if !drawn {
        eprintln!("xtask: make-bitmaps.ps1 -Stamp failed; using the committed installer bitmaps as they are");
    }
    // The ready page says what the install takes: the bundle's files, in
    // the MB Apps & features uses (1024 by 1024).
    let on_disk = dir_size(&bundle)? as f64 / (1024.0 * 1024.0);
    let mut wix = Command::new("wix");
    wix.current_dir(root.join("installer/windows"))
        .args(["build", "-arch", arch.wix(), "-ext", "WixToolset.UI.wixext", "-ext", "WixToolset.Util.wixext"])
        .arg("-d")
        // Windows Installer takes numbers only: a test build's 0.1.8-dev.42 is 0.1.8.
        .arg(format!("Version={}", version.split('-').next().unwrap_or(version)))
        .arg("-d")
        .arg(format!("Bundle={}", bundle.display()))
        .arg("-d")
        .arg(format!("Size={on_disk:.1} MB"));
    if drawn {
        wix.arg("-d").arg(format!("Assets={}", assets.display()));
    }
    run(wix.args(["-loc", "skiff.wxl", "-o"]).arg(&out).arg("skiff.wxs"))?;
    // The update feed entry skiff-win reads (crates/skiff-win/src/update.rs),
    // as `release` writes the Mac's.
    let size = fs::metadata(&out)?.len();
    let sum = sha256(&out)?;
    let base = std::env::var("SKIFF_RELEASE_BASE").unwrap_or_else(|_| "https://github.com/shebashio/skiff/releases/latest/download".to_owned());
    let extra = feed_notes()?;
    let feed = format!("{{\"version\":\"{version}\",\"url\":\"{base}/{name}\",\"sha256\":\"{sum}\",\"size\":{size}{extra}}}\n");
    let feed_path = root.join("target").join(arch.feed());
    fs::write(&feed_path, &feed)?;
    println!("{} ({} KiB)\n{}", out.display(), size / 1024, feed_path.display());
    Ok(())
}

/// The docker CLI tools bundled into Skiff.app, pinned.
/// SHA-256s from the releases (GitHub's asset digests; the docker tarball
/// as download.docker.com served it), checked before anything is bundled.
#[path = "../../crates/skiff/src/cli_pins.rs"]
mod cli_pins;
use cli_pins::{WIN_ARM64_BUILDX, WIN_ARM64_COMPOSE, WIN_ARM64_DOCKER_SRC, WIN_BUILDX, WIN_COMPOSE, WIN_DOCKER_CLI};

/// Downloads `url` into the cache once; returns the cached path.
fn fetch(cache: &Path, name: &str, url: &str) -> io::Result<PathBuf> {
    let path = cache.join(name);
    if !path.is_file() {
        fs::create_dir_all(cache)?;
        let tmp = cache.join(format!("{name}.part"));
        run(Command::new("curl").args(["-fsSL", "--retry", "3", "-o"]).arg(&tmp).arg(url))?;
        fs::rename(&tmp, &path)?;
    }
    Ok(path)
}

fn sha256(path: &Path) -> io::Result<String> {
    if cfg!(windows) {
        // certutil prints a header line, the hash, then a footer.
        let out = Command::new("certutil").args(["-hashfile"]).arg(path).arg("SHA256").output()?;
        let text = String::from_utf8_lossy(&out.stdout).into_owned();
        return Ok(text.lines().nth(1).unwrap_or_default().replace(' ', "").to_ascii_lowercase());
    }
    let out = Command::new("shasum").args(["-a", "256"]).arg(path).output()?;
    Ok(String::from_utf8_lossy(&out.stdout).split_whitespace().next().unwrap_or_default().to_owned())
}

/// Checks a download against its pinned SHA-256; a mismatch deletes it.
fn verify(file: &Path, want: &str) -> io::Result<()> {
    let got = sha256(file)?;
    if got != want {
        let _ = fs::remove_file(file);
        return Err(io::Error::other(format!("{}: SHA-256 {got}, expected {want}", file.display())));
    }
    Ok(())
}

/// The pinned docker CLI, Compose and buildx for `arch`, downloaded once
/// into target/cli-cache and checked against their pinned SHA-256s.
fn cli_tools(root: &Path, arch: Arch) -> io::Result<[PathBuf; 3]> {
    // Intel downloads in their own folder: the docker tarball has the
    // same name for both architectures.
    let cache = root.join(if arch == Arch::X86_64 { "target/cli-cache/x86_64" } else { "target/cli-cache" });
    let mac = arch.cli();
    let [(docker_v, docker_sum), (compose_v, compose_sum), (buildx_v, buildx_sum)] = mac.tools();
    let tgz = fetch(&cache, &format!("docker-{docker_v}.tgz"), &mac.docker_url(docker_v))?;
    verify(&tgz, docker_sum)?;
    run(Command::new("tar").args(["-xzf"]).arg(&tgz).arg("-C").arg(&cache).arg("docker/docker"))?;
    let compose = fetch(&cache, &format!("docker-compose-darwin-{}-{compose_v}", mac.compose_arch()), &mac.compose_url(compose_v))?;
    verify(&compose, compose_sum)?;
    let buildx = fetch(&cache, &mac.buildx_name(buildx_v), &mac.buildx_url(buildx_v))?;
    verify(&buildx, buildx_sum)?;
    Ok([cache.join("docker/docker"), compose, buildx])
}

/// Everything `release` would otherwise compile, download or install, done
/// first so that CI can run it before the signing keychain exists: the host
/// binary, the pinned docker tools, dmgbuild's virtualenv and the disk
/// image's background. `release` then finds them all up to date, and only
/// the pinned dmgbuild and Apple's own tools run while the key is available.
fn release_prep(root: &Path, arch: Arch) -> io::Result<()> {
    host_mode(root, arch, false)?;
    cli_tools(root, arch)?;
    dmgbuild(root)?;
    dmg_background(root)?;
    Ok(())
}

/// target/Skiff.app, signed with the virtualization entitlement. `lite`:
/// target/lite/Skiff.app, without the docker CLI, Compose and buildx, which
/// Skiff Lite downloads on first launch (src/tools.rs).
fn app(root: &Path, arch: Arch, lite: bool, allow_notarization: bool) -> io::Result<PathBuf> {
    app_mode(root, arch, lite, allow_notarization, false)
}
fn app_mode(root: &Path, arch: Arch, lite: bool, allow_notarization: bool, sandboxed: bool) -> io::Result<PathBuf> {
    host_mode(root, arch, sandboxed)?;
    let version = env!("CARGO_PKG_VERSION");
    // Info.plist's versions are numbers only: a test build's 0.1.8-dev.42 is 0.1.8 there.
    let plain = version.split('-').next().unwrap_or(version);
    let app = root.join(if sandboxed { "target/testflight/Skiff.app" } else { arch.app_dir(lite) });
    let _ = fs::remove_dir_all(&app);
    let contents = app.join("Contents");
    let (macos, res) = (contents.join("MacOS"), contents.join("Resources"));
    for d in [&macos, &res.join("guest")] {
        fs::create_dir_all(d)?;
    }
    if !lite {
        fs::create_dir_all(res.join("bin/cli-plugins"))?;
    }
    fs::copy(arch.host_bin(root), macos.join("skiff"))?;
    // In Resources, not loose in Contents: codesign refuses a file there
    // ("code object is not signed at all ... In subcomponent: Contents/LICENSE").
    fs::copy(root.join("LICENSE"), res.join("LICENSE"))?;
    fs::copy(root.join("NOTICE"), res.join("NOTICE"))?;
    fs::copy(root.join("guest/kernel/SOURCE_OFFER.md"), res.join("KERNEL_SOURCE_OFFER.md"))?;
    let out = arch.guest_out(root)?;
    for f in [arch.kernel(), "initramfs.cpio"] {
        fs::copy(out.join(f), res.join("guest").join(f))?;
    }
    // The disk template is 64 GiB sparse. Packed as a sparse tarball it is a
    // few hundred KB, and zips, notarization and downloads never see the
    // 64 GiB; Skiff unpacks it on first boot.
    run(Command::new("tar").args(["-cSzf"]).arg(res.join("guest/data.img.tgz")).arg("-C").arg(Arch::Arm64.guest_out(root)?).arg("data.img"))?;

    if !lite {
        let [docker, compose, buildx] = cli_tools(root, arch)?;
        fs::copy(docker, res.join("bin/docker"))?;
        fs::copy(compose, res.join("bin/cli-plugins/docker-compose"))?;
        fs::copy(buildx, res.join("bin/cli-plugins/docker-buildx"))?;
        for f in ["bin/docker", "bin/cli-plugins/docker-compose", "bin/cli-plugins/docker-buildx"] {
            run(Command::new("chmod").arg("755").arg(res.join(f)))?;
        }
    }

    // The icon, from the brand PNGs.
    let iconset = root.join("target/Skiff.iconset");
    let _ = fs::remove_dir_all(&iconset);
    fs::create_dir_all(&iconset)?;
    let png = root.join("brand/icon/png");
    for (size, scale) in [(16, 1), (16, 2), (32, 1), (32, 2), (128, 1), (128, 2), (256, 1), (256, 2), (512, 1), (512, 2)] {
        let px = size * scale;
        let name = if scale == 1 { format!("icon_{size}x{size}.png") } else { format!("icon_{size}x{size}@2x.png") };
        fs::copy(png.join(format!("skiff-app-icon-{px}.png")), iconset.join(name))?;
    }
    run(Command::new("iconutil").args(["-c", "icns", "-o"]).arg(res.join("Skiff.icns")).arg(&iconset))?;
    fs::remove_dir_all(&iconset)?;

    fs::write(
        contents.join("Info.plist"),
        format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>Skiff</string>
  <key>CFBundleDisplayName</key><string>Skiff</string>
  <key>CFBundleIdentifier</key><string>io.shebash.skiff.app</string>
  <key>CFBundleExecutable</key><string>skiff</string>
  <key>CFBundleIconFile</key><string>Skiff</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>{plain}</string>
  <key>CFBundleVersion</key><string>{plain}</string>
  <key>LSMinimumSystemVersion</key><string>14.0</string>
  <key>LSApplicationCategoryType</key><string>public.app-category.developer-tools</string>
  <key>NSHighResolutionCapable</key><true/>
  <key>NSLocalNetworkUsageDescription</key><string>Skiff lets containers reach devices and services on your network, and serves the ports they publish.</string>
  <key>NSDesktopFolderUsageDescription</key><string>Skiff shares this folder with containers you mount it into.</string>
  <key>NSDocumentsFolderUsageDescription</key><string>Skiff shares this folder with containers you mount it into.</string>
  <key>NSDownloadsFolderUsageDescription</key><string>Skiff saves files you download from containers here, and shares the folder with containers you mount it into.</string>
  <key>NSRemovableVolumesUsageDescription</key><string>Skiff shares this drive with containers you mount it into.</string>
  <key>NSNetworkVolumesUsageDescription</key><string>Skiff shares this network drive with containers you mount it into.</string>
</dict>
</plist>
"#
        ),
    )?;
    if sandboxed {
        let info = contents.join("Info.plist");
        let plist = fs::read_to_string(&info)?.replace("<key>CFBundleName</key>", "<key>SkiffDistributionChannel</key><string>testflight</string>\n  <key>CFBundleName</key>");
        fs::write(info, plist)?;
        fs::copy(macos.join("skiff"), macos.join("skiff-helper"))?;
        let (cpu, sum) = match arch {
            Arch::Arm64 => ("arm64", "02fe4bcf08f2c09699cff30658228a739a0c47fb75f3653d1b8772a27ade4462"),
            Arch::X86_64 => ("amd64", "5e0403b188dfee71fbd9b995e7dc9910d61b661c52371935ab6475d41185632f"),
        };
        let cache = root.join("target/cli-cache/cloudflared");
        let name = format!("cloudflared-darwin-{cpu}.tgz");
        let archive = fetch(&cache, &name, &format!("https://github.com/cloudflare/cloudflared/releases/download/2026.11.1/{name}"))?;
        verify(&archive, sum)?;
        run(Command::new("tar").args(["-xzf"]).arg(&archive).arg("-C").arg(&cache).arg("cloudflared"))?;
        fs::copy(cache.join("cloudflared"), res.join("bin/cloudflared"))?;
        fs::copy(root.join("contrib/cloudflared/LICENSE"), res.join("CLOUDFLARED_LICENSE"))?;
        run(Command::new("python3").arg(root.join("scripts/testflight.py")).arg("sign-dev").arg(&app).arg("--version").arg(plain))?;
    } else {
        sign_app(root, &app, arch, lite, allow_notarization, false)?;
    }
    let size = Command::new("du").args(["-sh"]).arg(&app).output()?;
    println!("{} ({})", app.display(), String::from_utf8_lossy(&size.stdout).split_whitespace().next().unwrap_or("?"));
    Ok(app)
}

/// target/dev/Skiff Dev.app, from the app just built: its own name and
/// bundle identifier, which make the binary Skiff Dev, and the icon with a
/// DEV band. Signed again without the hardened runtime or a timestamp: it
/// is never notarized, and the timestamp asks Apple's server on every build.
/// With a feature, `builds`: target/dev/Skiff Builds.app, `io.shebash.skiff.app.builds`,
/// a BUILDS band, and its own engine beside Skiff's and Skiff Dev's.
fn app_dev(root: &Path, built: &Path, feature: Option<&str>) -> io::Result<()> {
    let (name, id, band, state) = match feature {
        None => ("Skiff Dev".to_owned(), "io.shebash.skiff.app.dev".to_owned(), "DEV".to_owned(), "skiff-dev".to_owned()),
        Some(f) => (format!("Skiff {}{}", f[..1].to_ascii_uppercase(), &f[1..]), format!("io.shebash.skiff.app.{f}"), f.to_ascii_uppercase(), format!("skiff-{f}")),
    };
    let dir = root.join("target/dev");
    let app = dir.join(format!("{name}.app"));
    let _ = fs::remove_dir_all(&app);
    fs::create_dir_all(&dir)?;
    run(Command::new("ditto").arg(built).arg(&app))?;
    let contents = app.join("Contents");
    let plist = fs::read_to_string(contents.join("Info.plist"))?
        .replace("<key>CFBundleName</key><string>Skiff</string>", &format!("<key>CFBundleName</key><string>{name}</string>"))
        .replace("<key>CFBundleDisplayName</key><string>Skiff</string>", &format!("<key>CFBundleDisplayName</key><string>{name}</string>"))
        .replace("<string>io.shebash.skiff.app</string>", &format!("<string>{id}</string>"));
    fs::write(contents.join("Info.plist"), plist)?;

    let png = root.join("target/skiff-dev-icon.png");
    run(Command::new("swift").arg(root.join("xtask/dev-icon.swift")).arg(root.join("brand/icon/png/skiff-app-icon-1024.png")).arg(&png).arg(&band))?;
    let iconset = root.join("target/SkiffDev.iconset");
    let _ = fs::remove_dir_all(&iconset);
    fs::create_dir_all(&iconset)?;
    for (size, scale) in [(16, 1), (16, 2), (32, 1), (32, 2), (128, 1), (128, 2), (256, 1), (256, 2), (512, 1), (512, 2)] {
        let name = if scale == 1 { format!("icon_{size}x{size}.png") } else { format!("icon_{size}x{size}@2x.png") };
        let px = (size * scale).to_string();
        run(Command::new("sips").args(["-z", &px, &px]).arg(&png).arg("--out").arg(iconset.join(name)).stdout(std::process::Stdio::null()))?;
    }
    run(Command::new("iconutil").args(["-c", "icns", "-o"]).arg(contents.join("Resources/Skiff.icns")).arg(&iconset))?;
    fs::remove_dir_all(&iconset)?;

    let identity = signing::identity(false)?;
    let entitlements = root.join("crates/skiff/skiff.entitlements");
    for target in [contents.join("MacOS/skiff"), app.clone()] {
        run(Command::new("codesign").args(["--force", "--sign", &identity, "--entitlements"]).arg(&entitlements).arg(target))?;
    }
    run(Command::new("codesign").args(["--verify", "--deep", "--strict"]).arg(&app))?;
    println!("{}: its own engine (~/.{state}, docker context {state}); open it from there or copy it to /Applications", app.display());
    Ok(())
}

/// codesign with the hardened runtime and a secure timestamp, which
/// notarization requires; ad hoc signatures get neither.
fn sign(identity: &str, entitlements: Option<&Path>) -> Command {
    let mut c = Command::new("codesign");
    c.args(["--force", "--sign", identity]);
    if identity != "-" {
        c.args(["--options", "runtime", "--timestamp"]);
    }
    if let Some(e) = entitlements {
        c.arg("--entitlements").arg(e);
    }
    c
}

/// target/Skiff-<version>-arm64.dmg (-x86_64 for Intel): the app and an Applications shortcut, signed,
/// then notarized by Apple in one submission, with the ticket stapled to the
/// DMG and the app so Gatekeeper opens both offline. `SKIFF_NOTARY_PROFILE`
/// names credentials stored with `xcrun notarytool store-credentials`;
/// unset or empty (as CI sets it when not notarizing) skips this.
/// The drag-to-install disk image: a fixed
/// Finder window with Skiff on the left, Applications on the right and an
/// arrow between them (scripts/dmg-background.swift draws it,
/// scripts/dmg-settings.py places the icons). dmgbuild writes the window's
/// .DS_Store directly, with no Finder or AppleScript; it is installed,
/// pinned, in a throwaway virtualenv under target/. Signed, not notarized.
/// Signs an app `app` built (here, or on a Mac without the Developer ID
/// identity, such as the Intel CI Mac), and notarizes it if
/// SKIFF_NOTARY_PROFILE names a profile.
fn sign_app(root: &Path, app: &Path, arch: Arch, lite: bool, allow_notarization: bool, require_identity: bool) -> io::Result<()> {
    let (macos, res) = (app.join("Contents/MacOS"), app.join("Contents/Resources"));
    // Inside out: the helper binaries first, then the app with the
    // entitlement Virtualization.framework requires.
    let profile = signing::profile().filter(|_| allow_notarization);
    let identity = signing::identity(require_identity || profile.is_some())?;
    if !lite {
        for f in ["bin/docker", "bin/cli-plugins/docker-compose", "bin/cli-plugins/docker-buildx"] {
            run(sign(&identity, None).arg(res.join(f)))?;
        }
    }
    run(sign(&identity, Some(&root.join("crates/skiff/skiff.entitlements"))).arg(macos.join("skiff")))?;
    run(sign(&identity, Some(&root.join("crates/skiff/skiff.entitlements"))).arg(app))?;
    run(Command::new("codesign").args(["--verify", "--deep", "--strict"]).arg(app))?;
    println!("signed: {}", if identity == "-" { "ad hoc (no Developer ID identity found)" } else { identity.as_str() });
    if let Some(profile) = profile {
        notarize(root, app, &profile, arch, lite)?;
    }
    Ok(())
}

/// dmgbuild in a throwaway virtualenv under target/, installed from
/// scripts/dmgbuild-requirements.txt: every package pinned with its SHA-256,
/// wheels only, so no install script runs and no unpinned release is picked
/// up. The virtualenv is named after that file's hash, so changing a pin
/// makes a new one.
fn dmgbuild(root: &Path) -> io::Result<PathBuf> {
    let requirements = root.join("scripts/dmgbuild-requirements.txt");
    let hash = sha256(&requirements)?;
    let tag = hash.get(..12).ok_or_else(|| io::Error::other(format!("could not hash {}", requirements.display())))?;
    let venv = root.join(format!("target/dmgbuild-venv-{tag}"));
    let dmgbuild = venv.join("bin/dmgbuild");
    if !venv.join(".complete").is_file() {
        let _ = fs::remove_dir_all(&venv);
        run(Command::new("python3").args(["-m", "venv"]).arg(&venv))?;
        run(Command::new(venv.join("bin/pip"))
            .args(["install", "--quiet", "--disable-pip-version-check", "--require-hashes", "--only-binary", ":all:", "-r"])
            .arg(&requirements))?;
        fs::write(venv.join(".complete"), "")?;
    }
    Ok(dmgbuild)
}

/// target/dmg-background/background.png (and @2x), drawn by
/// scripts/dmg-background.swift.
fn dmg_background(root: &Path) -> io::Result<PathBuf> {
    let bg = root.join("target/dmg-background");
    fs::create_dir_all(&bg)?;
    run(Command::new("swift").arg(root.join("scripts/dmg-background.swift")).arg(bg.join("background")))?;
    Ok(bg.join("background.png"))
}

fn dmg(root: &Path, app: &Path, arch: Arch, lite: bool) -> io::Result<PathBuf> {
    let version = env!("CARGO_PKG_VERSION");
    let dmgbuild = dmgbuild(root)?;
    let background = dmg_background(root)?;
    let dmg = root.join(format!("target/Skiff-{}{version}{}.dmg", if lite { "Lite-" } else { "" }, arch.suffix()));
    let _ = fs::remove_file(&dmg);
    run(Command::new(&dmgbuild)
        .arg("-s")
        .arg(root.join("scripts/dmg-settings.py"))
        .arg("-D")
        .arg(format!("app={}", app.display()))
        .arg("-D")
        .arg(format!("background={}", background.display()))
        .arg(match (lite, arch) {
            (false, Arch::Arm64) => "Skiff",
            (true, Arch::Arm64) => "Skiff Lite",
            (false, Arch::X86_64) => "Skiff for Intel",
            (true, Arch::X86_64) => "Skiff Lite for Intel",
        })
        .arg(&dmg))?;
    run(sign(&signing::identity(false)?, None).arg(&dmg))?;
    Ok(dmg)
}

fn notarize(root: &Path, app: &Path, profile: &str, arch: Arch, lite: bool) -> io::Result<()> {
    let dmg = dmg(root, app, arch, lite)?;
    run(Command::new("xcrun").args(["notarytool", "submit"]).arg(&dmg).args(["--keychain-profile", profile, "--wait"]))?;
    run(Command::new("xcrun").args(["stapler", "staple"]).arg(&dmg))?;
    run(Command::new("xcrun").args(["stapler", "staple"]).arg(app))?;
    run(Command::new("xcrun").args(["stapler", "validate"]).arg(&dmg))?;
    signing::verify_distribution(app)?;
    println!("notarized and stapled: {}", dmg.display());
    Ok(())
}

/// A string's JSON form, without the quotes.
fn json_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '"' => out += "\\\"",
            '\\' => out += "\\\\",
            '\n' => out += "\\n",
            '\r' => out += "\\r",
            '\t' => out += "\\t",
            c if (c as u32) < 0x20 => out += &format!("\\u{:04x}", c as u32),
            c => out.push(c),
        }
    }
    out
}

/// What's new, for a feed (Mac's and Windows'): SKIFF_RELEASE_NOTES names a
/// Markdown file ("## New", "- a line") shown in the window's What's new;
/// SKIFF_RELEASE_NOTES_URL a github.com/shebashio/skiff page. Empty when neither is set.
fn feed_notes() -> io::Result<String> {
    let mut extra = String::new();
    if let Ok(path) = std::env::var("SKIFF_RELEASE_NOTES") {
        extra += &format!(",\"notes\":\"{}\"", json_escape(&fs::read_to_string(&path)?));
    }
    if let Ok(url) = std::env::var("SKIFF_RELEASE_NOTES_URL") {
        extra += &format!(",\"notes_url\":\"{}\"", json_escape(&url));
    }
    Ok(extra)
}

/// target/Skiff-<version>-arm64.tar.gz (sparse-aware, so the disk template
/// stays small) and target/latest-mac-arm64.json, the feed that Mac's app
/// reads (crates/skiff/src/update.rs); for Intel, -x86_64 in both.
fn release(root: &Path, app: &Path, arch: Arch) -> io::Result<()> {
    let version = env!("CARGO_PKG_VERSION");
    let suffix = arch.suffix();
    let tarball = root.join(format!("target/Skiff-{version}{suffix}.tar.gz"));
    run(Command::new("tar").args(["-cSzf"]).arg(&tarball).arg("-C").arg(app.parent().unwrap_or(root)).arg("Skiff.app"))?;
    let sum = sha256(&tarball)?;
    let base = std::env::var("SKIFF_RELEASE_BASE").unwrap_or_else(|_| "https://github.com/shebashio/skiff/releases/latest/download".to_owned());
    let extra = feed_notes()?;
    let feed = format!("{{\"version\":\"{version}\",\"url\":\"{base}/Skiff-{version}{suffix}.tar.gz\",\"sha256\":\"{sum}\"{extra}}}\n");
    let feed_path = root.join(format!("target/latest-mac{suffix}.json"));
    fs::write(&feed_path, &feed)?;
    println!("{} ({} KiB)\n{}", tarball.display(), fs::metadata(&tarball)?.len() / 1024, feed_path.display());
    Ok(())
}

/// Minimal writer for the "newc" cpio format the kernel unpacks as initramfs.
#[derive(Default)]
struct Cpio {
    buf: Vec<u8>,
    ino: u32,
}

impl Cpio {
    fn dir(&mut self, name: &str) {
        self.entry(name, 0o040_755, 2, (0, 0), &[]);
    }

    fn char_dev(&mut self, name: &str, perm: u32, major: u32, minor: u32) {
        self.entry(name, 0o020_000 | perm, 1, (major, minor), &[]);
    }

    fn file(&mut self, name: &str, perm: u32, data: &[u8]) {
        self.entry(name, 0o100_000 | perm, 1, (0, 0), data);
    }

    fn entry(&mut self, name: &str, mode: u32, nlink: u32, rdev: (u32, u32), data: &[u8]) {
        self.ino += 1;
        let fields = [
            self.ino,
            mode,
            0, // uid
            0, // gid
            nlink,
            0, // mtime: fixed, so builds are reproducible
            data.len() as u32,
            0, // devmajor
            0, // devminor
            rdev.0,
            rdev.1,
            name.len() as u32 + 1,
            0, // check
        ];
        self.buf.extend_from_slice(b"070701");
        for f in fields {
            write!(self.buf, "{f:08x}").unwrap();
        }
        self.buf.extend_from_slice(name.as_bytes());
        self.buf.push(0);
        self.pad();
        self.buf.extend_from_slice(data);
        self.pad();
    }

    fn pad(&mut self) {
        while !self.buf.len().is_multiple_of(4) {
            self.buf.push(0);
        }
    }

    fn finish(mut self) -> Vec<u8> {
        self.ino = 0;
        self.entry("TRAILER!!!", 0, 1, (0, 0), &[]);
        self.buf
    }
}
