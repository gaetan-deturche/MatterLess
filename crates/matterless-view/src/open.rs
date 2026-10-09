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

/// Opens the folder a file was kept in, with the file chosen in it.
///
/// Only ever a path this program wrote, never one out of a message.
#[cfg(windows)]
pub fn reveal(path: &std::path::Path) -> bool {
    use windows::Win32::UI::Shell::ShellExecuteW;
    use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
    use windows::core::HSTRING;

    let action = HSTRING::from("open");
    let explorer = HSTRING::from("explorer.exe");
    let chosen = HSTRING::from(format!("/select,\"{}\"", path.display()));
    let result = unsafe { ShellExecuteW(None, &action, &explorer, &chosen, None, SW_SHOWNORMAL) };
    result.0 as usize > 32
}

#[cfg(not(windows))]
pub fn reveal(_path: &std::path::Path) -> bool {
    false
}

/// What the Save dialog came back with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Chosen {
    /// Keep it here.
    Here(std::path::PathBuf),
    /// The reader thought better of it.
    Cancelled,
    /// There was no dialog to ask with, which leaves the caller to choose.
    NoDialog,
}

/// Asks where to keep a file, in the system's own Save dialog, starting in
/// `folder` with `name` filled in.
#[cfg(windows)]
pub fn choose_where(name: &str, folder: &std::path::Path, owner: isize) -> Chosen {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::System::Com::{
        CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
        CoTaskMemFree,
    };
    use windows::Win32::UI::Shell::{
        FileSaveDialog, IFileSaveDialog, IShellItem, SHCreateItemFromParsingName, SIGDN_FILESYSPATH,
    };
    use windows::core::HSTRING;

    /// What `Show` answers when the reader cancels: `ERROR_CANCELLED`.
    const CANCELLED: i32 = 0x800704C7_u32 as i32;

    unsafe {
        // Already initialised is a success: the taskbar got there first.
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let Ok(dialog) =
            CoCreateInstance::<_, IFileSaveDialog>(&FileSaveDialog, None, CLSCTX_INPROC_SERVER)
        else {
            return Chosen::NoDialog;
        };
        let _ = dialog.SetFileName(&HSTRING::from(suggested(name)));
        if let Some((_, extension)) = name.rsplit_once('.') {
            let _ = dialog.SetDefaultExtension(&HSTRING::from(extension));
        }
        if let Ok(start) = SHCreateItemFromParsingName::<_, _, IShellItem>(
            &HSTRING::from(folder.as_os_str()),
            None,
        ) {
            let _ = dialog.SetFolder(&start);
        }
        let owner = (owner != 0).then_some(HWND(owner as *mut _));
        match dialog.Show(owner) {
            Ok(()) => {}
            Err(error) if error.code().0 == CANCELLED => return Chosen::Cancelled,
            Err(_) => return Chosen::NoDialog,
        }
        let Ok(path) = dialog
            .GetResult()
            .and_then(|chosen| chosen.GetDisplayName(SIGDN_FILESYSPATH))
        else {
            return Chosen::NoDialog;
        };
        let said = path.to_string();
        CoTaskMemFree(Some(path.0 as *const _));
        match said {
            Ok(said) => Chosen::Here(std::path::PathBuf::from(said)),
            Err(_) => Chosen::NoDialog,
        }
    }
}

#[cfg(not(windows))]
pub fn choose_where(_name: &str, _folder: &std::path::Path, _owner: isize) -> Chosen {
    Chosen::NoDialog
}

/// Asks which files to attach, in the system's own Open dialog. Empty when
/// the reader cancels, or when there is no dialog to ask with.
#[cfg(windows)]
pub fn choose_files(owner: isize) -> Vec<std::path::PathBuf> {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::System::Com::{
        CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
        CoTaskMemFree,
    };
    use windows::Win32::UI::Shell::{
        FOS_ALLOWMULTISELECT, FOS_FILEMUSTEXIST, FileOpenDialog, IFileOpenDialog, SIGDN_FILESYSPATH,
    };

    let mut chosen = Vec::new();
    unsafe {
        // Already initialised is a success: the taskbar got there first.
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let Ok(dialog) =
            CoCreateInstance::<_, IFileOpenDialog>(&FileOpenDialog, None, CLSCTX_INPROC_SERVER)
        else {
            return chosen;
        };
        if let Ok(options) = dialog.GetOptions() {
            let _ = dialog.SetOptions(options | FOS_ALLOWMULTISELECT | FOS_FILEMUSTEXIST);
        }
        let owner = (owner != 0).then_some(HWND(owner as *mut _));
        // Cancelling is an error here too, and means nothing was chosen.
        if dialog.Show(owner).is_err() {
            return chosen;
        }
        let Ok(items) = dialog.GetResults() else {
            return chosen;
        };
        for at in 0..items.GetCount().unwrap_or(0) {
            let Ok(path) = items
                .GetItemAt(at)
                .and_then(|item| item.GetDisplayName(SIGDN_FILESYSPATH))
            else {
                continue;
            };
            if let Ok(said) = path.to_string() {
                chosen.push(std::path::PathBuf::from(said));
            }
            CoTaskMemFree(Some(path.0 as *const _));
        }
    }
    chosen
}

#[cfg(not(windows))]
pub fn choose_files(_owner: isize) -> Vec<std::path::PathBuf> {
    Vec::new()
}

/// A file's name as the dialog should offer it: the server's string, with
/// nothing in it a file name cannot hold -- a separator in it would be a
/// folder nobody chose.
fn suggested(name: &str) -> String {
    let said: String = name
        .chars()
        .map(|character| match character {
            '\\' | '/' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            character if character.is_control() => '_',
            character => character,
        })
        .collect();
    let said = said.trim_matches(['.', ' ']).to_string();
    if said.is_empty() {
        "attachment".to_string()
    } else {
        said
    }
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

    /// A name offered in the Save dialog is a file's name and nothing more:
    /// no folder hidden in it, and never empty.
    #[test]
    fn a_suggested_name_holds_no_folder() {
        assert_eq!(suggested("shot 1.png"), "shot 1.png");
        assert_eq!(suggested(r"..\..\evil.exe"), "_.._evil.exe");
        assert_eq!(suggested("a/b:c?.png"), "a_b_c_.png");
        assert_eq!(suggested(" . "), "attachment");
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
