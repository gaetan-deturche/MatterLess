//! Looking for a newer build, and installing the one the reader accepts.
//!
//! Ported from `tauri-plugin-updater`, which the app reached through a builder
//! and a config block. The pieces it did for free are all here: the manifest's
//! shape, the platform key it is looked up by, the minisign signature the
//! download is checked against, and the arguments the installer is handed.
//!
//! Two rules carried over, both of them about who decides:
//!
//! * **Checked once, at start-up, in the background.** An update is not urgent,
//!   and asking mid-session interrupts reading to talk about the client rather
//!   than about anything the reader came for.
//! * **Offered, never taken.** Replacing the program somebody is running -- and
//!   restarting it under them -- is theirs to agree to. Nothing is downloaded
//!   until they do.
//!
//! And a dev build never asks. An unsigned binary whose version is whatever the
//! workspace says would either fail the check or, worse, replace the build
//! under development with a release one.

use serde::Deserialize;

/// Where the manifest lives. The same release the app updates from, so one
/// pipeline serves both.
pub const ENDPOINT: &str =
    "https://github.com/gaetan-deturche/MatterLess/releases/latest/download/latest.json";

/// The key the manifest's signatures are checked against, from the app's own
/// config.
///
/// A public key, and it is meant to be public: it proves a download came from
/// whoever holds the private half, which is not in this repository and should
/// never be.
pub const PUBKEY: &str = "dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXk6IENCQkVDNjkwMkI2RkNBNUYKUldSZnltOHJrTWEreTRqeE9ueS90d0hxcFFQWVZycW4xR21sZ3NvQ053SXBQVzcvRHBFcjlsZzMK";

/// What the manifest offers for one platform.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct Platform {
    pub url: String,
    /// The `.sig` file's whole contents, base64 again on top of minisign's own.
    pub signature: String,
}

/// The manifest itself.
#[derive(Debug, Clone, Deserialize)]
pub struct Manifest {
    pub version: String,
    #[serde(default)]
    pub notes: Option<String>,
    #[serde(default)]
    pub platforms: std::collections::HashMap<String, Platform>,
}

/// A newer build, and where to get it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Offer {
    pub version: String,
    pub url: String,
    pub signature: String,
}

/// Which entry of the manifest belongs to this build.
///
/// `windows-x86_64` and friends: the operating system and the architecture,
/// joined by a dash, exactly as the plugin spells them -- the manifest is
/// written by the same release that the app reads.
pub fn target() -> Option<String> {
    let os = if cfg!(target_os = "windows") {
        "windows"
    } else if cfg!(target_os = "macos") {
        // The plugin's own spelling, which is what the manifest is keyed by.
        "darwin"
    } else if cfg!(target_os = "linux") {
        "linux"
    } else {
        return None;
    };
    let arch = if cfg!(target_arch = "x86_64") {
        "x86_64"
    } else if cfg!(target_arch = "aarch64") {
        "aarch64"
    } else if cfg!(target_arch = "x86") {
        "i686"
    } else if cfg!(target_arch = "arm") {
        "armv7"
    } else {
        return None;
    };
    Some(format!("{os}-{arch}"))
}

/// This build's version, as the release numbers it.
pub fn running() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// Whether `theirs` is a build worth offering over `ours`.
///
/// By version rather than by "is it different": a manifest that has gone
/// backwards -- a release pulled, an endpoint serving an older channel --
/// must not walk somebody's install backwards with it.
pub fn newer(theirs: &str, ours: &str) -> bool {
    match (semver::Version::parse(theirs), semver::Version::parse(ours)) {
        (Ok(theirs), Ok(ours)) => theirs > ours,
        // An unparseable version is not an upgrade. Refusing is the safe way
        // to be wrong: the client keeps running the build it has.
        _ => false,
    }
}

/// What a manifest offers this build, if anything.
pub fn offered(manifest: &Manifest, target: &str, ours: &str) -> Option<Offer> {
    if !newer(&manifest.version, ours) {
        return None;
    }
    let platform = manifest.platforms.get(target)?;
    Some(Offer {
        version: manifest.version.clone(),
        url: platform.url.clone(),
        signature: platform.signature.clone(),
    })
}

/// Whether this build should be looking at all.
///
/// A debug build never does: it is unsigned, its version is whatever the
/// workspace says, and the one thing worse than a check that fails is a check
/// that succeeds and replaces the build being worked on.
pub fn asks() -> bool {
    !cfg!(debug_assertions)
}

/// Checks that `bytes` is what the key's holder signed.
///
/// Before anything is written or run, and the only thing standing between an
/// endpoint and running whatever it serves. Both the key and the signature
/// arrive base64-encoded on top of minisign's own encoding, which is the
/// plugin's format rather than minisign's.
pub fn verified(bytes: &[u8], signature: &str, pubkey: &str) -> Result<(), String> {
    use base64::Engine;
    let decode = |what: &str, value: &str| -> Result<String, String> {
        let raw = base64::engine::general_purpose::STANDARD
            .decode(value.trim())
            .map_err(|error| format!("the {what} is not base64: {error}"))?;
        String::from_utf8(raw).map_err(|error| format!("the {what} is not text: {error}"))
    };
    let key = minisign_verify::PublicKey::decode(&decode("public key", pubkey)?)
        .map_err(|error| format!("the public key will not load: {error}"))?;
    let signature = minisign_verify::Signature::decode(&decode("signature", signature)?)
        .map_err(|error| format!("the signature will not load: {error}"))?;
    key.verify(bytes, &signature, true)
        .map_err(|error| format!("the download is not what was signed: {error}"))
}

/// What the downloaded file is, by looking at it rather than at its name.
///
/// A URL can say anything; these are the first bytes of the actual file. An
/// installer this cannot identify is one it will not run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Installer {
    /// A Windows executable: the NSIS setup the release builds.
    Nsis,
    /// A Windows Installer package.
    Msi,
}

impl Installer {
    pub fn extension(self) -> &'static str {
        match self {
            Installer::Nsis => "exe",
            Installer::Msi => "msi",
        }
    }
}

/// Which of them `bytes` is, from its leading bytes.
pub fn kind_of(bytes: &[u8]) -> Option<Installer> {
    // `MZ`: a DOS header, which every Windows executable still begins with.
    if bytes.starts_with(b"MZ") {
        return Some(Installer::Nsis);
    }
    // A compound-file header, which is what an MSI is.
    if bytes.starts_with(&[0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1]) {
        return Some(Installer::Msi);
    }
    None
}

/// The arguments the NSIS installer is handed.
///
/// `/P` is the passive mode the app's config leaves as the default, `/UPDATE`
/// tells the installer it is replacing rather than arriving, and `/R` restarts
/// into the new build afterwards -- which is the half that makes this worth
/// offering rather than just downloading.
pub fn nsis_arguments() -> Vec<String> {
    vec![
        "/P".to_string(),
        "/UPDATE".to_string(),
        "/R".to_string(),
        "/ARGS".to_string(),
    ]
}

/// Writes the installer beside the reader's temporary files and runs it.
///
/// Never returns when it works: the installer replaces this executable, so the
/// process it is replacing has to be gone first. `/R` brings it back.
///
/// The bytes are verified before this is called, which is the ordering that
/// matters -- nothing unsigned is ever written to disk, let alone run.
#[cfg(windows)]
pub fn install(bytes: &[u8], version: &str) -> Result<std::convert::Infallible, String> {
    use std::os::windows::ffi::OsStrExt;
    use windows::Win32::UI::Shell::ShellExecuteW;
    use windows::Win32::UI::WindowsAndMessaging::SW_SHOW;
    use windows::core::PCWSTR;

    let kind = kind_of(bytes).ok_or_else(|| {
        "what was downloaded is not an installer this knows how to run".to_string()
    })?;
    // Named for the version so two of them cannot collide, and left behind:
    // this process is about to end, so there is nobody to clean it up.
    let path = std::env::temp_dir().join(format!(
        "MatterLess-{version}-update.{}",
        kind.extension()
    ));
    std::fs::write(&path, bytes).map_err(|error| format!("could not write the installer: {error}"))?;

    let arguments = match kind {
        Installer::Nsis => nsis_arguments().join(" "),
        // `msiexec` is what runs a package, and the package is its argument.
        Installer::Msi => format!("/i \"{}\" /passive /promptrestart", path.display()),
    };
    let runs = match kind {
        Installer::Nsis => path.clone(),
        Installer::Msi => std::path::PathBuf::from("msiexec.exe"),
    };
    let wide = |text: &std::ffi::OsStr| -> Vec<u16> {
        text.encode_wide().chain(std::iter::once(0)).collect()
    };
    let file = wide(runs.as_os_str());
    let parameters = wide(std::ffi::OsStr::new(&arguments));
    let opened: Vec<u16> = "open".encode_utf16().chain(std::iter::once(0)).collect();

    println!("running the installer for {version}");
    let outcome = unsafe {
        ShellExecuteW(
            None,
            PCWSTR(opened.as_ptr()),
            PCWSTR(file.as_ptr()),
            PCWSTR(parameters.as_ptr()),
            PCWSTR::null(),
            SW_SHOW,
        )
    };
    // `ShellExecuteW` answers with a number that is an error below 33, which is
    // the one Win32 call whose success is measured rather than flagged.
    if outcome.0 as isize <= 32 {
        return Err(format!(
            "the installer would not start: {}",
            std::io::Error::last_os_error()
        ));
    }
    // The installer is waiting for this process to let go of its own file.
    std::process::exit(0);
}

#[cfg(not(windows))]
pub fn install(_bytes: &[u8], _version: &str) -> Result<std::convert::Infallible, String> {
    Err("this platform has no installer to run".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest(version: &str) -> Manifest {
        let mut platforms = std::collections::HashMap::new();
        platforms.insert(
            "windows-x86_64".to_string(),
            Platform {
                url: "https://example.invalid/setup.exe".to_string(),
                signature: "c2ln".to_string(),
            },
        );
        Manifest {
            version: version.to_string(),
            notes: None,
            platforms,
        }
    }

    /// The manifest the release actually writes, parsed as it is written.
    #[test]
    fn the_release_manifest_parses() {
        let written = r#"{
            "version": "0.1.6",
            "notes": "what changed",
            "pub_date": "2026-09-11T10:00:00Z",
            "platforms": {
                "windows-x86_64": {
                    "signature": "dW50cnVzdGVk",
                    "url": "https://example.invalid/MatterLess_0.1.6_x64-setup.exe"
                }
            }
        }"#;
        let manifest: Manifest = serde_json::from_str(written).expect("the manifest parses");
        assert_eq!(manifest.version, "0.1.6");
        let offer = offered(&manifest, "windows-x86_64", "0.1.5").expect("an offer");
        assert_eq!(offer.version, "0.1.6");
        assert!(offer.url.ends_with("x64-setup.exe"));
        // A platform this build is not is not an offer for this build.
        assert_eq!(offered(&manifest, "darwin-aarch64", "0.1.5"), None);
    }

    /// Only forwards. A manifest that has gone backwards -- a release pulled,
    /// an endpoint serving an older channel -- must not walk an install back
    /// with it.
    #[test]
    fn only_a_newer_build_is_offered() {
        assert!(newer("0.1.6", "0.1.5"));
        assert!(newer("0.2.0", "0.1.9"));
        assert!(newer("1.0.0", "0.9.9"));
        assert!(!newer("0.1.5", "0.1.5"), "the same build is not an upgrade");
        assert!(!newer("0.1.4", "0.1.5"), "an older build is not an upgrade");
        // A pre-release is older than the release it leads to, which is
        // semver's rule and not string order.
        assert!(newer("0.1.6", "0.1.6-rc.1"));
        assert!(!newer("0.1.6-rc.1", "0.1.6"));
        assert_eq!(offered(&manifest("0.1.5"), "windows-x86_64", "0.1.5"), None);
        assert_eq!(offered(&manifest("0.1.0"), "windows-x86_64", "0.1.5"), None);
    }

    /// Nonsense is not an upgrade. Refusing is the safe way to be wrong: the
    /// client keeps running the build it has.
    #[test]
    fn an_unreadable_version_is_not_an_upgrade() {
        assert!(!newer("latest", "0.1.5"));
        assert!(!newer("", "0.1.5"));
        assert!(!newer("0.1.6", "not-a-version"));
    }

    /// The platform key is the one the manifest is written with.
    #[test]
    fn the_platform_key_is_the_one_the_release_writes() {
        let key = target().expect("this platform has a key");
        assert!(key.contains('-'), "{key}");
        #[cfg(all(windows, target_arch = "x86_64"))]
        assert_eq!(key, "windows-x86_64");
    }

    /// A download is checked before it is written, and rubbish fails.
    ///
    /// The whole point of the signature: an endpoint can serve anything, and
    /// this is what stands between it and running whatever it served.
    #[test]
    fn an_unsigned_download_is_refused() {
        let refused = verified(b"a file", "bm90LWEtc2lnbmF0dXJl", PUBKEY);
        assert!(refused.is_err(), "unsigned bytes were accepted");
        // And the real key loads, so the failure above is the signature's and
        // not the key's.
        assert!(
            refused.expect_err("refused").contains("signature"),
            "the key failed to load, so nothing was really tested"
        );
    }

    /// What the file is, from the file rather than from its name: a URL can
    /// say anything.
    #[test]
    fn an_installer_is_recognised_by_its_bytes() {
        assert_eq!(kind_of(b"MZ\x90\x00rest"), Some(Installer::Nsis));
        assert_eq!(
            kind_of(&[0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1, 0x00]),
            Some(Installer::Msi)
        );
        assert_eq!(kind_of(b"<html>not an installer"), None);
        assert_eq!(kind_of(b""), None);
    }

    /// This build is numbered by the release that would replace it.
    ///
    /// One pipeline serves both clients, so the version the updater compares
    /// against has to be the one the release writes into its manifest. Left to
    /// drift, this client would either never update or offer to update to the
    /// build it is already running.
    #[test]
    fn the_version_is_the_one_the_release_numbers() {
        const CONFIG: &str = include_str!("../../../app/src-tauri/tauri.conf.json");
        let config: serde_json::Value =
            serde_json::from_str(CONFIG).expect("the app's config parses");
        let released = config["version"].as_str().expect("a version");
        assert_eq!(
            running(),
            released,
            "this crate is numbered {} and the release is numbered {released}",
            running()
        );
    }

    /// A dev build never looks: unsigned, versioned by the workspace, and the
    /// one thing worse than a check that fails is one that succeeds and
    /// replaces the build being worked on.
    #[test]
    fn a_dev_build_does_not_ask() {
        assert_eq!(asks(), !cfg!(debug_assertions));
        #[cfg(debug_assertions)]
        assert!(!asks());
    }
}
