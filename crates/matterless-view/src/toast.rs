//! Raising a notification, and getting the click back to the window.
//!
//! The decision was already made: `notify::decide` runs inside the sync engine
//! on every message, in this window exactly as it does in the app, so a message
//! that interrupts you here is one that would have interrupted you there. What
//! was missing was only the delivery.
//!
//! Windows only for now. Everything above this is portable, and the one thing
//! that is not -- how an operating system shows a notification -- is behind the
//! `raise` a caller sees.

/// What a click on a notification should do, once it reaches the window.
///
/// The toast fires its callback on a thread of its own, so the answer travels
/// back the same way everything else does: as an update, on the thread that
/// owns the window.
/// It is handed the conversation and the message, in that order.
pub type Clicked = Box<dyn Fn(String, String) + Send + Sync + 'static>;

/// Shows a notification, and reports whether the platform accepted it.
///
/// `channel_id` and `post_id` ride through the callback so a click lands the
/// reader on the message it came from -- in its thread, for a reply -- rather
/// than wherever they were last.
#[cfg(target_os = "windows")]
pub fn raise(
    channel_id: &str,
    post_id: &str,
    title: &str,
    body: &str,
    clicked: std::sync::Arc<Clicked>,
) -> bool {
    use tauri_winrt_notification::{Duration, Toast};

    let target = (channel_id.to_string(), post_id.to_string());
    // An installed build says who it is; anything else borrows PowerShell's
    // id, because a toast is refused outright under an AppUserModelID the
    // shell does not know and a dev build has nothing registering one.
    let who = match crate::identity::claimed() {
        true => crate::identity::AUMID,
        false => Toast::POWERSHELL_APP_ID,
    };
    let result = Toast::new(who)
        .title(title)
        .text1(body)
        // Short: a chat message is worth a glance, not a quarter of a minute of
        // screen real estate.
        .duration(Duration::Short)
        .on_activated(move |_action| {
            clicked(target.0.clone(), target.1.clone());
            Ok(())
        })
        .show();
    if let Err(error) = &result {
        eprintln!("a notification could not be shown: {error}");
    }
    result.is_ok()
}

/// Everywhere else, for now: the window still works, it simply stays quiet.
#[cfg(not(target_os = "windows"))]
pub fn raise(
    _channel_id: &str,
    _post_id: &str,
    _title: &str,
    _body: &str,
    _clicked: std::sync::Arc<Clicked>,
) -> bool {
    false
}

/// What a notification is titled and what it says.
///
/// A direct message is titled by the person, because the conversation *is* the
/// person and naming it twice wastes the only line a toast has. A channel is
/// titled by itself with the author in the body, which is the question a reader
/// actually has: where, and then who.
///
/// A group is a channel for this purpose, not a direct message. It has several
/// people in it, so the author alone does not say which one it was -- and with
/// nine groups of overlapping membership, that is the whole question.
///
/// A reply says it is one in the title, as the official client's "Reply in"
/// does: the reader then knows to look in a thread and not down the channel.
pub fn wording(announcement: &matterless_sync::notify::Announcement) -> (String, String) {
    if announcement.kind == matterless_sync::notify::Kind::Direct {
        let title = match announcement.reply {
            true => format!("Reply from {}", announcement.author),
            false => announcement.author.clone(),
        };
        (title, announcement.preview.clone())
    } else {
        let title = if announcement.channel.is_empty() {
            announcement.author.clone()
        } else {
            announcement.channel.clone()
        };
        let title = match announcement.reply {
            true => format!("Reply in {title}"),
            false => title,
        };
        (
            title,
            format!("{}: {}", announcement.author, announcement.preview),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::wording;
    use matterless_sync::notify::{Announcement, Kind};

    fn from(channel: &str, kind: Kind) -> Announcement {
        Announcement {
            resolved: true,
            author: "ada".into(),
            author_id: "u1".into(),
            channel: channel.into(),
            preview: "the build is green".into(),
            kind,
            reply: false,
        }
    }

    /// A reply says so, or the reader looks for it down the channel and finds
    /// nothing new there.
    #[test]
    fn a_reply_says_it_is_one() {
        let reply = |channel: &str, kind: Kind| Announcement {
            reply: true,
            ..from(channel, kind)
        };
        assert_eq!(
            wording(&reply("Dev", Kind::Channel)),
            (
                "Reply in Dev".to_string(),
                "ada: the build is green".to_string()
            )
        );
        assert_eq!(
            wording(&reply("ada", Kind::Direct)),
            (
                "Reply from ada".to_string(),
                "the build is green".to_string()
            )
        );
    }

    /// A direct message is the person, so the person is the title and saying it
    /// again in the body would waste the only line there is.
    #[test]
    fn a_direct_message_is_titled_by_the_person() {
        assert_eq!(
            wording(&from("ada", Kind::Direct)),
            ("ada".to_string(), "the build is green".to_string())
        );
    }

    /// A channel answers where first and who second.
    /// A group is titled by who is in it, with the author in the body.
    ///
    /// It used to be announced as a direct message: the title was one person's
    /// name and nothing said which group, which is no use to somebody in nine
    /// of them with overlapping membership.
    #[test]
    fn a_group_is_titled_by_the_group_and_says_who_spoke() {
        assert_eq!(
            wording(&from("florine, leo-paul", Kind::Group)),
            (
                "florine, leo-paul".to_string(),
                "ada: the build is green".to_string()
            )
        );
    }

    #[test]
    fn a_channel_is_titled_by_itself() {
        assert_eq!(
            wording(&from("Dev", Kind::Channel)),
            ("Dev".to_string(), "ada: the build is green".to_string())
        );
    }

    /// A channel the store has never met has no name to show, and the author is
    /// better than an empty title.
    #[test]
    fn an_unknown_channel_falls_back_to_the_author() {
        let (title, _) = wording(&from("", Kind::Channel));
        assert_eq!(title, "ada");
    }
}
