//! Handing a link to whatever the reader opens links with.
//!
//! Through the shell's own API rather than a command line. A URL from a
//! message is text somebody else wrote, and passing it through `cmd /c start`
//! means the shell parses it: `&` ends the command, `%VAR%` expands, and a
//! carefully shaped link becomes a carefully shaped command. `ShellExecuteW`
//! takes the string as one argument and never parses it.

/// Opens a link, if it is one worth opening.
///
/// Checked again here rather than trusted from the layout. The layout drops
/// every scheme but http and https when it builds the span, and this is the
/// last place before the string leaves the process -- the two together mean a
/// change to either one alone cannot open `file:` on somebody's disk.
pub fn link(href: &str) -> bool {
    if !worth_opening(href) {
        eprintln!("not opening a {} link", scheme_of(href).unwrap_or("bare"));
        return false;
    }
    // The link itself is never logged: it is the content of a message.
    println!("opening a link");
    show(href)
}

fn scheme_of(href: &str) -> Option<&str> {
    href.split_once("://").map(|(scheme, _)| scheme)
}

/// The web and nothing else.
fn worth_opening(href: &str) -> bool {
    let Some(scheme) = scheme_of(href) else {
        return false;
    };
    if !matches!(scheme.to_ascii_lowercase().as_str(), "http" | "https") {
        return false;
    }
    // A control character cannot appear in a real URL and is how a string is
    // made to look like one thing and act as another.
    !href.chars().any(|character| character.is_control())
}

#[cfg(windows)]
fn show(href: &str) -> bool {
    use windows::Win32::UI::Shell::ShellExecuteW;
    use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
    use windows::core::HSTRING;

    let action = HSTRING::from("open");
    let target = HSTRING::from(href);
    // The documented success test: anything above 32 is a handle, anything at
    // or below it is an error code.
    let result = unsafe { ShellExecuteW(None, &action, &target, None, None, SW_SHOWNORMAL) };
    let opened = result.0 as usize > 32;
    if !opened {
        eprintln!("the shell refused to open it ({})", result.0 as usize);
    }
    opened
}

#[cfg(not(windows))]
fn show(_href: &str) -> bool {
    eprintln!("opening links is only wired up on Windows");
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The web, and only the web. A message is text somebody else wrote, and
    /// a href in it can name any scheme the shell has been taught to run.
    #[test]
    fn only_a_web_link_is_worth_opening() {
        assert!(worth_opening("https://example.invalid/thing"));
        assert!(worth_opening("http://example.invalid/thing?a=1&b=2"));
        assert!(worth_opening("HTTPS://EXAMPLE.INVALID/"));
        assert!(!worth_opening("file:///c:/windows/system32"));
        assert!(!worth_opening("javascript:alert(1)"));
        assert!(!worth_opening("ms-settings://x"));
        assert!(!worth_opening("mailto:someone@example.invalid"));
        assert!(!worth_opening("example.invalid"));
        assert!(!worth_opening(""));
    }

    /// A newline inside a link is not a link. It cannot appear in a real URL,
    /// and it is how one string is made to read as two.
    #[test]
    fn a_control_character_disqualifies_it() {
        assert!(!worth_opening("https://example.invalid/\nsomething"));
        assert!(!worth_opening("https://example.invalid/\u{0}"));
        assert!(!worth_opening("https://example.invalid/\r\n"));
    }

    /// The characters a shell would have eaten are ordinary here, because
    /// nothing parses the string on its way out.
    #[test]
    fn a_query_string_survives_intact() {
        let awkward = "https://example.invalid/a?x=1&y=%20two&z=a b";
        assert!(worth_opening(awkward));
    }
}
