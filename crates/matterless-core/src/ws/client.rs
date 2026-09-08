use crate::auth::AuthToken;
use crate::error::{Error, Result};
use crate::ws::event::{self, Envelope, Event};
use futures_util::{SinkExt, StreamExt};
use std::time::Duration;
use std::time::Instant;
use tokio::net::TcpStream;
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, connect_async};
use url::Url;

/// The Go driver waits 60 s plus a buffer for a server ping before declaring the
/// connection dead. Any inbound frame counts as liveness, not only a ping.
const PING_GRACE: Duration = Duration::from_secs(60 + 15);
const BACKOFF_MAX: Duration = Duration::from_secs(30);

type Socket = WebSocketStream<MaybeTlsStream<TcpStream>>;

/// What the session reports upward. The sync engine cares about `ResyncRequired`
/// as much as about the events themselves.
#[derive(Debug)]
pub enum Signal {
    Connected {
        connection_id: String,
        /// True only when the server preserved the id we asked to resume.
        resumed: bool,
    },
    /// Server-pushed event, with the sequence number it arrived under and the
    /// instant its frame was read.
    ///
    /// The instant is the start of the websocket-to-glyph measurement: the whole
    /// point is to time from "the bytes were here" to "the reader can see it",
    /// so anything stamped later would flatter the number.
    Event {
        event: Event,
        seq: i64,
        read_at: Instant,
    },
    Disconnected {
        reason: String,
    },
    /// Replay was not honoured: every channel needs a `?since=` catch-up.
    ResyncRequired,
}

#[derive(Debug)]
pub enum Command {
    /// Close the TCP connection with a reset rather than a handshake, which is
    /// the failure mode reliable-websocket resume actually exists for. Phase 0's
    /// clean-close test proved nothing; this is the retest.
    HardReset,
    Shutdown,
    /// The one thing this client *sends* over the websocket. Everything else it
    /// asks for goes over REST.
    Typing {
        channel_id: String,
        root_id: String,
    },
}

pub struct WsHandle {
    commands: mpsc::Sender<Command>,
    task: tokio::task::JoinHandle<()>,
}

impl WsHandle {
    pub async fn hard_reset(&self) {
        let _ = self.commands.send(Command::HardReset).await;
    }

    /// Tells the server this reader is typing. `root_id` is empty for the
    /// channel itself.
    ///
    /// `try_send`, and dropped when the queue is full or the socket is down:
    /// typing is ephemeral by definition -- a "still typing" that arrives late
    /// says nothing true, and blocking a keystroke on a socket would be the
    /// wrong trade in the other direction.
    pub fn typing(&self, channel_id: &str, root_id: &str) {
        let _ = self.commands.try_send(Command::Typing {
            channel_id: channel_id.to_string(),
            root_id: root_id.to_string(),
        });
    }

    pub async fn shutdown(self) {
        let _ = self.commands.send(Command::Shutdown).await;
        let _ = self.task.await;
    }
}

pub struct WsSession {
    websocket_url: Url,
    token: AuthToken,
    /// Set once a hello arrives; reused as the resume key.
    connection_id: Option<String>,
    highest_seq: i64,
    action_seq: i64,
}

impl WsSession {
    pub fn new(base_url: &Url, token: AuthToken) -> Result<Self> {
        let mut websocket_url = base_url.join("/api/v4/websocket")?;
        let scheme = match base_url.scheme() {
            "http" => "ws",
            _ => "wss",
        };
        websocket_url
            .set_scheme(scheme)
            .map_err(|_| Error::Protocol("cannot derive a websocket scheme".into()))?;
        Ok(Self {
            websocket_url,
            token,
            connection_id: None,
            highest_seq: 0,
            action_seq: 0,
        })
    }

    /// Runs the connect/authenticate/read loop with reconnection until told to
    /// stop, reporting everything over `signals`.
    pub fn spawn(mut self, signals: mpsc::Sender<Signal>) -> WsHandle {
        let (command_tx, mut command_rx) = mpsc::channel::<Command>(8);
        let task = tokio::spawn(async move {
            let mut backoff = Duration::from_secs(1);
            loop {
                match self.connect_once(&signals, &mut command_rx).await {
                    Ok(Outcome::Shutdown) => break,
                    Ok(Outcome::Dropped { reason }) => {
                        let _ = signals.send(Signal::Disconnected { reason }).await;
                    }
                    Err(error) => {
                        let _ = signals
                            .send(Signal::Disconnected {
                                reason: error.to_string(),
                            })
                            .await;
                    }
                }

                // Jitter keeps a fleet of clients from reconnecting in lockstep.
                let jitter = Duration::from_millis(u64::from(rand_u16()) % 500);
                tokio::time::sleep(backoff + jitter).await;
                backoff = (backoff * 2).min(BACKOFF_MAX);

                if let Ok(Command::Shutdown) = command_rx.try_recv() {
                    break;
                }
            }
        });
        WsHandle {
            commands: command_tx,
            task,
        }
    }

    fn resume_url(&self) -> Url {
        let mut url = self.websocket_url.clone();
        if let Some(connection_id) = &self.connection_id {
            url.query_pairs_mut()
                .append_pair("connection_id", connection_id)
                .append_pair("sequence_number", &(self.highest_seq + 1).to_string());
        }
        url
    }

    async fn connect_once(
        &mut self,
        signals: &mpsc::Sender<Signal>,
        commands: &mut mpsc::Receiver<Command>,
    ) -> Result<Outcome> {
        let requested_resume = self.connection_id.clone();
        let url = self.resume_url();
        tracing::debug!(%url, resuming = requested_resume.is_some(), "websocket connect");

        let (mut socket, _response) = connect_async(url.as_str()).await?;

        self.action_seq += 1;
        let challenge = serde_json::json!({
            "seq": self.action_seq,
            "action": "authentication_challenge",
            "data": { "token": self.token.bearer() },
        });
        socket
            .send(Message::Text(challenge.to_string().into()))
            .await?;

        let mut deadline = tokio::time::Instant::now() + PING_GRACE;

        loop {
            tokio::select! {
                command = commands.recv() => {
                    match command {
                        Some(Command::Shutdown) | None => {
                            let _ = socket.close(None).await;
                            return Ok(Outcome::Shutdown);
                        }
                        Some(Command::HardReset) => {
                            abort_with_reset(&socket);
                            drop(socket);
                            return Ok(Outcome::Dropped { reason: "hard reset (test)".into() });
                        }
                        Some(Command::Typing { channel_id, root_id }) => {
                            self.action_seq += 1;
                            // `parent_id` is the wire name for the thread, as on
                            // the inbound event.
                            let frame = serde_json::json!({
                                "seq": self.action_seq,
                                "action": "user_typing",
                                "data": {
                                    "channel_id": channel_id,
                                    "parent_id": root_id,
                                },
                            });
                            // A failed send means the socket is going anyway;
                            // the reconnect will handle it and this keystroke is
                            // not worth an error path.
                            if let Err(error) =
                                socket.send(Message::Text(frame.to_string().into())).await
                            {
                                tracing::debug!(%error, "typing not sent");
                            }
                        }
                    }
                }

                _ = tokio::time::sleep_until(deadline) => {
                    // No frame at all inside the grace window: assume a dead peer
                    // that never sent a FIN.
                    return Ok(Outcome::Dropped { reason: "ping watchdog expired".into() });
                }

                incoming = socket.next() => {
                    let Some(message) = incoming else {
                        return Ok(Outcome::Dropped { reason: "stream ended".into() });
                    };
                    deadline = tokio::time::Instant::now() + PING_GRACE;

                    match message? {
                        Message::Text(text) => {
                            self.handle_frame(&text, &requested_resume, signals).await?;
                        }
                        // tungstenite answers pings itself; seeing one only
                        // refreshes the watchdog, which happened above.
                        Message::Ping(_) | Message::Pong(_) | Message::Binary(_) => {}
                        Message::Close(frame) => {
                            let reason = frame
                                .map(|frame| format!("close: {} {}", frame.code, frame.reason))
                                .unwrap_or_else(|| "close".to_string());
                            return Ok(Outcome::Dropped { reason });
                        }
                        Message::Frame(_) => {}
                    }
                }
            }
        }
    }

    async fn handle_frame(
        &mut self,
        text: &str,
        requested_resume: &Option<String>,
        signals: &mpsc::Sender<Signal>,
    ) -> Result<()> {
        let read_at = Instant::now();
        let envelope: Envelope = match serde_json::from_str(text) {
            Ok(envelope) => envelope,
            Err(error) => {
                tracing::warn!(%error, "undecodable websocket frame");
                return Ok(());
            }
        };

        if let Some(seq) = envelope.seq {
            self.highest_seq = self.highest_seq.max(seq);
        }

        let Some(parsed) = event::parse(&envelope)? else {
            return Ok(());
        };

        if let Event::Hello(hello) = &parsed {
            let resumed = requested_resume
                .as_ref()
                .is_some_and(|requested| requested == &hello.connection_id);
            if requested_resume.is_some() && !resumed {
                // A fresh id means the buffer could not replay for us.
                let _ = signals.send(Signal::ResyncRequired).await;
            }
            if requested_resume.is_none() {
                // First connection of the process: nothing local to trust yet.
                let _ = signals.send(Signal::ResyncRequired).await;
            }
            self.connection_id = Some(hello.connection_id.clone());
            // The server restarts its counter for a connection it did not resume.
            if !resumed {
                self.highest_seq = envelope.seq.unwrap_or(0);
            }
            let _ = signals
                .send(Signal::Connected {
                    connection_id: hello.connection_id.clone(),
                    resumed,
                })
                .await;
            return Ok(());
        }

        let seq = envelope.seq.unwrap_or(self.highest_seq);
        signals
            .send(Signal::Event {
                event: parsed,
                seq,
                read_at,
            })
            .await
            .map_err(|_| Error::Protocol("signal consumer dropped".into()))?;
        Ok(())
    }
}

enum Outcome {
    Shutdown,
    Dropped { reason: String },
}

/// Sets `SO_LINGER` to zero so closing sends an RST instead of a FIN.
fn abort_with_reset(socket: &Socket) {
    let tcp = match socket.get_ref() {
        MaybeTlsStream::Plain(stream) => Some(stream),
        MaybeTlsStream::Rustls(stream) => Some(stream.get_ref().0),
        _ => None,
    };
    if let Some(tcp) = tcp {
        let reference = socket2::SockRef::from(tcp);
        if let Err(error) = reference.set_linger(Some(Duration::ZERO)) {
            tracing::warn!(%error, "could not set SO_LINGER for the reset test");
        }
    }
}

/// Small non-cryptographic jitter source; a dependency would be overkill.
fn rand_u16() -> u16 {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.subsec_nanos())
        .unwrap_or(0);
    (nanos ^ (nanos >> 16)) as u16
}
