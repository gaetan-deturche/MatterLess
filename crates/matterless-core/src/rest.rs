use crate::auth::{AuthToken, LoginRequest};
use crate::error::{ApiError, Error, Result};
use crate::model::*;
use reqwest::{Method, RequestBuilder, Response, StatusCode};
use serde::Serialize;
use serde::de::DeserializeOwned;
use std::sync::Arc;
use std::sync::RwLock;
use std::time::Duration;
use url::Url;

use crate::rate_limit::RateLimiter;

pub const API_PREFIX: &str = "/api/v4";

/// Told how much of an upload has reached the wire: bytes sent, bytes total.
///
/// Reported against the *whole* multipart body rather than the file alone,
/// because that is what the socket is actually carrying -- the difference would
/// make a progress bar arrive at 100% before the request finished.
pub type UploadProgress = Arc<dyn Fn(u64, u64) + Send + Sync>;

/// A public channel as the browse list needs it.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct PublicChannel {
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub display_name: String,
    /// What the channel says it is for. Empty on most, and the one line worth
    /// showing beside a name that does not explain itself.
    #[serde(default)]
    pub purpose: String,
    #[serde(default, rename = "type")]
    pub channel_type: String,
    #[serde(default)]
    pub delete_at: Timestamp,
}

pub struct RestClient {
    /// Which server this client talks to.
    ///
    /// Behind a lock because it is chosen at runtime rather than compiled in:
    /// the app is pointed at a Mattermost server by whoever runs it, and a
    /// client that could only ever reach one host would have that host's name
    /// baked into every build.
    base: RwLock<Url>,
    http: reqwest::Client,
    token: RwLock<Option<AuthToken>>,
    server_version: RwLock<Option<String>>,
    /// The signed-in user, remembered for as long as the token is.
    ///
    /// `GET /users/me` is a network round trip, and twelve commands were each
    /// making one just to learn the viewer's id. Measured on this server:
    /// opening a channel cost 226-657ms, and nearly all of it was this call.
    /// The identity cannot change while a token lasts -- a session login is the
    /// only way in -- so it is cleared wherever the token is.
    identity: RwLock<Option<User>>,
    limiter: RateLimiter,
}

impl RestClient {
    pub fn new(base_url: &str) -> Result<Self> {
        let base = Url::parse(base_url)?;
        let http = reqwest::Client::builder()
            .user_agent(concat!("MatterLess/", env!("CARGO_PKG_VERSION")))
            // One pooled connection is what makes the warm path 9 ms rather than 50.
            .pool_idle_timeout(Duration::from_secs(90))
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(30))
            .build()?;
        Ok(Self {
            base: RwLock::new(base),
            http,
            token: RwLock::new(None),
            server_version: RwLock::new(None),
            identity: RwLock::new(None),
            limiter: RateLimiter::conservative(),
        })
    }

    /// Returns a clone rather than a borrow: the base is behind a lock now, and
    /// handing out a reference into it would hold that lock for the caller's
    /// lifetime.
    pub fn base_url(&self) -> Url {
        self.base.read().expect("base url").clone()
    }

    /// Points this client at a different server.
    ///
    /// Everything already fetched belongs to the old one, so the caller is
    /// expected to be starting fresh -- signing in for the first time, or
    /// switching servers deliberately.
    pub fn set_base_url(&self, base_url: &str) -> Result<()> {
        let parsed = Url::parse(base_url)?;
        *self.base.write().expect("base url") = parsed;
        *self.identity.write().expect("identity lock") = None;
        Ok(())
    }

    pub fn set_token(&self, token: AuthToken) {
        *self.token.write().expect("token lock") = Some(token);
        // A different token may be a different person.
        *self.identity.write().expect("identity lock") = None;
    }

    pub fn token(&self) -> Option<AuthToken> {
        self.token.read().expect("token lock").clone()
    }

    pub fn server_version(&self) -> Option<String> {
        self.server_version.read().expect("version lock").clone()
    }

    fn endpoint(&self, path: &str) -> Result<Url> {
        Ok(self.base_url().join(&format!("{API_PREFIX}{path}"))?)
    }

    fn builder(&self, method: Method, path: &str) -> Result<RequestBuilder> {
        let mut builder = self.http.request(method, self.endpoint(path)?);
        if let Some(token) = self.token() {
            builder = builder.bearer_auth(token.bearer());
        }
        Ok(builder)
    }

    /// Sends the request, honouring the limiter and retrying once on a 429.
    ///
    /// The copy for that retry is taken opportunistically rather than demanded.
    /// A streamed body cannot be replayed, so `try_clone` returns nothing for an
    /// upload -- and insisting on a copy up front meant every attachment failed
    /// before a byte of it was sent.
    async fn send(&self, builder: RequestBuilder) -> Result<Response> {
        let mut pending = Some(builder);
        for attempt in 0..2 {
            self.limiter.acquire().await;
            let attempt_builder = pending.take().expect("a builder for each attempt");
            let copy = attempt_builder.try_clone();
            let response = attempt_builder.send().await?;

            if let Some(version) = response
                .headers()
                .get("x-version-id")
                .and_then(|value| value.to_str().ok())
            {
                *self.server_version.write().expect("version lock") = Some(version.to_string());
            }

            if response.status() == StatusCode::TOO_MANY_REQUESTS && attempt == 0 {
                // Nothing to send a second time: hand the 429 back and let the
                // caller see the server's own answer.
                let Some(copy) = copy else {
                    return Ok(response);
                };
                let retry_after = response
                    .headers()
                    .get("retry-after")
                    .and_then(|value| value.to_str().ok())
                    .and_then(|value| value.parse::<u64>().ok())
                    .unwrap_or(1)
                    .min(30);
                tracing::warn!(retry_after, "rate limited, backing off");
                tokio::time::sleep(Duration::from_secs(retry_after)).await;
                pending = Some(copy);
                continue;
            }
            return Ok(response);
        }
        Err(Error::RateLimited {
            retry_after_secs: 1,
        })
    }

    async fn decode<T: DeserializeOwned>(response: Response) -> Result<T> {
        let status = response.status();
        let body = response.bytes().await?;
        if status.is_success() {
            return Ok(serde_json::from_slice(&body)?);
        }
        // A non-2xx always carries the error envelope; fall back to the raw text
        // if a proxy replaced it with something else.
        match serde_json::from_slice::<ApiError>(&body) {
            Ok(envelope) => Err(Error::api(envelope, status.as_u16())),
            Err(_) => Err(Error::Protocol(format!(
                "{status}: {}",
                String::from_utf8_lossy(&body)
                    .chars()
                    .take(200)
                    .collect::<String>()
            ))),
        }
    }

    async fn get<T: DeserializeOwned>(&self, path: &str) -> Result<T> {
        let response = self.send(self.builder(Method::GET, path)?).await?;
        Self::decode(response).await
    }

    async fn post_json<B: Serialize, T: DeserializeOwned>(
        &self,
        path: &str,
        body: &B,
    ) -> Result<T> {
        let response = self
            .send(self.builder(Method::POST, path)?.json(body))
            .await?;
        Self::decode(response).await
    }

    // ---------------------------------------------------------------- system

    pub async fn ping(&self) -> Result<serde_json::Value> {
        self.get("/system/ping").await
    }

    /// Works unauthenticated, returning a reduced set; authenticated it returns
    /// 258 keys on this server including `CollapsedThreads`.
    pub async fn client_config(&self) -> Result<std::collections::HashMap<String, String>> {
        self.get("/config/client?format=old").await
    }

    // ------------------------------------------------------------------ auth

    /// Logs in and stores the session token from the `Token` response header.
    ///
    /// Returns `Error::MfaRequired` when the account needs a code; call again
    /// with `mfa_token` set.
    pub async fn login(
        &self,
        login_id: &str,
        password: &str,
        mfa_token: Option<&str>,
    ) -> Result<User> {
        let request = LoginRequest {
            login_id,
            password,
            token: mfa_token,
        };
        let response = self
            .send(self.builder(Method::POST, "/users/login")?.json(&request))
            .await?;

        let status = response.status();
        let header_token = response
            .headers()
            .get("token")
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        let body = response.bytes().await?;

        if !status.is_success() {
            let envelope: ApiError = serde_json::from_slice(&body).unwrap_or_else(|_| ApiError {
                id: String::new(),
                message: String::from_utf8_lossy(&body).into_owned(),
                detailed_error: String::new(),
                request_id: String::new(),
                status_code: status.as_u16(),
            });
            return Err(Error::api(envelope, status.as_u16()));
        }

        let token = header_token.ok_or(Error::MissingTokenHeader)?;
        self.set_token(AuthToken::Session(token));
        Ok(serde_json::from_slice(&body)?)
    }

    pub async fn logout(&self) -> Result<()> {
        let response = self
            .send(self.builder(Method::POST, "/users/logout")?)
            .await?;
        *self.token.write().expect("token lock") = None;
        *self.identity.write().expect("identity lock") = None;
        if response.status().is_success() {
            Ok(())
        } else {
            Err(Error::Protocol(format!("logout: {}", response.status())))
        }
    }

    // ----------------------------------------------------------------- users

    /// The signed-in user, fetched once per token.
    ///
    /// A concurrent first call can fetch twice; both answers are the same user,
    /// and the second write is harmless. Worth less than the lock it would take
    /// to prevent.
    pub async fn me(&self) -> Result<User> {
        if let Some(user) = self.identity.read().expect("identity lock").clone() {
            return Ok(user);
        }
        let user: User = self.get("/users/me").await?;
        *self.identity.write().expect("identity lock") = Some(user.clone());
        Ok(user)
    }

    /// The signed-in user, asked of the server rather than remembered.
    ///
    /// For the one caller whose question is "is this token still good?" -- a
    /// cached answer would report a signed-in session long after the server had
    /// ended it. Refreshes the cache on the way through.
    pub async fn verify_me(&self) -> Result<User> {
        let user: User = self.get("/users/me").await?;
        *self.identity.write().expect("identity lock") = Some(user.clone());
        Ok(user)
    }

    /// Forgets the cached identity, for a caller that has reason to believe the
    /// profile changed under it.
    pub fn forget_identity(&self) {
        *self.identity.write().expect("identity lock") = None;
    }

    /// Batch lookup. One request per author would be the fastest way to hit the
    /// rate limit on a busy channel.
    /// Asks the server to remind this reader about a post.
    ///
    /// `target_time` is epoch *seconds*, and the caller computes it: "tomorrow
    /// morning" is a question about the reader's clock and timezone, which the
    /// shell knows and this does not.
    ///
    /// The server posts the reminder itself, from the system bot, so nothing
    /// here has to be stored or scheduled locally.
    pub async fn set_reminder(&self, me_id: &str, post_id: &str, target_time: i64) -> Result<()> {
        #[derive(Serialize)]
        struct Reminder {
            target_time: i64,
        }
        let path = format!("/users/{me_id}/posts/{post_id}/reminder");
        let _: serde_json::Value = self.post_json(&path, &Reminder { target_time }).await?;
        Ok(())
    }

    pub async fn users_by_ids(&self, ids: &[String]) -> Result<Vec<User>> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        self.post_json("/users/ids", &ids).await
    }

    pub async fn preferences(&self, user_id: &str) -> Result<Vec<Preference>> {
        self.get(&format!("/users/{user_id}/preferences")).await
    }

    pub async fn statuses_by_ids(&self, ids: &[String]) -> Result<Vec<Status>> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        self.post_json("/users/status/ids", &ids).await
    }

    /// This reader's own presence, which is what the do-not-disturb rule is
    /// decided against.
    pub async fn my_status(&self, user_id: &str) -> Result<Status> {
        self.get(&format!("/users/{user_id}/status")).await
    }

    /// Sets this reader's presence. `manual` is implied by asking.
    pub async fn set_my_status(&self, user_id: &str, status: &str) -> Result<Status> {
        let response = self
            .send(
                self.builder(Method::PUT, &format!("/users/{user_id}/status"))?
                    .json(&serde_json::json!({ "user_id": user_id, "status": status })),
            )
            .await?;
        Self::decode(response).await
    }

    // ------------------------------------------------------ teams & channels

    pub async fn my_teams(&self) -> Result<Vec<Team>> {
        self.get("/users/me/teams").await
    }

    pub async fn my_channels(&self, team_id: &str) -> Result<Vec<Channel>> {
        self.get(&format!("/users/me/teams/{team_id}/channels"))
            .await
    }

    pub async fn my_channel_members(&self, team_id: &str) -> Result<Vec<ChannelMember>> {
        self.get(&format!("/users/me/teams/{team_id}/channels/members"))
            .await
    }

    pub async fn view_channel(&self, user_id: &str, channel_id: &str) -> Result<serde_json::Value> {
        self.post_json(
            &format!("/channels/members/{user_id}/view"),
            &serde_json::json!({ "channel_id": channel_id }),
        )
        .await
    }

    /// Fetches a binary resource, with its content type.
    ///
    /// Images live behind the session token, so the webview cannot fetch them
    /// itself -- this is what the custom URI scheme resolves them through.
    /// `None` for a 404, which is an ordinary answer here: a user with no
    /// avatar and a team with no icon are both normal.
    pub async fn fetch_bytes(&self, path: &str) -> Result<Option<(Vec<u8>, String)>> {
        let response = self.send(self.builder(Method::GET, path)?).await?;
        if response.status() == StatusCode::NOT_FOUND {
            return Ok(None);
        }
        if !response.status().is_success() {
            return Err(Error::Protocol(format!(
                "fetch {path}: {}",
                response.status()
            )));
        }
        let content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or("application/octet-stream")
            .to_string();
        Ok(Some((response.bytes().await?.to_vec(), content_type)))
    }

    /// One page of the server's custom emoji.
    ///
    /// The whole list is available, which is worth more than asking name by
    /// name: scanning messages for `:names:` can only ever find the ones
    /// someone has already used, so a custom emoji nobody typed yet would
    /// render as its name the first time it appeared.
    pub async fn custom_emoji_page(&self, page: u32, per_page: u32) -> Result<Vec<CustomEmoji>> {
        self.get(&format!("/emoji?page={page}&per_page={per_page}"))
            .await
    }

    /// A custom emoji by name, or `None` when the name is not one.
    ///
    /// A 404 is the *answer* here rather than a failure: the endpoint serves
    /// only custom emoji, so "not found" means "this is a standard one", and
    /// that result is worth caching so the question is asked once.
    pub async fn emoji_by_name(&self, name: &str) -> Result<Option<CustomEmoji>> {
        let response = self
            .send(self.builder(Method::GET, &format!("/emoji/name/{name}"))?)
            .await?;
        if response.status() == StatusCode::NOT_FOUND {
            return Ok(None);
        }
        if !response.status().is_success() {
            return Err(Error::Protocol(format!(
                "emoji {name}: {}",
                response.status()
            )));
        }
        Ok(Some(response.json().await?))
    }

    // ------------------------------------------------------------- reactions

    /// Adds a reaction. The server echoes it back over the websocket.
    pub async fn add_reaction(
        &self,
        user_id: &str,
        post_id: &str,
        emoji_name: &str,
    ) -> Result<Reaction> {
        self.post_json(
            "/reactions",
            &serde_json::json!({
                "user_id": user_id,
                "post_id": post_id,
                "emoji_name": emoji_name,
            }),
        )
        .await
    }

    /// Removes one. The path carries all three parts; there is no body.
    pub async fn remove_reaction(
        &self,
        user_id: &str,
        post_id: &str,
        emoji_name: &str,
    ) -> Result<()> {
        let path = format!("/users/{user_id}/posts/{post_id}/reactions/{emoji_name}");
        let response = self.send(self.builder(Method::DELETE, &path)?).await?;
        if response.status().is_success() {
            Ok(())
        } else {
            Err(Error::Protocol(format!(
                "remove reaction: {}",
                response.status()
            )))
        }
    }

    /// How this user has organised a team's sidebar.
    ///
    /// **The direct-messages category is returned for every team**, holding the
    /// same conversations each time -- the same duplication as the channel list,
    /// so DM categories have to be merged rather than shown per team.
    pub async fn sidebar_categories(
        &self,
        user_id: &str,
        team_id: &str,
    ) -> Result<SidebarCategories> {
        self.get(&format!(
            "/users/{user_id}/teams/{team_id}/channels/categories"
        ))
        .await
    }

    // --------------------------------------------------------------- threads

    /// The threads this user follows in a team, newest activity first.
    ///
    /// **Dedupe by thread id across teams.** A thread in a DM or group channel
    /// is returned for *every* team, exactly as channels are -- so summing the
    /// per-team lists double-counts it, the same trap that turned 114 channels
    /// into 172. Confirmed live: one thread came back identical under both
    /// the teams the reader belongs to.
    ///
    /// `extended=false` keeps the payload small; the participants then arrive as
    /// user objects with only `id` filled, and names come from the local users
    /// table as everywhere else.
    pub async fn my_threads(
        &self,
        user_id: &str,
        team_id: &str,
        per_page: u32,
        unread_only: bool,
    ) -> Result<UserThreads> {
        self.get(&format!(
            "/users/{user_id}/teams/{team_id}/threads\
             ?per_page={per_page}&extended=false&unread={unread_only}"
        ))
        .await
    }

    /// Follows or unfollows a thread. The verb is the whole request.
    pub async fn follow_thread(
        &self,
        user_id: &str,
        team_id: &str,
        thread_id: &str,
        following: bool,
    ) -> Result<()> {
        let method = if following {
            Method::PUT
        } else {
            Method::DELETE
        };
        let path = format!("/users/{user_id}/teams/{team_id}/threads/{thread_id}/following");
        let response = self.send(self.builder(method, &path)?).await?;
        if response.status().is_success() {
            Ok(())
        } else {
            Err(Error::Protocol(format!(
                "follow thread: {}",
                response.status()
            )))
        }
    }

    /// Marks a thread read up to `timestamp`.
    ///
    /// The timestamp rides in the path rather than a body, and must be a
    /// *server* clock value: it is compared against post `create_at` later, the
    /// same rule that made channel read state work.
    pub async fn mark_thread_read(
        &self,
        user_id: &str,
        team_id: &str,
        thread_id: &str,
        timestamp: Timestamp,
    ) -> Result<()> {
        let path = format!("/users/{user_id}/teams/{team_id}/threads/{thread_id}/read/{timestamp}");
        let response = self.send(self.builder(Method::PUT, &path)?).await?;
        if response.status().is_success() {
            Ok(())
        } else {
            Err(Error::Protocol(format!(
                "mark thread read: {}",
                response.status()
            )))
        }
    }

    /// Searches a team's posts.
    ///
    /// Server-side by necessity: the local index only holds what has been
    /// fetched. `is_or_search` false means every term must appear, which is
    /// what a reader typing two words expects.
    ///
    /// Measured on this server: 100 hits per team per call, `matches` comes
    /// back null (no Elasticsearch, so no server-side highlight spans), and the
    /// `in:` / `from:` / `before:` / `after:` modifiers work inside `terms`.
    pub async fn search_posts(&self, team_id: &str, terms: &str) -> Result<PostList> {
        self.post_json(
            &format!("/teams/{team_id}/posts/search"),
            &serde_json::json!({ "terms": terms, "is_or_search": false }),
        )
        .await
    }

    /// One person by username, for a mention of somebody this client has never
    /// seen post.
    pub async fn user_by_username(&self, username: &str) -> Result<User> {
        self.get(&format!("/users/username/{username}")).await
    }

    /// People the server suggests for a partial name.
    ///
    /// Takes the whole route because the useful parameters differ per call
    /// site: in a channel, in a team, or across the server.
    pub async fn autocomplete_users(&self, route: &str) -> Result<Vec<User>> {
        // The response is an object with `users` (and `out_of_channel` when a
        // channel was named), not a bare array.
        #[derive(serde::Deserialize)]
        struct Suggestions {
            #[serde(default)]
            users: Vec<User>,
        }
        let found: Suggestions = self.get(route).await?;
        Ok(found.users)
    }

    /// Finds or creates the direct message channel between two people.
    pub async fn direct_channel(&self, me_id: &str, user_id: &str) -> Result<Channel> {
        self.post_json("/channels/direct", &[me_id, user_id]).await
    }

    /// Finds or creates the group conversation between these people.
    ///
    /// The list includes the reader: the server keys a group channel on its
    /// whole membership, and leaving oneself out asks for a different one.
    pub async fn group_channel(&self, user_ids: &[String]) -> Result<Channel> {
        self.post_json("/channels/group", &user_ids).await
    }

    /// The public channels of a team, joined or not.
    ///
    /// Paged: this is the browse list, and a large team has more of them than
    /// one response carries.
    ///
    /// Its own shape rather than `Channel`, because a browse row wants the
    /// channel's *purpose* and nothing else here does -- putting that field on
    /// the shared model for one screen's sake would have meant every fixture
    /// that builds a channel growing a line about it.
    pub async fn public_channels(
        &self,
        team_id: &str,
        page: u32,
        per_page: u32,
    ) -> Result<Vec<PublicChannel>> {
        self.get(&format!(
            "/teams/{team_id}/channels?page={page}&per_page={per_page}"
        ))
        .await
    }

    /// Joins a channel, by adding this reader to its members.
    pub async fn join_channel(&self, channel_id: &str, user_id: &str) -> Result<serde_json::Value> {
        #[derive(Serialize)]
        struct Joining<'a> {
            user_id: &'a str,
        }
        self.post_json(
            &format!("/channels/{channel_id}/members"),
            &Joining { user_id },
        )
        .await
    }

    /// Mutes or unmutes a channel for this reader.
    ///
    /// There is no "muted" field to set: a muted channel is one whose membership
    /// only counts unread on a mention, which is exactly what `is_muted` reads
    /// back off the other side.
    pub async fn set_channel_muted(
        &self,
        channel_id: &str,
        user_id: &str,
        muted: bool,
    ) -> Result<()> {
        let path = format!("/channels/{channel_id}/members/{user_id}/notify_props");
        let body = serde_json::json!({
            "mark_unread": if muted { "mention" } else { "all" },
        });
        let response = self
            .send(self.builder(Method::PUT, &path)?.json(&body))
            .await?;
        if response.status().is_success() {
            Ok(())
        } else {
            Err(Error::Protocol(format!(
                "muting the channel: {}",
                response.status()
            )))
        }
    }

    /// Rewrites a team's sidebar categories.
    ///
    /// The server replaces each category wholesale, `channel_ids` and all, so
    /// moving one channel means sending two categories: the one losing it and
    /// the one gaining it.
    pub async fn update_sidebar_categories(
        &self,
        user_id: &str,
        team_id: &str,
        categories: &[SidebarCategory],
    ) -> Result<()> {
        let path = format!("/users/{user_id}/teams/{team_id}/channels/categories");
        let response = self
            .send(self.builder(Method::PUT, &path)?.json(categories))
            .await?;
        if response.status().is_success() {
            Ok(())
        } else {
            Err(Error::Protocol(format!(
                "moving the channel: {}",
                response.status()
            )))
        }
    }

    /// Leaves a channel.
    ///
    /// The server refuses for a direct or group message -- those are left by
    /// hiding them, which is a preference rather than a membership change.
    pub async fn leave_channel(&self, channel_id: &str, user_id: &str) -> Result<()> {
        let response = self
            .send(self.builder(
                Method::DELETE,
                &format!("/channels/{channel_id}/members/{user_id}"),
            )?)
            .await?;
        if response.status().is_success() {
            Ok(())
        } else {
            Err(Error::Protocol(format!(
                "leaving the channel: {}",
                response.status()
            )))
        }
    }

    // ------------------------------------------------------- message actions

    /// Pins or unpins a post. Channel-wide: everyone sees the result.
    pub async fn set_post_pinned(&self, post_id: &str, pinned: bool) -> Result<()> {
        let action = if pinned { "pin" } else { "unpin" };
        let response = self
            .send(self.builder(Method::POST, &format!("/posts/{post_id}/{action}"))?)
            .await?;
        if response.status().is_success() {
            Ok(())
        } else {
            Err(Error::Protocol(format!("{action}: {}", response.status())))
        }
    }

    /// Saves or unsaves a post for this reader.
    ///
    /// A preference rather than a post field, which is also why unsaving is a
    /// preference *delete* and not a value of `false`: captured against the live
    /// server before it was written.
    pub async fn set_post_saved(&self, user_id: &str, post_id: &str, saved: bool) -> Result<()> {
        let body = vec![Preference {
            user_id: user_id.to_string(),
            category: "flagged_post".to_string(),
            name: post_id.to_string(),
            value: "true".to_string(),
        }];
        let (method, path) = if saved {
            (Method::PUT, format!("/users/{user_id}/preferences"))
        } else {
            (Method::POST, format!("/users/{user_id}/preferences/delete"))
        };
        let response = self.send(self.builder(method, &path)?.json(&body)).await?;
        if response.status().is_success() {
            Ok(())
        } else {
            Err(Error::Protocol(format!("save post: {}", response.status())))
        }
    }

    /// Marks the channel unread from this post down, and answers with the
    /// membership the server ended up with -- which is what the sidebar counts
    /// and the badge are drawn from.
    pub async fn set_post_unread(&self, user_id: &str, post_id: &str) -> Result<ChannelMember> {
        self.post_json(
            &format!("/users/{user_id}/posts/{post_id}/set_unread"),
            &serde_json::json!({}),
        )
        .await
    }

    // ----------------------------------------------------------------- files

    /// Uploads one file into a channel, ahead of the post that will carry it.
    ///
    /// Two steps because that is how Mattermost works: the bytes go up first
    /// and come back with an id, then the post is created with that id in
    /// `file_ids`. An upload with no post attached is simply orphaned, which is
    /// why a failed send loses nothing but disk on the server.
    ///
    /// The multipart body is assembled here rather than through a dependency:
    /// it is two parts, and `reqwest`'s multipart feature would pull a
    /// streaming body that `send()` cannot retry after a 429.
    pub async fn upload_file(
        &self,
        channel_id: &str,
        filename: &str,
        bytes: &[u8],
        progress: Option<UploadProgress>,
    ) -> Result<FileUploadResponse> {
        let name = sanitise_filename(filename);
        let boundary = multipart_boundary(bytes);
        // Written out with an explicit CRLF: multipart is a wire format, and a
        // bare newline is not a line ending in it.
        const CRLF: &str = "\r\n";
        let header = format!(
            "--{boundary}{CRLF}\
             Content-Disposition: form-data; name=\"channel_id\"{CRLF}{CRLF}\
             {channel_id}{CRLF}\
             --{boundary}{CRLF}\
             Content-Disposition: form-data; name=\"files\"; filename=\"{name}\"{CRLF}\
             Content-Type: application/octet-stream{CRLF}{CRLF}"
        );
        let trailer = format!("{CRLF}--{boundary}--{CRLF}");

        let mut body: Vec<u8> = Vec::with_capacity(bytes.len() + header.len() + trailer.len());
        body.extend_from_slice(header.as_bytes());
        body.extend_from_slice(bytes);
        body.extend_from_slice(trailer.as_bytes());

        // Streamed rather than handed over whole, so the caller can be told how
        // far it has got. There is no resumable upload in Mattermost -- a large
        // file is one long request -- which is exactly why a reader needs to see
        // it moving and needs to be able to stop it.
        const CHUNK: usize = 64 * 1024;
        let total = body.len() as u64;
        let body = bytes::Bytes::from(body);
        let report = progress.clone();
        let chunks = futures_util::stream::unfold((body, 0usize), move |(body, at)| {
            let report = report.clone();
            async move {
                if at >= body.len() {
                    return None;
                }
                let end = (at + CHUNK).min(body.len());
                // Zero-copy: a slice of the same allocation, not a second one.
                let chunk = body.slice(at..end);
                if let Some(report) = &report {
                    report(end as u64, total);
                }
                Some((Ok::<_, std::io::Error>(chunk), (body, end)))
            }
        });

        let request = self
            .builder(Method::POST, "/files")?
            .header(
                reqwest::header::CONTENT_TYPE,
                format!("multipart/form-data; boundary={boundary}"),
            )
            // Set explicitly: a streamed body has no length of its own, and
            // without this reqwest sends it chunked, which this server's nginx
            // is not guaranteed to accept for an upload.
            .header(reqwest::header::CONTENT_LENGTH, total)
            .body(reqwest::Body::wrap_stream(chunks));
        let response = self.send(request).await?;
        Self::decode(response).await
    }

    // ----------------------------------------------------------------- posts

    /// Newest page. Measured at ~53 ms median for `per_page = 60`.
    /// The posts this reader has saved, newest first.
    ///
    /// Saving is a *preference* on the server, so the local store already knows
    /// which ids are saved; this is how their content is fetched when the
    /// reader has never had those channels open.
    pub async fn flagged_posts(&self, me_id: &str, per_page: u32) -> Result<PostList> {
        let path = format!("/users/{me_id}/posts/flagged?per_page={per_page}");
        self.get(&path).await
    }

    /// The posts pinned in a channel.
    pub async fn pinned_posts(&self, channel_id: &str) -> Result<PostList> {
        let path = format!("/channels/{channel_id}/pinned");
        self.get(&path).await
    }

    pub async fn posts(&self, channel_id: &str, per_page: u32) -> Result<PostList> {
        self.get(&format!("/channels/{channel_id}/posts?per_page={per_page}"))
            .await
    }

    /// Older page, for scroll-up backfill.
    pub async fn posts_before(
        &self,
        channel_id: &str,
        before_post_id: &str,
        per_page: u32,
    ) -> Result<PostList> {
        self.get(&format!(
            "/channels/{channel_id}/posts?before={before_post_id}&per_page={per_page}"
        ))
        .await
    }

    /// The posts *after* one, oldest first in `order`.
    ///
    /// The other half of `posts_before`, and the reason a jump to an old
    /// message can show context on both sides of it rather than only above.
    pub async fn posts_after(
        &self,
        channel_id: &str,
        after_post_id: &str,
        per_page: u32,
    ) -> Result<PostList> {
        self.get(&format!(
            "/channels/{channel_id}/posts?after={after_post_id}&per_page={per_page}"
        ))
        .await
    }

    /// One post, by id. Used when jumping to a permalink for a message this
    /// client has never held.
    pub async fn post(&self, post_id: &str) -> Result<Post> {
        self.get(&format!("/posts/{post_id}")).await
    }

    /// Catch-up after a reconnect. `since` is a server millisecond timestamp.
    pub async fn posts_since(&self, channel_id: &str, since: Timestamp) -> Result<PostList> {
        self.get(&format!("/channels/{channel_id}/posts?since={since}"))
            .await
    }

    pub async fn thread(&self, root_post_id: &str) -> Result<PostList> {
        self.get(&format!("/posts/{root_post_id}/thread")).await
    }

    pub async fn create_post(&self, post: &NewPost<'_>) -> Result<Post> {
        self.post_json("/posts", post).await
    }

    /// Edits a post's text, answering with the post the server ended up with.
    ///
    /// A *patch*, not a put: sending the whole post would risk clobbering
    /// fields this client does not model. Measured against the live server --
    /// the reply carries a fresh `update_at`, which is the markdown cache's key,
    /// so an edit invalidates that cache by construction rather than by anyone
    /// remembering to.
    pub async fn patch_post(&self, post_id: &str, message: &str) -> Result<Post> {
        let response = self
            .send(
                self.builder(Method::PUT, &format!("/posts/{post_id}/patch"))?
                    .json(&serde_json::json!({ "message": message })),
            )
            .await?;
        Self::decode(response).await
    }

    /// Deletes a post. Afterwards the server 404s it, so the local copy is a
    /// tombstone rather than a cache of something fetchable.
    pub async fn delete_post(&self, post_id: &str) -> Result<()> {
        let response = self
            .send(self.builder(Method::DELETE, &format!("/posts/{post_id}"))?)
            .await?;
        if response.status().is_success() {
            Ok(())
        } else {
            Err(Error::Protocol(format!(
                "delete post: {}",
                response.status()
            )))
        }
    }
}

/// A filename the multipart header can carry verbatim.
///
/// Quotes and backslashes would close or escape the header parameter, and a
/// newline would inject a header outright -- the name comes from a file the
/// user dropped, so it is not trusted to be well behaved. UTF-8 is left alone:
/// Go's multipart reader takes it as-is, which is how the official client sends
/// accented names too.
fn sanitise_filename(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|character| match character {
            '"' | '\\' => '_',
            control if control.is_control() => '_',
            other => other,
        })
        .collect();
    let trimmed = cleaned.trim();
    if trimmed.is_empty() {
        "attachment".to_string()
    } else {
        trimmed.to_string()
    }
}

/// A boundary that does not occur in the payload.
///
/// Checked rather than assumed: a boundary that appears inside the bytes would
/// truncate the upload, and the bytes here are arbitrary -- a zip or a video can
/// contain anything. The counter is only there to make the retry terminate.
fn multipart_boundary(payload: &[u8]) -> String {
    for attempt in 0..64u32 {
        let candidate = format!(
            "----matterless{:x}{:x}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|since| since.as_nanos())
                .unwrap_or(0),
            attempt
        );
        if !contains(payload, candidate.as_bytes()) {
            return candidate;
        }
    }
    // 64 timestamped candidates all appearing in one payload is not a thing
    // that happens; a length no candidate can match ends it regardless.
    format!("----matterless{}", "z".repeat(70))
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    if needle.is_empty() || haystack.len() < needle.len() {
        return false;
    }
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

#[cfg(test)]
mod file_tests {
    use super::*;

    #[test]
    fn filenames_cannot_break_out_of_the_header() {
        assert_eq!(sanitise_filename("holiday.png"), "holiday.png");
        assert_eq!(sanitise_filename("rap\"port.pdf"), "rap_port.pdf");
        assert_eq!(
            sanitise_filename("a\r\nContent-Type: text/html"),
            "a__Content-Type: text/html"
        );
        // Accents survive: the server reads the parameter as UTF-8.
        assert_eq!(sanitise_filename("répétition.txt"), "répétition.txt");
        assert_eq!(sanitise_filename("   "), "attachment");
    }

    #[test]
    fn the_boundary_avoids_the_payload() {
        let boundary = multipart_boundary(b"nothing to see");
        assert!(!contains(b"nothing to see", boundary.as_bytes()));
        // A payload containing an earlier candidate still gets a fresh one.
        let hostile = format!("padding{boundary}padding").into_bytes();
        let second = multipart_boundary(&hostile);
        assert!(!contains(&hostile, second.as_bytes()));
    }
}

#[cfg(test)]
mod send_tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    /// True once the whole declared body has arrived, so the fake server does
    /// not answer half an upload.
    fn body_complete(seen: &[u8]) -> bool {
        let Some(headers_end) = seen.windows(4).position(|window| window == b"\r\n\r\n") else {
            return false;
        };
        // The header block is ASCII; the body after it may be anything.
        let headers = String::from_utf8_lossy(&seen[..headers_end]);
        let declared = headers
            .lines()
            .find_map(|line| {
                let (name, value) = line.split_once(':')?;
                if name.eq_ignore_ascii_case("content-length") {
                    value.trim().parse::<usize>().ok()
                } else {
                    None
                }
            })
            .unwrap_or(0);
        seen.len() >= headers_end + 4 + declared
    }

    /// Answers exactly one request with an empty JSON object, and hands back
    /// every byte it read. Enough to settle the only question here: whether the
    /// request reached a socket at all.
    async fn one_shot_server() -> (String, tokio::task::JoinHandle<Vec<u8>>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let address = listener.local_addr().expect("address");
        let served = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.expect("accept");
            let mut seen: Vec<u8> = Vec::new();
            let mut chunk = [0u8; 4096];
            loop {
                let read = socket.read(&mut chunk).await.expect("read");
                if read == 0 {
                    break;
                }
                seen.extend_from_slice(&chunk[..read]);
                if body_complete(&seen) {
                    break;
                }
            }
            let answer = concat!(
                "HTTP/1.1 200 OK\r\n",
                "Content-Type: application/json\r\n",
                "Content-Length: 2\r\n\r\n",
                "{}"
            );
            socket.write_all(answer.as_bytes()).await.expect("write");
            socket.flush().await.expect("flush");
            seen
        });
        (format!("http://{address}"), served)
    }

    /// The regression this guards: `send` used to demand a clone of the request
    /// so it could replay a 429, and a streamed body cannot be cloned -- so
    /// every attachment failed before a byte was sent, with the bare message
    /// "request body is not retryable".
    #[tokio::test]
    async fn a_streamed_upload_reaches_the_server() {
        let (base, served) = one_shot_server().await;
        let client = RestClient::new(&base).expect("client");

        let reached = Arc::new(AtomicU64::new(0));
        let whole = Arc::new(AtomicU64::new(0));
        let progress: UploadProgress = {
            let reached = Arc::clone(&reached);
            let whole = Arc::clone(&whole);
            Arc::new(move |so_far, total| {
                reached.store(so_far, Ordering::SeqCst);
                whole.store(total, Ordering::SeqCst);
            })
        };

        let payload = b"the file's bytes";
        let response = client
            .upload_file("channel-id", "holiday.png", payload, Some(progress))
            .await
            .expect("a streamed upload must not be refused before it is sent");
        assert!(response.file_infos.is_empty());

        let request = served.await.expect("server");
        assert!(contains(&request, payload));
        assert!(contains(&request, b"name=\"channel_id\""));
        assert!(contains(&request, b"filename=\"holiday.png\""));
        // Progress ran to the end, and that end is the whole multipart body
        // rather than just the payload.
        assert_eq!(reached.load(Ordering::SeqCst), whole.load(Ordering::SeqCst));
        assert!(whole.load(Ordering::SeqCst) > payload.len() as u64);
    }
}
