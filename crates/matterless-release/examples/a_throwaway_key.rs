//! A minisign keypair for rehearsing a release, and nothing else.
//!
//! Prints the secret key file, a separator, then the public key as the app
//! carries it -- base64 on top of minisign's own, which is the spelling
//! `update::PUBKEY` uses and the one `what_update --pubkey` wants.
//!
//! Never the real key: that one is in this repository's secrets and nowhere
//! else. This exists so the pipeline can be run end to end on a machine that
//! has no business holding it.
//!
//!     cargo run -p matterless-release --example a_throwaway_key

fn main() {
    use base64::Engine;
    let password = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "rehearsal".to_string());
    let pair =
        minisign::KeyPair::generate_encrypted_keypair(Some(password.clone())).expect("a keypair");
    let secret = pair.sk.to_box(None).expect("a secret key box").to_string();
    let public = pair.pk.to_box().expect("a public key box").to_string();
    print!("{secret}");
    println!("---");
    println!(
        "{}",
        base64::engine::general_purpose::STANDARD.encode(&public)
    );
    eprintln!("password: {password}");
}
