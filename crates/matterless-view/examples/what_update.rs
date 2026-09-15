//! What the release is offering, and whether this build understands it.
//!
//! The manifest is written by a pipeline this code does not control, so the
//! two things most likely to be wrong are its shape and the platform key it is
//! looked up by -- and neither can be checked by a unit test against a string
//! somebody typed here.
//!
//! Against the real endpoint, read-only, downloading nothing:
//!
//!     cargo run -p matterless-view --example what_update
//!     cargo run -p matterless-view --example what_update -- --verify
//!
//! Or against files on disk, which is how a release is rehearsed before it is
//! published -- the one step a tag cannot be pushed twice to get right:
//!
//!     cargo run -p matterless-view --example what_update --
//!         --manifest latest.json --installer setup.exe [--pubkey <base64>]
//!
//! `--pubkey` takes the app's own key when it is left out, so a real release
//! is checked against the key the app was built with. It is there for
//! rehearsing with a throwaway key, which is what a test of the pipeline
//! itself has to use.

use matterless_view::update;

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let asked = Asked::read();
    let target = update::target().unwrap_or_else(|| "<none for this platform>".to_string());
    println!("this build is {} and looks for {target}", update::running());

    let body = match &asked.manifest {
        Some(path) => match std::fs::read_to_string(path) {
            Ok(body) => {
                println!("reading {path}");
                body
            }
            Err(error) => return eprintln!("could not read {path}: {error}"),
        },
        None => match fetched(update::ENDPOINT).await {
            Ok(bytes) => match String::from_utf8(bytes) {
                Ok(body) => body,
                Err(error) => return eprintln!("the manifest is not text: {error}"),
            },
            Err(error) => return eprintln!("{error}"),
        },
    };

    let manifest: update::Manifest = match serde_json::from_str(&body) {
        Ok(manifest) => manifest,
        Err(error) => {
            eprintln!("the manifest does not parse as this build expects: {error}");
            // The first line of it, so the shape can be seen without printing
            // a signature into a terminal.
            eprintln!("it begins: {}", body.chars().take(120).collect::<String>());
            return;
        }
    };
    println!("the newest build is {}", manifest.version);
    let mut keys: Vec<&String> = manifest.platforms.keys().collect();
    keys.sort();
    println!("it carries: {keys:?}");
    match manifest.platforms.get(&target) {
        Some(platform) => println!(
            "this platform has an installer ({} bytes of signature)",
            platform.signature.len()
        ),
        None => println!("this platform has nothing in the manifest"),
    }

    // What the card would show before asking for a restart. A release that
    // says nothing about itself offers no button, so its absence is worth
    // seeing here rather than discovering on the day.
    match manifest.notes.as_deref().map(str::trim).unwrap_or_default() {
        "" => println!("it says nothing about itself: the card will offer no \"What's new\""),
        notes => println!(
            "the change list is {} line(s), beginning {:?}",
            notes.lines().count(),
            notes.lines().next().unwrap_or_default()
        ),
    }

    match update::offered(&manifest, &target, update::running()) {
        Some(offer) => println!("would offer {}", offer.version),
        None => println!("nothing to offer: this build is already the newest"),
    }

    let Some(platform) = manifest.platforms.get(&target) else {
        return;
    };
    // The signature path, against the artefact the release actually signed.
    // Opt-in when it has to be fetched, because that is the whole installer --
    // and it still runs nothing: the bytes are checked and dropped.
    let bytes = match &asked.installer {
        Some(path) => match std::fs::read(path) {
            Ok(bytes) => {
                println!("reading {path}");
                bytes
            }
            Err(error) => return eprintln!("could not read {path}: {error}"),
        },
        None if !asked.verify => {
            println!("pass --verify to check the signature against the real installer");
            return;
        }
        None => {
            println!("fetching {}", platform.url);
            match fetched(&platform.url).await {
                Ok(bytes) => bytes,
                Err(error) => return eprintln!("{error}"),
            }
        }
    };
    println!(
        "{} bytes, looks like {:?}",
        bytes.len(),
        update::kind_of(&bytes)
    );
    let pubkey = asked.pubkey.as_deref().unwrap_or(update::PUBKEY);
    match update::verified(&bytes, &platform.signature, pubkey) {
        Ok(()) => println!("SIGNED: the installer is what this key's holder signed"),
        Err(error) => eprintln!("REFUSED: {error}"),
    }
}

/// Whatever was asked for on the command line.
#[derive(Default)]
struct Asked {
    manifest: Option<String>,
    installer: Option<String>,
    pubkey: Option<String>,
    verify: bool,
}

impl Asked {
    fn read() -> Self {
        let mut asked = Self::default();
        let mut args = std::env::args().skip(1);
        while let Some(flag) = args.next() {
            match flag.as_str() {
                "--manifest" => asked.manifest = args.next(),
                "--installer" => asked.installer = args.next(),
                "--pubkey" => asked.pubkey = args.next(),
                "--verify" => asked.verify = true,
                other => eprintln!("ignoring {other}"),
            }
        }
        asked
    }
}

/// One GET, with the app's own user agent.
async fn fetched(url: &str) -> Result<Vec<u8>, String> {
    let client = reqwest::Client::builder()
        .user_agent(concat!("MatterLess/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|error| format!("no client: {error}"))?;
    println!("asking {url}");
    let answered = client
        .get(url)
        .send()
        .await
        .map_err(|error| format!("{url} did not answer: {error}"))?;
    println!("it answered {}", answered.status());
    answered
        .bytes()
        .await
        .map(|body| body.to_vec())
        .map_err(|error| format!("no body: {error}"))
}
