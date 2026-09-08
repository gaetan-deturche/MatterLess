//! Test harness for the MatterLess core. Three modes, all read-only against the
//! server except the login/logout they need:
//!
//!     cargo run -p matterless-probe -- --minutes 60 --rst-every 15
//!     cargo run -p matterless-probe -- --test-resume
//!     cargo run -p matterless-probe -- --offline-gate
//!
//! Credentials come from a token minted once by `tools/mint_token.py`, or from
//! the environment, or from a no-echo prompt. Nothing is ever written to disk.

mod offline_gate;
mod recording;
mod soak;

use anyhow::{Context, Result, bail};
use matterless_core::ws::{Signal, WsSession};
use matterless_core::{AuthToken, Error, RestClient};
use std::time::{Duration, Instant};
use tokio::sync::mpsc;

/// The server the probe talks to, unless `--server` says otherwise.
///
/// From the environment at build time, and empty when it was not set: no host
/// is compiled in, so a checkout of this repository is not a statement about
/// whose Mattermost it was written against.
const DEFAULT_SERVER: &str = match option_env!("MATTERLESS_SERVER") {
    Some(server) => server,
    None => "",
};

struct Args {
    server: String,
    minutes: u64,
    rst_every_minutes: u64,
    only_channel: Option<String>,
    test_resume: bool,
    offline_gate: bool,
    offline_minutes: u64,
    channel_budget: usize,
    record_path: Option<String>,
    replay_path: Option<String>,
}

fn parse_args() -> Args {
    let mut args = Args {
        server: std::env::var("MATTERLESS_SERVER").unwrap_or_else(|_| DEFAULT_SERVER.to_string()),
        minutes: 60,
        rst_every_minutes: 0,
        only_channel: None,
        test_resume: false,
        offline_gate: false,
        offline_minutes: 10,
        channel_budget: 15,
        record_path: None,
        replay_path: None,
    };
    let mut raw = std::env::args().skip(1);
    let number = |value: Option<String>, fallback: u64| -> u64 {
        value.and_then(|text| text.parse().ok()).unwrap_or(fallback)
    };
    while let Some(flag) = raw.next() {
        match flag.as_str() {
            "--server" => args.server = raw.next().unwrap_or(args.server),
            "--minutes" => args.minutes = number(raw.next(), args.minutes),
            "--rst-every" => args.rst_every_minutes = number(raw.next(), args.rst_every_minutes),
            "--offline-minutes" => args.offline_minutes = number(raw.next(), args.offline_minutes),
            "--channels" => args.channel_budget = number(raw.next(), 15) as usize,
            "--channel" => args.only_channel = raw.next(),
            "--record" => args.record_path = raw.next(),
            "--replay" => args.replay_path = raw.next(),
            "--test-resume" => args.test_resume = true,
            "--offline-gate" => args.offline_gate = true,
            other => eprintln!("ignoring unknown flag {other}"),
        }
    }
    args
}

fn read_credentials() -> Result<(String, String)> {
    let login = match std::env::var("MATTERLESS_LOGIN") {
        Ok(value) if !value.is_empty() => value,
        _ => {
            print!("login (email or username): ");
            use std::io::Write;
            std::io::stdout().flush()?;
            let mut line = String::new();
            std::io::stdin().read_line(&mut line)?;
            line.trim().to_string()
        }
    };
    let password = match std::env::var("MATTERLESS_PASSWORD") {
        Ok(value) if !value.is_empty() => value,
        _ => rpassword::prompt_password("password (not echoed): ")?,
    };
    Ok((login, password))
}

async fn authenticate(client: &RestClient) -> Result<matterless_core::User> {
    let (login, password) = read_credentials()?;
    match client.login(&login, &password, None).await {
        Ok(user) => Ok(user),
        Err(Error::MfaRequired) => {
            let code = rpassword::prompt_password("MFA code: ")?;
            client
                .login(&login, &password, Some(code.trim()))
                .await
                .context("login with MFA code")
        }
        Err(other) => Err(other).context("login"),
    }
}

/// A minted session token, from the environment or the file `mint_token.py`
/// writes. Long-lived (server default 30 days, measured), which is how the
/// official app avoids a login at every startup.
fn stored_token() -> Option<String> {
    if let Ok(token) = std::env::var("MATTERLESS_TOKEN")
        && !token.is_empty()
    {
        return Some(token);
    }
    let path = std::env::var("MATTERLESS_TOKEN_FILE")
        .unwrap_or_else(|_| "tools/.mm_token.json".to_string());
    let raw = std::fs::read_to_string(&path).ok()?;
    let parsed: serde_json::Value = serde_json::from_str(&raw).ok()?;
    parsed
        .get("token")
        .and_then(|value| value.as_str())
        .map(str::to_owned)
        .filter(|token| !token.is_empty())
}

/// Returns the account plus whether *this run* created the session -- because a
/// borrowed token must not be revoked by our logout at the end.
async fn resolve_auth(client: &RestClient) -> Result<(matterless_core::User, bool)> {
    if let Some(token) = stored_token() {
        client.set_token(AuthToken::Session(token));
        match client.me().await {
            Ok(user) => {
                println!("using a stored session token (no login needed)");
                return Ok((user, false));
            }
            Err(Error::SessionExpired) => {
                eprintln!("stored token was rejected; falling back to an interactive login");
                eprintln!("(mint a fresh one with: python tools/mint_token.py)");
            }
            Err(other) => return Err(other).context("validating the stored token"),
        }
    }
    let user = authenticate(client).await?;
    Ok((user, true))
}

/// Phase 0 left this open: a clean close proved nothing about whether the server
/// will replay, so force an RST -- the case reliable resume exists for -- and
/// see whether the connection id survives.
async fn test_resume(client: &RestClient) -> Result<()> {
    let token = client.token().context("not authenticated")?;
    let (signals_tx, mut signals) = mpsc::channel(256);
    let handle = WsSession::new(&client.base_url(), token)?.spawn(signals_tx);

    let mut first_connection_id = None;
    let mut verdict = None;
    let deadline = Instant::now() + Duration::from_secs(90);

    while Instant::now() < deadline {
        let Ok(signal) = tokio::time::timeout(Duration::from_secs(20), signals.recv()).await else {
            break;
        };
        let Some(signal) = signal else { break };

        match signal {
            Signal::Connected {
                connection_id,
                resumed,
            } => {
                if first_connection_id.is_none() {
                    println!("connected, id {connection_id}");
                    first_connection_id = Some(connection_id);
                    tokio::time::sleep(Duration::from_secs(10)).await;
                    println!("forcing an RST (SO_LINGER 0) ...");
                    handle.hard_reset().await;
                } else {
                    verdict = Some(resumed);
                    println!(
                        "reconnected, id {connection_id} -- resume {}",
                        if resumed { "HONOURED" } else { "REFUSED" }
                    );
                    break;
                }
            }
            Signal::Disconnected { reason } => println!("disconnected: {reason}"),
            Signal::ResyncRequired => println!("signal: resync required"),
            Signal::Event { .. } => {}
        }
    }

    handle.shutdown().await;
    match verdict {
        Some(true) => println!(
            "\nRESULT: the server replays after an abrupt close. Resume is usable, \
             so a reconnect need not always mean a full resync."
        ),
        Some(false) => println!(
            "\nRESULT: the server issues a new connection id even after an RST. \
             Treat every reconnect as a full resync -- as the plan already assumes."
        ),
        None => println!("\nRESULT: inconclusive, no second connection was observed."),
    }
    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "matterless_core=info,matterless_probe=info".into()),
        )
        .init();

    let args = parse_args();

    // A replay needs neither the network nor a session, so it short-circuits
    // everything below -- that is the whole point of it.
    if let Some(path) = args.replay_path.as_deref() {
        return match soak::replay(path) {
            Ok(true) => Ok(()),
            Ok(false) => bail!("replayed session does not pass"),
            Err(error) => Err(error),
        };
    }

    let client = RestClient::new(&args.server).context("build client")?;
    client.ping().await.context("server unreachable")?;
    println!(
        "server {} version {}",
        args.server,
        client.server_version().unwrap_or_else(|| "unknown".into())
    );

    let (me, session_is_ours) = resolve_auth(&client).await?;
    println!("authenticated as {}", me.username);

    let result = if args.test_resume {
        test_resume(&client).await
    } else if args.offline_gate {
        let config = offline_gate::GateConfig {
            offline_minutes: args.offline_minutes,
            channel_budget: args.channel_budget,
            ..Default::default()
        };
        match offline_gate::run(&client, &me, config).await {
            Ok(true) => Ok(()),
            Ok(false) => bail!("phase 2 gate not met"),
            Err(error) => Err(error),
        }
    } else {
        let config = soak::SoakConfig {
            minutes: args.minutes,
            rst_every_minutes: args.rst_every_minutes,
            only_channel: args.only_channel.clone(),
            record_path: args.record_path.clone(),
        };
        match soak::run(&client, &me, config).await {
            Ok(true) => Ok(()),
            Ok(false) => bail!("phase 1 gate not met"),
            Err(error) => Err(error),
        }
    };

    // Revoking a borrowed token would force a re-mint on the next run.
    if session_is_ours && let Err(error) = client.logout().await {
        eprintln!("logout failed: {error}");
    }
    result
}
