use crate::error::Result;
use crate::model::{ChannelMember, Post, Preference, Reaction, Timestamp, UserThread};
use serde::Deserialize;

/// The raw frame as it arrives. `data` is deliberately untyped: each event puts
/// a different shape in it, and several of them are double-encoded.
#[derive(Debug, Clone, Deserialize)]
pub struct Envelope {
    #[serde(default)]
    pub event: Option<String>,
    #[serde(default)]
    pub data: Option<serde_json::Value>,
    #[serde(default)]
    pub broadcast: Option<Broadcast>,
    #[serde(default)]
    pub seq: Option<i64>,
    /// Present on replies to an action rather than on server-pushed events.
    #[serde(default)]
    pub seq_reply: Option<i64>,
    #[serde(default)]
    pub status: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Broadcast {
    #[serde(default)]
    pub channel_id: String,
    #[serde(default)]
    pub team_id: String,
    #[serde(default)]
    pub user_id: String,
    /// The server sets this so the originating client can ignore its own echo.
    #[serde(default)]
    pub omit_connection_id: String,
}

#[derive(Debug, Clone)]
pub struct Hello {
    pub connection_id: String,
    pub server_version: String,
    pub server_hostname: String,
}

#[derive(Debug, Clone)]
pub enum Event {
    Hello(Hello),
    Posted {
        post: Box<Post>,
        channel_id: String,
    },
    PostEdited(Box<Post>),
    PostDeleted(Box<Post>),
    Typing {
        channel_id: String,
        user_id: String,
        /// The thread being typed in, empty for the channel itself.
        ///
        /// The wire field is `parent_id` and holds a root post id: without it a
        /// reply being typed in a thread reads as typing in the channel.
        root_id: String,
    },
    StatusChange {
        user_id: String,
        status: String,
    },
    ChannelViewed {
        channel_id: String,
    },
    /// This reader's membership of a channel changed -- muting it, or changing
    /// its notification level, here or on another device.
    ///
    /// The member carries `notify_props`, which is where muting lives, so
    /// without this a channel muted elsewhere kept notifying until something
    /// else happened to refetch membership.
    ChannelMemberUpdated {
        member: Box<ChannelMember>,
    },
    /// Read state changed elsewhere -- another device marking channels read.
    /// Without this, reading on your phone never clears the unread here.
    ///
    /// Shape confirmed against the live server: `data.channel_times` is a map of
    /// channel id to viewed-at milliseconds (**not** a `channel_ids` array).
    ChannelsViewed {
        channel_times: Vec<(String, Timestamp)>,
    },
    ReactionAdded(Box<Reaction>),
    ReactionRemoved(Box<Reaction>),
    /// A followed thread gained a reply, with the server's own counts attached.
    ///
    /// `thread` carries the whole `UserThread` when the server sends it, which
    /// is what makes per-thread unread possible without a round trip. Optional
    /// rather than required: the field is parsed defensively because this shape
    /// was written from the documented one and confirmed later -- guessing
    /// `multiple_channels_viewed` cost this project a silent bug once already.
    ThreadUpdated {
        thread_id: String,
        thread: Option<Box<UserThread>>,
    },
    /// Read state for one thread changed, here or on another device.
    ///
    /// An empty `thread_id` means every thread in the team was marked read.
    ThreadReadChanged {
        thread_id: String,
        channel_id: String,
        timestamp: Timestamp,
        unread_replies: i64,
        unread_mentions: i64,
    },
    /// A thread was followed or unfollowed.
    ThreadFollowChanged {
        thread_id: String,
        following: bool,
        reply_count: i64,
    },
    /// A preference changed at runtime. This is how `collapsed_reply_threads`
    /// can flip while the app runs, which changes the whole data model, so the
    /// caller must re-resolve the thread mode rather than trusting startup.
    PreferencesChanged {
        preferences: Vec<Preference>,
    },
    /// Sidebar ordering changed. Carried on the broadcast's team.
    SidebarCategoriesUpdated {
        team_id: String,
    },
    /// Custom emoji added: invalidate the emoji cache.
    EmojiAdded {
        emoji_id: String,
    },
    /// A user's profile changed (name, avatar): the cached row is stale.
    UserUpdated {
        user_id: String,
    },
    /// Kept rather than dropped so an unhandled event is visible in a soak run.
    Other {
        name: String,
    },
}

impl Event {
    /// Cheap discriminator for tallies and for routing typing away from the post
    /// store -- typing measured between 72% and 93% of traffic.
    pub fn name(&self) -> &str {
        match self {
            Event::Hello(_) => "hello",
            Event::Posted { .. } => "posted",
            Event::PostEdited(_) => "post_edited",
            Event::PostDeleted(_) => "post_deleted",
            Event::Typing { .. } => "typing",
            Event::StatusChange { .. } => "status_change",
            Event::ChannelViewed { .. } => "channel_viewed",
            Event::ChannelMemberUpdated { .. } => "channel_member_updated",
            Event::ChannelsViewed { .. } => "multiple_channels_viewed",
            Event::ReactionAdded(_) => "reaction_added",
            Event::ReactionRemoved(_) => "reaction_removed",
            Event::ThreadUpdated { .. } => "thread_updated",
            Event::ThreadReadChanged { .. } => "thread_read_changed",
            Event::ThreadFollowChanged { .. } => "thread_follow_changed",
            Event::PreferencesChanged { .. } => "preferences_changed",
            Event::SidebarCategoriesUpdated { .. } => "sidebar_category_updated",
            Event::EmojiAdded { .. } => "emoji_added",
            Event::UserUpdated { .. } => "user_updated",
            Event::Other { name } => name,
        }
    }

    /// True for events that must never touch the post store or trigger a message
    /// list re-render.
    pub fn is_ephemeral(&self) -> bool {
        matches!(
            self,
            Event::Typing { .. } | Event::StatusChange { .. } | Event::Hello(_)
        )
    }
}

/// Decodes a nested field that the server may send either as a JSON-encoded
/// **string** or as a native value.
///
/// `posted`'s `data.post` is confirmed to be a string; several others are not
/// confirmed either way, so both are accepted rather than guessed at. A wrong
/// guess here does not fail loudly -- it silently drops the event.
fn decode_nested<T: serde::de::DeserializeOwned>(
    data: Option<&serde_json::Value>,
    key: &str,
) -> Result<Option<T>> {
    let Some(raw) = data.and_then(|value| value.get(key)) else {
        return Ok(None);
    };
    let decoded = match raw {
        serde_json::Value::String(encoded) => serde_json::from_str::<T>(encoded)?,
        other => serde_json::from_value::<T>(other.clone())?,
    };
    Ok(Some(decoded))
}

fn decode_nested_post(data: Option<&serde_json::Value>, key: &str) -> Result<Option<Box<Post>>> {
    Ok(decode_nested::<Post>(data, key)?.map(Box::new))
}

/// A number that may arrive as a number or as a string, which Mattermost does
/// inconsistently across events.
fn number_field(data: Option<&serde_json::Value>, key: &str) -> i64 {
    match data.and_then(|value| value.get(key)) {
        Some(serde_json::Value::Number(number)) => number.as_i64().unwrap_or(0),
        Some(serde_json::Value::String(text)) => text.parse().unwrap_or(0),
        _ => 0,
    }
}

fn bool_field(data: Option<&serde_json::Value>, key: &str) -> bool {
    match data.and_then(|value| value.get(key)) {
        Some(serde_json::Value::Bool(value)) => *value,
        Some(serde_json::Value::String(text)) => text == "true",
        _ => false,
    }
}

fn string_field(data: Option<&serde_json::Value>, key: &str) -> String {
    data.and_then(|value| value.get(key))
        .and_then(|value| value.as_str())
        .unwrap_or_default()
        .to_string()
}

/// Pulls the `id` out of a nested object that may itself be string-encoded.
fn nested_id(data: Option<&serde_json::Value>, key: &str) -> String {
    let Some(raw) = data.and_then(|value| value.get(key)) else {
        return String::new();
    };
    let owned: serde_json::Value = match raw {
        serde_json::Value::String(encoded) => match serde_json::from_str(encoded) {
            Ok(parsed) => parsed,
            // Not JSON: the field is the id itself.
            Err(_) => return encoded.clone(),
        },
        other => other.clone(),
    };
    owned
        .get("id")
        .and_then(|value| value.as_str())
        .unwrap_or_default()
        .to_string()
}

pub fn parse(envelope: &Envelope) -> Result<Option<Event>> {
    let Some(name) = envelope.event.as_deref() else {
        // A seq_reply frame: the acknowledgement of our own action.
        return Ok(None);
    };
    let data = envelope.data.as_ref();
    let broadcast_channel = envelope
        .broadcast
        .as_ref()
        .map(|broadcast| broadcast.channel_id.clone())
        .unwrap_or_default();
    let broadcast_team = envelope
        .broadcast
        .as_ref()
        .map(|broadcast| broadcast.team_id.clone())
        .unwrap_or_default();

    let event = match name {
        "hello" => Event::Hello(Hello {
            connection_id: string_field(data, "connection_id"),
            server_version: string_field(data, "server_version"),
            server_hostname: string_field(data, "server_hostname"),
        }),
        "posted" | "ephemeral_message" => {
            let Some(post) = decode_nested_post(data, "post")? else {
                return Ok(Some(Event::Other { name: name.into() }));
            };
            let channel_id = if broadcast_channel.is_empty() {
                post.channel_id.clone()
            } else {
                broadcast_channel
            };
            Event::Posted { post, channel_id }
        }
        "post_edited" => match decode_nested_post(data, "post")? {
            Some(post) => Event::PostEdited(post),
            None => Event::Other { name: name.into() },
        },
        "post_deleted" => match decode_nested_post(data, "post")? {
            Some(post) => Event::PostDeleted(post),
            None => Event::Other { name: name.into() },
        },
        "typing" => Event::Typing {
            channel_id: broadcast_channel,
            user_id: string_field(data, "user_id"),
            root_id: string_field(data, "parent_id"),
        },
        "status_change" => Event::StatusChange {
            user_id: string_field(data, "user_id"),
            status: string_field(data, "status"),
        },
        "channel_viewed" => Event::ChannelViewed {
            channel_id: string_field(data, "channel_id"),
        },
        "multiple_channels_viewed" => {
            let times = data
                .and_then(|value| value.get("channel_times"))
                .and_then(|value| value.as_object())
                .map(|map| {
                    map.iter()
                        .map(|(channel_id, viewed_at)| {
                            (channel_id.clone(), viewed_at.as_i64().unwrap_or(0))
                        })
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            Event::ChannelsViewed {
                channel_times: times,
            }
        }
        // The server spells the payload key in camel case here, unlike the
        // snake_case it uses for `post` and `thread`.
        "channel_member_updated" => match decode_nested::<ChannelMember>(data, "channelMember")? {
            Some(member) => Event::ChannelMemberUpdated {
                member: Box::new(member),
            },
            None => Event::Other {
                name: "channel_member_updated".into(),
            },
        },
        "reaction_added" | "reaction_removed" => {
            let Some(reaction) = decode_nested::<Reaction>(data, "reaction")? else {
                return Ok(Some(Event::Other { name: name.into() }));
            };
            if name == "reaction_added" {
                Event::ReactionAdded(Box::new(reaction))
            } else {
                Event::ReactionRemoved(Box::new(reaction))
            }
        }
        // The three thread events were collapsed into one while threads were
        // out of scope. They carry different things and now mean different
        // things, so they are parsed apart.
        "thread_updated" => {
            let thread = decode_nested::<UserThread>(data, "thread")?.map(Box::new);
            let thread_id = match (&thread, string_field(data, "thread_id")) {
                (Some(thread), id) if id.is_empty() => thread.id.clone(),
                (_, id) => id,
            };
            Event::ThreadUpdated { thread_id, thread }
        }
        "thread_read_changed" => Event::ThreadReadChanged {
            thread_id: string_field(data, "thread_id"),
            channel_id: if broadcast_channel.is_empty() {
                string_field(data, "channel_id")
            } else {
                broadcast_channel
            },
            timestamp: number_field(data, "timestamp"),
            unread_replies: number_field(data, "unread_replies"),
            unread_mentions: number_field(data, "unread_mentions"),
        },
        "thread_follow_changed" => Event::ThreadFollowChanged {
            thread_id: string_field(data, "thread_id"),
            following: bool_field(data, "state"),
            reply_count: number_field(data, "reply_count"),
        },
        "preferences_changed" | "preferences_deleted" => Event::PreferencesChanged {
            preferences: decode_nested::<Vec<Preference>>(data, "preferences")?.unwrap_or_default(),
        },
        "sidebar_category_updated"
        | "sidebar_category_created"
        | "sidebar_category_deleted"
        | "sidebar_category_order_updated" => Event::SidebarCategoriesUpdated {
            team_id: broadcast_team,
        },
        "emoji_added" => Event::EmojiAdded {
            emoji_id: nested_id(data, "emoji"),
        },
        "user_updated" => Event::UserUpdated {
            user_id: nested_id(data, "user"),
        },
        other => Event::Other { name: other.into() },
    };
    Ok(Some(event))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn posted_decodes_the_double_encoded_post() {
        // Shaped exactly as the live server sends it.
        let frame = r#"{
            "event": "posted",
            "data": {
                "channel_type": "P",
                "post": "{\"id\":\"abc\",\"message\":\"hi\",\"channel_id\":\"chan1\",\"root_id\":\"root9\"}",
                "sender_name": "someone"
            },
            "broadcast": {"channel_id": "chan1"},
            "seq": 7
        }"#;
        let envelope: Envelope = serde_json::from_str(frame).unwrap();
        let event = parse(&envelope).unwrap().unwrap();
        match event {
            Event::Posted { post, channel_id } => {
                assert_eq!(post.id, "abc");
                assert_eq!(post.message, "hi");
                assert!(post.is_reply(), "root_id must survive the inner decode");
                assert_eq!(channel_id, "chan1");
            }
            other => panic!("expected Posted, got {other:?}"),
        }
    }

    #[test]
    fn typing_is_ephemeral_and_carries_the_broadcast_channel() {
        let frame = r#"{
            "event": "typing",
            "data": {"parent_id": "", "user_id": "u1"},
            "broadcast": {"channel_id": "chan1"},
            "seq": 3
        }"#;
        let envelope: Envelope = serde_json::from_str(frame).unwrap();
        let event = parse(&envelope).unwrap().unwrap();
        assert!(event.is_ephemeral());
        match event {
            Event::Typing {
                channel_id,
                user_id,
                root_id,
            } => {
                assert_eq!(channel_id, "chan1");
                assert_eq!(user_id, "u1");
                assert!(root_id.is_empty(), "typing in the channel itself");
            }
            other => panic!("expected Typing, got {other:?}"),
        }
    }

    #[test]
    fn typing_in_a_thread_names_the_thread() {
        // `parent_id` is what separates a reply being typed from a message in
        // the channel -- the two arrive as the same event on the same channel.
        let frame = r#"{
            "event": "typing",
            "data": {"parent_id": "root7", "user_id": "u1"},
            "broadcast": {"channel_id": "chan1"},
            "seq": 4
        }"#;
        let envelope: Envelope = serde_json::from_str(frame).unwrap();
        match parse(&envelope).unwrap().unwrap() {
            Event::Typing { root_id, .. } => assert_eq!(root_id, "root7"),
            other => panic!("expected Typing, got {other:?}"),
        }
    }

    #[test]
    fn seq_reply_frames_are_not_events() {
        let envelope: Envelope = serde_json::from_str(r#"{"status":"OK","seq_reply":1}"#).unwrap();
        assert!(parse(&envelope).unwrap().is_none());
    }

    #[test]
    fn unknown_events_are_surfaced_not_swallowed() {
        let envelope: Envelope =
            serde_json::from_str(r#"{"event":"some_new_thing","data":{},"seq":9}"#).unwrap();
        let event = parse(&envelope).unwrap().unwrap();
        assert_eq!(event.name(), "some_new_thing");
    }

    /// Verbatim capture from the live server (2026-09-03), which is why this
    /// reads `channel_times` and not the `channel_ids` the name suggests.
    #[test]
    fn multiple_channels_viewed_uses_channel_times() {
        let frame = r#"{
            "event": "multiple_channels_viewed",
            "data": {"channel_times": {"6sbj9yggztnd5fwpdc17noe7fo": 1788431356625}},
            "broadcast": {"channel_id":"","team_id":"","user_id":"7gwdwjg1zjf7tb5xxdp6ieazgr"},
            "seq": 12
        }"#;
        let envelope: Envelope = serde_json::from_str(frame).unwrap();
        match parse(&envelope).unwrap().unwrap() {
            Event::ChannelsViewed { channel_times } => {
                assert_eq!(channel_times.len(), 1);
                assert_eq!(channel_times[0].0, "6sbj9yggztnd5fwpdc17noe7fo");
                assert_eq!(channel_times[0].1, 1_788_431_356_625);
            }
            other => panic!("expected ChannelsViewed, got {other:?}"),
        }
    }

    #[test]
    fn preferences_changed_accepts_either_encoding() {
        // Encoding unconfirmed against the live server, so both must work.
        let as_string = r#"{
            "event": "preferences_changed",
            "data": {"preferences": "[{\"user_id\":\"me\",\"category\":\"display_settings\",\"name\":\"collapsed_reply_threads\",\"value\":\"off\"}]"}
        }"#;
        let as_array = r#"{
            "event": "preferences_changed",
            "data": {"preferences": [{"user_id":"me","category":"display_settings","name":"collapsed_reply_threads","value":"off"}]}
        }"#;

        for frame in [as_string, as_array] {
            let envelope: Envelope = serde_json::from_str(frame).unwrap();
            match parse(&envelope).unwrap().unwrap() {
                Event::PreferencesChanged { preferences } => {
                    assert_eq!(preferences.len(), 1);
                    assert_eq!(preferences[0].name, "collapsed_reply_threads");
                    assert_eq!(preferences[0].value, "off");
                }
                other => panic!("expected PreferencesChanged, got {other:?}"),
            }
        }
    }

    #[test]
    fn emoji_and_user_ids_survive_either_encoding() {
        let emoji_string =
            r#"{"event":"emoji_added","data":{"emoji":"{\"id\":\"emo1\",\"name\":\"shipit\"}"}}"#;
        let emoji_object =
            r#"{"event":"emoji_added","data":{"emoji":{"id":"emo1","name":"shipit"}}}"#;
        for frame in [emoji_string, emoji_object] {
            let envelope: Envelope = serde_json::from_str(frame).unwrap();
            match parse(&envelope).unwrap().unwrap() {
                Event::EmojiAdded { emoji_id } => assert_eq!(emoji_id, "emo1"),
                other => panic!("expected EmojiAdded, got {other:?}"),
            }
        }

        let user_object =
            r#"{"event":"user_updated","data":{"user":{"id":"u9","username":"someone"}}}"#;
        let envelope: Envelope = serde_json::from_str(user_object).unwrap();
        match parse(&envelope).unwrap().unwrap() {
            Event::UserUpdated { user_id } => assert_eq!(user_id, "u9"),
            other => panic!("expected UserUpdated, got {other:?}"),
        }
    }

    #[test]
    fn sidebar_events_carry_the_team_from_the_broadcast() {
        let frame = r#"{
            "event": "sidebar_category_updated",
            "data": {},
            "broadcast": {"channel_id":"","team_id":"team7","user_id":"me"}
        }"#;
        let envelope: Envelope = serde_json::from_str(frame).unwrap();
        match parse(&envelope).unwrap().unwrap() {
            Event::SidebarCategoriesUpdated { team_id } => assert_eq!(team_id, "team7"),
            other => panic!("expected SidebarCategoriesUpdated, got {other:?}"),
        }
    }
}

#[cfg(test)]
mod thread_event_tests {
    use super::*;

    /// These three arrived collapsed into one variant while threads were out of
    /// scope. They now mean different things, and the shapes are written from
    /// the documented ones -- so each is pinned, and each tolerates the field
    /// being absent rather than dropping the event.
    #[test]
    fn thread_updated_carries_the_servers_own_counts() {
        let frame = r#"{"event":"thread_updated","data":{"thread":{"id":"root1",
            "reply_count":7,"last_reply_at":900,"last_viewed_at":100,
            "unread_replies":3,"unread_mentions":1,"is_urgent":false,"delete_at":0,
            "post":{"id":"root1","channel_id":"c1","user_id":"u1","create_at":10,
                    "update_at":10,"message":"x"}}},"seq":4}"#;
        let envelope: Envelope = serde_json::from_str(frame).unwrap();
        let Some(Event::ThreadUpdated { thread_id, thread }) = parse(&envelope).unwrap() else {
            panic!("not a thread update");
        };
        // The id comes from the thread when the frame does not repeat it.
        assert_eq!(thread_id, "root1");
        let thread = thread.expect("the counts are the point of this event");
        assert_eq!(thread.unread_replies, 3);
        assert_eq!(thread.unread_mentions, 1);
        assert_eq!(thread.post.channel_id, "c1");
    }

    #[test]
    fn a_thread_update_without_its_thread_is_still_an_update() {
        let frame = r#"{"event":"thread_updated","data":{"thread_id":"root9"},"seq":5}"#;
        let envelope: Envelope = serde_json::from_str(frame).unwrap();
        let Some(Event::ThreadUpdated { thread_id, thread }) = parse(&envelope).unwrap() else {
            panic!("not a thread update");
        };
        assert_eq!(thread_id, "root9");
        assert!(thread.is_none(), "a fetch will fill it in");
    }

    /// Mattermost sends numbers as numbers in some events and as strings in
    /// others, so the counts are read either way.
    #[test]
    fn thread_read_changed_reads_numbers_in_either_encoding() {
        for frame in [
            r#"{"event":"thread_read_changed","data":{"thread_id":"r1","channel_id":"c1",
                "timestamp":1234,"unread_replies":2,"unread_mentions":1},"seq":6}"#,
            r#"{"event":"thread_read_changed","data":{"thread_id":"r1","channel_id":"c1",
                "timestamp":"1234","unread_replies":"2","unread_mentions":"1"},"seq":6}"#,
        ] {
            let envelope: Envelope = serde_json::from_str(frame).unwrap();
            let Some(Event::ThreadReadChanged {
                thread_id,
                channel_id,
                timestamp,
                unread_replies,
                unread_mentions,
            }) = parse(&envelope).unwrap()
            else {
                panic!("not a read change");
            };
            assert_eq!(thread_id, "r1");
            assert_eq!(channel_id, "c1");
            assert_eq!(timestamp, 1234);
            assert_eq!((unread_replies, unread_mentions), (2, 1));
        }
    }

    /// The member arrives under `channelMember`, in camel case -- unlike the
    /// snake_case `post` and `thread` keys beside it -- and as an encoded string
    /// rather than an object. The whole point of this event is `notify_props`,
    /// so the test asserts the muting reaches through.
    #[test]
    fn a_channel_member_update_carries_its_notify_props() {
        let member = r#"{"channel_id":"c1","user_id":"me","last_viewed_at":7,"msg_count":3,"mention_count":0,"notify_props":{"mark_unread":"mention"}}"#;
        let frame = serde_json::json!({
            "event": "channel_member_updated",
            "data": { "channelMember": member },
            "seq": 9,
        })
        .to_string();
        let envelope: Envelope = serde_json::from_str(&frame).unwrap();
        let Some(Event::ChannelMemberUpdated { member }) = parse(&envelope).unwrap() else {
            panic!("not a member update");
        };
        assert_eq!(member.channel_id, "c1");
        assert_eq!(member.user_id, "me");
        assert_eq!(
            member.notify_props.get("mark_unread").map(String::as_str),
            Some("mention"),
            "muting is the setting this event exists to carry"
        );
    }

    /// An empty thread id means the whole team was marked read, which is a
    /// different action -- not a malformed frame to be dropped.
    #[test]
    fn an_empty_thread_id_means_every_thread_was_read() {
        let frame = r#"{"event":"thread_read_changed","data":{"timestamp":99},"seq":7}"#;
        let envelope: Envelope = serde_json::from_str(frame).unwrap();
        let Some(Event::ThreadReadChanged { thread_id, .. }) = parse(&envelope).unwrap() else {
            panic!("not a read change");
        };
        assert!(thread_id.is_empty());
    }

    #[test]
    fn thread_follow_changed_carries_the_new_state() {
        let frame = r#"{"event":"thread_follow_changed",
            "data":{"thread_id":"r1","state":true,"reply_count":4},"seq":8}"#;
        let envelope: Envelope = serde_json::from_str(frame).unwrap();
        let Some(Event::ThreadFollowChanged {
            thread_id,
            following,
            reply_count,
        }) = parse(&envelope).unwrap()
        else {
            panic!("not a follow change");
        };
        assert_eq!(thread_id, "r1");
        assert!(following);
        assert_eq!(reply_count, 4);
    }
}
