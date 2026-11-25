//! Mac App Sandbox storage, explicit folder grants, and app-owned engine lifetime.
//! This module is compiled only for the App Store / TestFlight edition.
use std::cell::RefCell;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Mutex, OnceLock};

use objc2::{MainThreadMarker, rc::Retained, runtime::Bool};
use objc2_app_kit::NSOpenPanel;
use objc2_foundation::{NSData, NSHomeDirectory, NSString, NSURL, NSURLBookmarkCreationOptions, NSURLBookmarkResolutionOptions};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
struct Grant { id: String, path: PathBuf, bookmark: Vec<u8> }

static ROOTS: OnceLock<Vec<(String, PathBuf)>> = OnceLock::new();
static ENGINE: Mutex<Option<Child>> = Mutex::new(None);
thread_local! {
    // Keep each security scope alive until this process exits. The engine gets
    // its own scopes at launch; changing grants takes effect on app relaunch.
    static SCOPES: RefCell<Vec<Retained<NSURL>>> = const { RefCell::new(Vec::new()) };
}

fn grants_file() -> io::Result<PathBuf> { Ok(crate::daemon::state_dir()?.join("folders.json")) }
fn session_file() -> io::Result<PathBuf> { Ok(crate::daemon::state_dir()?.join("folder-session.json")) }
fn grants() -> io::Result<Vec<Grant>> { read_grants(grants_file()?) }
fn read_grants(path: PathBuf) -> io::Result<Vec<Grant>> {
    match std::fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes).map_err(io::Error::other),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(e) => Err(e),
    }
}
fn save(grants: &[Grant]) -> io::Result<()> { save_at(grants, grants_file()?) }
fn save_at(grants: &[Grant], path: PathBuf) -> io::Result<()> {
    use std::os::unix::fs::OpenOptionsExt;
    use std::io::Write;
    std::fs::create_dir_all(path.parent().unwrap())?;
    let tmp = path.with_extension(format!("{}.tmp", std::process::id()));
    let mut file = std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&tmp)?;
    file.write_all(&serde_json::to_vec(grants).map_err(io::Error::other)?)?;
    file.sync_all()?;
    std::fs::rename(tmp, path)
}

/// The data folder's `etc`, the one part of it shared with the VM, made a
/// folder of its own first. An earlier TestFlight build shared the whole data
/// folder, so a container could have left a link at `etc` leading back into
/// it, and the share would follow the link. What is in it is cleared by
/// `clean_etc_share`, in the engine's process only.
fn etc_share(data: &Path) -> io::Result<PathBuf> {
    let etc = data.join("etc");
    if std::fs::symlink_metadata(&etc).is_ok_and(|m| !m.is_dir()) {
        std::fs::remove_file(&etc)?;
    }
    std::fs::create_dir_all(&etc)?;
    Ok(etc)
}

/// Leaves nothing in the shared `etc` folder but Skiff's kept files of their
/// own (one link). The earlier TestFlight build shared the whole data folder,
/// so a container could have left anything in it: another name, a folder, or
/// a hard link to the VM's disk, which the share would expose. Skiff writes
/// the kept files again as they are mounted (`hostetc`).
///
/// Only the engine's process calls this, holding the engine lock and before
/// the VM starts (`daemon::run`). Every process runs `init`, and some start
/// while containers run (Cloudflare's helper): one clearing the folder then
/// would remove `hostetc`'s file before its rename, or files a container
/// wrote through a bind of /etc.
pub fn clean_etc_share() -> io::Result<()> {
    clean_etc(&crate::daemon::hostetc::dir()?)
}

fn clean_etc(etc: &Path) -> io::Result<()> {
    use std::os::unix::fs::MetadataExt;
    // An entry gone since the listing is fine.
    let gone = |e: io::Error| if e.kind() == io::ErrorKind::NotFound { Ok(()) } else { Err(e) };
    for entry in std::fs::read_dir(etc)? {
        let entry = entry?;
        // DirEntry::metadata does not follow a link.
        let meta = match entry.metadata() {
            Ok(meta) => meta,
            Err(e) => { gone(e)?; continue; }
        };
        let own = meta.is_file() && meta.nlink() == 1 && entry.file_name().to_str().is_some_and(crate::daemon::hostetc::kept);
        if own {
            continue;
        }
        // remove_dir_all follows no link; removing a hard link leaves its file.
        let removed = if meta.is_dir() { std::fs::remove_dir_all(entry.path()) } else { std::fs::remove_file(entry.path()) };
        removed.or_else(gone)?;
    }
    Ok(())
}

/// Called before logging or threads, including in helper processes. Cocoa's
/// home is the sandbox container; an inherited shell HOME is not authoritative.
pub fn init() -> io::Result<()> {
    let home = PathBuf::from(NSHomeDirectory().to_string());
    // Refuse an accidentally unsigned build instead of writing to the real home.
    if !home.components().any(|c| c.as_os_str() == "Containers") {
        return Err(io::Error::other("the TestFlight build must be launched from its sandbox-signed app bundle"));
    }
    let state = home.join(".skiff");
    std::fs::create_dir_all(&state)?;
    // SAFETY: main calls this before starting threads.
    unsafe {
        std::env::set_var("HOME", &home);
        std::env::set_var("SKIFF_DATA_DIR", home.join("Library/Application Support/Skiff"));
        std::env::set_var("DOCKER_CONFIG", state.join("docker"));
        // Keep the Unix socket below macOS's 104-byte limit, even for long user names.
        std::env::set_var("SKIFF_SOCKET", std::env::temp_dir().join("skiff.sock"));
        std::env::remove_var("SSH_AUTH_SOCK");
    }
    // Only the Linux /etc files Skiff keeps for bind mounts (`hostetc::dir`)
    // are shared with the VM, never the rest of the data folder: its VM disk,
    // image store and engine.lock must stay out of containers' reach. It
    // must exist before the VM starts, or the share is left out.
    let etc = etc_share(&home.join("Library/Application Support/Skiff"))?;
    let mut roots = vec![("etc".into(), etc)];
    let helper = std::env::current_exe()?.file_name().is_some_and(|n| n == "skiff-helper");
    let mut saved = if helper { read_grants(session_file()?)? } else { grants()? };
    let mut session = Vec::new();
    let mut refreshed = unique_grants(&mut saved);
    for grant in &mut saved {
        let mut stale = Bool::NO;
        // SAFETY: stale is live for the duration of this synchronous call.
        let url = unsafe { NSURL::URLByResolvingBookmarkData_options_relativeToURL_bookmarkDataIsStale_error(
            &NSData::with_bytes(&grant.bookmark),
            NSURLBookmarkResolutionOptions::WithoutUI | if helper { NSURLBookmarkResolutionOptions::empty() } else { NSURLBookmarkResolutionOptions::WithSecurityScope },
            None, &mut stale,
        ) };
        let Ok(url) = url else { eprintln!("skiff: reselect an unavailable folder in Shared Folders"); continue };
        // SAFETY: balanced by process teardown; retained in SCOPES below.
        if !helper && !unsafe { url.startAccessingSecurityScopedResource() } { continue; }
        let Some(path) = url.path().map(|p| PathBuf::from(p.to_string())) else { continue };
        if !helper && stale.as_bool() {
            if let Ok(data) = bookmark(&url, true) { grant.bookmark = data; refreshed = true; }
        }
        if path != grant.path { grant.path = path.clone(); refreshed = true; }
        if !helper { session.push(Grant { id: grant.id.clone(), path: path.clone(), bookmark: bookmark(&url, false)? }); }
        roots.push((grant.id.clone(), path));
        SCOPES.with(|s| s.borrow_mut().push(url));
    }
    if !helper {
        if refreshed { save(&saved)?; }
        // Standard bookmarks carry temporary sandbox extensions to our child.
        // App-scoped persistent bookmarks are bound to the GUI signing identity.
        save_at(&session, session_file()?)?;
    }
    ROOTS.set(roots).map_err(|_| io::Error::other("sandbox initialized twice"))?;
    Ok(())
}

fn bookmark(url: &NSURL, scoped: bool) -> io::Result<Vec<u8>> {
    url.bookmarkDataWithOptions_includingResourceValuesForKeys_relativeToURL_error(
        if scoped { NSURLBookmarkCreationOptions::WithSecurityScope } else { NSURLBookmarkCreationOptions::empty() }, None, None,
    ).map(|d| d.to_vec()).map_err(|e| io::Error::other(e.to_string()))
}

/// Native user selection creates the only grants for paths outside our container.
pub fn choose_folder() -> io::Result<()> {
    let mtm = MainThreadMarker::new().ok_or_else(|| io::Error::other("folder selection needs the main thread"))?;
    let panel = NSOpenPanel::openPanel(mtm);
    panel.setCanChooseDirectories(true);
    panel.setCanChooseFiles(false);
    panel.setAllowsMultipleSelection(false);
    panel.setPrompt(Some(&NSString::from_str("Share Folder")));
    panel.setMessage(Some(&NSString::from_str("Allow Skiff containers to read and write this folder. Quit and reopen Skiff after changing shared folders.")));
    if panel.runModal() != 1 { return Ok(()); }
    let Some(url) = panel.URL() else { return Ok(()) };
    let path = PathBuf::from(url.path().ok_or_else(|| io::Error::other("choose a local folder"))?.to_string()).canonicalize()?;
    let mut saved = grants()?;
    if let Some(grant) = saved.iter_mut().find(|g| g.path == path) {
        grant.bookmark = bookmark(&url, true)?;
        return save(&saved);
    }
    let id = new_grant_id(&saved, || {
        let mut bytes = [0u8; 8];
        std::fs::File::open("/dev/urandom")?.read_exact(&mut bytes)?;
        Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
    })?;
    saved.push(Grant { id, path, bookmark: bookmark(&url, true)? });
    save(&saved)
}

fn valid_grant_id(id: &str) -> bool { id.len() == 16 && id.bytes().all(|b| b.is_ascii_hexdigit()) }
fn unique_grants(saved: &mut Vec<Grant>) -> bool {
    let before = saved.len();
    let mut ids = std::collections::HashSet::new();
    saved.retain(|g| valid_grant_id(&g.id) && ids.insert(g.id.clone()));
    saved.len() != before
}
fn new_grant_id(saved: &[Grant], mut candidate: impl FnMut() -> io::Result<String>) -> io::Result<String> {
    loop {
        let id = candidate()?;
        if valid_grant_id(&id) && !saved.iter().any(|g| g.id == id) { return Ok(id); }
    }
}

pub fn folders() -> serde_json::Value {
    match grants() {
        Ok(g) => serde_json::json!({"type":"sharedfolders", "folders":g.iter().map(|g| serde_json::json!({"id":g.id,"path":g.path})).collect::<Vec<_>>() }),
        Err(e) => serde_json::json!({"type":"sharedfolders", "folders":[], "error":e.to_string()}),
    }
}
pub fn remove_folder(id: &str) -> io::Result<()> {
    let mut saved = grants()?;
    saved.retain(|g| g.id != id);
    save(&saved)
}

pub fn shares() -> Vec<(String, PathBuf)> { ROOTS.get().cloned().unwrap_or_default() }

fn map_host(roots: &[(String, PathBuf)], path: &Path) -> Option<String> {
    roots.iter().filter_map(|(id, root)| path.strip_prefix(root).ok().map(|rest| (root.components().count(), id, rest)))
        .max_by_key(|(depth, _, _)| *depth)
        .map(|(_, id, rest)| if rest.as_os_str().is_empty() { format!("/mnt/mac/{id}") } else { format!("/mnt/mac/{id}/{}", rest.display()) })
}
pub fn guest_path(path: &str) -> Option<String> {
    // Resolve symlinks before checking the grant boundary. Never authorize a
    // symlink that escapes a selected folder or a lexical '..' traversal.
    map_existing(ROOTS.get()?, Path::new(path))
}
fn map_existing(roots: &[(String, PathBuf)], path: &Path) -> Option<String> {
    if path.components().any(|c| matches!(c, std::path::Component::ParentDir)) { return None; }
    map_host(roots, &path.canonicalize().ok()?)
}

/// Deleted files still need watch notifications. Resolve the nearest existing
/// parent, rejecting dangling links and parent traversals, before mapping it.
pub fn guest_event_path(path: &str, guest_root: &str) -> Option<String> {
    map_event(ROOTS.get()?, Path::new(path), guest_root)
}

fn map_event(roots: &[(String, PathBuf)], path: &Path, guest_root: &str) -> Option<String> {
    let host_root = map_guest(roots, guest_root)?;
    path.strip_prefix(&host_root).ok()?;
    let id = Path::new(guest_root).strip_prefix("/mnt/mac").ok()?.components().next()?.as_os_str();
    let grant = roots.iter().find(|(key, _)| key.as_str() == id)?;
    // The bind's stored grant decides, even if a more specific grant was
    // selected later. Events must reach every alias used by a watched bind.
    map_missing(std::slice::from_ref(grant), path)
}

/// Also used before Docker's `-v` creates a missing bind directory. Check
/// its existing ancestor first, and recheck the final path after creation.
pub fn guest_bind_path(path: &str) -> Option<String> {
    map_missing(ROOTS.get()?, Path::new(path))
}

fn map_missing(roots: &[(String, PathBuf)], path: &Path) -> Option<String> {
    if path.components().any(|c| matches!(c, std::path::Component::ParentDir)) { return None; }
    let mut parent = path;
    let mut suffix = Vec::new();
    let mut canonical = loop {
        match parent.canonicalize() {
            Ok(p) => break p,
            Err(e) if e.kind() == io::ErrorKind::NotFound && parent.symlink_metadata().is_err() => {
                suffix.push(parent.file_name()?);
                parent = parent.parent()?;
            }
            Err(_) => return None,
        }
    };
    for name in suffix.into_iter().rev() { canonical.push(name); }
    map_host(roots, &canonical)
}

pub fn host_path(path: &str) -> Option<String> {
    map_guest(ROOTS.get()?, path)
}
fn map_guest(roots: &[(String, PathBuf)], path: &str) -> Option<String> {
    let relative = Path::new(path).strip_prefix("/mnt/mac").ok()?;
    let mut parts = relative.components();
    let id = parts.next()?.as_os_str();
    let (_, root) = roots.iter().find(|(key, _)| key.as_str() == id)?;
    let rest = parts.as_path();
    if rest.components().any(|p| !matches!(p, std::path::Component::Normal(_))) { return None; }
    Some(if rest.as_os_str().is_empty() { root.clone() } else { root.join(rest) }.to_string_lossy().into_owned())
}

pub fn helper() -> io::Result<PathBuf> {
    Ok(std::env::current_exe()?.parent().ok_or_else(|| io::Error::other("app bundle missing"))?.join("skiff-helper"))
}

/// Held by the helper until process exit, including after its socket closes
/// for shutdown. A newly opened app must not boot the same disk meanwhile.
pub fn lock_engine() -> io::Result<std::fs::File> {
    lock_engine_at(&skiff_image::Store::default_root()?)
}

fn lock_engine_at(data: &Path) -> io::Result<std::fs::File> {
    use std::os::fd::AsRawFd;
    use std::os::unix::fs::OpenOptionsExt;
    std::fs::create_dir_all(data)?;
    let lock = std::fs::OpenOptions::new().write(true).create(true).truncate(false).mode(0o600).open(data.join("engine.lock"))?;
    // SAFETY: the descriptor remains open for the returned guard's lifetime.
    if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
        let error = io::Error::last_os_error();
        return Err(if error.kind() == io::ErrorKind::WouldBlock {
            io::Error::new(io::ErrorKind::WouldBlock, "another Skiff engine is still running or shutting down; try again when it has stopped")
        } else { error });
    }
    Ok(lock)
}

pub fn start_engine() -> io::Result<()> {
    let mut engine = ENGINE.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(child) = engine.as_mut() {
        if child.try_wait()?.is_none() {
            if child.stdin.is_none() { return Err(io::Error::new(io::ErrorKind::WouldBlock, "the engine is still shutting down; try again when it has stopped")); }
            return Ok(());
        }
    }
    // Fail promptly while a previous app's helper still owns the data. The
    // helper acquires and retains its own lock before touching the socket.
    drop(lock_engine()?);
    let log = std::fs::OpenOptions::new().create(true).append(true).open(crate::daemon::state_dir()?.join("engine.log"))?;
    *engine = Some(Command::new(helper()?).arg("daemon").arg("--app-owned").stdin(Stdio::piped()).stdout(log.try_clone()?).stderr(log).spawn()?);
    Ok(())
}
pub fn stop_engine() -> io::Result<()> {
    let mut engine = ENGINE.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(child) = engine.as_mut() {
        wait_for_shutdown(child, std::time::Duration::from_secs(30))?;
        *engine = None;
    }
    Ok(())
}

fn wait_for_shutdown(child: &mut Child, limit: std::time::Duration) -> io::Result<()> {
    // EOF requests graceful VM shutdown. Container stop timeouts are user
    // controlled, so a deadline may stop waiting but must never kill the VM.
    // Keep the child tracked on timeout: restart must not launch a second one.
    drop(child.stdin.take());
    let deadline = std::time::Instant::now() + limit;
    while child.try_wait()?.is_none() {
        if std::time::Instant::now() >= deadline {
            return Err(io::Error::new(io::ErrorKind::TimedOut, "the engine is still shutting down; it will finish in the background"));
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    Ok(())
}
pub fn watch_parent() -> io::Result<()> {
    std::thread::Builder::new().name("skiff-app-owner".into()).spawn(|| {
        let _ = std::io::stdin().read(&mut [0]);
        // SAFETY: signal our own process, whose daemon signal thread performs cleanup.
        unsafe { libc::kill(libc::getpid(), libc::SIGTERM); }
    })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_link_left_at_the_etc_share_is_replaced_by_a_folder() {
        let base = std::env::temp_dir().join(format!("skiff-etc-share-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let data = base.join("data");
        std::fs::create_dir_all(data.join("vm")).unwrap();
        std::os::unix::fs::symlink(&data, data.join("etc")).unwrap();
        let etc = super::etc_share(&data).unwrap();
        let kind = std::fs::symlink_metadata(&etc).unwrap().file_type();
        assert!(kind.is_dir() && !kind.is_symlink(), "etc is a folder of its own");
        assert!(data.join("vm").is_dir(), "what the link led to is left alone");
        assert_eq!(super::etc_share(&data).unwrap(), etc, "and is kept as it is after");
        std::fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn only_skiffs_own_files_stay_in_the_etc_share() {
        let base = std::env::temp_dir().join(format!("skiff-etc-share-clean-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let (data, etc) = (base.join("data"), base.join("data/etc"));
        std::fs::create_dir_all(data.join("vm")).unwrap();
        std::fs::create_dir_all(etc.join("junk/sub")).unwrap();
        let disk = data.join("vm/data.img");
        std::fs::write(&disk, "the VM's disk").unwrap();
        std::fs::write(etc.join("passwd"), "root:x:0:0:root:/root:/bin/sh\n").unwrap();
        std::fs::write(etc.join("machine-id"), "0123456789abcdef0123456789abcdef\n").unwrap();
        std::fs::hard_link(&disk, etc.join("saved.img")).unwrap();
        std::fs::hard_link(&disk, etc.join("group")).unwrap();
        std::os::unix::fs::symlink(&disk, etc.join("hostname")).unwrap();
        std::fs::write(etc.join("passwd.skiff-tmp-0011223344556677"), "x").unwrap();
        // init only makes sure etc is a folder; the engine clears what is in it.
        super::etc_share(&data).unwrap();
        assert!(etc.join("saved.img").exists(), "init leaves what is in etc alone");
        super::clean_etc(&etc).unwrap();
        let mut left: Vec<String> = std::fs::read_dir(&etc).unwrap().map(|e| e.unwrap().file_name().into_string().unwrap()).collect();
        left.sort();
        assert_eq!(left, ["machine-id", "passwd"], "only kept files of their own stay");
        assert_eq!(std::fs::read_to_string(&disk).unwrap(), "the VM's disk", "the disk itself is left alone");
        std::fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn engine_data_lock_excludes_a_fresh_process_until_shutdown_finishes() {
        use super::*;
        let dir = std::env::temp_dir().join(format!("skiff-engine-lock-{}", std::process::id()));
        let lock = lock_engine_at(&dir).unwrap();
        // Shutdown has already removed the socket; ownership must still hold.
        assert!(!dir.join("docker.sock").exists());
        let probe = |expected| {
            let out = Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "app_store::tests::engine_lock_probe", "--nocapture"])
                .env("SKIFF_TEST_ENGINE_LOCK_DIR", &dir).env("SKIFF_TEST_ENGINE_LOCK_EXPECT", expected)
                .output().unwrap();
            assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
            assert!(String::from_utf8_lossy(&out.stdout).contains("1 passed"));
        };
        probe("blocked");
        drop(lock);
        probe("free");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn engine_lock_probe() {
        let Some(dir) = std::env::var_os("SKIFF_TEST_ENGINE_LOCK_DIR") else { return };
        let result = super::lock_engine_at(std::path::Path::new(&dir));
        if std::env::var("SKIFF_TEST_ENGINE_LOCK_EXPECT").unwrap() == "blocked" {
            assert_eq!(result.unwrap_err().kind(), std::io::ErrorKind::WouldBlock);
        } else { assert!(result.is_ok()); }
    }

    #[test]
    fn nested_folder_events_follow_each_binds_original_grant() {
        use super::*;
        let base = std::env::temp_dir().join(format!("skiff-nested-watch-{}", std::process::id()));
        std::fs::create_dir_all(base.join("app")).unwrap();
        let parent = base.canonicalize().unwrap();
        let child = parent.join("app");
        let file = child.join("x.js");
        let roots = vec![("a".into(), parent.clone()), ("b".into(), child.clone())];
        std::fs::write(&file, "changed").unwrap();
        for deleted in [false, true] {
            if deleted { std::fs::remove_file(&file).unwrap(); }
            assert_eq!(map_event(&roots, &file, "/mnt/mac/a"), Some("/mnt/mac/a/app/x.js".into()));
            assert_eq!(map_event(&roots, &file, "/mnt/mac/b"), Some("/mnt/mac/b/x.js".into()));
            // A persisted child bind may still use A after B was selected.
            assert_eq!(map_event(&roots, &file, "/mnt/mac/a/app"), Some("/mnt/mac/a/app/x.js".into()));
            assert_eq!(map_event(&roots, &parent.join("outside.js"), "/mnt/mac/b"), None);
        }
        std::fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn a_slow_engine_finishes_gracefully_after_the_wait_deadline() {
        use super::*;
        let mut child = Command::new("/bin/sh").args(["-c", "cat >/dev/null; sleep 0.2; printf finished"])
            .stdin(Stdio::piped()).stdout(Stdio::piped()).spawn().unwrap();
        let error = wait_for_shutdown(&mut child, std::time::Duration::ZERO).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
        assert!(child.stdin.is_none());
        assert!(child.try_wait().unwrap().is_none());
        let output = child.wait_with_output().unwrap();
        assert!(output.status.success());
        assert_eq!(output.stdout, b"finished");
    }

    #[test]
    fn moved_folder_ids_cannot_collide_with_new_grants() {
        use super::*;
        let mut saved = vec![
            Grant { id: "0123456789abcdef".into(), path: "/project-moved".into(), bookmark: vec![] },
            Grant { id: "0123456789abcdef".into(), path: "/project".into(), bookmark: vec![] },
        ];
        assert!(unique_grants(&mut saved));
        assert_eq!(saved.len(), 1);
        assert_eq!(saved[0].path, Path::new("/project-moved"));
        let mut candidates = ["0123456789abcdef", "fedcba9876543210"].into_iter();
        assert_eq!(new_grant_id(&saved, || Ok(candidates.next().unwrap().into())).unwrap(), "fedcba9876543210");
        assert!(!unique_grants(&mut saved));
    }

    #[test]
    fn whole_folder_watch_roots_round_trip_without_a_trailing_slash() {
        let roots = vec![("a".into(), "/Users/me/project".into())];
        assert_eq!(super::map_guest(&roots, "/mnt/mac/a"), Some("/Users/me/project".into()));
        assert_eq!(super::map_guest(&roots, "/mnt/mac/a/src/file"), Some("/Users/me/project/src/file".into()));
        assert_eq!(super::map_guest(&roots, "/mnt/mac/a/../secret"), None);
    }

    #[test]
    fn missing_bind_directories_must_resolve_inside_the_grant() {
        let base = std::env::temp_dir().join(format!("skiff-missing-grant-{}", std::process::id()));
        let selected = base.join("selected");
        let outside = base.join("outside");
        std::fs::create_dir_all(&selected).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        let roots = vec![("a".into(), selected.canonicalize().unwrap())];
        let missing = selected.join("data/nested");
        assert_eq!(super::map_missing(&roots, &missing), Some("/mnt/mac/a/data/nested".into()));
        std::fs::create_dir_all(&missing).unwrap();
        assert_eq!(super::map_existing(&roots, &missing), Some("/mnt/mac/a/data/nested".into()));
        std::os::unix::fs::symlink(&outside, selected.join("escape")).unwrap();
        std::os::unix::fs::symlink(outside.join("absent"), selected.join("dangling")).unwrap();
        for p in [selected.join("escape/new"), selected.join("dangling/new"), selected.join("../outside/new")] {
            assert_eq!(super::map_missing(&roots, &p), None, "{}", p.display());
        }
        std::fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn grants_are_component_bounded_and_use_the_most_specific_root() {
        let roots = vec![("a".into(), "/Users/me/project".into()), ("b".into(), "/Users/me/project/nested".into())];
        assert_eq!(super::map_host(&roots, std::path::Path::new("/Users/me/project/file")), Some("/mnt/mac/a/file".into()));
        assert_eq!(super::map_host(&roots, std::path::Path::new("/Users/me/project/nested")), Some("/mnt/mac/b".into()));
        assert_eq!(super::map_host(&roots, std::path::Path::new("/Users/me/project-secret")), None);
        assert_eq!(super::map_host(&roots, std::path::Path::new("/etc/passwd")), None);
    }
    #[test]
    fn symlink_targets_must_stay_in_the_granted_folder() {
        let base = std::env::temp_dir().join(format!("skiff-grants-{}", std::process::id()));
        let selected = base.join("selected");
        let outside = base.join("outside");
        std::fs::create_dir_all(&selected).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(selected.join("ok"), "test").unwrap();
        let roots = vec![("a".into(), selected.canonicalize().unwrap())];
        let link = selected.join("link");
        std::os::unix::fs::symlink("ok", &link).unwrap();
        assert_eq!(super::map_existing(&roots, &link), Some("/mnt/mac/a/ok".into()));
        std::fs::remove_file(&link).unwrap();
        std::os::unix::fs::symlink(&outside, &link).unwrap();
        assert_eq!(super::map_existing(&roots, &link), None);
        assert_eq!(super::map_existing(&roots, &selected.join("../outside")), None);
        std::fs::remove_dir_all(base).unwrap();
    }

}
