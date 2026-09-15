//! Signs an installer and writes the manifest the app updates from.
//!
//! Run by the release workflow, after the installer is built and before the
//! release is published:
//!
//! ```text
//! matterless-release --installer <path> --tag v0.1.6 --url <where it will be>
//!                    [--notes <text>] [--out latest.json]
//! ```
//!
//! The signing key arrives in the environment rather than on the command line,
//! because a command line is visible to anything on the machine and a CI log
//! is visible to anyone:
//!
//! * `MATTERLESS_SIGNING_KEY` -- the minisign secret key file's contents.
//! * `MATTERLESS_SIGNING_KEY_PASSWORD` -- what it was encrypted with.

fn main() {
    if let Err(why) = run() {
        eprintln!("{why}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let asked = Asked::from(std::env::args().skip(1))?;
    let key = env("MATTERLESS_SIGNING_KEY")?;
    let password = env("MATTERLESS_SIGNING_KEY_PASSWORD")?;

    let installer = std::fs::read(&asked.installer)
        .map_err(|error| format!("could not read {}: {error}", asked.installer))?;
    // The app decides what it downloaded by looking at its first bytes and
    // refuses to run anything it cannot identify. Better to fail here, where a
    // person is watching, than to publish a release nobody can install.
    if !installer.starts_with(b"MZ") {
        return Err(format!(
            "{} does not begin with MZ, so the app would refuse to run it",
            asked.installer
        ));
    }

    let version = matterless_release::version_of(&asked.tag);
    let signature = matterless_release::signature(&installer, &key, &password)?;
    let manifest = matterless_release::manifest(version, &asked.notes, &asked.url, signature);
    let written =
        serde_json::to_string_pretty(&manifest).map_err(|error| format!("{error}"))? + "\n";
    std::fs::write(&asked.out, &written)
        .map_err(|error| format!("could not write {}: {error}", asked.out))?;

    println!(
        "signed {} ({} bytes) as {version}, and wrote {}",
        asked.installer,
        installer.len(),
        asked.out
    );
    Ok(())
}

/// What was asked for on the command line.
#[derive(Debug)]
struct Asked {
    installer: String,
    tag: String,
    url: String,
    notes: String,
    out: String,
}

impl Asked {
    /// Read by hand rather than with a parser: five flags, on a tool that runs
    /// once per release.
    fn from(args: impl Iterator<Item = String>) -> Result<Self, String> {
        let mut installer = None;
        let mut tag = None;
        let mut url = None;
        let mut notes = String::new();
        let mut out = "latest.json".to_string();
        let mut args = args.peekable();
        while let Some(flag) = args.next() {
            let mut value = || {
                args.next()
                    .ok_or_else(|| format!("{flag} wants a value after it"))
            };
            match flag.as_str() {
                "--installer" => installer = Some(value()?),
                "--tag" => tag = Some(value()?),
                "--url" => url = Some(value()?),
                "--notes" => notes = value()?,
                "--out" => out = value()?,
                other => return Err(format!("{other} is not one of this tool's flags")),
            }
        }
        Ok(Self {
            installer: installer.ok_or("--installer is required")?,
            tag: tag.ok_or("--tag is required")?,
            url: url.ok_or("--url is required")?,
            notes,
            out,
        })
    }
}

/// One of the two secrets, or a sentence saying which one is missing.
fn env(name: &str) -> Result<String, String> {
    std::env::var(name).map_err(|_| format!("{name} is not set"))
}

#[cfg(test)]
mod tests {
    use super::Asked;

    fn asked(line: &[&str]) -> Result<Asked, String> {
        Asked::from(line.iter().map(|word| word.to_string()))
    }

    #[test]
    fn the_flags_are_read() {
        let asked = asked(&[
            "--installer",
            "setup.exe",
            "--tag",
            "v0.1.6",
            "--url",
            "https://example.invalid/setup.exe",
            "--notes",
            "what changed",
            "--out",
            "manifest.json",
        ])
        .expect("it parses");
        assert_eq!(asked.installer, "setup.exe");
        assert_eq!(asked.tag, "v0.1.6");
        assert_eq!(asked.notes, "what changed");
        assert_eq!(asked.out, "manifest.json");
    }

    /// A release with no manifest to publish is not a release, so a missing
    /// flag stops the build rather than defaulting to something.
    #[test]
    fn a_missing_flag_is_refused() {
        let why = asked(&["--installer", "setup.exe"]).expect_err("it refuses");
        assert!(why.contains("--tag"), "{why}");
    }

    #[test]
    fn a_flag_with_nothing_after_it_is_refused() {
        let why = asked(&["--installer"]).expect_err("it refuses");
        assert!(why.contains("wants a value"), "{why}");
    }

    /// A typo must not be read as a positional argument and ignored.
    #[test]
    fn an_unknown_flag_is_refused() {
        let why = asked(&["--instalr", "setup.exe"]).expect_err("it refuses");
        assert!(why.contains("not one of this tool's flags"), "{why}");
    }
}
