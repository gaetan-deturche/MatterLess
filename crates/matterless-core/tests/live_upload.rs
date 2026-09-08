//! Proves the hand-assembled multipart body against the real server.
//!
//! Ignored by default: it needs a session token and the network, so it is not
//! part of `cargo test`. Run it deliberately after touching `upload_file`:
//!
//! ```text
//! MM_TOKEN=$(python -c "import json;print(json.load(open('tools/.mm_token.json'))['token'])") \
//!   cargo test -p matterless-core --test live_upload -- --ignored --nocapture
//! ```
//!
//! It uploads into the account's direct message with itself and attaches the
//! file to nothing, so no channel gets a message out of it. An unclaimed upload
//! is orphaned server-side, which is exactly what happens when somebody
//! attaches a file in the composer and then changes their mind.

use matterless_core::{AuthToken, RestClient};

/// The server these live tests talk to, from the environment at build time.
///
/// `option_env!`, not `env!`: an unset variable must not fail the build for
/// everyone else. These tests are `#[ignore]`d and need a minted token anyway,
/// so an empty server simply means they were not set up to run.
const SERVER: &str = match option_env!("MATTERLESS_TEST_SERVER") {
    Some(server) => server,
    None => "",
};

/// A 1x1 PNG, small enough to inline and real enough that the server decodes
/// it. The decode is the point: dimensions coming back is what proves the body
/// was parsed as a file rather than stored as a blob of bytes.
const PIXEL: &[u8] = &[
    0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44, 0x52,
    0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1f, 0x15, 0xc4,
    0x89, 0x00, 0x00, 0x00, 0x0a, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9c, 0x63, 0x00, 0x01, 0x00, 0x00,
    0x05, 0x00, 0x01, 0x0d, 0x0a, 0x2d, 0xb4, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4e, 0x44, 0xae,
    0x42, 0x60, 0x82,
];

#[tokio::test]
#[ignore = "needs MM_TOKEN and the network"]
async fn the_multipart_body_is_one_the_server_accepts() {
    let token = std::env::var("MM_TOKEN").expect("MM_TOKEN");
    let rest = RestClient::new(SERVER).expect("client");
    rest.set_token(AuthToken::Session(token));

    let me = rest.me().await.expect("me");
    let teams = rest.my_teams().await.expect("teams");
    let team = teams.first().expect("a team");
    // The account's own direct message channel, which is the one place an
    // upload can be parked without anyone else being able to see it.
    let mine = format!("{}__{}", me.id, me.id);
    let channels = rest.my_channels(&team.id).await.expect("channels");
    let channel = channels
        .iter()
        .find(|channel| channel.name == mine)
        .expect("the direct message with myself; open it once in any client");

    // Watched as it goes, so the streamed body is exercised rather than just
    // compiled: a progress report that never fires, or one that stops short of
    // the total, is the failure a reader would see as a bar that sticks.
    let seen: std::sync::Arc<std::sync::Mutex<Vec<(u64, u64)>>> = Default::default();
    let recorder = std::sync::Arc::clone(&seen);
    let progress: matterless_core::rest::UploadProgress =
        std::sync::Arc::new(move |sent, total| {
            recorder.lock().expect("progress").push((sent, total));
        });

    // A name with an accent and a space: a header parameter is ASCII, so this
    // is the case that breaks if the filename is mishandled anywhere.
    let uploaded = rest
        .upload_file(&channel.id, "réunion été.png", PIXEL, Some(progress))
        .await
        .expect("upload");

    let reports = seen.lock().expect("progress").clone();
    println!("progress reports: {reports:?}");
    let (sent, total) = *reports.last().expect("at least one progress report");
    assert_eq!(sent, total, "the last report is the whole body");
    assert!(
        total > PIXEL.len() as u64,
        "progress counts the multipart envelope, not just the file"
    );
    let info = uploaded.file_infos.first().expect("one file info");
    println!(
        "uploaded id={} name={} size={} {}x{} preview={} mini={}",
        info.id,
        info.name,
        info.size,
        info.width,
        info.height,
        info.has_preview_image,
        info.mini_preview.as_ref().map(String::len).unwrap_or(0)
    );

    assert_eq!(
        info.size,
        PIXEL.len() as i64,
        "the whole file, not a prefix"
    );
    assert_eq!((info.width, info.height), (1, 1), "decoded as an image");
    assert_eq!(info.name, "réunion été.png", "the name survived the header");
    assert!(
        info.mini_preview.is_some(),
        "the server ships a placeholder with an image, which the plan relies on"
    );
    assert!(info.post_id.is_empty(), "nothing has claimed it yet");
}
