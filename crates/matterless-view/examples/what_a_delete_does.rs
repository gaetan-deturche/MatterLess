//! Posts a message, and deletes it, so a window can be watched doing both.
//!
//! A message deleted elsewhere was reported as staying on screen here. The
//! machinery for it exists and reads as correct -- `Event::PostDeleted` stores
//! the tombstone and answers `Delta::PostTombstoned`, and every feed query
//! filters `delete_at = 0` -- so the question is not what the code says but
//! what actually arrives, which only a real delete answers.
//!
//! Two commands rather than one with a wait in it, so the looking happens
//! between them:
//!
//! ```text
//!   what_a_delete_does aim               -- opens the window there next start
//!   what_a_delete_does post              -- prints the channel and the post id
//!   what_a_delete_does delete <post id>
//! ```
//!
//! `aim` writes the `left_off` row, because the window reopens where the
//! reader was and a channel named on the command line only feeds the read it
//! does before the window exists. It writes through this build's own store --
//! the sandbox one -- and it is a binary rather than a script on purpose:
//! `%AppData%` is virtualised per container here, so the only thing that sees
//! what the window sees is something launched the same way the window is.
//!
//! It writes to the reader's own note to self, which is where a probe belongs:
//! a direct channel with both halves the same person, private to them.

fn main() {
    let mut args = std::env::args().skip(1);
    let verb = args.next().unwrap_or_default();
    let which = args.next();
    if !matches!(verb.as_str(), "aim" | "post" | "delete" | "look") {
        eprintln!("usage: what_a_delete_does aim | post | delete <post id> | look <post id>");
        return;
    }
    // What this build's own store says about one post, which is how to tell
    // "the event never arrived" from "the window did not act on it".
    if verb == "look" {
        let Some(path) = matterless_view::feed::default_store() else {
            eprintln!("nowhere to keep a store");
            return;
        };
        let Some(post_id) = which else {
            eprintln!("which post?");
            return;
        };
        match matterless_store::Store::open(&path) {
            Ok(store) => match store.post(&post_id) {
                Ok(Some(post)) => println!(
                    "{post_id}: held, delete_at {}, message {:?}",
                    post.delete_at, post.message
                ),
                Ok(None) => println!("{post_id}: the store has never heard of it"),
                Err(error) => eprintln!("reading it: {error}"),
            },
            Err(error) => eprintln!("open {}: {error}", path.display()),
        }
        return;
    }
    let Some(token) = matterless_view::live::stored_token() else {
        eprintln!("no stored session");
        return;
    };
    let Some(store_path) = matterless_view::feed::default_store() else {
        eprintln!("nowhere to keep a store");
        return;
    };
    // From the file beside the database rather than an environment variable:
    // it is the address this machine is already pointed at, and a sandbox
    // borrows the installed program's.
    let Some(server) = matterless_view::live::stored_server(&store_path) else {
        eprintln!("no server.txt to say where to talk to");
        return;
    };

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("a runtime");
    runtime.block_on(async move {
        let rest = matterless_core::rest::RestClient::new(&server).expect("a client");
        rest.set_token(matterless_core::auth::AuthToken::Session(token));
        let me = match rest.me().await {
            Ok(me) => me,
            Err(error) => {
                eprintln!("who am I: {error}");
                return;
            }
        };
        if verb == "delete" {
            let Some(post_id) = which else {
                eprintln!("which post?");
                return;
            };
            match rest.delete_post(&post_id).await {
                Ok(()) => println!("deleted {post_id}"),
                Err(error) => eprintln!("deleting {post_id}: {error}"),
            }
            return;
        }
        let channel_id = match rest.direct_channel(&me.id, &me.id).await {
            Ok(channel) => channel.id,
            Err(error) => {
                eprintln!("finding your own conversation: {error}");
                return;
            }
        };
        if verb == "aim" {
            let store = match matterless_store::Store::open(&store_path) {
                Ok(store) => store,
                Err(error) => {
                    eprintln!("open {}: {error}", store_path.display());
                    return;
                }
            };
            match store.leave_off(&me.id, &channel_id, matterless_view::clock::now() * 1_000) {
                Ok(()) => println!("the next window opens {channel_id}"),
                Err(error) => eprintln!("aiming it: {error}"),
            }
            return;
        }
        let said = format!(
            "a probe about to be deleted -- {}",
            matterless_view::clock::now()
        );
        let post = matterless_core::model::NewPost {
            channel_id: &channel_id,
            message: &said,
            root_id: "",
            file_ids: &[],
            pending_post_id: "",
        };
        match rest.create_post(&post).await {
            Ok(post) => {
                println!("channel {channel_id}");
                println!("posted {}", post.id);
            }
            Err(error) => eprintln!("posting: {error}"),
        }
    });
}
