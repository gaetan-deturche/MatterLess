//! Whether a post should raise a Windows toast, and why.
//!
//! Firing the toast is one call; deciding is the feature. Mattermost keeps the
//! policy server-side in two places -- the user's global `notify_props` and a
//! per-channel `channel_members.notify_props` that overrides it -- and a client
//! that ignores either is worse than no client at all.
//!
//! Mention detection is the part most often got wrong, so it is tested against
//! the awkward cases rather than the easy one.

use matterless_core::model::{Post, ThreadMode, User};
use matterless_core::text::is_username_char;
use std::collections::HashMap;

/// Why a mention counted, in ascending order of how much it should interrupt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MentionVerdict {
    None,
    /// A custom keyword or the user's first name.
    Keyword,
    /// `@all`, `@here` or `@channel`.
    ChannelWide,
    /// `@username`.
    Direct,
}

impl MentionVerdict {
    pub fn is_mention(self) -> bool {
        self != MentionVerdict::None
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotifyReason {
    OwnPost,
    SystemMessage,
    DoNotDisturb,
    AlreadyLooking,
    ChannelMuted,
    LevelNone,
    NoMentionAtMentionLevel,
    UnfollowedThread,
    Mentioned,
    AllMessages,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NotifyDecision {
    pub notify: bool,
    pub reason: NotifyReason,
    pub mention: MentionVerdict,
}

/// Everything the decision needs, gathered by the caller so this stays pure.
pub struct NotifyContext<'a> {
    pub me: &'a User,
    /// From `channel_members.notify_props` for this channel.
    pub channel_notify_props: &'a HashMap<String, String>,
    /// "online", "away", "dnd", "offline", "ooo".
    pub status: &'a str,
    pub active_channel: Option<&'a str>,
    pub window_focused: bool,
    pub thread_mode: ThreadMode,
    /// Only consulted for replies under collapsed threads.
    pub following_thread: bool,
}

/// True when `needle` appears in `haystack` on word boundaries, case-insensitive.
fn contains_word(haystack_lower: &str, needle_lower: &str) -> bool {
    if needle_lower.is_empty() {
        return false;
    }
    let mut from = 0;
    while let Some(found) = haystack_lower[from..].find(needle_lower) {
        let start = from + found;
        let end = start + needle_lower.len();
        let before_ok = start == 0
            || !haystack_lower[..start]
                .chars()
                .next_back()
                .is_some_and(|character| character.is_alphanumeric());
        let after_ok = end == haystack_lower.len()
            || !haystack_lower[end..]
                .chars()
                .next()
                .is_some_and(|character| character.is_alphanumeric());
        if before_ok && after_ok {
            return true;
        }
        from = start + 1;
        if from >= haystack_lower.len() {
            break;
        }
    }
    false
}

/// Every `@token` in the message, lowercased, with trailing punctuation trimmed
/// progressively -- `@ada.` must match the username `gaetan`, while
/// `@adai` must not.
fn at_mentions(message: &str) -> Vec<String> {
    let mut found = Vec::new();
    let characters: Vec<char> = message.chars().collect();
    let mut index = 0;
    while index < characters.len() {
        if characters[index] != '@' {
            index += 1;
            continue;
        }
        let mut end = index + 1;
        while end < characters.len() && is_username_char(characters[end]) {
            end += 1;
        }
        if end > index + 1 {
            let token: String = characters[index + 1..end]
                .iter()
                .collect::<String>()
                .to_lowercase();
            // The full token plus each trailing-punctuation trim.
            let mut candidate = token.as_str();
            found.push(candidate.to_string());
            while let Some(trimmed) = candidate
                .strip_suffix('.')
                .or_else(|| candidate.strip_suffix('-'))
                .or_else(|| candidate.strip_suffix('_'))
            {
                found.push(trimmed.to_string());
                candidate = trimmed;
            }
        }
        index = end.max(index + 1);
    }
    found
}

/// Resolves the strongest mention the post contains for this user.
pub fn resolve_mention(post: &Post, context: &NotifyContext<'_>) -> MentionVerdict {
    let message_lower = post.message.to_lowercase();
    let mentions = at_mentions(&post.message);
    let username_lower = context.me.username.to_lowercase();

    if !username_lower.is_empty() && mentions.iter().any(|token| token == &username_lower) {
        return MentionVerdict::Direct;
    }

    // @all/@here/@channel are suppressible per user.
    let channel_wide_enabled =
        context.me.notify_props.get("channel").map(String::as_str) != Some("false");
    if channel_wide_enabled
        && mentions
            .iter()
            .any(|token| matches!(token.as_str(), "all" | "here" | "channel"))
    {
        return MentionVerdict::ChannelWide;
    }

    for keyword in context
        .me
        .notify_props
        .get("mention_keys")
        .map(String::as_str)
        .unwrap_or("")
        .split(',')
        .map(str::trim)
        .filter(|keyword| !keyword.is_empty())
    {
        if contains_word(&message_lower, &keyword.to_lowercase()) {
            return MentionVerdict::Keyword;
        }
    }

    let first_name_enabled = context
        .me
        .notify_props
        .get("first_name")
        .map(String::as_str)
        == Some("true");
    if first_name_enabled && !context.me.first_name.is_empty() {
        let first_name_lower = context.me.first_name.to_lowercase();
        if contains_word(&message_lower, &first_name_lower) {
            return MentionVerdict::Keyword;
        }
    }

    MentionVerdict::None
}

/// The desktop notification level in force, after the per-channel override.
fn desktop_level(context: &NotifyContext<'_>) -> String {
    let channel_level = context
        .channel_notify_props
        .get("desktop")
        .map(String::as_str)
        .unwrap_or("default");
    if channel_level != "default" {
        return channel_level.to_string();
    }
    context
        .me
        .notify_props
        .get("desktop")
        .cloned()
        .unwrap_or_else(|| "mention".to_string())
}

fn is_muted(context: &NotifyContext<'_>) -> bool {
    context
        .channel_notify_props
        .get("mark_unread")
        .map(String::as_str)
        == Some("mention")
}

/// The whole policy, in the order the checks have to happen.
pub fn decide(post: &Post, context: &NotifyContext<'_>) -> NotifyDecision {
    let mention = resolve_mention(post, context);
    let decision = |notify: bool, reason: NotifyReason| NotifyDecision {
        notify,
        reason,
        mention,
    };

    if post.user_id == context.me.id {
        // Includes the optimistic echo of a message we just sent.
        return decision(false, NotifyReason::OwnPost);
    }
    if post.is_system() {
        return decision(false, NotifyReason::SystemMessage);
    }
    if matches!(context.status, "dnd" | "ooo") {
        return decision(false, NotifyReason::DoNotDisturb);
    }
    if context.window_focused && context.active_channel == Some(post.channel_id.as_str()) {
        // The webapp stays quiet for the channel you are already reading.
        return decision(false, NotifyReason::AlreadyLooking);
    }

    // Under collapsed threads a reply only notifies if the thread is followed,
    // unless it names you outright.
    if context.thread_mode == ThreadMode::Collapsed
        && post.is_reply()
        && !context.following_thread
        && mention != MentionVerdict::Direct
    {
        return decision(false, NotifyReason::UnfollowedThread);
    }

    if is_muted(context) && !mention.is_mention() {
        return decision(false, NotifyReason::ChannelMuted);
    }

    match desktop_level(context).as_str() {
        "none" => decision(false, NotifyReason::LevelNone),
        "mention" => {
            if mention.is_mention() {
                decision(true, NotifyReason::Mentioned)
            } else {
                decision(false, NotifyReason::NoMentionAtMentionLevel)
            }
        }
        // "all" and anything unrecognised behave as all.
        _ => {
            if mention.is_mention() {
                decision(true, NotifyReason::Mentioned)
            } else {
                decision(true, NotifyReason::AllMessages)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use matterless_core::model::PostMetadata;

    fn me() -> User {
        let mut notify_props = HashMap::new();
        notify_props.insert("desktop".into(), "mention".into());
        notify_props.insert("first_name".into(), "false".into());
        notify_props.insert("mention_keys".into(), String::new());
        User {
            id: "me".into(),
            username: "ada".into(),
            first_name: "Gaetan".into(),
            last_name: "Deturche".into(),
            nickname: String::new(),
            email: String::new(),
            last_picture_update: 0,
            notify_props,
            roles: String::new(),
        }
    }

    fn post_from(author: &str, message: &str) -> Post {
        Post {
            id: "p1".into(),
            channel_id: "c1".into(),
            user_id: author.into(),
            root_id: String::new(),
            create_at: 100,
            update_at: 100,
            edit_at: 0,
            delete_at: 0,
            message: message.into(),
            post_type: String::new(),
            file_ids: Vec::new(),
            props: serde_json::Value::Null,
            metadata: PostMetadata::default(),
            pending_post_id: String::new(),
            is_pinned: false,
        }
    }

    fn context<'a>(
        user: &'a User,
        channel_props: &'a HashMap<String, String>,
    ) -> NotifyContext<'a> {
        NotifyContext {
            me: user,
            channel_notify_props: channel_props,
            status: "online",
            active_channel: None,
            window_focused: false,
            thread_mode: ThreadMode::Flat,
            following_thread: false,
        }
    }

    #[test]
    fn a_direct_mention_wins() {
        let user = me();
        let props = HashMap::new();
        let decision = decide(
            &post_from("other", "hey @ada look at this"),
            &context(&user, &props),
        );
        assert_eq!(decision.mention, MentionVerdict::Direct);
        assert!(decision.notify);
    }

    #[test]
    fn trailing_punctuation_still_mentions_but_a_longer_name_does_not() {
        let user = me();
        let props = HashMap::new();
        let ctx = context(&user, &props);

        assert_eq!(
            resolve_mention(&post_from("other", "thanks @ada."), &ctx),
            MentionVerdict::Direct,
            "a trailing period must not break the match"
        );
        assert_eq!(
            resolve_mention(&post_from("other", "ping @ada, please"), &ctx),
            MentionVerdict::Direct
        );
        assert_eq!(
            resolve_mention(&post_from("other", "cc @adai"), &ctx),
            MentionVerdict::None,
            "a longer username must NOT match -- this is the classic false positive"
        );
        assert_eq!(
            resolve_mention(&post_from("other", "mail ada@example.com"), &ctx),
            MentionVerdict::None,
            "an email address is not a mention of gaetan"
        );
    }

    #[test]
    fn channel_wide_mentions_can_be_suppressed_by_the_user() {
        let mut user = me();
        let props = HashMap::new();
        assert_eq!(
            resolve_mention(
                &post_from("other", "@here standup in 5"),
                &context(&user, &props)
            ),
            MentionVerdict::ChannelWide
        );

        user.notify_props.insert("channel".into(), "false".into());
        assert_eq!(
            resolve_mention(
                &post_from("other", "@here standup in 5"),
                &context(&user, &props)
            ),
            MentionVerdict::None
        );
    }

    #[test]
    fn keywords_and_first_name_match_on_word_boundaries() {
        let mut user = me();
        user.notify_props
            .insert("mention_keys".into(), "nanite, lumen".into());
        let props = HashMap::new();

        assert_eq!(
            resolve_mention(
                &post_from("other", "the Nanite path is slow"),
                &context(&user, &props)
            ),
            MentionVerdict::Keyword
        );
        assert_eq!(
            resolve_mention(
                &post_from("other", "nanitex is not a word"),
                &context(&user, &props)
            ),
            MentionVerdict::None,
            "keywords must not match inside a longer word"
        );

        user.notify_props.insert("first_name".into(), "true".into());
        assert_eq!(
            resolve_mention(
                &post_from("other", "ask Gaetan about it"),
                &context(&user, &props)
            ),
            MentionVerdict::Keyword,
            "first-name matching is opt-in and this user opted in"
        );
    }

    #[test]
    fn own_posts_and_system_messages_never_notify() {
        let user = me();
        let props = HashMap::new();
        let ctx = context(&user, &props);

        let mine = post_from("me", "@ada talking to myself");
        assert_eq!(decide(&mine, &ctx).reason, NotifyReason::OwnPost);

        let mut joined = post_from("other", "someone joined");
        joined.post_type = "system_join_channel".into();
        assert_eq!(decide(&joined, &ctx).reason, NotifyReason::SystemMessage);
    }

    #[test]
    fn dnd_and_the_channel_youre_reading_stay_quiet() {
        let user = me();
        let props = HashMap::new();

        let mut ctx = context(&user, &props);
        ctx.status = "dnd";
        assert_eq!(
            decide(&post_from("other", "@ada urgent"), &ctx).reason,
            NotifyReason::DoNotDisturb
        );

        let mut ctx = context(&user, &props);
        ctx.active_channel = Some("c1");
        ctx.window_focused = true;
        assert_eq!(
            decide(&post_from("other", "@ada hi"), &ctx).reason,
            NotifyReason::AlreadyLooking
        );

        // Same channel but the window is elsewhere: notify.
        let mut ctx = context(&user, &props);
        ctx.active_channel = Some("c1");
        ctx.window_focused = false;
        assert!(decide(&post_from("other", "@ada hi"), &ctx).notify);
    }

    #[test]
    fn the_per_channel_override_beats_the_global_level() {
        let mut user = me();
        user.notify_props.insert("desktop".into(), "all".into());

        // Global "all" would notify, but this channel says mentions only.
        let mut channel_props = HashMap::new();
        channel_props.insert("desktop".to_string(), "mention".to_string());
        let ctx = context(&user, &channel_props);
        assert_eq!(
            decide(&post_from("other", "just chatting"), &ctx).reason,
            NotifyReason::NoMentionAtMentionLevel
        );

        // "default" defers back to the global level.
        let mut channel_props = HashMap::new();
        channel_props.insert("desktop".to_string(), "default".to_string());
        let ctx = context(&user, &channel_props);
        assert_eq!(
            decide(&post_from("other", "just chatting"), &ctx).reason,
            NotifyReason::AllMessages
        );
    }

    #[test]
    fn a_muted_channel_only_notifies_on_a_mention() {
        let mut user = me();
        user.notify_props.insert("desktop".into(), "all".into());
        let mut channel_props = HashMap::new();
        channel_props.insert("mark_unread".to_string(), "mention".to_string());
        let ctx = context(&user, &channel_props);

        assert_eq!(
            decide(&post_from("other", "background noise"), &ctx).reason,
            NotifyReason::ChannelMuted
        );
        assert!(decide(&post_from("other", "@ada look"), &ctx).notify);
    }

    #[test]
    fn collapsed_threads_mute_unfollowed_replies_but_not_direct_mentions() {
        let mut user = me();
        user.notify_props.insert("desktop".into(), "all".into());
        let props = HashMap::new();

        let mut reply = post_from("other", "more on this");
        reply.root_id = "root1".into();

        let mut ctx = context(&user, &props);
        ctx.thread_mode = ThreadMode::Collapsed;
        ctx.following_thread = false;
        assert_eq!(decide(&reply, &ctx).reason, NotifyReason::UnfollowedThread);

        ctx.following_thread = true;
        assert!(decide(&reply, &ctx).notify, "a followed thread notifies");

        // Being named outright cuts through regardless.
        let mut named = post_from("other", "@ada thoughts?");
        named.root_id = "root1".into();
        ctx.following_thread = false;
        assert!(decide(&named, &ctx).notify);
    }
}
