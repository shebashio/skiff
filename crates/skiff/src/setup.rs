//! `skiff setup`: makes this Mac use Skiff the way OrbStack's installer does,
//! without touching anything another engine owns.
//!
//! - a `skiff` docker context (`--use` also makes it the current one)
//! - a LaunchAgent that starts `skiff daemon` at login (`--no-agent` skips it)
//! - the app's bundled `docker`, Compose and buildx, when run from Skiff.app
//! - `docker` and `skiff` in `~/.skiff/bin`, which a marked line in the shell
//!   profiles (zsh's, bash's, and fish's if used) adds to the end of PATH,
//!   so a docker CLI already installed keeps priority; and
//!   `/usr/local/bin/skiff` when that directory is the user's
//! - on a Mac with no other Docker engine, `skiff` as docker's default context
//! - a buildx builder, `skiff-build`, that loads what it builds into Skiff,
//!   and the choice of builder for plain `docker build` (src/buildwith.rs)
//! - `*.skiff.local` names for containers: `/etc/resolver/skiff.local`, which
//!   needs an administrator once (`--no-domains` skips it; `--domains` does
//!   only this)
//! - `/var/run/docker.sock` linked to Skiff's socket, for tools that ignore
//!   docker contexts (docker-py, Go's `client.FromEnv`, docker-java,
//!   Testcontainers, IDE plugins), when no other engine has it: asked for
//!   with `*.skiff.local`'s password (`--docker-socket` does only this), and
//!   put back at boot by a LaunchDaemon. Until then, or if the password is
//!   declined, `~/.testcontainers.properties` points Testcontainers at Skiff,
//!   unless the user has a file of their own there
//!
//! `skiff setup --uninstall` takes back exactly those, and the `skiff` kubectl
//! context that `skiff k8s` adds.
//!
//! Skiff Dev (src/flavor.rs) sets up only its own: the `skiff-dev` context,
//! its LaunchAgent, its builder and `~/.skiff-dev/bin`. PATH, docker's
//! default context and `*.skiff.local` stay the installed Skiff's.

use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::{Value, json};

// The bundle identifier (`APP_ID`) named in the LaunchAgent makes macOS
// say "Skiff" in Login Items, rather than the name on the signing
// certificate.
use crate::flavor::{AGENT, APP_ID, CONTEXT, dev};

use crate::builds::{BUILDER, builder};
pub use crate::builds::ensure_builder;

/// `~/.skiff/bin`, or `~/.skiff-dev/bin`.
fn bin_dir() -> io::Result<PathBuf> {
    Ok(home()?.join(crate::flavor::STATE.get()).join("bin"))
}

fn home() -> io::Result<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from).ok_or_else(|| io::Error::other("HOME is not set"))
}

fn docker_dir() -> io::Result<PathBuf> {
    match std::env::var_os("DOCKER_CONFIG") {
        Some(d) => Ok(PathBuf::from(d)),
        None => Ok(home()?.join(".docker")),
    }
}

/// The user's CLI configuration, before Dev selects its plugin overlay.
/// Explicit UI choices (switch context / build with Skiff) still target it.
pub fn user_docker_dir() -> io::Result<PathBuf> {
    if dev() && let Some(source) = std::env::var_os("SKIFF_USER_DOCKER_CONFIG") {
        return Ok(source.into());
    }
    docker_dir()
}

/// Called before any threads start. Dev's Docker clients use a private
/// config, so discovering its plugins cannot change another app's clients.
pub fn init_dev_docker() -> io::Result<Vec<String>> {
    if !dev() {
        return Ok(Vec::new());
    }
    let source = std::path::absolute(user_docker_dir()?)?;
    let isolated = home()?.join(crate::flavor::STATE.get()).join("docker");
    // SAFETY: main calls this before diagnostics or any worker threads.
    unsafe {
        std::env::set_var("SKIFF_USER_DOCKER_CONFIG", &source);
        std::env::set_var("DOCKER_CONFIG", &isolated);
        // Keep the existing user-selected builder and its restore state in
        // one place. Plugin isolation must not strand a previous selection.
        if std::env::var_os("BUILDX_CONFIG").is_none() {
            std::env::set_var("BUILDX_CONFIG", source.join("buildx"));
        }
    }
    let warnings = isolate_docker_config(&source, &isolated, CONTEXT.get())?;
    if let Some(bin) = bundle_contents().map(|c| c.join("Resources/bin")).filter(|b| b.join("docker").is_file()).or_else(|| {
        crate::tools::dir().ok().filter(|_| crate::tools::installed())
    }) {
        prefer_plugins(&isolated, &bin.join("cli-plugins"))?;
    }
    Ok(warnings)
}

/// Seed credentials/settings once, then keep Dev's logins and preferences.
/// Context metadata remains shared: `docker --context skiff-dev` still
/// works from the user's normal CLI. Seeding never rewrites the source.
fn isolate_docker_config(source: &Path, isolated: &Path, context: &str) -> io::Result<Vec<String>> {
    use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
    use std::io::Write;
    std::fs::DirBuilder::new().recursive(true).mode(0o700).create(isolated)?;
    let mut warnings = Vec::new();
    if source == isolated {
        return Ok(warnings);
    }
    let source_cfg = match read_json(&source.join("config.json")) {
        Ok(cfg) if cfg.is_object() => cfg,
        result => {
            warnings.push(format!("could not seed Docker settings from {}: {}", source.display(), result.err().map_or_else(|| "config must be an object".into(), |e| e.to_string())));
            json!({})
        }
    };
    let path = isolated.join("config.json");
    if !path.exists() {
        let mut cfg = source_cfg.clone();
        cfg["currentContext"] = context.into();
        add_plugin_dir(&mut cfg, &source.join("cli-plugins").to_string_lossy());
        match std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(&path) {
            Ok(mut f) => f.write_all(&serde_json::to_vec_pretty(&cfg).map_err(io::Error::other)?)?,
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {},
            Err(e) => return Err(e),
        }
    }
    // Include plugins installed after Dev's first run, without replacing
    // Dev's credentials, preferences, or manually configured extra dirs.
    let mut cfg = read_json(&path)?;
    if !cfg.is_object() { return Err(io::Error::other("Dev Docker config must be an object")); }
    let mut changed = add_plugin_dir(&mut cfg, &source.join("cli-plugins").to_string_lossy());
    if cfg["currentContext"].is_null() {
        cfg["currentContext"] = context.into();
        changed = true;
    }
    for dir in source_cfg["cliPluginsExtraDirs"].as_array().into_iter().flatten().filter_map(Value::as_str) {
        changed |= add_plugin_dir(&mut cfg, dir);
    }
    if changed { write_json(&path, &cfg)?; }
    let contexts = isolated.join("contexts");
    // Keep a local fallback directory, but repair our symlink after Docker
    // resets its config or the user selects a different source directory.
    if !std::fs::symlink_metadata(&contexts).is_ok_and(|m| m.is_dir()) {
        let shared = source.join("contexts");
        let link = || -> io::Result<()> {
            std::fs::create_dir_all(&shared)?;
            let target = shared.canonicalize()?;
            if std::fs::read_link(&contexts).is_ok_and(|p| p == target) { return Ok(()); }
            let tmp = isolated.join(format!("contexts-{}.tmp", std::process::id()));
            let _ = std::fs::remove_file(&tmp);
            std::os::unix::fs::symlink(target, &tmp)?;
            std::fs::rename(tmp, &contexts)
        };
        match link() {
            Ok(()) => {},
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {},
            Err(e) => {
                warnings.push(format!("could not share Docker contexts from {}: {e}", source.display()));
                if std::fs::symlink_metadata(&contexts).is_ok_and(|m| m.file_type().is_symlink()) {
                    std::fs::remove_file(&contexts)?;
                }
                std::fs::create_dir_all(contexts)?;
            },
        }
    }
    Ok(warnings)
}

fn prefer_plugins(config: &Path, plugins: &Path) -> io::Result<()> {
    let path = config.join("config.json");
    let mut cfg = read_json(&path)?;
    let own = Value::String(plugins.to_string_lossy().into_owned());
    let mut dirs = cfg["cliPluginsExtraDirs"].as_array().cloned().unwrap_or_default();
    dirs.retain(|dir| dir != &own);
    dirs.insert(0, own);
    let dirs = Value::Array(dirs);
    if cfg["cliPluginsExtraDirs"] != dirs {
        cfg["cliPluginsExtraDirs"] = dirs;
        write_json(&path, &cfg)?;
    }
    Ok(())
}

fn docker_launcher(link: &Path, docker: &Path, config: &Path) -> io::Result<()> {
    use std::os::unix::fs::OpenOptionsExt;
    use std::io::Write;
    let buildx = std::env::var_os("BUILDX_CONFIG").map(PathBuf::from).unwrap_or(user_docker_dir()?.join("buildx"));
    let script = format!("#!/bin/sh\nexport DOCKER_CONFIG={}\nif [ -z \"${{BUILDX_CONFIG+x}}\" ]; then export BUILDX_CONFIG={}; fi\nexec {} \"$@\"\n", sh_quote(&config.to_string_lossy()), sh_quote(&buildx.to_string_lossy()), sh_quote(&docker.to_string_lossy()));
    std::fs::OpenOptions::new().write(true).create_new(true).mode(0o700).open(link)?.write_all(script.as_bytes())
}

/// The docker CLI's directory for a context: named by the SHA-256 of its name.
fn context_dir(name: &str) -> io::Result<PathBuf> {
    Ok(context_dir_in(&docker_dir()?, name))
}

fn context_dir_in(docker: &Path, name: &str) -> PathBuf {
    let digest = skiff_image::store::sha256_digest(name.as_bytes());
    docker.join("contexts/meta").join(digest.trim_start_matches("sha256:"))
}

fn read_json(path: &Path) -> io::Result<Value> {
    match std::fs::read(path) {
        Ok(b) => serde_json::from_slice(&b).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, format!("{}: {e}", path.display()))),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(json!({})),
        Err(e) => Err(e),
    }
}

/// Writes JSON through a temporary file, so a crash never leaves half a config.
fn write_json(path: &Path, v: &Value) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("skiff-tmp");
    std::fs::write(&tmp, serde_json::to_vec_pretty(v).map_err(io::Error::other)?)?;
    std::fs::rename(tmp, path)
}

/// Skiff.app's `Contents`, when this binary runs from the app.
fn bundle_contents() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?.canonicalize().ok()?;
    let contents = exe.parent()?.parent()?.to_path_buf();
    contents.join("Info.plist").is_file().then_some(contents)
}

/// Whether this process is Skiff.app being opened from Finder or the Dock:
/// the app's binary, started by launchd (LaunchServices), not from a shell,
/// where running it bare must not change anything.
pub fn launched_from_app() -> bool {
    // SAFETY: getppid cannot fail.
    bundle_contents().is_some() && unsafe { libc::getppid() } == 1
}

/// The first time the app opens: the same as `skiff setup`, quietly, except
/// `*.skiff.local`, which the window offers and the user chooses.
pub fn first_launch() -> io::Result<()> {
    let set_up = agent_plist().and_then(|p| Ok(p.is_file() && agent_runs_this()?)).map_err(step("checking how Skiff starts at login"))?;
    if set_up {
        // Set up by an older Skiff: make sure the commands are there too.
        if let Some(contents) = bundle_contents() {
            let _ = bundled_cli(&contents);
        }
        let _ = name_agent();
        if !dev() {
            let _ = crate::daemon::socket_path().map(|s| docker_socket_fallback(&s));
        }
        return Ok(());
    }
    run(&["--no-domains".to_owned()])
}

/// Each step's error says which step it was: on first launch it is all the
/// log has to go on (#250).
fn step(what: &'static str) -> impl Fn(io::Error) -> io::Error {
    move |e| io::Error::new(e.kind(), format!("{what}: {e}"))
}

pub fn run(args: &[String]) -> io::Result<()> {
    let flag = |f: &str| args.iter().any(|a| a == f);
    if flag("--uninstall") {
        return uninstall();
    }
    if flag("--domains") {
        return if dev() { Err(io::Error::other(format!("*.skiff.local belongs to the installed Skiff, not {}", crate::flavor::APP))) } else { domains() };
    }
    if flag("--docker-socket") {
        if dev() {
            return Err(io::Error::other(format!("/var/run/docker.sock belongs to the installed Skiff, not {}", crate::flavor::APP)));
        }
        let result = docker_socket();
        println!("{}", docker_socket_fallback(&crate::daemon::socket_path()?));
        return result;
    }
    let socket = crate::daemon::socket_path().map_err(step("finding the engine's socket"))?;
    context(&socket).map_err(step("adding the skiff docker context"))?;
    println!("docker context `{CONTEXT}` → unix://{}", socket.display());
    if flag("--use") && !dev() {
        let path = docker_dir().map(|d| d.join("config.json")).map_err(step("making skiff docker's current context"))?;
        let mut cfg = read_json(&path).map_err(step("making skiff docker's current context"))?;
        cfg["currentContext"] = CONTEXT.get().into();
        write_json(&path, &cfg).map_err(step("making skiff docker's current context"))?;
        println!("docker now uses Skiff by default (`docker context use default` to switch back)");
    }
    if let Some(contents) = bundle_contents() {
        bundled_cli(&contents).map_err(step("setting up the skiff and docker commands"))?;
    }
    if !flag("--no-agent") {
        let plist = agent().map_err(step("starting the engine at login"))?;
        println!("skiff daemon starts at login ({})", plist.display());
    }
    match builder() {
        Ok(true) => match crate::buildwith::places().map(|p| p.saved()) {
            Ok(Some(crate::buildwith::With::Skiff)) => println!("docker build uses BuildKit in Skiff (builder `{BUILDER}`)"),
            _ => println!("docker build follows docker's context, and builds on Skiff when docker uses Skiff (`skiff build-with skiff` sends every build to Skiff)"),
        },
        Ok(false) => {}
        Err(e) => println!("docker build: the `{BUILDER}` builder comes later ({e}); the window makes it once Skiff is running"),
    }
    if !flag("--no-domains") && !dev() {
        // domains() asks for /var/run/docker.sock with the same password;
        // with the names already there, it is asked for on its own.
        let named = crate::daemon::dns::installed();
        if let Err(e) = domains() {
            println!("container names stay on *.skiff.localhost ({e}); `skiff setup --domains` tries again");
        } else if named {
            if let Err(e) = docker_socket() {
                println!("{DOCKER_SOCK} stays as it is ({e})");
            }
        }
    }
    if !dev() {
        println!("{}", docker_socket_fallback(&socket));
    }
    Ok(())
}

/// Points macOS at Skiff for `*.skiff.local`. Writing /etc/resolver needs an
/// administrator, so macOS asks for the password, once.
pub fn domains() -> io::Result<()> {
    use crate::daemon::dns;
    if dns::installed() {
        println!("containers answer at http://<name>.skiff.local");
        return Ok(());
    }
    let tmp = std::env::temp_dir().join(format!("skiff-resolver-{}", std::process::id()));
    std::fs::write(&tmp, format!("# Skiff: container names (skiff setup --uninstall removes this)\nnameserver 127.0.0.1\nport {}\n", dns::PORT))?;
    // /var/run/docker.sock with the same password, when it is free. Its
    // failing must not undo the names: how it went is read back after.
    let link = docker_socket_script()?;
    let (command, why) = match &link {
        Some(l) => (
            format!("mkdir -p /etc/resolver && cp '{}' {} && chmod 644 {} && ({l} || true)", tmp.display(), dns::RESOLVER, dns::RESOLVER),
            "Skiff wants to give your containers names like myapp.skiff.local, and to answer at /var/run/docker.sock for tools that look for Docker there.",
        ),
        None => (format!("mkdir -p /etc/resolver && cp '{}' {} && chmod 644 {}", tmp.display(), dns::RESOLVER, dns::RESOLVER), "Skiff wants to give your containers names like myapp.skiff.local."),
    };
    let result = as_admin(&command, why);
    let _ = std::fs::remove_file(&tmp);
    if let Some(l) = &link {
        let _ = std::fs::remove_file(&l.tmp);
        let _ = docker_socket_fallback(&crate::daemon::socket_path()?);
    }
    result?;
    println!("containers answer at http://<name>.skiff.local");
    Ok(())
}

/// Where tools that ignore docker contexts look for Docker: docker-py, Go's
/// `client.FromEnv`, docker-java, Testcontainers, many IDE plugins.
const DOCKER_SOCK: &str = "/var/run/docker.sock";
/// macOS empties /var/run at every boot, so a LaunchDaemon puts Skiff's link
/// back then, only if nothing else has taken the path by that time. Docker
/// Desktop's and OrbStack's helpers replace it when they start, as they
/// replace each other's.
const SOCKET_LABEL: &str = "io.shebash.skiff.docker-socket";
const SOCKET_DAEMON: &str = "/Library/LaunchDaemons/io.shebash.skiff.docker-socket.plist";

/// Who has /var/run/docker.sock.
#[derive(Debug, PartialEq)]
enum DockerSock {
    Skiff,
    Free,
    /// Another engine: what it points at.
    Taken(String),
}

/// The engines whose links stay theirs while the app is installed, running
/// or not: their socket is missing only until they start.
fn other_engine_dirs() -> Vec<PathBuf> {
    let Ok(h) = home() else { return Vec::new() };
    [("/Applications/Docker.app", ".docker"), ("/Applications/OrbStack.app", ".orbstack"), ("/Applications/Rancher Desktop.app", ".rd")]
        .iter()
        .filter(|(app, _)| Path::new(app).exists())
        .map(|(_, dir)| h.join(dir))
        .collect()
}

fn docker_sock(ours: &Path) -> DockerSock {
    docker_sock_at(Path::new(DOCKER_SOCK), ours, &other_engine_dirs())
}

fn docker_sock_at(path: &Path, ours: &Path, engines: &[PathBuf]) -> DockerSock {
    let Ok(meta) = std::fs::symlink_metadata(path) else { return DockerSock::Free };
    if meta.file_type().is_symlink() {
        let Ok(target) = std::fs::read_link(path) else { return DockerSock::Taken(path.display().to_string()) };
        if target == ours {
            return DockerSock::Skiff;
        }
        if target.exists() || engines.iter().any(|e| target.starts_with(e)) {
            return DockerSock::Taken(target.display().to_string());
        }
        // Left behind by an engine that has been removed.
        return DockerSock::Free;
    }
    // A socket (or anything else) of its own: an engine's, serving or not.
    DockerSock::Taken(path.display().to_string())
}

/// `'…'` for sh.
fn sh_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// The root commands that link /var/run/docker.sock to Skiff's socket and
/// install the LaunchDaemon that puts it back at boot.
struct LinkScript {
    command: String,
    /// The LaunchDaemon's plist, written here for the command to copy.
    tmp: PathBuf,
}

impl std::fmt::Display for LinkScript {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.command)
    }
}

/// The commands for when /var/run/docker.sock is free; None when it is
/// Skiff's already or another engine's, or for Skiff Dev.
fn docker_socket_script() -> io::Result<Option<LinkScript>> {
    let ours = crate::daemon::socket_path()?;
    if dev() || docker_sock(&ours) != DockerSock::Free {
        return Ok(None);
    }
    let esc = |s: &str| s.replace('&', "&amp;").replace('<', "&lt;");
    let ours_s = ours.to_string_lossy();
    let boot = format!("[ -e {DOCKER_SOCK} ] || [ -L {DOCKER_SOCK} ] || ln -s \"$1\" {DOCKER_SOCK}");
    let plist = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key><string>{SOCKET_LABEL}</string>
  <key>ProgramArguments</key><array><string>/bin/sh</string><string>-c</string><string>{}</string><string>skiff-docker-socket</string><string>{}</string></array>
  <key>RunAtLoad</key><true/>
</dict>
</plist>
"#,
        esc(&boot),
        esc(&ours_s)
    );
    let tmp = std::env::temp_dir().join(format!("skiff-docker-socket-{}.plist", std::process::id()));
    std::fs::write(&tmp, plist)?;
    // `[ ! -e ]` again as root: another engine may have started since. A
    // link left dangling by a removed engine is replaced.
    let command = format!(
        "cp {} {SOCKET_DAEMON} && chown root:wheel {SOCKET_DAEMON} && chmod 644 {SOCKET_DAEMON} && [ ! -e {DOCKER_SOCK} ] && rm -f {DOCKER_SOCK} && ln -s {} {DOCKER_SOCK}",
        sh_quote(&tmp.to_string_lossy()),
        sh_quote(&ours_s)
    );
    Ok(Some(LinkScript { command, tmp }))
}

/// `skiff setup --docker-socket`: /var/run/docker.sock to Skiff, if free
/// (macOS asks for the password).
pub fn docker_socket() -> io::Result<()> {
    let Some(l) = docker_socket_script()? else { return Ok(()) };
    let result = as_admin(&l.command, "Skiff wants to answer at /var/run/docker.sock, for tools that look for Docker there.");
    let _ = std::fs::remove_file(&l.tmp);
    result
}

/// Says where tools that ignore docker contexts find Skiff, and keeps
/// `~/.testcontainers.properties` for when /var/run/docker.sock is not
/// Skiff's but could be (a declined password, not yet asked).
fn docker_socket_fallback(ours: &Path) -> String {
    match docker_sock(ours) {
        DockerSock::Skiff => {
            let _ = testcontainers(ours, false);
            format!("{DOCKER_SOCK} → Skiff (docker-py, Testcontainers and other tools that ignore docker contexts)")
        }
        DockerSock::Taken(t) => {
            let _ = testcontainers(ours, false);
            format!("{DOCKER_SOCK} is another engine's ({t}) and stays so; DOCKER_HOST=unix://{} points a tool at Skiff", ours.display())
        }
        DockerSock::Free => match testcontainers(ours, true) {
            Ok(true) => format!("{DOCKER_SOCK} is not Skiff's yet (`skiff setup --docker-socket` asks for the password); Testcontainers finds Skiff through ~/.testcontainers.properties"),
            _ => format!("{DOCKER_SOCK} is not Skiff's yet (`skiff setup --docker-socket` asks for the password); until then DOCKER_HOST=unix://{} points a tool at Skiff", ours.display()),
        },
    }
}

/// The first line of the `~/.testcontainers.properties` Skiff writes: only a
/// file that starts with it is Skiff's to change or remove.
const TESTCONTAINERS_MARK: &str = "# Written by Skiff while /var/run/docker.sock is not Skiff's (skiff setup --uninstall removes it)";

fn testcontainers_text(ours: &Path) -> String {
    format!("{TESTCONTAINERS_MARK}\ndocker.host=unix://{}\n", ours.display())
}

/// Writes (`on`) or removes Skiff's `~/.testcontainers.properties`. A file
/// the user (or Testcontainers itself) wrote is never touched. True when
/// Skiff's file is there afterwards.
fn testcontainers(ours: &Path, on: bool) -> io::Result<bool> {
    testcontainers_at(&home()?.join(".testcontainers.properties"), ours, on)
}

fn testcontainers_at(path: &Path, ours: &Path, on: bool) -> io::Result<bool> {
    let ours_text = testcontainers_text(ours);
    match std::fs::read_to_string(path) {
        // Only exactly what Skiff wrote: Testcontainers adds keys of its own.
        Ok(text) if text.starts_with(TESTCONTAINERS_MARK) && text.lines().filter(|l| !l.trim().is_empty() && !l.starts_with('#')).all(|l| l.starts_with("docker.host=")) => {
            if on {
                if text != ours_text {
                    std::fs::write(path, ours_text)?;
                }
                Ok(true)
            } else {
                std::fs::remove_file(path)?;
                Ok(false)
            }
        }
        Ok(_) => Ok(false),
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            if on {
                std::fs::write(path, ours_text)?;
            }
            Ok(on)
        }
        Err(e) => Err(e),
    }
}

/// For the window, after the password prompt: how /var/run/docker.sock
/// went, if it is not Skiff's.
pub fn docker_socket_note() -> Option<String> {
    let ours = crate::daemon::socket_path().ok()?;
    match docker_sock(&ours) {
        DockerSock::Free if !dev() => Some(docker_socket_fallback(&ours)),
        _ => None,
    }
}

/// Takes /var/run/docker.sock back, if it is still Skiff's, and the
/// LaunchDaemon that put it there.
fn remove_docker_socket() -> io::Result<()> {
    let ours = crate::daemon::socket_path()?;
    let _ = testcontainers(&ours, false);
    let linked = docker_sock(&ours) == DockerSock::Skiff;
    if !linked && !Path::new(SOCKET_DAEMON).exists() {
        return Ok(());
    }
    let mut command = format!("launchctl bootout system/{SOCKET_LABEL} 2>/dev/null; rm -f {SOCKET_DAEMON}");
    if linked {
        command.push_str(&format!("; [ \"$(readlink {DOCKER_SOCK})\" = {} ] && rm -f {DOCKER_SOCK}; true", sh_quote(&ours.to_string_lossy())));
    }
    as_admin(&command, "Skiff wants to remove its /var/run/docker.sock.")?;
    println!("removed {DOCKER_SOCK}");
    Ok(())
}

/// Takes `*.skiff.local` back; containers stay on `*.skiff.localhost`.
pub fn remove_domains() -> io::Result<()> {
    let resolver = crate::daemon::dns::RESOLVER;
    if !Path::new(resolver).exists() {
        return Ok(());
    }
    as_admin(&format!("rm -f {resolver}"), "Skiff wants to remove its *.skiff.local names.")?;
    println!("removed *.skiff.local");
    Ok(())
}

/// Runs a shell command as root through macOS's own password prompt. The
/// script runs in this process, not through `osascript`, so the prompt says
/// "Skiff wants to make changes". AppleScript belongs on the main thread: the
/// window's workers hand it there, and the CLI is already on it.
pub(crate) fn as_admin(command: &str, why: &str) -> io::Result<()> {
    let script = format!("do shell script \"{}\" with prompt \"{}\" with administrator privileges", command.replace('"', "\\\""), why.replace('"', "\\\""));
    // SAFETY: pthread_main_np only reads the calling thread's state.
    if unsafe { libc::pthread_main_np() } == 1 {
        return apple_script(&script);
    }
    let mut result = None;
    dispatch2::DispatchQueue::main().exec_sync(|| result = Some(apple_script(&script)));
    result.unwrap_or_else(|| Err(io::Error::other("the password prompt did not run")))
}

fn apple_script(source: &str) -> io::Result<()> {
    use objc2::AllocAnyThread;
    use objc2_foundation::{NSAppleScript, NSString};
    let script = NSAppleScript::initWithSource(NSAppleScript::alloc(), &NSString::from_str(source)).ok_or_else(|| io::Error::other("bad AppleScript"))?;
    let mut error: Option<objc2::rc::Retained<objc2_foundation::NSDictionary<NSString, objc2::runtime::AnyObject>>> = None;
    // Called directly: the generated binding panics on the nil it returns
    // when the script fails.
    // SAFETY: the selector takes an NSDictionary** out-parameter, and the
    // result is an optional NSAppleEventDescriptor.
    let _: Option<objc2::rc::Retained<objc2::runtime::AnyObject>> = unsafe { objc2::msg_send![&*script, executeAndReturnError: Some(&mut error)] };
    let Some(info) = error else { return Ok(()) };
    // "message (number)"; the number is -128 when the prompt is cancelled.
    let field = |key: &str| {
        info.objectForKey(&NSString::from_str(key)).map(|v| {
            // SAFETY: every object answers -description with an NSString.
            let d: objc2::rc::Retained<NSString> = unsafe { objc2::msg_send![&*v, description] };
            d.to_string()
        })
    };
    Err(io::Error::other(format!("{} ({})", field("NSAppleScriptErrorMessage").unwrap_or_default(), field("NSAppleScriptErrorNumber").unwrap_or_default())))
}

/// Skiff Lite, once its tools are downloaded: the CLI links and the build
/// builder that `skiff setup` could not make without them.
pub fn after_tools() -> io::Result<()> {
    if let Some(contents) = bundle_contents() {
        bundled_cli(&contents)?;
    }
    builder().map(drop)
}

/// The context, as `docker context create skiff --docker host=…` writes it.
fn context(socket: &Path) -> io::Result<()> {
    context_in(&docker_dir()?, socket)
}

fn context_in(docker: &Path, socket: &Path) -> io::Result<()> {
    let meta = json!({
        "Name": CONTEXT.get(),
        "Metadata": {"Description": crate::flavor::APP.get()},
        "Endpoints": {"docker": {"Host": format!("unix://{}", socket.display()), "SkipTLSVerify": false}},
    });
    write_json(&context_dir_in(docker, CONTEXT.get()).join("meta.json"), &meta)
}

/// Makes `name` the context plain `docker` uses: Skiff's own (made first if
/// missing), "default", or another context that exists, to switch back to.
pub fn use_context(name: &str) -> io::Result<()> {
    let docker = user_docker_dir()?;
    if name == CONTEXT.get() {
        context_in(&docker, &crate::daemon::socket_path()?)?;
    } else if name != "default" && !context_dir_in(&docker, name).join("meta.json").is_file() {
        return Err(io::Error::other(format!("there is no docker context `{name}`")));
    }
    set_current_context(&docker, name)
}

fn set_current_context(docker: &Path, name: &str) -> io::Result<()> {
    let path = docker.join("config.json");
    let mut cfg = read_json(&path)?;
    match cfg.as_object_mut() {
        Some(o) if name == "default" => {
            o.remove("currentContext");
        }
        _ => cfg["currentContext"] = name.into(),
    }
    write_json(&path, &cfg)
}

fn reset_current_context(docker: &Path, name: &str) -> io::Result<()> {
    if read_json(&docker.join("config.json"))?["currentContext"].as_str() == Some(name) {
        set_current_context(docker, "default")?;
    }
    Ok(())
}

/// Links `skiff` and the app's `docker` into `~/.skiff/bin`, puts that
/// directory on PATH, and adds docker's plugins (Compose, buildx) to the
/// CLI's plugin search path. An existing `docker` on PATH is left alone.
/// The docker CLI, Compose and buildx are bundled in Skiff.app, or in Skiff
/// Lite downloaded to tools::dir() (nothing yet if not downloaded); `skiff`
/// is linked either way.
fn bundled_cli(contents: &Path) -> io::Result<()> {
    let ours = bin_dir()?;
    std::fs::create_dir_all(&ours)?;
    // `skiff` itself, for `skiff k8s`, `skiff machine`, `skiff import`…
    let skiff = ours.join("skiff");
    let _ = std::fs::remove_file(&skiff);
    std::os::unix::fs::symlink(contents.join("MacOS/skiff"), &skiff)?;
    // Skiff Dev's commands are run by path: ~/.skiff-dev/bin/skiff.
    if !dev() {
        let h = home()?;
        shell_path(&h, &zdotdir(&h))?;
        system_link(&skiff);
    }
    let mut bin = contents.join("Resources/bin");
    if !bin.join("docker").is_file() {
        match crate::tools::dir() {
            Ok(d) if crate::tools::installed() => bin = d,
            _ => return Ok(()),
        }
    }
    let link = ours.join("docker");
    let _ = std::fs::remove_file(&link);
    if dev() {
        docker_launcher(&link, &bin.join("docker"), &docker_dir()?)?;
    } else {
        std::os::unix::fs::symlink(bin.join("docker"), &link)?;
    }
    // docker's default context stays the installed Skiff's to take.
    if !dev() {
        default_context()?;
    }
    // Dev's DOCKER_CONFIG is private (init_dev_docker), including when
    // invoked through ~/.skiff-dev/bin/docker. Never register Dev plugins
    // in the global config: Docker's first matching directory wins.
    let plugins = bin.join("cli-plugins").to_string_lossy().into_owned();
    let path = docker_dir()?.join("config.json");
    let mut cfg = read_json(&path)?;
    if add_plugin_dir(&mut cfg, &plugins) {
        write_json(&path, &cfg)?;
    }
    if dev() {
        println!("docker and skiff commands: {}", ours.display());
    } else {
        println!("docker and skiff commands: {} (on PATH in new terminal windows)", ours.display());
    }
    Ok(())
}

/// Adds `dir` to docker's cliPluginsExtraDirs, once. True if it was added.
fn add_plugin_dir(cfg: &mut Value, dir: &str) -> bool {
    let mut dirs: Vec<Value> = cfg["cliPluginsExtraDirs"].as_array().cloned().unwrap_or_default();
    if dirs.iter().any(|d| d.as_str() == Some(dir)) {
        return false;
    }
    dirs.push(dir.into());
    cfg["cliPluginsExtraDirs"] = dirs.into();
    true
}

/// Takes `app`'s plugin folders (`<app>.app`, wherever it was installed)
/// out of docker's cliPluginsExtraDirs, and no other app's: Skiff's
/// uninstall leaves Skiff Dev's, and Skiff Dev's leaves Skiff's. True if
/// any was removed.
fn remove_plugin_dirs(cfg: &mut Value, app: &str) -> bool {
    let ours = format!("/{app}.app/Contents/Resources/bin");
    // `get_mut`: indexing would add the key to a config that lacks it.
    let Some(dirs) = cfg.get_mut("cliPluginsExtraDirs").and_then(Value::as_array_mut) else { return false };
    let before = dirs.len();
    dirs.retain(|d| !d.as_str().is_some_and(|s| s.contains(&ours)));
    dirs.len() != before
}

/// `/usr/local/bin/skiff`, when the user can write there (Homebrew on Intel
/// Macs makes it theirs). That directory is on every shell's PATH already,
/// so `skiff` works even in a terminal opened before Skiff was. A `skiff`
/// there that isn't Skiff's is left alone.
const SYSTEM_LINK: &str = "/usr/local/bin/skiff";

fn system_link(skiff: &Path) {
    let link = Path::new(SYSTEM_LINK);
    // Missing, or a link an older Skiff left (to a Skiff.app or ~/.skiff/bin
    // that moved, or to nothing): made again to this one. Anything else there
    // isn't Skiff's to replace; stale::stale_skiff() names it.
    let replace = match std::fs::read_link(link) {
        Ok(target) => target != skiff && (skiff_link(&target) || !link.exists()),
        Err(_) => link.symlink_metadata().is_err(),
    };
    if replace {
        let _ = std::fs::remove_file(link);
        let _ = std::os::unix::fs::symlink(skiff, link);
    }
}

/// A link target that is some Skiff's `skiff`: inside a Skiff app bundle, or a
/// Skiff bin directory (~/.skiff/bin, ~/.skiff-dev/bin).
fn skiff_link(target: &Path) -> bool {
    let t = target.to_string_lossy();
    t.ends_with("/skiff") && (t.contains(".app/Contents/MacOS/") || t.contains("/.skiff/bin/") || t.contains("/.skiff-dev/bin/"))
}

/// Whether `/usr/local/bin/skiff` is the link system_link() makes.
fn system_link_is_ours() -> bool {
    bin_dir().is_ok_and(|b| std::fs::read_link(SYSTEM_LINK).is_ok_and(|t| t == b.join("skiff")))
}

/// Every other process running this app's `skiff` binary, asked to quit,
/// then made to after a few seconds.
fn stop_others() {
    let Some(exe) = std::env::current_exe().ok().and_then(|e| std::fs::canonicalize(e).ok()) else { return };
    let me = std::process::id().to_string();
    let pattern = format!("^{}( |$)", regex_escape(&exe.to_string_lossy()));
    for (sig, wait) in [("-TERM", 3), ("-KILL", 0)] {
        let Ok(out) = Command::new("/usr/bin/pgrep").args(["-f", &pattern]).output() else { return };
        let pids: Vec<String> = String::from_utf8_lossy(&out.stdout).split_whitespace().filter(|p| *p != me).map(str::to_owned).collect();
        if pids.is_empty() {
            return;
        }
        let _ = Command::new("/bin/kill").arg(sig).args(&pids).status();
        std::thread::sleep(std::time::Duration::from_secs(wait));
    }
}

fn regex_escape(s: &str) -> String {
    s.chars().fold(String::new(), |mut out, c| {
        if "\\.^$|?*+()[]{}".contains(c) {
            out.push('\\');
        }
        out.push(c);
        out
    })
}

/// The marked lines Skiff adds to shell profiles, and removes on uninstall.
const PROFILE_MARK: &str = "# Added by Skiff: its docker and skiff commands (skiff setup --uninstall removes this)";

/// Adds `~/.skiff/bin` to the end of PATH once, however many of the
/// profiles a shell reads. Plain sh, for zsh and bash alike.
const SH_LINE: &str = r#"case ":$PATH:" in *":$HOME/.skiff/bin:"*) ;; *) export PATH="$PATH:$HOME/.skiff/bin" ;; esac"#;

/// The line Skiff 0.1.0 wrote, only to `~/.zprofile`; taken back on uninstall.
const OLD_SH_LINE: &str = "export PATH=\"$PATH:$HOME/.skiff/bin\"";

const FISH_LINE: &str = "fish_add_path --append $HOME/.skiff/bin";

/// The shell profiles Skiff puts `~/.skiff/bin` in, so `skiff` is found in
/// every terminal:
/// - zsh (the macOS default) reads `.zprofile` only in login shells, which
///   Terminal opens but VS Code, tmux and IDE terminals do not, and a
///   `.zshrc` that sets PATH runs after it: both files, in `$ZDOTDIR` if set
/// - bash reads the first of `.bash_profile`, `.bash_login` and `.profile`
///   in login shells (`.bash_profile` is made if there is none) and
///   `.bashrc` in others, when the user has one
/// - fish's `conf.d`, when the user has fish
fn profiles(home: &Path, zdotdir: &Path) -> Vec<(PathBuf, &'static str)> {
    let mut v = vec![(zdotdir.join(".zprofile"), SH_LINE), (zdotdir.join(".zshrc"), SH_LINE)];
    let login = [".bash_profile", ".bash_login", ".profile"].iter().map(|f| home.join(f)).find(|p| p.is_file());
    v.push((login.unwrap_or_else(|| home.join(".bash_profile")), SH_LINE));
    if home.join(".bashrc").is_file() {
        v.push((home.join(".bashrc"), SH_LINE));
    }
    if home.join(".config/fish").is_dir() {
        v.push((home.join(".config/fish/conf.d/skiff.fish"), FISH_LINE));
    }
    v
}

fn zdotdir(home: &Path) -> PathBuf {
    std::env::var_os("ZDOTDIR").filter(|d| !d.is_empty()).map_or_else(|| home.to_path_buf(), PathBuf::from)
}

/// Puts ~/.skiff/bin on the shell's PATH in each profile. A profile that
/// can't be written is passed over (and logged), not the end of setup: a
/// link to a dotfiles folder that moved made the app's first launch quit
/// with only "No such file or directory" (#250).
fn shell_path(home: &Path, zdotdir: &Path) -> io::Result<()> {
    for (file, line) in profiles(home, zdotdir) {
        // A link to nothing (or to a place Skiff can't reach): writing
        // through it fails when the target's folder is gone (#250), or makes
        // a file where it points. It stays as the user left it.
        if file.symlink_metadata().is_ok() && !file.try_exists().unwrap_or(false) {
            crate::diag::warn!("shell profile skipped: a link to nothing", file = file.display());
            continue;
        }
        // Only a profile Skiff could read, or one that isn't there, is
        // written: a read error (not UTF-8, no read permission) must never
        // turn into a profile holding only Skiff's lines.
        let text = match std::fs::read_to_string(&file) {
            Ok(t) => t,
            Err(e) if e.kind() == io::ErrorKind::NotFound => String::new(),
            Err(e) => {
                crate::diag::warn!("shell profile skipped", file = file.display(), error = e);
                continue;
            }
        };
        if text.contains(PROFILE_MARK) {
            continue;
        }
        let sep = if text.is_empty() || text.ends_with('\n') { "" } else { "\n" };
        let written = file.parent().map_or(Ok(()), std::fs::create_dir_all).and_then(|()| std::fs::write(&file, format!("{text}{sep}\n{PROFILE_MARK}\n{line}\n")));
        if let Err(e) = written {
            crate::diag::warn!("shell profile skipped", file = file.display(), error = e);
        }
    }
    Ok(())
}

fn remove_shell_path(home: &Path, zdotdir: &Path) -> io::Result<()> {
    for (file, line) in profiles(home, zdotdir) {
        let Ok(text) = std::fs::read_to_string(&file) else { continue };
        if !text.contains(PROFILE_MARK) {
            continue;
        }
        let mut kept = text;
        for line in [line, OLD_SH_LINE] {
            kept = kept.replace(&format!("\n{PROFILE_MARK}\n{line}\n"), "\n").replace(&format!("{PROFILE_MARK}\n{line}\n"), "");
        }
        if kept.trim().is_empty() && file.extension().is_some_and(|e| e == "fish") {
            std::fs::remove_file(&file)?;
        } else {
            std::fs::write(&file, kept)?;
        }
    }
    Ok(())
}

/// On a Mac with no other Docker engine, plain `docker` uses Skiff. With
/// OrbStack or Docker Desktop around, the user's choice stays as it is.
fn default_context() -> io::Result<()> {
    let h = home()?;
    // Skiff's own /var/run/docker.sock is not another engine.
    let ours = crate::daemon::socket_path()?;
    let system = !matches!(docker_sock(&ours), DockerSock::Skiff) && Path::new(DOCKER_SOCK).exists();
    let other = system
        || [h.join(".orbstack/run/docker.sock"), h.join(".docker/run/docker.sock")].iter().any(|p| p.exists())
        || Path::new("/Applications/OrbStack.app").exists()
        || Path::new("/Applications/Docker.app").exists();
    let path = docker_dir()?.join("config.json");
    let mut cfg = read_json(&path)?;
    if other || cfg["currentContext"].as_str().is_some_and(|c| !c.is_empty() && c != "default") {
        return Ok(());
    }
    cfg["currentContext"] = CONTEXT.get().into();
    write_json(&path, &cfg)?;
    println!("docker uses Skiff by default (no other Docker engine on this Mac)");
    Ok(())
}

fn agent_plist() -> io::Result<PathBuf> {
    Ok(home()?.join("Library/LaunchAgents").join(format!("{AGENT}.plist")))
}

fn uid() -> u32 {
    // SAFETY: getuid cannot fail.
    unsafe { libc::getuid() }
}

/// A LaunchAgent for `skiff daemon`: started at login, restarted if it
/// crashes, never restarted after a clean exit.
fn agent() -> io::Result<PathBuf> {
    let exe = std::env::current_exe()?.canonicalize()?;
    let log = home()?.join(crate::flavor::STATE.get()).join("daemon.log");
    let esc = |p: &Path| p.to_string_lossy().replace('&', "&amp;").replace('<', "&lt;");
    let plist = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key><string>{AGENT}</string>
  <key>AssociatedBundleIdentifiers</key><array><string>{APP_ID}</string></array>
  <key>ProgramArguments</key><array><string>{}</string><string>daemon</string></array>
  <key>RunAtLoad</key><true/>
  <key>KeepAlive</key><dict><key>SuccessfulExit</key><false/></dict>
  <key>ProcessType</key><string>Interactive</string>
  <key>StandardOutPath</key><string>{}</string>
  <key>StandardErrorPath</key><string>{}</string>
</dict>
</plist>
"#,
        esc(&exe),
        esc(&log),
        esc(&log)
    );
    let path = agent_plist()?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    if let Some(dir) = log.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(&path, plist)?;
    // Reload, so a new path to the binary takes effect.
    let _ = Command::new("/bin/launchctl").args(["bootout", &format!("gui/{}/{AGENT}", uid())]).stderr(std::process::Stdio::null()).status();
    // bootout returns before launchd has let the old job go, and a
    // bootstrap in that moment fails (error 5): try again for a few seconds.
    let mut ok = false;
    for _ in 0..20 {
        ok = Command::new("/bin/launchctl").args(["bootstrap", &format!("gui/{}", uid())]).arg(&path).stderr(std::process::Stdio::null()).status()?.success();
        if ok {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(250));
    }
    if !ok {
        return Err(io::Error::other(format!("launchctl could not load {}", path.display())));
    }
    Ok(path)
}

/// Starts the engine through the LaunchAgent, loading it first if launchd
/// doesn't have it (Login Items turned off and on, or a plist written but
/// never loaded). Ok(false) when there is no agent for this copy of Skiff,
/// and the caller starts the engine itself.
pub fn start_agent() -> io::Result<bool> {
    if !agent_plist()?.is_file() || !agent_runs_this()? {
        return Ok(false);
    }
    let job = format!("gui/{}/{AGENT}", uid());
    let kicked = Command::new("/bin/launchctl").args(["kickstart", &job]).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).status()?.success();
    if kicked {
        return Ok(true);
    }
    // Not loaded: bootstrap it, and RunAtLoad starts it.
    let loaded = Command::new("/bin/launchctl").args(["bootstrap", &format!("gui/{}", uid())]).arg(agent_plist()?).stderr(std::process::Stdio::null()).status()?.success();
    if !loaded {
        crate::diag::info!("the LaunchAgent would not load; starting the engine directly");
    }
    Ok(loaded)
}

/// Whether the LaunchAgent starts this copy of Skiff. One left by another
/// copy (an older download, a build elsewhere) is replaced when this one opens.
fn agent_runs_this() -> io::Result<bool> {
    let exe = std::env::current_exe()?.canonicalize()?;
    let esc = exe.to_string_lossy().replace('&', "&amp;").replace('<', "&lt;");
    Ok(std::fs::read_to_string(agent_plist()?)?.contains(&format!("<string>{esc}</string><string>daemon</string>")))
}

/// Adds the app's name to a LaunchAgent written by an older Skiff. launchd
/// reads it at the next login, so the running daemon is left alone.
fn name_agent() -> io::Result<()> {
    let path = agent_plist()?;
    let text = std::fs::read_to_string(&path)?;
    if text.contains("AssociatedBundleIdentifiers") {
        return Ok(());
    }
    let named = text.replacen("  <key>ProgramArguments</key>", &format!("  <key>AssociatedBundleIdentifiers</key><array><string>{APP_ID}</string></array>\n  <key>ProgramArguments</key>"), 1);
    std::fs::write(&path, named)
}

/// The docker CLI to drive buildx with: on PATH, else the app's.
pub fn docker_cli() -> Option<PathBuf> {
    crate::client::docker_cli()
}

fn uninstall() -> io::Result<()> {
    let _ = Command::new("/bin/launchctl").args(["bootout", &format!("gui/{}/{AGENT}", uid())]).stderr(std::process::Stdio::null()).status();
    // Uninstalling stops everything this Skiff runs: the engine (the agent,
    // just booted out, which stops the VM and its containers) and any window
    // or skiff command still open from this app (`skiff share`, a second
    // daemon). Not this process: it's the one uninstalling.
    stop_others();
    match std::fs::remove_file(agent_plist()?) {
        Ok(()) => println!("removed the LaunchAgent"),
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(e) => return Err(e),
    }
    // Only a context that still points at Skiff's socket is ours to remove.
    let dir = context_dir(CONTEXT.get())?;
    let meta = read_json(&dir.join("meta.json"))?;
    if meta["Endpoints"]["docker"]["Host"].as_str().is_some_and(|h| h.ends_with(&format!("{}/run/docker.sock", crate::flavor::STATE))) {
        std::fs::remove_dir_all(&dir)?;
        println!("removed docker context `{CONTEXT}`");
    }
    let path = docker_dir()?.join("config.json");
    let mut cfg = read_json(&path)?;
    let mut changed = false;
    if !dev() && cfg["currentContext"].as_str() == Some(CONTEXT.get()) {
        if let Some(o) = cfg.as_object_mut() {
            o.remove("currentContext");
        }
        changed = true;
    }
    changed |= remove_plugin_dirs(&mut cfg, crate::flavor::APP.get());
    if changed {
        write_json(&path, &cfg)?;
    }
    if dev() && let Err(e) = user_docker_dir().and_then(|d| reset_current_context(&d, CONTEXT.get())) {
        println!("could not reset the user's Docker context ({e})");
    }
    // Docker's builder back as it was, before Skiff's goes.
    if let Err(e) = crate::buildwith::places().and_then(|p| p.uninstall()) {
        println!("could not give docker build back its own builder ({e})");
    }
    if let Some(docker) = docker_cli() {
        let _ = Command::new(docker).args(["buildx", "rm", BUILDER.get()]).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).status();
    }
    if !dev() && system_link_is_ours() {
        let _ = std::fs::remove_file(SYSTEM_LINK);
    }
    let _ = std::fs::remove_file(bin_dir()?.join("docker"));
    let _ = std::fs::remove_file(bin_dir()?.join("skiff"));
    // The cluster's kubectl context (`skiff k8s`); the cluster stays in Skiff's data.
    crate::k8s::forget();
    // Every flavor has a helper of its own.
    if let Err(e) = crate::usb::helper::remove() {
        println!("could not remove the USB helper ({e})");
    }
    if dev() {
        println!("{0}'s data stays in ~/Library/Application Support/{0}; delete it to remove images and volumes too", crate::flavor::APP);
        return Ok(());
    }
    match home().and_then(|h| remove_shell_path(&h, &zdotdir(&h))) {
        Ok(()) => println!("removed ~/.skiff/bin from shell profiles"),
        Err(e) => println!("could not tidy the shell profiles ({e})"),
    }
    if let Err(e) = remove_domains() {
        println!("could not remove {} ({e})", crate::daemon::dns::RESOLVER);
    }
    if let Err(e) = remove_docker_socket() {
        println!("could not remove {DOCKER_SOCK} ({e})");
    }
    println!("Skiff's data stays in ~/Library/Application Support/Skiff; delete it to remove images and volumes too");
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn dev_and_feature_plugins_are_isolated_in_both_install_orders() {
        use super::*;
        for stable_first in [true, false] {
            let root = scratch(if stable_first { "stable-first" } else { "dev-first" });
            let global = root.join("global");
            let original = json!({"currentContext": "desktop-linux", "auths": {"registry.example": {"auth": "test-only"}}, "credsStore": "osxkeychain"});
            write_json(&global.join("config.json"), &original).unwrap();
            for flavor in ["dev", "builds"] {
                if stable_first {
                    prefer_plugins(&global, Path::new("/Applications/Skiff.app/Contents/Resources/bin/cli-plugins")).unwrap();
                }
                let isolated = root.join(flavor);
                isolate_docker_config(&global, &isolated, &format!("skiff-{flavor}")).unwrap();
                let plugins = root.join(flavor).join("bundled-plugins");
                prefer_plugins(&isolated, &plugins).unwrap();
                if !stable_first {
                    prefer_plugins(&global, Path::new("/Applications/Skiff.app/Contents/Resources/bin/cli-plugins")).unwrap();
                }
                let mut cfg = read_json(&isolated.join("config.json")).unwrap();
                assert_eq!(cfg["currentContext"], format!("skiff-{flavor}"));
                assert_eq!(cfg["auths"], original["auths"]);
                assert_eq!(cfg["credsStore"], original["credsStore"]);
                assert_eq!(cfg["cliPluginsExtraDirs"][0], json!(plugins));
                assert!(cfg["cliPluginsExtraDirs"].as_array().unwrap().contains(&json!(global.join("cli-plugins"))));
                assert_eq!(isolated.join("contexts").canonicalize().unwrap(), global.join("contexts").canonicalize().unwrap());
                // A later setup/launch must not overwrite a login in Dev.
                cfg["auths"]["new-registry"] = json!({"auth": "dev-only"});
                write_json(&isolated.join("config.json"), &cfg).unwrap();
                isolate_docker_config(&global, &isolated, &format!("skiff-{flavor}")).unwrap();
                let after = read_json(&isolated.join("config.json")).unwrap();
                assert_eq!(after["auths"], cfg["auths"]);
                assert_eq!(after["currentContext"], cfg["currentContext"]);
            }
            let mut global_cfg = read_json(&global.join("config.json")).unwrap();
            assert_eq!(global_cfg.as_object_mut().unwrap().remove("cliPluginsExtraDirs").unwrap(), json!(["/Applications/Skiff.app/Contents/Resources/bin/cli-plugins"]));
            assert_eq!(global_cfg, original);
            std::fs::remove_dir_all(root).unwrap();
        }
    }

    #[test]
    fn damaged_source_config_does_not_prevent_private_setup() {
        use super::*;
        let root = scratch("bad-source");
        for (i, contents) in ["not json", "[]", "null"].iter().enumerate() {
            let source = root.join(format!("source-{i}"));
            std::fs::create_dir_all(&source).unwrap();
            std::fs::write(source.join("config.json"), contents).unwrap();
            let private = root.join(format!("dev-{i}"));
            assert!(!isolate_docker_config(&source, &private, "skiff-dev").unwrap().is_empty());
            assert_eq!(read_json(&private.join("config.json")).unwrap()["currentContext"], "skiff-dev");
            assert_eq!(std::fs::read_to_string(source.join("config.json")).unwrap(), *contents);
        }
        let source = root.join("not-a-directory");
        std::fs::write(&source, "unusable Docker directory").unwrap();
        let private = root.join("fallback");
        assert_eq!(isolate_docker_config(&source, &private, "skiff-dev").unwrap().len(), 2);
        assert!(private.join("contexts").is_dir());
        assert!(std::fs::symlink_metadata(private.join("contexts")).unwrap().is_dir());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn reinstalls_and_docker_resets_recover_the_private_context() {
        use super::*;
        let root = scratch("context-recovery");
        let source = root.join("global");
        let private = root.join("dev");
        isolate_docker_config(&source, &private, "skiff-dev").unwrap();
        // A previous uninstall left the private config, but cleared context.
        set_current_context(&private, "default").unwrap();
        std::fs::remove_dir_all(source.join("contexts")).unwrap();
        assert!(!private.join("contexts").exists());
        isolate_docker_config(&source, &private, "skiff-dev").unwrap();
        assert_eq!(read_json(&private.join("config.json")).unwrap()["currentContext"], "skiff-dev");
        assert!(private.join("contexts").is_dir());
        assert_eq!(private.join("contexts").canonicalize().unwrap(), source.join("contexts").canonicalize().unwrap());
        // Changing DOCKER_CONFIG must relink to that source, not the old one.
        let other = root.join("other-docker");
        isolate_docker_config(&other, &private, "skiff-dev").unwrap();
        assert_eq!(private.join("contexts").canonicalize().unwrap(), other.join("contexts").canonicalize().unwrap());
        // A local fallback still works after the source becomes unavailable.
        std::fs::remove_dir_all(&other).unwrap();
        std::fs::write(&other, "not a directory").unwrap();
        assert!(!isolate_docker_config(&other, &private, "skiff-dev").unwrap().is_empty());
        assert!(std::fs::symlink_metadata(private.join("contexts")).unwrap().is_dir());
        std::fs::write(private.join("contexts/keep"), "local context data").unwrap();
        isolate_docker_config(&source, &private, "skiff-dev").unwrap();
        assert!(private.join("contexts/keep").is_file());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn private_plugin_precedence_keeps_other_plugins_and_user_choices() {
        use super::*;
        let root = scratch("plugin-fallbacks");
        let source = root.join("global");
        let private = root.join("dev");
        write_json(&source.join("config.json"), &json!({"cliPluginsExtraDirs": ["/other/plugins"]})).unwrap();
        isolate_docker_config(&source, &private, "skiff-dev").unwrap();
        prefer_plugins(&private, Path::new("/dev/plugins")).unwrap();
        let mut cfg = read_json(&private.join("config.json")).unwrap();
        add_plugin_dir(&mut cfg, "/custom/private/plugins");
        write_json(&private.join("config.json"), &cfg).unwrap();
        prefer_plugins(&private, Path::new("/dev/plugins")).unwrap();
        let dirs = read_json(&private.join("config.json")).unwrap()["cliPluginsExtraDirs"].clone();
        assert_eq!(dirs, json!(["/dev/plugins", "/other/plugins", source.join("cli-plugins"), "/custom/private/plugins"]));
        set_current_context(&source, "skiff-dev").unwrap();
        reset_current_context(&source, "skiff-dev").unwrap();
        assert!(read_json(&source.join("config.json")).unwrap()["currentContext"].is_null());
        assert_eq!(read_json(&private.join("config.json")).unwrap()["currentContext"], "skiff-dev");
        set_current_context(&source, "someone-else").unwrap();
        reset_current_context(&source, "skiff-dev").unwrap();
        assert_eq!(read_json(&source.join("config.json")).unwrap()["currentContext"], "someone-else");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn docker_launcher_uses_private_config_and_preserves_arguments() {
        use std::os::unix::fs::PermissionsExt;
        let root = scratch("launcher");
        let config = root.join("config with a ' quote");
        let docker = root.join("docker with a ' quote");
        std::fs::write(&docker, "#!/bin/sh\nprintf '%s\\n' \"$DOCKER_CONFIG\" \"$@\"\n").unwrap();
        std::fs::set_permissions(&docker, std::fs::Permissions::from_mode(0o700)).unwrap();
        let launcher = root.join("docker");
        super::docker_launcher(&launcher, &docker, &config).unwrap();
        let out = std::process::Command::new(launcher).env("DOCKER_CONFIG", "someone-elses-config").args(["compose", "a b", "'quoted'", "$(touch never)"]).output().unwrap();
        assert!(out.status.success());
        assert_eq!(String::from_utf8(out.stdout).unwrap(), format!("{}\ncompose\na b\n'quoted'\n$(touch never)\n", config.display()));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn runs_apple_script() {
        super::apple_script("do shell script \"true\"").unwrap();
        let e = super::apple_script("error number -128").unwrap_err();
        assert!(e.to_string().contains("-128"), "{e}");
    }

    fn scratch(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("skiff-setup-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn docker_sock_owner() {
        use super::DockerSock::*;
        let d = scratch("sock");
        let ours = d.join("skiff/run/docker.sock");
        let link = d.join("docker.sock");
        let at = |engines: &[std::path::PathBuf]| super::docker_sock_at(&link, &ours, engines);
        assert_eq!(at(&[]), Free);
        std::os::unix::fs::symlink(&ours, &link).unwrap();
        assert_eq!(at(&[]), Skiff, "Skiff's link, even with the daemon stopped");
        // OrbStack's link with OrbStack not running: still OrbStack's.
        std::fs::remove_file(&link).unwrap();
        let orb = d.join(".orbstack/run/docker.sock");
        std::os::unix::fs::symlink(&orb, &link).unwrap();
        assert_eq!(at(&[d.join(".orbstack")]), Taken(orb.display().to_string()));
        // ... and free once OrbStack is gone.
        assert_eq!(at(&[]), Free);
        // Another engine's own socket.
        std::fs::remove_file(&link).unwrap();
        let listener = std::os::unix::net::UnixListener::bind(&link).unwrap();
        assert!(matches!(at(&[]), Taken(_)));
        drop(listener);
        std::fs::remove_file(&link).unwrap();
        std::fs::write(&link, "").unwrap();
        assert!(matches!(at(&[]), Taken(_)), "a plain file is left alone");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn testcontainers_file_is_skiffs_only_when_marked() {
        let d = scratch("tc");
        let path = d.join(".testcontainers.properties");
        let ours = std::path::Path::new("/Users/me/.skiff/run/docker.sock");
        assert!(super::testcontainers_at(&path, ours, true).unwrap());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), super::testcontainers_text(ours));
        assert!(std::fs::read_to_string(&path).unwrap().contains("docker.host=unix:///Users/me/.skiff/run/docker.sock\n"));
        assert!(!super::testcontainers_at(&path, ours, false).unwrap());
        assert!(!path.exists());
        // The user's own file, or one Testcontainers has added to, stays.
        for text in ["docker.host=tcp://remote:2375\n", &format!("{}testcontainers.reuse.enable=true\n", super::testcontainers_text(ours))] {
            std::fs::write(&path, text).unwrap();
            assert!(!super::testcontainers_at(&path, ours, true).unwrap());
            assert!(!super::testcontainers_at(&path, ours, false).unwrap());
            assert_eq!(std::fs::read_to_string(&path).unwrap(), text);
        }
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn quotes_for_sh() {
        assert_eq!(super::sh_quote("/Users/o'neil/.skiff"), r"'/Users/o'\''neil/.skiff'");
    }

    // #250: a profile that is a link to nowhere (a dotfiles folder that
    // moved) is passed over, not the end of the app's first launch.
    #[test]
    fn a_dangling_profile_link_does_not_stop_setup() {
        let h = scratch("dangling");
        std::os::unix::fs::symlink(h.join("dotfiles-gone/zshrc"), h.join(".zshrc")).unwrap();
        super::shell_path(&h, &h).unwrap();
        assert!(std::fs::read_link(h.join(".zshrc")).is_ok(), "the user's link stays a link");
        assert!(!h.join("dotfiles-gone").exists(), "nothing made where it points");
        assert!(std::fs::read_to_string(h.join(".zprofile")).unwrap().contains(super::PROFILE_MARK), "the other profiles still get the line");
        super::remove_shell_path(&h, &h).unwrap();
        let _ = std::fs::remove_dir_all(&h);
    }

    // #250, as Nix home-manager sets a Mac up: a profile Skiff can read but
    // not write (a link into a read-only store) is passed over, whether or
    // not Skiff's line is in it already, and the other profiles get the line.
    #[test]
    fn a_read_only_profile_does_not_stop_setup() {
        use std::os::unix::fs::PermissionsExt;
        let with_line = format!("# managed elsewhere\n\n{}\n{}\n", super::PROFILE_MARK, super::SH_LINE);
        for (round, text) in ["# managed elsewhere\n", with_line.as_str()].into_iter().enumerate() {
            let h = scratch(&format!("readonly{round}"));
            let managed = h.join("store/zshrc");
            std::fs::create_dir_all(h.join("store")).unwrap();
            std::fs::write(&managed, text).unwrap();
            std::fs::set_permissions(&managed, std::fs::Permissions::from_mode(0o444)).unwrap();
            // root writes through the mode: nothing read-only to try then.
            if std::fs::OpenOptions::new().append(true).open(&managed).is_ok() {
                let _ = std::fs::remove_dir_all(&h);
                return;
            }
            std::os::unix::fs::symlink(&managed, h.join(".zshrc")).unwrap();
            super::shell_path(&h, &h).unwrap();
            assert_eq!(std::fs::read_to_string(&managed).unwrap(), text, "the read-only profile is as it was");
            assert!(std::fs::read_link(h.join(".zshrc")).is_ok(), "the user's link stays a link");
            assert!(std::fs::read_to_string(h.join(".zprofile")).unwrap().contains(super::PROFILE_MARK), "the other profiles still get the line");
            let _ = std::fs::remove_dir_all(&h);
        }
    }

    // A profile Skiff can't read as text is left exactly as it was, never
    // replaced by Skiff's lines alone.
    #[test]
    fn an_unreadable_profile_is_left_alone() {
        let h = scratch("unreadable");
        let latin1: &[u8] = b"# caf\xe9 settings\nexport EDITOR=vim\n";
        std::fs::write(h.join(".zshrc"), latin1).unwrap();
        super::shell_path(&h, &h).unwrap();
        assert_eq!(std::fs::read(h.join(".zshrc")).unwrap(), latin1);
        assert!(std::fs::read_to_string(h.join(".zprofile")).unwrap().contains(super::PROFILE_MARK));
        let _ = std::fs::remove_dir_all(&h);
    }

    #[test]
    fn shell_profiles_put_skiff_on_path_once_and_take_it_back() {
        let h = scratch("profiles");
        std::fs::write(h.join(".zprofile"), "export A=1").unwrap();
        // An install by Skiff 0.1.0: the old line, in .zprofile only.
        std::fs::write(h.join(".profile"), format!("export B=2\n\n{}\n{}\n", super::PROFILE_MARK, super::OLD_SH_LINE)).unwrap();
        std::fs::write(h.join(".bashrc"), "").unwrap();
        super::shell_path(&h, &h).unwrap();
        super::shell_path(&h, &h).unwrap();
        let read = |f: &str| std::fs::read_to_string(h.join(f)).unwrap();
        for f in [".zprofile", ".zshrc", ".bashrc"] {
            assert_eq!(read(f).matches(super::SH_LINE).count(), 1, "{f}: {}", read(f));
        }
        assert!(read(".zprofile").starts_with("export A=1\n\n"));
        // bash's login file is .profile here: no .bash_profile to shadow it.
        assert!(!h.join(".bash_profile").exists());
        assert!(!read(".profile").contains(super::SH_LINE));
        super::remove_shell_path(&h, &h).unwrap();
        assert_eq!(read(".zprofile").trim_end(), "export A=1");
        assert_eq!(read(".profile").trim_end(), "export B=2");
        assert_eq!(read(".zshrc").trim(), "");
        assert!(!read(".bashrc").contains("skiff"));
    }

    #[test]
    fn path_line_works_in_sh_and_adds_once() {
        let out = std::process::Command::new("/bin/sh")
            .arg("-c")
            .arg(format!("{0}\n{0}\nprintf %s \"$PATH\"", super::SH_LINE))
            .env("HOME", "/h")
            .env("PATH", "/usr/bin:/bin")
            .output()
            .unwrap();
        assert_eq!(String::from_utf8_lossy(&out.stdout), "/usr/bin:/bin:/h/.skiff/bin");
    }

    #[test]
    fn each_app_adds_and_removes_only_its_own_plugin_folder() {
        use serde_json::json;
        let skiff = "/Applications/Skiff.app/Contents/Resources/bin/cli-plugins";
        let skiff_moved = "/Users/me/Downloads/Skiff.app/Contents/Resources/bin/cli-plugins";
        let dev = "/Users/me/src/skiff/target/dev/Skiff Dev.app/Contents/Resources/bin/cli-plugins";
        let builds = "/Users/me/src/skiff/target/dev/Skiff Builds.app/Contents/Resources/bin/cli-plugins";
        let theirs = "/opt/other/cli-plugins";
        let mut cfg = json!({"auths": {}, "cliPluginsExtraDirs": [theirs]});
        for dir in [skiff, skiff_moved, dev, builds] {
            assert!(super::add_plugin_dir(&mut cfg, dir), "{dir}");
            assert!(!super::add_plugin_dir(&mut cfg, dir), "listed twice: {dir}");
        }
        assert_eq!(cfg["cliPluginsExtraDirs"], json!([theirs, skiff, skiff_moved, dev, builds]));

        // Skiff Dev's uninstall leaves Skiff's folders and a feature app's.
        assert!(super::remove_plugin_dirs(&mut cfg, "Skiff Dev"));
        assert_eq!(cfg["cliPluginsExtraDirs"], json!([theirs, skiff, skiff_moved, builds]));
        // Skiff's takes both of its own, wherever it was, and no other app's.
        assert!(super::remove_plugin_dirs(&mut cfg, "Skiff"));
        assert_eq!(cfg["cliPluginsExtraDirs"], json!([theirs, builds]));
        assert!(!super::remove_plugin_dirs(&mut cfg, "Skiff"));
        assert_eq!(cfg["auths"], json!({}));

        // A config without the list is left without one.
        let mut bare = json!({"auths": {}});
        assert!(!super::remove_plugin_dirs(&mut bare, "Skiff"));
        assert_eq!(bare, json!({"auths": {}}));
        // Adding to it makes the list.
        assert!(super::add_plugin_dir(&mut bare, dev));
        assert_eq!(bare["cliPluginsExtraDirs"], json!([dev]));
    }
}
