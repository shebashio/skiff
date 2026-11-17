//! Local Developer ID selection and notarization preflight. Private keys stay
//! in Keychain; the environment contains only a certificate ID and profile name.
use std::io;
use std::path::Path;
use std::process::Command;

#[derive(Debug, PartialEq, Eq)]
struct Identity {
    hash: String,
    name: String,
}

fn identities(output: &str) -> Vec<Identity> {
    output
        .lines()
        .filter_map(|line| {
            let (prefix, rest) = line.split_once('"')?;
            let (name, _) = rest.split_once('"')?;
            let hash = prefix.split_whitespace().last()?;
            (hash.len() == 40 && hash.bytes().all(|b| b.is_ascii_hexdigit()) && name.starts_with("Developer ID Application:")).then(|| Identity { hash: hash.to_owned(), name: name.to_owned() })
        })
        .collect()
}

fn select(available: &[Identity], requested: Option<&str>, required: bool) -> io::Result<String> {
    let requested = requested.map(str::trim).filter(|s| !s.is_empty());
    if requested == Some("-") {
        return if required { Err(io::Error::other("distribution requires a Developer ID Application certificate; ad hoc signing is only for local builds")) } else { Ok("-".to_owned()) };
    }
    let matches: Vec<_> = available.iter().filter(|id| requested.is_none_or(|r| r.eq_ignore_ascii_case(&id.hash) || r == id.name)).collect();
    match matches.as_slice() {
        [id] => Ok(id.hash.clone()),
        [] if !required && requested.is_none() => Ok("-".to_owned()),
        [] => Err(io::Error::other("no matching valid Developer ID Application certificate with its private key in Keychain. In Xcode → Settings → Apple Accounts → your team → Manage Certificates, create or import Developer ID Application; Apple Development is not a distribution identity. See docs/macos-signing.md")),
        _ => Err(io::Error::other("multiple Developer ID Application certificates match; set SKIFF_SIGN_IDENTITY to the intended certificate's SHA-1 from `security find-identity -v -p codesigning`")),
    }
}

pub fn identity(required: bool) -> io::Result<String> {
    let requested = std::env::var("SKIFF_SIGN_IDENTITY").ok();
    if requested.as_deref().is_some_and(|id| id.trim() == "-") {
        return select(&[], requested.as_deref(), required);
    }
    let output = Command::new("security").args(["find-identity", "-v", "-p", "codesigning"]).output()?;
    if !output.status.success() {
        return Err(io::Error::other("could not read signing identities from Keychain; unlock the login keychain and retry"));
    }
    select(&identities(&String::from_utf8_lossy(&output.stdout)), requested.as_deref(), required)
}

pub fn profile() -> Option<String> {
    std::env::var("SKIFF_NOTARY_PROFILE").ok().map(|s| s.trim().to_owned()).filter(|s| !s.is_empty())
}

/// Check before compiling or changing an existing app's signature. The notary
/// history request validates the stored credentials without uploading software.
pub fn check(notarization: bool) -> io::Result<()> {
    let id = identity(true)?;
    println!("Developer ID Application: {id}");
    if notarization {
        let profile = profile().ok_or_else(|| io::Error::other("set SKIFF_NOTARY_PROFILE to a notarytool Keychain profile. Store credentials interactively with `xcrun notarytool store-credentials skiff-notary`; never put passwords or private keys in the repository. See docs/macos-signing.md"))?;
        super::run(Command::new("xcrun").args(["notarytool", "history", "--keychain-profile", &profile, "--output-format", "json"]).stdout(std::process::Stdio::null()))?;
        super::run(Command::new("xcrun").args(["--find", "stapler"]).stdout(std::process::Stdio::null()))?;
        println!("Notarization profile validated: {profile}");
    }
    Ok(())
}

/// `pack` may run on a different machine, without access to the signing key.
pub fn verify_distribution(app: &Path) -> io::Result<()> {
    super::run(Command::new("codesign").args(["--verify", "--deep", "--strict"]).arg(app))?;
    super::run(Command::new("xcrun").args(["stapler", "validate"]).arg(app))?;
    super::run(Command::new("spctl").args(["--assess", "--type", "execute", "--verbose=2"]).arg(app))
}

#[cfg(test)]
mod tests {
    use super::*;
    const HASH: &str = "0123456789ABCDEF0123456789ABCDEF01234567";
    fn cert() -> Identity {
        Identity { hash: HASH.to_owned(), name: "Developer ID Application: Example LLC (ABCDE12345)".to_owned() }
    }

    #[test]
    fn excludes_development_installer_and_configurator_certificates() {
        let output = format!("  1) {HASH} \"Apple Development: Example (ABCDE12345)\"\n  2) {HASH} \"Developer ID Installer: Example LLC (ABCDE12345)\"\n  3) {HASH} \"Apple Configurator: Example\"\n  4) {HASH} \"{}\"\n     4 valid identities found", cert().name);
        assert_eq!(identities(&output), vec![cert()]);
    }

    #[test]
    fn empty_keychain_is_only_allowed_for_local_builds() {
        assert_eq!(select(&[], None, false).unwrap(), "-");
        assert!(select(&[], None, true).is_err());
        assert!(select(&[], Some("-"), true).is_err());
        assert_eq!(select(&[], Some("-"), false).unwrap(), "-");
    }

    #[test]
    fn explicit_missing_certificate_never_falls_back() {
        assert!(select(&[cert()], Some("missing"), false).is_err());
        assert!(select(&[], Some(HASH), false).is_err());
    }

    #[test]
    fn selects_by_name_or_fingerprint_and_ignores_empty_override() {
        for requested in [None, Some("  "), Some(HASH), Some(cert().name.as_str()), Some(HASH.to_lowercase().as_str())] {
            assert_eq!(select(&[cert()], requested, true).unwrap(), HASH);
        }
    }

    #[test]
    fn ambiguous_names_require_a_fingerprint() {
        let mut other = cert();
        other.hash = "FEDCBA9876543210FEDCBA9876543210FEDCBA98".to_owned();
        let available = [cert(), other];
        assert!(select(&available, None, false).is_err());
        assert!(select(&available, Some(&cert().name), true).is_err());
        assert_eq!(select(&available, Some(HASH), true).unwrap(), HASH);
    }
}
