//! Distribution policy is compiled in; environment variables cannot turn it off.
pub const fn sandboxed() -> bool {
    cfg!(all(target_os = "macos", feature = "app-store"))
}

pub fn require_desktop(feature: &str) -> std::io::Result<()> {
    if sandboxed() {
        Err(std::io::Error::new(std::io::ErrorKind::Unsupported, format!("{feature} is unavailable in the TestFlight edition")))
    } else {
        Ok(())
    }
}
