//! Sends a file the way a drop on the window does.
//!
//! The window's own path cannot be driven from outside -- a drop is a gesture
//! -- so this calls the same `upload` with the same arguments the event
//! handler builds. It sends a real message, so it takes the conversation and
//! the file as arguments rather than guessing either.
//!
//! `drop_a_file <channel id> <path>`, or `--self` for the reader's own note
//! to self, which is where a probe belongs.

fn main() {
    let mut args = std::env::args().skip(1);
    let target = args.next();
    let path = args.next().map(std::path::PathBuf::from);
    let (Some(target), Some(path)) = (target, path) else {
        eprintln!("usage: drop_a_file <channel id|--self> <path>");
        return;
    };
    if !path.is_file() {
        eprintln!("{} is not a file", path.display());
        return;
    }
    let Some(token) = matterless_view::live::stored_token() else {
        eprintln!("no stored session");
        return;
    };
    let Some(store_path) = matterless_view::feed::default_store() else {
        eprintln!("no store");
        return;
    };
    let Ok(store) = matterless_store::Store::open(&store_path) else {
        eprintln!("no store");
        return;
    };
    let server = std::env::var("MATTERLESS_SERVER")
        .unwrap_or_else(|_| "https://mattermost.sloclap.net".to_string());

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

        // A note to self is a direct channel with both halves the same person,
        // which the server will find or create like any other.
        let channel_id = if target == "--self" {
            match rest.direct_channel(&me.id, &me.id).await {
                Ok(channel) => channel.id,
                Err(error) => {
                    eprintln!("finding your own conversation: {error}");
                    return;
                }
            }
        } else {
            target
        };

        let engine = matterless_sync::SyncEngine::new(std::sync::Arc::new(store));
        let context =
            matterless_sync::SyncContext::new(me, matterless_core::model::ThreadMode::Collapsed);
        matterless_view::live::upload(&rest, &engine, &context, &channel_id, "", &path).await;
        // Named so the window can be pointed at it to look.
        println!("sent to {channel_id}");
        match rest.posts(&channel_id, 3).await {
            Ok(list) => {
                for id in list.order.iter().take(3) {
                    let Some(post) = list.posts.get(id) else {
                        continue;
                    };
                    println!("  {} carries {} file(s)", post.id, post.file_ids.len());
                }
            }
            Err(error) => eprintln!("reading it back: {error}"),
        }
    });
}
