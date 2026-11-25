//! Linux's files in /etc, for bind mounts on a Mac.
//!
//! Compose files and tools written for a Linux host mount /etc/passwd,
//! /etc/group, /etc/resolv.conf, /etc/os-release, /etc/machine-id and
//! /etc/timezone, and expect Linux's. The VM keeps next to nothing in /etc,
//! and the Mac's own are macOS's: its passwd has neither the person's
//! account (those are in Directory Services) nor Linux's `nobody`. So Skiff
//! keeps plain Linux ones in its data folder, written again whenever one is
//! mounted, and a mount of those paths gets them. /etc/hosts and
//! /etc/localtime are the Mac's own (see `api::mac_source`).

use std::io;
use std::path::{Path, PathBuf};

/// The files kept, by their name in /etc.
pub const FILES: &[&str] = &["passwd", "group", "resolv.conf", "os-release", "machine-id", "timezone"];

/// Whether `name` is one of the files Skiff keeps in `dir()`.
pub fn kept(name: &str) -> bool {
    name == "hostname" || FILES.contains(&name)
}

/// The person running Skiff, who gets a line in passwd and group: what
/// `--user $(id -u):$(id -g)` with a mounted /etc/passwd is for.
pub struct Who {
    pub user: String,
    pub uid: u32,
    pub gid: u32,
    pub home: String,
}

impl Who {
    fn current() -> Who {
        // SAFETY: getuid and getgid take no arguments and cannot fail.
        let (uid, gid) = unsafe { (libc::getuid(), libc::getgid()) };
        let var = |k: &str| std::env::var(k).ok().filter(|v| !v.is_empty() && !v.contains([':', '\n']));
        Who { user: var("USER").unwrap_or_else(|| "skiff".into()), uid, gid, home: var("HOME").unwrap_or_else(|| "/".into()) }
    }
}

/// Where the files are kept: `etc` in Skiff's data folder. The VM sees it
/// with the rest of the Mac, or in the TestFlight edition as the one part of
/// the data folder it shares (`app_store::init`).
pub fn dir() -> io::Result<PathBuf> {
    Ok(skiff_image::Store::default_root()?.join("etc"))
}

/// One of `FILES`, written as it should read now; its path.
pub fn file(name: &str) -> io::Result<PathBuf> {
    file_in(&dir()?, name, &Who::current())
}

/// All of them, for a mount of /etc itself; the folder.
pub fn all() -> io::Result<PathBuf> {
    let (dir, who) = (dir()?, Who::current());
    for name in FILES {
        file_in(&dir, name, &who)?;
    }
    // The VM's own name, as its /etc/hostname has it.
    {
        let _one = writing();
        put(&dir.join("hostname"), "skiff\n")?;
    }
    Ok(dir)
}

fn file_in(dir: &Path, name: &str, who: &Who) -> io::Result<PathBuf> {
    let path = dir.join(name);
    // One request at a time decides and writes: each API request runs on
    // its own thread, and containers created together may mount the same
    // file (or the first two would each make a machine-id).
    let _one = writing();
    sweep_temps(dir);
    let text = match name {
        // Made once and kept: a machine-id that changed would not be one.
        // Read one byte past what a machine-id may be, so a valid start
        // with more after it is not taken for one.
        "machine-id" => match read_own(&path, 65) {
            Some(id) if id.len() <= 64 && is_machine_id(&id) => return Ok(path),
            _ => format!("{}\n", super::engine::random_hex(16)),
        },
        "timezone" => format!("{}\n", zone_of(std::fs::read_link("/etc/localtime").ok().as_deref())),
        _ => contents(name, who).ok_or_else(|| io::Error::other(format!("no /etc/{name} is kept")))?,
    };
    put(&path, &text)?;
    Ok(path)
}

static WRITING: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Held while a kept file is read and written.
fn writing() -> std::sync::MutexGuard<'static, ()> {
    WRITING.lock().unwrap_or_else(|e| e.into_inner())
}

/// Removes the temporary files `put` writes beside a kept file, left by a
/// daemon that stopped between writing one and renaming it (and the fixed
/// `<name>.skiff-tmp` older builds used). Called with `writing()` held, so
/// no `put` of this daemon is midway. A folder at such a name is not ours
/// and is left.
fn sweep_temps(dir: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let ours = name.to_str().and_then(|n| n.split_once(".skiff-tmp")).is_some_and(|(kept_name, rest)| kept(kept_name) && (rest.is_empty() || rest.starts_with('-')));
        if ours && entry.file_type().is_ok_and(|t| !t.is_dir()) {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

/// Opens `path` only if it is a file of its own. Containers can write this
/// folder through their mounts, so what is there may be a link (never
/// followed: O_NOFOLLOW), a FIFO or device (never waited on: O_NONBLOCK,
/// and never read), or a hard link to another file, such as the VM's disk
/// (never read or written through). None for those and for a missing file.
fn open_own(path: &Path, write: bool) -> io::Result<Option<std::fs::File>> {
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
    match std::fs::OpenOptions::new().read(true).write(write).custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK).open(path) {
        Ok(file) if file.metadata().is_ok_and(|m| m.is_file() && m.nlink() == 1) => Ok(Some(file)),
        Ok(_) => Ok(None),
        Err(e) if e.kind() == io::ErrorKind::NotFound || e.raw_os_error() == Some(libc::ELOOP) => Ok(None),
        Err(e) => Err(e),
    }
}

/// At most `limit` bytes of `path`, if it is a file of its own (`open_own`).
fn read_own(path: &Path, limit: u64) -> Option<String> {
    use std::io::Read;
    let mut text = String::new();
    open_own(path, false).ok()??.take(limit).read_to_string(&mut text).ok()?;
    Some(text)
}

/// Writes `text` unless the file says it already, so a mount does not
/// change a file other containers have open for nothing. Called with
/// `writing()` held.
///
/// A file is rewritten in place, so containers that already bind-mount it
/// keep the same file and see the change. Anything else there (`open_own`)
/// is replaced by a new file written beside it and renamed over it: a
/// read-only file, a folder or a socket too, which can't be opened for
/// writing. Left there, the bind would quietly be the VM's own file
/// instead.
fn put(path: &Path, text: &str) -> io::Result<()> {
    use std::io::{Seek, Write};
    // Read before asking to write: a file that says it already is left as
    // it is, even one a container made read-only.
    // Only as much as could match: a container may have left a huge file.
    if read_own(path, text.len() as u64 + 1).is_some_and(|old| old == text) {
        return Ok(());
    }
    match open_own(path, true) {
        Ok(Some(mut file)) => {
            file.set_len(0)?;
            file.seek(std::io::SeekFrom::Start(0))?;
            // On disk before it counts as written: a machine-id lost to a
            // power cut would come back as a new one.
            file.write_all(text.as_bytes())?;
            return file.sync_all();
        }
        Ok(None) => {}
        // A read-only file, or a folder or socket, which can't be opened
        // for writing: replaced below, as the folder can be written.
        Err(e) if e.kind() == io::ErrorKind::PermissionDenied => {}
        Err(_) if std::fs::symlink_metadata(path).is_ok_and(|m| !m.is_file()) => {}
        Err(e) => return Err(e),
    }
    // A rename replaces anything but a folder; remove_dir_all follows no link.
    if std::fs::symlink_metadata(path).is_ok_and(|m| m.is_dir()) {
        std::fs::remove_dir_all(path)?;
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    // Written beside it under a name no container can have taken first (at a
    // fixed one it could leave a folder, which nothing here removes), then
    // renamed over it. create_new never follows a link.
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(format!(".skiff-tmp-{}", super::engine::random_hex(8)));
    let tmp = PathBuf::from(tmp);
    let done = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&tmp)
        .and_then(|mut file| file.write_all(text.as_bytes()).and_then(|()| file.sync_all()))
        .and_then(|()| std::fs::rename(&tmp, path));
    if done.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    done
}

fn is_machine_id(id: &str) -> bool {
    let id = id.trim_end_matches('\n');
    id.len() == 32 && id.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// The zone's name from where /etc/localtime leads
/// (`/var/db/timezone/zoneinfo/Africa/Lagos`); UTC when it says nothing.
fn zone_of(link: Option<&Path>) -> String {
    link.and_then(Path::to_str).and_then(|l| l.split_once("zoneinfo/")).map(|(_, zone)| zone.to_owned()).filter(|z| !z.is_empty()).unwrap_or_else(|| "UTC".into())
}

/// What a file says, for those that depend on nothing on disk.
fn contents(name: &str, who: &Who) -> Option<String> {
    // The accounts every Linux has, with Debian's numbers, then the person.
    const USERS: &[(&str, u32, u32, &str)] =
        &[("root", 0, 0, "/root"), ("daemon", 1, 1, "/usr/sbin"), ("bin", 2, 2, "/bin"), ("sys", 3, 3, "/dev"), ("www-data", 33, 33, "/var/www"), ("nobody", 65534, 65534, "/nonexistent")];
    const GROUPS: &[(&str, u32)] = &[("root", 0), ("daemon", 1), ("bin", 2), ("sys", 3), ("tty", 5), ("www-data", 33), ("nogroup", 65534)];
    Some(match name {
        "passwd" => {
            let mut out = String::new();
            for (user, uid, gid, home) in USERS {
                let shell = if *uid == 0 { "/bin/sh" } else { "/usr/sbin/nologin" };
                out.push_str(&format!("{user}:x:{uid}:{gid}:{user}:{home}:{shell}\n"));
            }
            if !USERS.iter().any(|(u, id, ..)| *id == who.uid || *u == who.user) {
                out.push_str(&format!("{}:x:{}:{}:{}:{}:/bin/sh\n", who.user, who.uid, who.gid, who.user, who.home));
            }
            out
        }
        "group" => {
            let mut out = String::new();
            for (group, gid) in GROUPS {
                out.push_str(&format!("{group}:x:{gid}:\n"));
            }
            // The person's group, under the Mac's name for it (20 is staff).
            if !GROUPS.iter().any(|(_, id)| *id == who.gid) {
                let group = if who.gid == 20 { "staff" } else { who.user.as_str() };
                out.push_str(&format!("{group}:x:{}:{}\n", who.gid, who.user));
            }
            out
        }
        // The Mac, which answers DNS for every container network.
        "resolv.conf" => format!("nameserver {}\n", super::ports::HOST_ADDR),
        "os-release" => {
            let v = env!("CARGO_PKG_VERSION");
            format!("NAME=\"Skiff\"\nID=skiff\nID_LIKE=linux\nVERSION_ID=\"{v}\"\nPRETTY_NAME=\"Skiff {v}\"\n")
        }
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn me() -> Who {
        Who { user: "ada".into(), uid: 501, gid: 20, home: "/Users/ada".into() }
    }

    #[test]
    fn passwd_and_group_are_linuxs_with_the_person_added() {
        // Account lines only (name, ids, home, shell): no secrets are in
        // these files. The messages still leave their text out.
        let accounts = contents("passwd", &me()).unwrap();
        assert!(accounts.starts_with("root:x:0:0:root:/root:/bin/sh\n"), "root comes first");
        assert!(accounts.contains("\nnobody:x:65534:65534:"), "Linux's nobody is listed");
        assert!(accounts.ends_with("ada:x:501:20:ada:/Users/ada:/bin/sh\n"), "the person comes last");
        assert!(accounts.lines().all(|l| l.split(':').count() == 7), "every line has seven fields");
        let groups = contents("group", &me()).unwrap();
        assert!(groups.contains("root:x:0:\n") && groups.contains("nogroup:x:65534:\n"), "Linux's groups are listed");
        assert!(groups.ends_with("staff:x:20:ada\n"), "the person's group comes last");
        // root running Skiff (a Linux host's layout, or sudo) is not listed twice.
        let root = Who { user: "root".into(), uid: 0, gid: 0, home: "/root".into() };
        assert_eq!(contents("passwd", &root).unwrap().matches("\nroot:").count() + 1, 1);
        assert_eq!(contents("group", &root).unwrap().lines().filter(|l| l.split(':').nth(2) == Some("0")).count(), 1);
    }

    #[test]
    fn the_rest_say_what_a_linux_host_would() {
        assert_eq!(contents("resolv.conf", &me()).unwrap(), format!("nameserver {}\n", super::super::ports::HOST_ADDR));
        let os = contents("os-release", &me()).unwrap();
        assert!(os.contains("ID=skiff\n") && os.contains("PRETTY_NAME=\"Skiff "), "{os}");
        assert_eq!(contents("hosts", &me()), None);
        assert_eq!(zone_of(Some(Path::new("/var/db/timezone/zoneinfo/Africa/Lagos"))), "Africa/Lagos");
        assert_eq!(zone_of(Some(Path::new("/usr/share/zoneinfo/UTC"))), "UTC");
        assert_eq!(zone_of(Some(Path::new("/somewhere/else"))), "UTC");
        assert_eq!(zone_of(None), "UTC");
    }

    #[test]
    fn files_are_written_once_and_the_machine_id_is_kept() {
        let dir = std::env::temp_dir().join(format!("skiff-hostetc-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        for name in FILES {
            let path = file_in(&dir, name, &me()).unwrap();
            assert!(std::fs::metadata(&path).unwrap().len() > 0, "{name}");
        }
        let id = std::fs::read_to_string(dir.join("machine-id")).unwrap();
        assert!(is_machine_id(&id), "{id}");
        file_in(&dir, "machine-id", &me()).unwrap();
        assert_eq!(std::fs::read_to_string(dir.join("machine-id")).unwrap(), id);
        // A container that changed a mounted file does not change the next one's.
        std::fs::write(dir.join("passwd"), "gone\n").unwrap();
        file_in(&dir, "passwd", &me()).unwrap();
        assert!(std::fs::read_to_string(dir.join("passwd")).unwrap().starts_with("root:x:0:0:"));
        assert!(file_in(&dir, "shadow", &me()).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_link_a_container_put_in_place_is_replaced_not_followed() {
        let base = std::env::temp_dir().join(format!("skiff-hostetc-link-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let (dir, outside) = (base.join("etc"), base.join("outside"));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(&outside, "the Mac's file\n").unwrap();
        let machine_id = "0123456789abcdef0123456789abcdef\n";
        std::fs::write(base.join("id"), machine_id).unwrap();
        for (name, target) in [("passwd", &outside), ("machine-id", &base.join("id"))] {
            std::os::unix::fs::symlink(target, dir.join(name)).unwrap();
            // And a link at the name it was once written under first.
            std::os::unix::fs::symlink(&outside, dir.join(format!("{name}.skiff-tmp"))).unwrap();
            let path = file_in(&dir, name, &me()).unwrap();
            assert!(std::fs::symlink_metadata(&path).unwrap().is_file(), "{name} is a file of its own again");
        }
        assert_eq!(std::fs::read_to_string(&outside).unwrap(), "the Mac's file\n");
        assert_eq!(std::fs::read_to_string(base.join("id")).unwrap(), machine_id);
        assert_ne!(std::fs::read_to_string(dir.join("machine-id")).unwrap(), machine_id, "a linked machine-id is not taken as Skiff's");
        std::fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn a_folder_or_socket_a_container_put_in_place_is_replaced() {
        let base = std::env::temp_dir().join(format!("skiff-hostetc-kinds-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let (dir, outside) = (base.join("etc"), base.join("outside"));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(&outside, "the Mac's file\n").unwrap();
        // A folder with a file and a link out in it: the link is removed, not followed.
        std::fs::create_dir_all(dir.join("passwd/sub")).unwrap();
        std::fs::write(dir.join("passwd/sub/f"), "x").unwrap();
        std::os::unix::fs::symlink(&outside, dir.join("passwd/out")).unwrap();
        std::fs::create_dir(dir.join("machine-id")).unwrap();
        let _socket = std::os::unix::net::UnixListener::bind(dir.join("group")).unwrap();
        for name in ["passwd", "machine-id", "group"] {
            let path = file_in(&dir, name, &me()).unwrap();
            assert!(std::fs::symlink_metadata(&path).unwrap().is_file(), "{name} is a file of its own again");
        }
        assert!(std::fs::read_to_string(dir.join("passwd")).unwrap().starts_with("root:x:0:0:"));
        assert!(is_machine_id(&std::fs::read_to_string(dir.join("machine-id")).unwrap()));
        assert_eq!(std::fs::read_to_string(&outside).unwrap(), "the Mac's file\n");
        std::fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn a_read_only_file_is_kept_if_right_and_replaced_if_not() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("skiff-hostetc-ro-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let right = contents("passwd", &me()).unwrap();
        let ro = |p: &Path| std::fs::set_permissions(p, std::fs::Permissions::from_mode(0o444)).unwrap();
        std::fs::write(dir.join("passwd"), &right).unwrap();
        ro(&dir.join("passwd"));
        let path = file_in(&dir, "passwd", &me()).unwrap();
        assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o444, "a right file is left as it is");
        std::fs::write(dir.join("group"), "wrong\n").unwrap();
        ro(&dir.join("group"));
        let path = file_in(&dir, "group", &me()).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), contents("group", &me()).unwrap());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn what_a_container_left_beside_a_file_does_not_stop_its_repair() {
        let dir = std::env::temp_dir().join(format!("skiff-hostetc-beside-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        // No passwd, and a folder at the name it used to be written under first.
        std::fs::create_dir_all(dir.join("passwd.skiff-tmp/sub")).unwrap();
        let path = file_in(&dir, "passwd", &me()).unwrap();
        assert!(std::fs::read_to_string(&path).unwrap().starts_with("root:x:0:0:"));
        let names: Vec<String> = std::fs::read_dir(&dir).unwrap().map(|e| e.unwrap().file_name().into_string().unwrap()).collect();
        assert!(!names.iter().any(|n| n.starts_with("passwd.skiff-tmp-")), "nothing of Skiff's is left beside it: {names:?}");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_hard_link_a_container_put_in_place_is_replaced_not_written_through() {
        use std::os::unix::fs::MetadataExt;
        let base = std::env::temp_dir().join(format!("skiff-hostetc-hardlink-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let (dir, disk) = (base.join("etc"), base.join("data.img"));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(&disk, "the VM's disk").unwrap();
        for name in ["passwd", "machine-id"] {
            std::fs::hard_link(&disk, dir.join(name)).unwrap();
            let path = file_in(&dir, name, &me()).unwrap();
            assert_eq!(std::fs::metadata(&path).unwrap().nlink(), 1, "{name} is a file of its own again");
        }
        assert_eq!(std::fs::read_to_string(&disk).unwrap(), "the VM's disk");
        assert!(is_machine_id(&std::fs::read_to_string(dir.join("machine-id")).unwrap()));
        std::fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn temp_files_a_stopped_daemon_left_are_swept() {
        let dir = std::env::temp_dir().join(format!("skiff-hostetc-sweep-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("machine-id.skiff-tmp")).unwrap();
        for left in ["passwd.skiff-tmp-0011223344556677", "group.skiff-tmp", "notes.skiff-tmp-1", "passwd.skiff-tmpx"] {
            std::fs::write(dir.join(left), "x").unwrap();
        }
        file_in(&dir, "passwd", &me()).unwrap();
        let mut names: Vec<String> = std::fs::read_dir(&dir).unwrap().map(|e| e.unwrap().file_name().into_string().unwrap()).collect();
        names.sort();
        // Ours are gone; a folder, another name's and a near miss stay.
        assert_eq!(names, ["machine-id.skiff-tmp", "notes.skiff-tmp-1", "passwd", "passwd.skiff-tmpx"]);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_machine_id_with_more_after_it_is_made_again() {
        let dir = std::env::temp_dir().join(format!("skiff-hostetc-long-id-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        // A valid id and newlines fill the first 64 bytes; more follows.
        let long = format!("0123456789abcdef0123456789abcdef{}junk", "\n".repeat(32));
        std::fs::write(dir.join("machine-id"), &long).unwrap();
        let id = std::fs::read_to_string(file_in(&dir, "machine-id", &me()).unwrap()).unwrap();
        assert!(is_machine_id(&id) && id.len() == 33, "made again: {id:?}");
        // One that is exactly an id, with its newline, is kept.
        let kept = "fedcba9876543210fedcba9876543210\n";
        std::fs::write(dir.join("machine-id"), kept).unwrap();
        assert_eq!(std::fs::read_to_string(file_in(&dir, "machine-id", &me()).unwrap()).unwrap(), kept);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_changed_file_stays_the_one_running_containers_mount() {
        use std::os::unix::fs::MetadataExt;
        let dir = std::env::temp_dir().join(format!("skiff-hostetc-inode-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("timezone");
        std::fs::write(&path, "Europe/Paris\n").unwrap();
        let before = std::fs::metadata(&path).unwrap().ino();
        put(&path, "Africa/Lagos\n").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "Africa/Lagos\n");
        assert_eq!(std::fs::metadata(&path).unwrap().ino(), before, "a bind mount keeps the file it was given");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn containers_created_together_all_get_the_file() {
        let dir = std::env::temp_dir().join(format!("skiff-hostetc-race-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let passwd = contents("passwd", &me()).unwrap();
        for round in 0..50 {
            for name in ["passwd", "machine-id"] {
                let _ = std::fs::remove_file(dir.join(name));
            }
            let start = std::sync::Arc::new(std::sync::Barrier::new(8));
            let requests: Vec<_> = (0..8)
                .map(|i| {
                    let (dir, start) = (dir.clone(), start.clone());
                    std::thread::spawn(move || {
                        start.wait();
                        let name = if i % 2 == 0 { "passwd" } else { "machine-id" };
                        let path = file_in(&dir, name, &me())?;
                        std::fs::read_to_string(path).map(|text| (name, text))
                    })
                })
                .collect();
            let mut ids = std::collections::HashSet::new();
            for request in requests {
                match request.join().unwrap() {
                    Ok(("passwd", text)) => assert_eq!(text, passwd, "round {round}"),
                    Ok((_, id)) => drop(ids.insert(id)),
                    Err(e) => panic!("round {round}: {e}"),
                }
            }
            assert_eq!(ids.len(), 1, "round {round}: one machine-id, not {ids:?}");
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn what_a_container_left_at_machine_id_never_hangs_skiff() {
        let dir = std::env::temp_dir().join(format!("skiff-hostetc-fifo-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let fifo = std::ffi::CString::new(dir.join("fifo").into_os_string().into_encoded_bytes()).unwrap();
        // SAFETY: a valid NUL-terminated path.
        assert_eq!(unsafe { libc::mkfifo(fifo.as_ptr(), 0o600) }, 0);
        for (what, make) in [
            ("a link to /dev/zero", Box::new(|p: &Path| std::os::unix::fs::symlink("/dev/zero", p)) as Box<dyn Fn(&Path) -> io::Result<()>>),
            ("a FIFO", Box::new(|p: &Path| std::fs::rename(Path::new(fifo.to_str().unwrap()), p))),
        ] {
            let path = dir.join("machine-id");
            let _ = std::fs::remove_file(&path);
            make(&path).unwrap();
            let (done, wait) = std::sync::mpsc::channel();
            let worker = dir.clone();
            std::thread::spawn(move || drop(done.send(file_in(&worker, "machine-id", &me()).map(|_| ()))));
            match wait.recv_timeout(std::time::Duration::from_secs(5)) {
                Ok(result) => result.unwrap_or_else(|e| panic!("{what}: {e}")),
                Err(_) => panic!("{what} at machine-id hung the request"),
            }
            assert!(std::fs::symlink_metadata(&path).unwrap().is_file(), "{what} is replaced by a file");
            assert!(is_machine_id(&std::fs::read_to_string(&path).unwrap()), "{what}");
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
