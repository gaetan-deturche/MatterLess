//! Signs an installer and writes the manifest the app updates from.
//!
//! Run by the release workflow, after the installer is built and before the
//! release is published:
//!
//! ```text
//! matterless-release --installer <path> --tag v0.1.6 --url <where it will be>
//!                    [--notes <text> | --notes-file <path>] [--out latest.json]
//! ```
//!
//! A change list is what the app shows the reader before asking them to
//! restart, so it is usually several lines. `--notes-file` exists because
//! those lines came from `git log` and putting them through a shell is a way
//! to lose them: a quoting rule that differs between two shells is not
//! something a release should depend on.
//!
//! The signing key arrives in the environment rather than on the command line,
//! because a command line is visible to anything on the machine and a CI log
//! is visible to anyone:
//!
//! * `MATTERLESS_SIGNING_KEY` -- the minisign secret key file's contents.
//! * `MATTERLESS_SIGNING_KEY_PASSWORD` -- what it was encrypted with, if
//!   anything. Optional: a key generated with an empty passphrase is still an
//!   encrypted key file, and this repository's is one.

fn main() {
    if let Err(why) = run() {
        eprintln!("{why}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let asked = Asked::from(std::env::args().skip(1))?;
    let key = env("MATTERLESS_SIGNING_KEY")?;
    // Absent means empty, which is a real answer rather than a missing one: a
    // key can be generated with no passphrase, and this repository's was.
    // Required, it would also have made the pipeline depend on how an unset
    // secret reaches a step -- which is the kind of thing that is discovered
    // on a tag.
    let password = std::env::var("MATTERLESS_SIGNING_KEY_PASSWORD").unwrap_or_default();

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
        let mut notes_file = None;
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
                "--notes-file" => notes_file = Some(value()?),
                "--out" => out = value()?,
                other => return Err(format!("{other} is not one of this tool's flags")),
            }
        }
        if notes_file.is_some() && !notes.is_empty() {
            return Err("--notes and --notes-file both given: pick one".to_string());
        }
        if let Some(path) = notes_file {
            notes = std::fs::read_to_string(&path)
                .map_err(|error| format!("could not read {path}: {error}"))?
                .trim()
                .to_string();
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

    /// A change list comes from `git log` and is several lines. Reading it
    /// from a file rather than an argument keeps it out of a shell, where the
    /// quoting rules differ and the newlines are what gets lost.
    #[test]
    fn notes_can_come_from_a_file() {
        let path = std::env::temp_dir().join("matterless-release-notes-test.txt");
        std::fs::write(
            &path,
            "- fixed the thing
- fixed the other

",
        )
        .expect("written");
        let asked = asked(&[
            "--installer",
            "setup.exe",
            "--tag",
            "v0.1.6",
            "--url",
            "https://example.invalid/setup.exe",
            "--notes-file",
            path.to_str().expect("a path"),
        ])
        .expect("it parses");
        assert_eq!(
            asked.notes,
            "- fixed the thing
- fixed the other"
        );
        std::fs::remove_file(&path).ok();
    }

    /// Two sources for one field is a question about which one won, asked on
    /// a tag. Refused instead.
    #[test]
    fn notes_cannot_come_from_both() {
        let why = asked(&[
            "--installer",
            "setup.exe",
            "--tag",
            "v0.1.6",
            "--url",
            "https://example.invalid/setup.exe",
            "--notes",
            "one",
            "--notes-file",
            "other.txt",
        ])
        .expect_err("it refuses");
        assert!(why.contains("pick one"), "{why}");
    }

    /// A typo must not be read as a positional argument and ignored.
    #[test]
    fn an_unknown_flag_is_refused() {
        let why = asked(&["--instalr", "setup.exe"]).expect_err("it refuses");
        assert!(why.contains("not one of this tool's flags"), "{why}");
    }
}
