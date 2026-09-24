//! Does a sign-in against a real server come back at all?
//!
//! `live::sign_in` is the one path in this program that is a thread, a runtime,
//! a request and a wake, with nothing in between that a unit test can reach --
//! and "the button sticks on Signing in..." is exactly what it looks like when
//! any one of those four does not finish. This asks the question without a
//! window and without a password: a login that is *meant* to fail still has to
//! come back and say so.
//!
//! ```text
//! MATTERLESS_TEST_SERVER=https://mattermost.example.com \
//!   cargo test -p matterless-view --test live_signin -- --ignored --nocapture
//! ```

use matterless_view::live::{Update, Wake};
use std::sync::mpsc;

/// Hands whatever arrives straight to the test thread.
///
/// The real one is winit's event-loop proxy. What matters here is the half
/// `sign_in` owns: that it calls `wake` exactly once, from the thread it
/// spawned, whatever the server said.
/// Cloneable for the same reason the window's proxy is: a picture fetch runs
/// on a task of its own and carries a waker with it. A `Sender` clone is
/// another handle to the same channel, so the test still sees everything.
#[derive(Clone)]
struct Told(mpsc::Sender<Update>);

impl Wake for Told {
    fn wake(&self, update: Update) {
        let _ = self.0.send(update);
    }
}

#[test]
#[ignore = "needs MATTERLESS_TEST_SERVER and the network"]
fn a_refused_sign_in_comes_back() {
    let server = std::env::var("MATTERLESS_TEST_SERVER").expect("MATTERLESS_TEST_SERVER");
    let (sender, inbox) = mpsc::channel();
    matterless_view::live::sign_in(
        server,
        // Deliberately nobody. A real login would be a password in a test, and
        // what is being checked is that an answer arrives -- not which answer.
        "claude-signin-probe-does-not-exist".to_string(),
        "not-a-password".to_string(),
        None,
        Told(sender),
    );
    let update = inbox
        .recv_timeout(std::time::Duration::from_secs(45))
        .expect("nothing came back from sign_in within 45s");
    match update {
        Update::SignInRefused { why, needs_a_code } => {
            println!("refused: {why} (needs_a_code={needs_a_code})");
        }
        Update::SessionOpened { username, .. } => {
            panic!("a made-up login signed in as {username}")
        }
        other => panic!("sign_in woke the window with {other:?}"),
    }
}
