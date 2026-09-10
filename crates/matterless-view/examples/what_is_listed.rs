//! What the saved and pinned lists actually come back with.
//!
//! The panels open on a keystroke, which cannot be pressed from outside the
//! window -- so this runs the same two requests and the same resolution the
//! panels do, and prints what the rows would say. An empty panel is then
//! decidable: either this account has saved nothing, or the resolution is
//! wrong.
//!
//! Names and conversations, never message text.

fn main() {
    let channel = std::env::args().nth(1);
    let Some(token) = matterless_view::live::stored_token() else {
        eprintln!("no stored session");
        return;
    };
    let Some(store) = matterless_view::feed::default_store()
        .and_then(|path| matterless_store::Store::open(&path).ok())
    else {
        eprintln!("no store");
        return;
    };
    let server = std::env::var("MATTERLESS_SERVER")
        .unwrap_or_else(|_| "https://mattermost.northwind.invalid".to_string());

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

        match rest.flagged_posts(&me.id, 60).await {
            Ok(list) => show("Saved", &store, list, &me.id),
            Err(error) => eprintln!("saved: {error}"),
        }
        let Some(channel) = channel else {
            println!("pass a channel id to also list what is pinned in it");
            return;
        };
        match rest.pinned_posts(&channel).await {
            Ok(list) => show("Pinned", &store, list, &me.id),
            Err(error) => eprintln!("pinned: {error}"),
        }
    });
}

fn show(
    title: &str,
    store: &matterless_store::Store,
    list: matterless_core::model::PostList,
    me_id: &str,
) {
    let posts: Vec<matterless_core::Post> = list
        .order
        .iter()
        .filter_map(|id| list.posts.get(id).cloned())
        .filter(|post| post.delete_at == 0)
        .collect();
    let found = matterless_view::listing::found_for(store, posts, me_id);
    println!("{title}: {} messages", found.len());
    for one in &found {
        // The two things the resolution can get wrong, and nothing else.
        println!("  {} in {}", one.author, one.channel);
    }
}
