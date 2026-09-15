//! What the release is offering, and whether this build understands it.
//!
//! The manifest is written by a pipeline this code does not control, so the
//! two things most likely to be wrong are its shape and the platform key it is
//! looked up by -- and neither can be checked by a unit test against a string
//! somebody typed here. This asks the real endpoint.
//!
//! Read-only, and it downloads nothing: the installer is never fetched.
//!
//!     cargo run -p matterless-view --example what_update

use matterless_view::update;

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let target = update::target().unwrap_or_else(|| "<none for this platform>".to_string());
    println!("this build is {} and looks for {target}", update::running());
    println!("asking {}", update::ENDPOINT);

    let client = match reqwest::Client::builder()
        .user_agent(concat!("MatterLess/", env!("CARGO_PKG_VERSION")))
        .build()
    {
        Ok(client) => client,
        Err(error) => return eprintln!("no client: {error}"),
    };
    let answered = match client.get(update::ENDPOINT).send().await {
        Ok(answered) => answered,
        Err(error) => return eprintln!("the endpoint did not answer: {error}"),
    };
    println!("the endpoint answered {}", answered.status());
    let body = match answered.text().await {
        Ok(body) => body,
        Err(error) => return eprintln!("no body: {error}"),
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

    match update::offered(&manifest, &target, update::running()) {
        Some(offer) => println!("would offer {}", offer.version),
        None => println!("nothing to offer: this build is already the newest"),
    }

    // The signature path, against the artefact the release actually signed.
    // Opt-in because it fetches the whole installer -- and it still runs
    // nothing: the bytes are checked and dropped.
    if !std::env::args().any(|arg| arg == "--verify") {
        println!("pass --verify to check the signature against the real installer");
        return;
    }
    let Some(platform) = manifest.platforms.get(&target) else {
        return;
    };
    println!("fetching {}", platform.url);
    let bytes = match client.get(&platform.url).send().await {
        Ok(answered) => match answered.bytes().await {
            Ok(bytes) => bytes,
            Err(error) => return eprintln!("no body: {error}"),
        },
        Err(error) => return eprintln!("the installer did not come: {error}"),
    };
    println!(
        "{} bytes, looks like {:?}",
        bytes.len(),
        update::kind_of(&bytes)
    );
    match update::verified(&bytes, &platform.signature, update::PUBKEY) {
        Ok(()) => println!("SIGNED: the installer is what this key's holder signed"),
        Err(error) => eprintln!("REFUSED: {error}"),
    }
    // And a tampered copy must fail, or the check above proves nothing.
    let mut tampered = bytes.to_vec();
    if let Some(byte) = tampered.last_mut() {
        *byte = byte.wrapping_add(1);
    }
    match update::verified(&tampered, &platform.signature, update::PUBKEY) {
        Ok(()) => eprintln!("BROKEN: a changed installer passed the check"),
        Err(_) => println!("and one byte changed is refused, so the check is real"),
    }
}
