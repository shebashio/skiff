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
    let data = home.join("Library/Application Support/Skiff");
    std::fs::create_dir_all(&data)?;
    let mut roots = vec![("data".into(), data)];
    let helper = std::env::current_exe()?.file_name().is_some_and(|n| n == "skiff-helper");
    let mut saved = if helper { read_grants(session_file()?)? } else { grants()? };
    let mut session = Vec::new();
    let mut refreshed = false;
    for grant in &mut saved {
        if grant.id.len() != 16 || !grant.id.bytes().all(|b| b.is_ascii_hexdigit()) { continue; }
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
    let digest = skiff_image::store::sha256_digest(path.as_os_str().as_encoded_bytes());
    saved.push(Grant { id: digest.trim_start_matches("sha256:")[..16].into(), path, bookmark: bookmark(&url, true)? });
    save(&saved)
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
    map_host(roots, &path.canonicalize().ok()?)
}

/// Deleted files still need watch notifications. Resolve the nearest existing
/// parent, rejecting dangling links and parent traversals, before mapping it.
pub fn guest_event_path(path: &str) -> Option<String> {
    let path = Path::new(path);
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
    map_host(ROOTS.get()?, &canonical)
}

pub fn host_path(path: &str) -> Option<String> {
    let relative = Path::new(path).strip_prefix("/mnt/mac").ok()?;
    let mut parts = relative.components();
    let id = parts.next()?.as_os_str();
    let (_, root) = ROOTS.get()?.iter().find(|(key, _)| key.as_str() == id)?;
    let rest = parts.as_path();
    if rest.components().any(|p| !matches!(p, std::path::Component::Normal(_))) { return None; }
    Some(root.join(rest).to_string_lossy().into_owned())
}

pub fn helper() -> io::Result<PathBuf> {
    Ok(std::env::current_exe()?.parent().ok_or_else(|| io::Error::other("app bundle missing"))?.join("skiff-helper"))
}
pub fn start_engine() -> io::Result<()> {
    let mut engine = ENGINE.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(child) = engine.as_mut() {
        if child.try_wait()?.is_none() { return Ok(()); }
    }
    let log = std::fs::OpenOptions::new().create(true).append(true).open(crate::daemon::state_dir()?.join("engine.log"))?;
    *engine = Some(Command::new(helper()?).arg("daemon").arg("--app-owned").stdin(Stdio::piped()).stdout(log.try_clone()?).stderr(log).spawn()?);
    Ok(())
}
pub fn stop_engine() -> io::Result<()> {
    let mut engine = ENGINE.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(mut child) = engine.take() {
        // EOF requests graceful VM shutdown. It also fires if the UI crashes.
        drop(child.stdin.take());
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while child.try_wait()?.is_none() {
            if std::time::Instant::now() >= deadline { child.kill()?; break; }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        child.wait()?;
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
