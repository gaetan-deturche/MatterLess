use serde::Serialize;
use std::fmt;

/// Every flow collapses to one bearer header, so the transport never knows which
/// one produced the token.
///
/// Phase 0 found only `Session` available on the server this was built against
/// (`EnableOAuthServiceProvider` and `EnableUserAccessTokens` are both false),
/// but the enum keeps the others reachable for other servers.
#[derive(Clone)]
pub enum AuthToken {
    Session(String),
    PersonalAccessToken(String),
    OAuth2 {
        access_token: String,
        refresh_token: Option<String>,
    },
}

impl AuthToken {
    pub fn bearer(&self) -> &str {
        match self {
            AuthToken::Session(token) | AuthToken::PersonalAccessToken(token) => token,
            AuthToken::OAuth2 { access_token, .. } => access_token,
        }
    }

    /// A session token cannot be renewed without user interaction here; there is
    /// no refresh path to try first.
    pub fn can_refresh_silently(&self) -> bool {
        matches!(
            self,
            AuthToken::OAuth2 {
                refresh_token: Some(_),
                ..
            }
        )
    }

    pub fn kind(&self) -> &'static str {
        match self {
            AuthToken::Session(_) => "session",
            AuthToken::PersonalAccessToken(_) => "pat",
            AuthToken::OAuth2 { .. } => "oauth2",
        }
    }
}

/// Redacted so a token can never reach a log line or a panic message.
impl fmt::Debug for AuthToken {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "AuthToken::{}(<redacted>)", self.kind())
    }
}

#[derive(Serialize)]
pub(crate) struct LoginRequest<'a> {
    pub login_id: &'a str,
    pub password: &'a str,
    /// The MFA code, sent only on the retry after an `MfaRequired`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token: Option<&'a str>,
}

/// What the caller must do when authentication fails mid-session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReauthAction {
    /// A refresh token exists; renew without bothering the user.
    RefreshSilently,
    /// No refresh path: the UI has to prompt. This is the case on this server.
    PromptUser,
}

pub fn reauth_action(token: Option<&AuthToken>) -> ReauthAction {
    match token {
        Some(token) if token.can_refresh_silently() => ReauthAction::RefreshSilently,
        _ => ReauthAction::PromptUser,
    }
}
