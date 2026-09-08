//! Proves the server accepts the one thing this client *sends* over the socket.
//!
//! Ignored by default: it needs a session token and the network.
//!
//! ```text
//! MM_TOKEN=$(python -c "import json;print(json.load(open('tools/.mm_token.json'))['token'])") \
//!   cargo test -p matterless-core --test live_typing -- --ignored --nocapture
//! ```
//!
//! Worth a live test rather than a unit test: a typing frame is fire and
//! forget, so a wrong field name (`parent_id` is the wire spelling of the
//! thread, not `root_id`) or a rejected shape would fail *silently* -- nobody
//! would see a typing indicator and nothing would log. The server's own
//! reaction is the only evidence.

use futures_util::{SinkExt, StreamExt};
use matterless_core::{AuthToken, RestClient};
use tokio_tungstenite::tungstenite::Message;

/// The server these live tests talk to, from the environment at build time.
///
/// `option_env!`, not `env!`: an unset variable must not fail the build for
/// everyone else. These tests are `#[ignore]`d and need a minted token anyway,
/// so an empty server simply means they were not set up to run.
const SERVER: &str = match option_env!("MATTERLESS_TEST_SERVER") {
    Some(server) => server,
    None => "",
};

#[tokio::test]
#[ignore = "needs MM_TOKEN and the network"]
async fn the_server_accepts_a_typing_frame() {
    let token = std::env::var("MM_TOKEN").expect("MM_TOKEN");
    let rest = RestClient::new(SERVER).expect("client");
    rest.set_token(AuthToken::Session(token.clone()));

    let me = rest.me().await.expect("me");
    let teams = rest.my_teams().await.expect("teams");
    let channels = rest
        .my_channels(&teams.first().expect("a team").id)
        .await
        .expect("channels");
    // The account's own direct message channel: typing into it tells nobody
    // anything, which is what a test should do.
    let mine = format!("{}__{}", me.id, me.id);
    let channel = channels
        .iter()
        .find(|channel| channel.name == mine)
        .expect("the direct message with myself");

    let (mut socket, _) = tokio_tungstenite::connect_async(format!(
        // The same server the rest of this test uses, as a websocket.
        "{}/api/v4/websocket",
        SERVER.replacen("https://", "wss://", 1)
    ))
    .await
    .expect("connect");

    let challenge = serde_json::json!({
        "seq": 1,
        "action": "authentication_challenge",
        "data": { "token": token },
    });
    socket
        .send(Message::Text(challenge.to_string().into()))
        .await
        .expect("authenticate");

    let typing = serde_json::json!({
        "seq": 2,
        "action": "user_typing",
        "data": { "channel_id": channel.id, "parent_id": "" },
    });
    socket
        .send(Message::Text(typing.to_string().into()))
        .await
        .expect("send typing");

    // The server answers an action with a `seq_reply` carrying its status. A
    // rejected frame comes back with an error on the same seq, which is exactly
    // the silent failure this test exists to catch.
    let mut authenticated = false;
    let mut accepted = false;
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(15);
    while tokio::time::Instant::now() < deadline {
        let Ok(Some(Ok(message))) =
            tokio::time::timeout(std::time::Duration::from_secs(5), socket.next()).await
        else {
            break;
        };
        let Message::Text(text) = message else {
            continue;
        };
        let frame: serde_json::Value = serde_json::from_str(&text).expect("json");
        let seq_reply = frame.get("seq_reply").and_then(serde_json::Value::as_i64);
        let status = frame.get("status").and_then(serde_json::Value::as_str);
        println!(
            "frame: seq_reply={:?} status={:?} event={:?} error={:?}",
            seq_reply,
            status,
            frame.get("event").and_then(serde_json::Value::as_str),
            frame.get("error")
        );
        match seq_reply {
            Some(1) => authenticated = status == Some("OK"),
            Some(2) => {
                accepted = status == Some("OK") && frame.get("error").is_none();
                break;
            }
            _ => {}
        }
    }

    assert!(authenticated, "the socket authenticated");
    assert!(
        accepted,
        "the server acknowledged the typing frame -- a rejection here is the \
         silent failure the whole test is for"
    );
    let _ = socket.close(None).await;
}
