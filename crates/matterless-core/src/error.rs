use serde::Deserialize;

/// Mattermost's error envelope, returned as the body of any non-2xx response.
#[derive(Debug, Clone, Deserialize)]
pub struct ApiError {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub message: String,
    #[serde(default)]
    pub detailed_error: String,
    #[serde(default)]
    pub request_id: String,
    #[serde(default)]
    pub status_code: u16,
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("transport: {0}")]
    Transport(#[from] reqwest::Error),

    #[error("websocket: {0}")]
    WebSocket(#[from] tokio_tungstenite::tungstenite::Error),

    #[error("decode: {0}")]
    Decode(#[from] serde_json::Error),

    #[error("invalid url: {0}")]
    Url(#[from] url::ParseError),

    /// The account has MFA enabled; retry the login with a `token`.
    #[error("multi-factor authentication required")]
    MfaRequired,

    /// Phase 0 established there is no refresh path on this server (OAuth2 and
    /// personal access tokens both disabled), so this is terminal: the app must
    /// prompt rather than retry.
    #[error("session expired or invalid; re-authentication required")]
    SessionExpired,

    #[error("rate limited; retry after {retry_after_secs}s")]
    RateLimited { retry_after_secs: u64 },

    #[error("api {} [{}]: {}", .1, .0.id, .0.message)]
    Api(Box<ApiError>, u16),

    #[error("login succeeded but no Token response header was present")]
    MissingTokenHeader,

    #[error("not authenticated")]
    NotAuthenticated,

    #[error("{0}")]
    Protocol(String),
}

impl Error {
    pub fn api(envelope: ApiError, status: u16) -> Self {
        if status == 401 {
            let id = envelope.id.to_ascii_lowercase();
            if id.contains("mfa") {
                return Error::MfaRequired;
            }
            return Error::SessionExpired;
        }
        // Some builds answer an MFA challenge with 400 rather than 401, which is
        // why this check is not nested under the 401 branch above.
        if envelope.id.to_ascii_lowercase().contains("mfa") {
            return Error::MfaRequired;
        }
        Error::Api(Box::new(envelope), status)
    }
}

pub type Result<T> = std::result::Result<T, Error>;
