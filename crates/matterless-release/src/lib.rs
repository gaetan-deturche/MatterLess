//! Numbering, signing and describing a release.
//!
//! The other half of `matterless-view::update`, which reads what this writes.
//! Neither depends on the other -- one runs on a build machine and one runs on
//! a reader's -- so what holds them together is that the shape is written down
//! twice and tested from both ends. Here, by signing and then verifying through
//! `minisign-verify`, which is the very crate the app checks a download with.
//!
//! **Two layers of base64, and that is not a mistake.** minisign's own `.sig`
//! file is already text, and the manifest carries that whole file base64'd
//! again. It is the format `tauri-plugin-updater` used, and the installs
//! already out there check against it -- a compatibility constraint, not a
//! choice.

use serde::Serialize;
use std::collections::BTreeMap;

/// What the manifest offers for one platform.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Platform {
    pub url: String,
    /// The whole `.sig` file, base64 again on top of minisign's own.
    pub signature: String,
}

/// The manifest itself, written as `latest.json`.
///
/// `BTreeMap` rather than `HashMap`: the app does not care what order the
/// platforms come in, but a release that writes its manifest differently every
/// time cannot be compared between builds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Manifest {
    pub version: String,
    pub notes: String,
    /// RFC 3339. The app ignores it; a person reading the release does not.
    pub pub_date: String,
    pub platforms: BTreeMap<String, Platform>,
}

/// How the app spells the platform it asks about.
///
/// Must match `matterless_view::update::target()` exactly, and is asserted at
/// both ends: a manifest keyed by a spelling the app never asks for offers it
/// nothing, silently, for as long as nobody tags a release and watches.
pub const WINDOWS: &str = "windows-x86_64";

/// Signs `bytes` with a minisign secret key.
///
/// `key` is the secret key *file's* contents -- the whole thing, comment line
/// included -- and `password` is what it was encrypted with, empty for a key
/// generated without one. Answers the signature in the form the manifest
/// carries it.
pub fn signature(bytes: &[u8], key: &str, password: &str) -> Result<String, String> {
    use base64::Engine;
    let secret = minisign::SecretKeyBox::from_string(&key_text(key))
        .map_err(|error| format!("the signing key will not parse: {error}"))?
        .into_secret_key(Some(password.to_string()))
        .map_err(|error| format!("the signing key will not open: {error}"))?;
    let signed = minisign::sign(None, &secret, bytes, None, None)
        .map_err(|error| format!("signing failed: {error}"))?;
    Ok(base64::engine::general_purpose::STANDARD.encode(signed.into_string()))
}

/// The secret key as minisign wants it: the file's own text.
///
/// Taken either way round, because there are two conventions for putting one
/// in an environment variable and picking the wrong one fails in CI, on a tag,
/// with a message about parsing. The retired shell's tooling passed the file's
/// contents; plenty of pipelines base64 the whole file first to keep it on one
/// line. A minisign key file always starts with its comment, so which one this
/// is can be read rather than configured.
fn key_text(key: &str) -> String {
    use base64::Engine;
    let key = key.trim();
    if key.starts_with("untrusted comment:") {
        return key.to_string();
    }
    base64::engine::general_purpose::STANDARD
        .decode(key)
        .ok()
        .and_then(|raw| String::from_utf8(raw).ok())
        .map(|text| text.trim().to_string())
        // Not base64 either: hand it on as it came, so the error names the
        // real problem rather than this guess at it.
        .unwrap_or_else(|| key.to_string())
}

/// The manifest for one Windows installer.
pub fn manifest(version: &str, notes: &str, url: &str, signature: String) -> Manifest {
    Manifest {
        version: version.to_string(),
        notes: notes.to_string(),
        pub_date: now(),
        platforms: BTreeMap::from([(
            WINDOWS.to_string(),
            Platform {
                url: url.to_string(),
                signature,
            },
        )]),
    }
}

/// The tag a release was cut from, as a version.
///
/// `v0.1.6` and `0.1.6` mean the same release. The manifest carries the bare
/// number, because that is what the app parses with `semver`.
pub fn version_of(tag: &str) -> &str {
    tag.strip_prefix('v').unwrap_or(tag)
}

/// Now, as RFC 3339, to the second.
///
/// Written out rather than pulled in: one field of one file, against a
/// dependency and its tree, on a tool whose whole job is three other things.
fn now() -> String {
    let seconds = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|since| since.as_secs())
        .unwrap_or(0);
    let (time, days) = (seconds % 86_400, seconds / 86_400);
    let (hour, minute, second) = (time / 3600, (time % 3600) / 60, time % 60);
    let (year, month, day) = civil(days as i64);
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z")
}

/// Days since the epoch as a calendar date, by Hinnant's `civil_from_days`.
fn civil(days: i64) -> (i64, u32, u32) {
    let shifted = days + 719_468;
    let era = shifted.div_euclid(146_097);
    let doe = shifted.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if month <= 2 { year + 1 } else { year }, month, day)
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine;

    /// A key made for the test, so nothing real is ever in the tree.
    fn keypair() -> (String, String, String) {
        guarded("a test password")
    }

    /// The same, with whatever passphrase is asked for -- including none.
    fn guarded(password: &str) -> (String, String, String) {
        let pair = minisign::KeyPair::generate_encrypted_keypair(Some(password.to_string()))
            .expect("a pair");
        (
            pair.sk.to_box(None).expect("a secret key box").to_string(),
            pair.pk.to_box().expect("a public key box").to_string(),
            password.to_string(),
        )
    }

    /// What the app does with the two fields it is handed, character for
    /// character: base64 off the outside, then minisign's own.
    fn accepts(public: &str, signed: &str, bytes: &[u8]) -> Result<(), String> {
        let decode = |value: &str| -> Result<String, String> {
            let raw = base64::engine::general_purpose::STANDARD
                .decode(value.trim())
                .map_err(|error| error.to_string())?;
            String::from_utf8(raw).map_err(|error| error.to_string())
        };
        let key = minisign_verify::PublicKey::decode(&decode(public)?)
            .map_err(|error| error.to_string())?;
        let signature = minisign_verify::Signature::decode(&decode(signed)?)
            .map_err(|error| error.to_string())?;
        key.verify(bytes, &signature, true)
            .map_err(|error| error.to_string())
    }

    /// The public key as the app carries it: base64 on top of minisign's own,
    /// which is what `update::PUBKEY` is.
    fn as_the_app_holds_it(public: &str) -> String {
        base64::engine::general_purpose::STANDARD.encode(public)
    }

    /// The property the whole crate exists for: what is signed here is what
    /// the app accepts there.
    #[test]
    fn what_is_signed_here_verifies_there() {
        let (secret, public, password) = keypair();
        let installer = b"MZ\x90\x00 a setup program, near enough";
        let signed = signature(installer, &secret, &password).expect("a signature");
        accepts(&as_the_app_holds_it(&public), &signed, installer).expect("it verifies");
    }

    /// And a download that is not what was signed is refused, which is the
    /// only reason any of this is here.
    #[test]
    fn a_tampered_download_is_refused() {
        let (secret, public, password) = keypair();
        let signed = signature(b"the real installer", &secret, &password).expect("a signature");
        assert!(accepts(&as_the_app_holds_it(&public), &signed, b"something else").is_err());
    }

    /// A signature is worth nothing against a key the app was not built with.
    #[test]
    fn another_key_is_refused() {
        let (secret, _, password) = keypair();
        let (_, stranger, _) = keypair();
        let installer = b"MZ the installer";
        let signed = signature(installer, &secret, &password).expect("a signature");
        assert!(accepts(&as_the_app_holds_it(&stranger), &signed, installer).is_err());
    }

    /// A key base64'd to keep it on one line signs the same as one pasted
    /// whole. Getting this wrong fails in CI, on a tag, with a message about
    /// parsing -- so it is read rather than configured.
    #[test]
    fn the_key_is_taken_either_way_round() {
        let (secret, public, password) = keypair();
        let folded = base64::engine::general_purpose::STANDARD.encode(&secret);
        assert_ne!(folded, secret);

        let installer = b"MZ the installer";
        let whole = signature(installer, &secret, &password).expect("a signature");
        let encoded = signature(installer, &folded, &password).expect("a signature");
        for signed in [whole, encoded] {
            accepts(&as_the_app_holds_it(&public), &signed, installer).expect("it verifies");
        }
    }

    /// Something that is neither is refused, and the sentence says it was the
    /// key -- not whatever the base64 guess happened to turn it into.
    #[test]
    fn a_key_that_is_neither_says_so() {
        let why = signature(b"anything", "not a key at all", "password").expect_err("it refuses");
        assert!(why.starts_with("the signing key"), "{why}");
    }

    /// A key generated with no passphrase is still an encrypted key file --
    /// scrypt over an empty password -- and signs exactly the same. This
    /// repository's key is one of those, so it is the path that actually runs.
    #[test]
    fn a_key_with_no_passphrase_signs() {
        let (secret, public, _) = guarded("");
        let installer = b"MZ the installer";
        let signed = signature(installer, &secret, "").expect("a signature");
        accepts(&as_the_app_holds_it(&public), &signed, installer).expect("it verifies");
    }

    /// The wrong password opens nothing, and says so rather than signing with
    /// something else.
    #[test]
    fn the_wrong_password_will_not_sign() {
        let (secret, _, _) = keypair();
        let why = signature(b"anything", &secret, "not the password").expect_err("it refuses");
        assert!(why.contains("will not open"), "{why}");
    }

    /// The shape the app deserialises: these field names are the contract.
    #[test]
    fn the_manifest_has_the_fields_the_app_reads() {
        let written = manifest(
            "0.1.6",
            "what changed",
            "https://example.invalid/setup.exe",
            "c2ln".into(),
        );
        let json: serde_json::Value =
            serde_json::from_str(&serde_json::to_string(&written).expect("it serialises"))
                .expect("it parses");
        assert_eq!(json["version"], "0.1.6");
        assert_eq!(json["notes"], "what changed");
        let platform = &json["platforms"][WINDOWS];
        assert_eq!(platform["url"], "https://example.invalid/setup.exe");
        assert_eq!(platform["signature"], "c2ln");
        assert!(
            json["pub_date"]
                .as_str()
                .is_some_and(|date| date.ends_with('Z')),
            "{json}"
        );
    }

    /// A tag is what CI has; a version is what the app compares.
    #[test]
    fn a_tag_is_read_as_a_version() {
        assert_eq!(version_of("v0.1.6"), "0.1.6");
        assert_eq!(version_of("0.1.6"), "0.1.6");
    }

    /// The date arithmetic, against dates known by other means.
    #[test]
    fn the_epoch_and_a_leap_day_land_where_they_should() {
        assert_eq!(civil(0), (1970, 1, 1));
        assert_eq!(civil(59), (1970, 3, 1));
        // A leap day: 2024-02-29 is 19782 days after the epoch.
        assert_eq!(civil(19_782), (2024, 2, 29));
        assert_eq!(civil(20_000), (2024, 10, 4));
    }
}
