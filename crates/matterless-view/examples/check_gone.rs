//! Asks the server whether the posts this window tombstoned are really gone.
//!
//! Written to check an inference rather than to ship with it: `vanished` calls
//! a message deleted when it is missing from a page that covers its time, and
//! a wrong range bound would hide messages that still exist. This is how that
//! is decided -- by asking, rather than by reasoning about it again.
//!
//! Run with the post ids as arguments. `--repair` also puts back the ones the
//! server still has, for a tombstone the reconcile cannot reach on its own:
//! `returned` only sees what a recent page carries, and a post older than that
//! page would stay hidden.

fn main() {
    let mut ids: Vec<String> = std::env::args().skip(1).collect();
    let repair = ids.iter().any(|arg| arg == "--repair");
    ids.retain(|arg| !arg.starts_with("--"));
    if ids.is_empty() {
        eprintln!("usage: check_gone <post id>...");
        return;
    }
    let Some(token) = matterless_view::live::stored_token() else {
        eprintln!("no stored session");
        return;
    };
    let server = std::env::var("MATTERLESS_SERVER")
        .unwrap_or_else(|_| "https://mattermost.sloclap.net".to_string());

    let store = matterless_view::feed::default_store()
        .and_then(|path| matterless_store::Store::open(&path).ok());
    if repair && store.is_none() {
        eprintln!("no store to repair");
        return;
    }

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("a runtime");
    runtime.block_on(async move {
        let rest = matterless_core::rest::RestClient::new(&server).expect("a client");
        rest.set_token(matterless_core::auth::AuthToken::Session(token));
        for id in ids {
            match rest.post(&id).await {
                // Still there, and not deleted: this one was wrongly hidden.
                Ok(post) if post.delete_at == 0 => {
                    println!("{id}: STILL THERE ({} characters)", post.message.len());
                    if repair {
                        match store.as_ref().map(|store| store.restore_post(&id)) {
                            Some(Ok(true)) => println!("  put back"),
                            Some(Ok(false)) => println!("  was not hidden"),
                            Some(Err(error)) => println!("  could not put back: {error}"),
                            None => println!("  no store to put it back in"),
                        }
                    }
                }
                Ok(_) => println!("{id}: the server calls it deleted too"),
                Err(error) => println!("{id}: gone ({error})"),
            }
        }
    });
}
