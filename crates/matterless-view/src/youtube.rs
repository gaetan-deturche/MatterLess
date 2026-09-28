//! Which links are YouTube videos, and where in them to start.
//!
//! A link somebody posts is played in the viewer by YouTube's own embedded
//! player (`watch.rs`), which needs the video's id and nothing else from the
//! URL -- the many shapes a link to one video takes are all read down to it.

/// One video: its id, and how far in to start, in seconds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Video {
    pub id: String,
    pub start: u32,
    /// The link as it was posted, for opening it in a browser instead.
    pub url: String,
}

/// The video a link is to, when it is to one.
pub fn video_of(url: &str) -> Option<Video> {
    let rest = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))?;
    let (host, path) = rest.split_once('/').unwrap_or((rest, ""));
    let host = host.to_ascii_lowercase();
    let host = host.strip_prefix("www.").unwrap_or(&host);
    let host = host.strip_prefix("m.").unwrap_or(host);
    let (path, query) = path.split_once('?').unwrap_or((path, ""));
    let query = query.split('#').next().unwrap_or_default();
    let param = |name: &str| {
        query
            .split('&')
            .find_map(|pair| pair.strip_prefix(name)?.strip_prefix('='))
    };
    let id = match host {
        "youtu.be" => path.split('/').next(),
        "youtube.com" | "youtube-nocookie.com" | "music.youtube.com" => {
            match path.split('/').collect::<Vec<_>>().as_slice() {
                ["watch", ..] => param("v"),
                ["shorts" | "live" | "embed" | "v", id, ..] => Some(*id),
                _ => None,
            }
        }
        _ => None,
    }?;
    let id = id.split('#').next().unwrap_or_default();
    // Eleven characters of URL-safe base64, which is what every video id is:
    // anything else is somebody's channel page or a typo.
    let valid = id.len() == 11
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    if !valid {
        return None;
    }
    let start = param("t").or_else(|| param("start")).map_or(0, seconds);
    Some(Video {
        id: id.to_string(),
        start,
        url: url.to_string(),
    })
}

/// `90`, `90s` or `1h2m3s`, in seconds.
fn seconds(said: &str) -> u32 {
    if let Ok(plain) = said.parse::<u32>() {
        return plain;
    }
    let mut total = 0u32;
    let mut number = 0u32;
    for c in said.chars() {
        match c {
            '0'..='9' => number = number * 10 + (c as u32 - '0' as u32),
            'h' => (total, number) = (total + number * 3600, 0),
            'm' => (total, number) = (total + number * 60, 0),
            's' => (total, number) = (total + number, 0),
            _ => return 0,
        }
    }
    total + number
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every shape a link to one video takes reads down to its id, with the
    /// time it starts at; anything that is not one video reads as nothing.
    #[test]
    fn a_link_to_a_video_is_read_down_to_its_id() {
        let id = |url: &str| video_of(url).map(|video| (video.id, video.start));
        let dq = || "dQw4w9WgXcQ".to_string();
        assert_eq!(
            id("https://www.youtube.com/watch?v=dQw4w9WgXcQ"),
            Some((dq(), 0))
        );
        assert_eq!(
            id("https://youtube.com/watch?feature=share&v=dQw4w9WgXcQ&t=42"),
            Some((dq(), 42))
        );
        assert_eq!(id("https://youtu.be/dQw4w9WgXcQ?t=1m30s"), Some((dq(), 90)));
        assert_eq!(
            id("https://m.youtube.com/watch?v=dQw4w9WgXcQ#x"),
            Some((dq(), 0))
        );
        assert_eq!(
            id("https://www.youtube.com/shorts/dQw4w9WgXcQ"),
            Some((dq(), 0))
        );
        assert_eq!(
            id("https://www.youtube.com/live/dQw4w9WgXcQ?si=abc"),
            Some((dq(), 0))
        );
        assert_eq!(
            id("https://www.youtube.com/embed/dQw4w9WgXcQ?start=7"),
            Some((dq(), 7))
        );
        assert_eq!(id("https://www.youtube.com/@somebody"), None);
        assert_eq!(id("https://www.youtube.com/watch?v=short"), None);
        assert_eq!(id("https://example.com/watch?v=dQw4w9WgXcQ"), None);
    }
}
