//! Who this program says it is to Windows.
//!
//! A toast is refused outright without a registered AppUserModelID, and on
//! Windows that identity does not come from the executable -- it comes from a
//! Start-menu shortcut carrying it. An uninstalled build has no shortcut,
//! which is why this window has been borrowing PowerShell's id, and why every
//! notification it raised was labelled PowerShell.
//!
//! The installer writes the shortcut; the AppUserModelID on it is set here
//! rather than there. Setting a property on a `.lnk` needs `IPropertyStore`,
//! which the stock NSIS cannot reach without a plugin it does not ship -- and
//! the `windows` crate is already a dependency of this window. So the
//! installer makes the shortcut and the program claims it.
//!
//! Only the installed build claims it, asked of the directory the installer
//! recorded rather than guessed from the path -- the installer offers a
//! directory page, so where it went is a decision somebody may have made
//! differently. A build run out of `target\debug` has no business branding its
//! toasts as the installed program, and an installed build beside it must not
//! have its identity taken over by one.

/// What this program calls itself to the shell.
///
/// Matches the store's own folder, which is the other place this program is
/// named on disk. Reverse-domain because that is the convention a shell
/// identity follows, and because it has to be unique across every program on
/// the machine.
pub const AUMID: &str = "com.gaetandeturche.matterless";

/// Claims the identity, and answers whether a toast may use it.
///
/// Once per process: the answer cannot change while it runs, and the work is a
/// file open and possibly a write.
pub fn claimed() -> bool {
    static ONCE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ONCE.get_or_init(claim)
}

/// Where the installer puts the shortcut, which is the only one this will
/// touch.
#[cfg(windows)]
fn shortcut() -> Option<std::path::PathBuf> {
    let base = std::env::var_os("APPDATA")?;
    Some(
        std::path::Path::new(&base)
            .join("Microsoft")
            .join("Windows")
            .join("Start Menu")
            .join("Programs")
            .join("MatterLess.lnk"),
    )
}

#[cfg(not(windows))]
fn claim() -> bool {
    false
}

/// Finds the shortcut, makes sure it carries this identity, and takes it.
#[cfg(windows)]
fn claim() -> bool {
    if !is_the_installed_build() {
        // A dev build: nothing has registered this identity for it, and a
        // toast sent under one the shell does not know is refused outright.
        return false;
    }
    let Some(path) = shortcut().filter(|path| path.is_file()) else {
        return false;
    };
    match windows_claim(&path) {
        Ok(()) => {
            println!("notifications are branded {AUMID}");
            true
        }
        Err(error) => {
            eprintln!("could not claim {AUMID}: {error} -- notifications stay borrowed");
            false
        }
    }
}

#[cfg(windows)]
fn windows_claim(path: &std::path::Path) -> Result<(), String> {
    use windows::Win32::Foundation::PROPERTYKEY;
    use windows::Win32::System::Com::StructuredStorage::{
        InitPropVariantFromStringVector, PropVariantToStringAlloc,
    };
    use windows::Win32::System::Com::{
        CLSCTX_ALL, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx, IPersistFile,
        STGM_READWRITE,
    };
    use windows::Win32::UI::Shell::PropertiesSystem::IPropertyStore;
    use windows::Win32::UI::Shell::{
        IShellLinkW, SetCurrentProcessExplicitAppUserModelID, ShellLink,
    };
    use windows::core::{HSTRING, Interface, PCWSTR};

    // `System.AppUserModel.ID`, which the crate has no constant for.
    const APP_USER_MODEL_ID: PROPERTYKEY = PROPERTYKEY {
        fmtid: windows::core::GUID::from_u128(0x9F4C2855_9F79_4B39_A8D0_E1D42DE1D5F3),
        pid: 5,
    };

    let wanted = HSTRING::from(AUMID);
    let path = HSTRING::from(path.as_os_str());

    unsafe {
        // Already initialised is a success, not a failure: winit may well have
        // got there first. The same rule the taskbar follows.
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let link: IShellLinkW =
            CoCreateInstance(&ShellLink, None, CLSCTX_ALL).map_err(|error| format!("{error}"))?;
        let file: IPersistFile = link.cast().map_err(|error| format!("{error}"))?;
        // Opened to write, not to read: a shortcut loaded read-only refuses
        // the save afterwards with STG_E_ACCESSDENIED, which is a sentence
        // about the file rather than about the property being set.
        file.Load(PCWSTR(path.as_ptr()), STGM_READWRITE)
            .map_err(|error| format!("{error}"))?;

        let store: IPropertyStore = link.cast().map_err(|error| format!("{error}"))?;
        let already = store
            .GetValue(&APP_USER_MODEL_ID)
            .ok()
            .and_then(|value| PropVariantToStringAlloc(&value).ok())
            .and_then(|got| got.to_string().ok())
            .unwrap_or_default();

        if already != AUMID {
            // Written once, when it is missing or wrong. Rewriting a file in
            // the Start menu on every start is not a thing to do casually.
            let value = InitPropVariantFromStringVector(Some(&[PCWSTR(wanted.as_ptr())]))
                .map_err(|error| format!("{error}"))?;
            store
                .SetValue(&APP_USER_MODEL_ID, &value)
                .map_err(|error| format!("{error}"))?;
            store.Commit().map_err(|error| format!("{error}"))?;
            file.Save(PCWSTR(path.as_ptr()), true)
                .map_err(|error| format!("{error}"))?;
            println!("the Start menu shortcut now carries {AUMID}");
        }

        // And the process says the same thing, or a toast raised from it is
        // attributed to whatever the shell can work out instead.
        SetCurrentProcessExplicitAppUserModelID(PCWSTR(wanted.as_ptr()))
            .map_err(|error| format!("{error}"))?;
    }
    Ok(())
}

/// Where the installer put this build, if this build was installed.
///
/// Asked of the registry rather than guessed from the path: the installer
/// offers a directory page, so where it went is a decision somebody may have
/// made differently.
#[cfg(windows)]
fn install_dir() -> Option<std::path::PathBuf> {
    use windows::Win32::System::Registry::{
        HKEY, HKEY_CURRENT_USER, KEY_READ, RegCloseKey, RegOpenKeyExW, RegQueryValueExW,
    };
    use windows::core::{PCWSTR, w};

    unsafe {
        let mut key = HKEY::default();
        if RegOpenKeyExW(
            HKEY_CURRENT_USER,
            w!("Software\\MatterLess"),
            Some(0),
            KEY_READ,
            &raw mut key,
        )
        .is_err()
        {
            return None;
        }
        let mut size = 0u32;
        let name = w!("InstallDir");
        let read = RegQueryValueExW(
            key,
            PCWSTR(name.as_ptr()),
            None,
            None,
            None,
            Some(&raw mut size),
        );
        if read.is_err() || size == 0 {
            let _ = RegCloseKey(key);
            return None;
        }
        let mut raw = vec![0u8; size as usize];
        let got = RegQueryValueExW(
            key,
            PCWSTR(name.as_ptr()),
            None,
            None,
            Some(raw.as_mut_ptr()),
            Some(&raw mut size),
        );
        let _ = RegCloseKey(key);
        if got.is_err() {
            return None;
        }
        let (pairs, _) = raw.as_chunks::<2>();
        let wide: Vec<u16> = pairs
            .iter()
            .map(|pair| u16::from_le_bytes(*pair))
            .take_while(|unit| *unit != 0)
            .collect();
        Some(std::path::PathBuf::from(String::from_utf16_lossy(&wide)))
    }
}

/// Whether this executable is the one the installer put there.
///
/// A build run out of `target\debug` has no business branding its toasts as
/// the installed program, and an installed build beside it must not have its
/// identity taken over by one.
#[cfg(windows)]
fn is_the_installed_build() -> bool {
    let (Some(dir), Ok(here)) = (install_dir(), std::env::current_exe()) else {
        return false;
    };
    match (dir.canonicalize(), here.canonicalize()) {
        (Ok(dir), Ok(here)) => here.starts_with(dir),
        // Cannot tell: say no. Branding a toast wrongly is worse than leaving
        // it borrowed.
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    /// The identity is the store's own name, which is the other place this
    /// program is named on disk. They are not derived from one another, so a
    /// test is what holds them together.
    #[test]
    fn the_identity_matches_what_the_program_is_called_on_disk() {
        assert_eq!(super::AUMID, "com.gaetandeturche.matterless");
        let store = matterless_view_store_folder();
        assert!(
            store.starts_with(super::AUMID),
            "{store} against {}",
            super::AUMID
        );
    }

    /// What `feed::default_store` joins onto `%APPDATA%`.
    fn matterless_view_store_folder() -> String {
        let path = crate::feed::default_store().expect("a store path");
        path.parent()
            .and_then(|parent| parent.file_name())
            .map(|name| name.to_string_lossy().to_string())
            .expect("a folder")
    }

    /// A dev build borrows an identity rather than inventing one, which is
    /// what this very test binary is: it does not live where the installer
    /// puts things, so it must not claim to be the installed program.
    #[cfg(windows)]
    #[test]
    fn a_build_that_was_not_installed_does_not_claim_to_be() {
        assert!(!super::is_the_installed_build());
        assert!(!super::claimed(), "a test binary branded itself");
    }
}
