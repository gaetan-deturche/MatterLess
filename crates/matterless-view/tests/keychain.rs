//! Does this machine's credential store keep what the app puts in it?
//!
//! The app depends on it for exactly one thing: not asking for a password
//! again on the next launch. When it fails, it fails quietly -- an entry is
//! listed, and reading it answers nothing -- so this asks the question through
//! the same crate and the same calls `remember_token` and `stored_token` make,
//! rather than through `cmdkey`, which can see an entry that the library
//! cannot read back.
//!
//! ```text
//! cargo test -p matterless-view --test keychain -- --ignored --nocapture
//! ```
//!
//! Neither of these writes to the entry the app uses. The round trip uses a
//! name of its own and removes it again; the second only reads.

/// A write, a read and a delete, under a name nothing else owns.
///
/// Answers whether the store works at all here. If this fails, the app can
/// never keep a session on this machine and should say so rather than
/// pretending the next launch will be fine.
#[test]
#[ignore = "touches the machine's credential store"]
fn the_credential_store_keeps_what_it_is_given() {
    let entry = keyring::Entry::new("matterless-keychain-test", "roundtrip")
        .expect("no entry could be made at all");
    let secret = "a-value-nothing-else-would-write";
    entry
        .set_password(secret)
        .expect("the store refused a write");
    let read = entry.get_password();
    // What Windows itself can see, while it is still there.
    //
    // This is the half that matters, and the half this test did not have. A
    // write and a read back inside one process prove nothing: with no platform
    // feature enabled, `keyring` falls back to a mock that lives in the
    // process, answers every read with what it was given, and keeps nothing.
    // That passes a round trip and loses the session at every launch, which is
    // exactly what the app did for as long as nobody asked this question.
    //
    // The target is `{user}.{service}`, which is how the crate names one.
    let listed = std::process::Command::new("cmdkey").arg("/list").output();
    let seen = listed
        .map(|out| {
            String::from_utf8_lossy(&out.stdout).contains("roundtrip.matterless-keychain-test")
        })
        .unwrap_or(false);
    // Removed before the asserts, so a failure does not leave it behind.
    let removed = entry.delete_credential();
    println!("delete: {removed:?}");
    assert_eq!(
        read.expect("the store took the write and would not read it back"),
        secret,
        "the store kept something other than what it was given"
    );
    assert!(
        seen,
        "the write never reached Windows: this is the in-process mock, which keeps nothing"
    );
}

/// What the app's own entry answers, without touching it.
///
/// Not an assertion: a machine that has never signed in has nothing here, and
/// that is not a failure. It prints what it found so the two cases -- absent,
/// and present but unreadable -- can be told apart, which is the whole
/// question and the thing `cmdkey` cannot settle.
#[test]
#[ignore = "reads the machine's credential store"]
fn what_the_app_has_in_there_now() {
    match keyring::Entry::new("matterless", "session") {
        Err(error) => println!("no entry handle: {error}"),
        Ok(entry) => match entry.get_password() {
            Ok(held) if held.is_empty() => println!("the entry is there and empty"),
            // Never the value itself: it is a thirty-day credential for an
            // account, and this prints to a terminal and a transcript.
            Ok(held) => println!("the entry holds {} characters", held.chars().count()),
            Err(error) => println!("the entry would not be read: {error}"),
        },
    }
}
