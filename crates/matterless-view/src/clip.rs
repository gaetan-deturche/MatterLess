//! The system clipboard, so copy and paste cross to other windows.
//!
//! The window kept a `String` of its own and called it the clipboard, which
//! meant copy and paste worked inside it and nowhere else -- a reader who had
//! copied a stack trace from their editor pressed `Ctrl+V` here and nothing
//! happened at all.
//!
//! Not text only any more. A picture or a file on the clipboard wants
//! attaching rather than inserting -- which is a decision about what a message
//! is, so this only says what is there and leaves the deciding to the window.

/// What the system is holding, as far as a message cares.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Held {
    /// Words, for whichever box has the keyboard.
    Words(String),
    /// Files somebody copied in a file manager. Attachments, not text.
    Files(Vec<std::path::PathBuf>),
    /// A picture with no file behind it: a screenshot, or an image copied out
    /// of a browser. The bytes, and what to call the file they are written
    /// to -- an upload is named after its file and a screenshot has no name
    /// of its own, so one has to be invented either way.
    Picture {
        bytes: Vec<u8>,
        extension: &'static str,
    },
}

/// Everything the clipboard is offering, in the order a message wants it.
///
/// Files first, then a picture, then words. A picture copied out of a browser
/// usually arrives with the page's text beside it, and the picture is what was
/// meant; a file copied in Explorer arrives with its path as text, and pasting
/// the path into the box is not what anybody asked for.
#[cfg(windows)]
pub fn held() -> Option<Held> {
    use windows::Win32::System::DataExchange::OpenClipboard;

    unsafe { OpenClipboard(None) }.ok()?;
    // Every path from here closes it again: the clipboard is one system-wide
    // lock, and a window that opens it and returns leaves every other
    // application unable to copy anything.
    let held = unsafe {
        files()
            .map(Held::Files)
            .or_else(|| picture().map(|(bytes, extension)| Held::Picture { bytes, extension }))
            .or_else(|| words().map(Held::Words))
    };
    shut();
    held
}

/// The paths of files copied in a file manager, from `CF_HDROP`.
///
/// # Safety
/// The clipboard must be open.
#[cfg(windows)]
unsafe fn files() -> Option<Vec<std::path::PathBuf>> {
    use windows::Win32::System::DataExchange::GetClipboardData;
    use windows::Win32::UI::Shell::{DragQueryFileW, HDROP};

    /// `CF_HDROP`, which `windows` keeps with the shell rather than with the
    /// other clipboard formats.
    const HDROP_FORMAT: u32 = 15;

    let handle = unsafe { GetClipboardData(HDROP_FORMAT) }.ok()?;
    let drop = HDROP(handle.0);
    // `0xFFFF_FFFF` asks how many there are rather than for one of them.
    let many = unsafe { DragQueryFileW(drop, u32::MAX, None) };
    let mut paths = Vec::new();
    for at in 0..many {
        // Asked for its length first: a path can be longer than `MAX_PATH`
        // and a fixed buffer would cut one in half without saying so.
        let wide = unsafe { DragQueryFileW(drop, at, None) } as usize;
        if wide == 0 {
            continue;
        }
        let mut name = vec![0u16; wide + 1];
        let written = unsafe { DragQueryFileW(drop, at, Some(&mut name)) } as usize;
        if written == 0 {
            continue;
        }
        name.truncate(written);
        paths.push(std::path::PathBuf::from(String::from_utf16_lossy(&name)));
    }
    (!paths.is_empty()).then_some(paths)
}

/// A picture, as PNG if the source offered one and as a bitmap otherwise.
///
/// # Safety
/// The clipboard must be open.
#[cfg(windows)]
unsafe fn picture() -> Option<(Vec<u8>, &'static str)> {
    use windows::Win32::System::DataExchange::{GetClipboardData, RegisterClipboardFormatW};
    use windows::core::w;

    // What a browser and most editors put there, and the one worth having:
    // it arrives as a file's worth of bytes and needs nothing done to it.
    let png = unsafe { RegisterClipboardFormatW(w!("PNG")) };
    if png != 0
        && let Ok(handle) = unsafe { GetClipboardData(png) }
        && let Some(bytes) = unsafe { bytes_of(handle) }
    {
        return Some((bytes, "png"));
    }
    // Print Screen offers no PNG -- only a bitmap -- and a bitmap is the
    // wrong thing to send: a screen's worth is some four megabytes where the
    // same picture is a few hundred kilobytes encoded, and the server will
    // not make a thumbnail of one, so it arrives as a nameless block nobody
    // can see. So it is encoded here, and only sent raw if that fails.
    let bitmap = unsafe { bitmap() }?;
    match as_a_png(&bitmap) {
        Some(png) => Some((png, "png")),
        None => Some((bitmap, "bmp")),
    }
}

/// A bitmap re-encoded as a PNG, which is what a picture should be sent as.
///
/// The alpha is the part worth knowing about. A screen grab arrives 32 bits
/// deep with every alpha byte left at zero -- not "transparent", just never
/// written -- and taken at its word the PNG is an empty pane of glass. A
/// picture that is transparent everywhere is not a picture, so that is read
/// as the bitmap not having an alpha channel at all.
fn as_a_png(bitmap: &[u8]) -> Option<Vec<u8>> {
    let decoded = image::load_from_memory_with_format(bitmap, image::ImageFormat::Bmp).ok()?;
    let mut pixels = decoded.into_rgba8();
    if pixels.pixels().all(|pixel| pixel.0[3] == 0) {
        for pixel in pixels.pixels_mut() {
            pixel.0[3] = 255;
        }
    }
    let mut png = std::io::Cursor::new(Vec::new());
    pixels
        .write_to(&mut png, image::ImageFormat::Png)
        .ok()
        .map(|()| png.into_inner())
}

/// `CF_DIB` with a file header put back on the front of it.
///
/// Print Screen and a good many tools leave only this: a bitmap stripped of
/// the fourteen bytes that make it a file. Putting them back is the whole of
/// the conversion, and it beats teaching the uploader about raw pixels.
///
/// # Safety
/// The clipboard must be open.
#[cfg(windows)]
unsafe fn bitmap() -> Option<Vec<u8>> {
    use windows::Win32::System::DataExchange::GetClipboardData;
    use windows::Win32::System::Ole::{CF_DIB, CF_DIBV5};

    // V5 first: the same pixels under a header that states its own channel
    // masks, which is the difference between knowing the alpha and guessing.
    for format in [CF_DIBV5.0 as u32, CF_DIB.0 as u32] {
        if let Ok(handle) = unsafe { GetClipboardData(format) }
            && let Some(dib) = unsafe { bytes_of(handle) }
            && let Some(file) = as_a_file(&dib)
        {
            return Some(file);
        }
    }
    None
}

/// A device-independent bitmap with the fourteen bytes that make it a file
/// put back on the front.
///
/// The offset in those bytes is where the pixels start, which is past the
/// header and past the colour table -- and the table's length is either
/// stated or implied by the depth, which is the only part of this worth a
/// test. Get it wrong and the picture arrives shifted rather than refused.
fn as_a_file(dib: &[u8]) -> Option<Vec<u8>> {
    // A `BITMAPINFOHEADER` is forty of them, and everything after depends on
    // reading it.
    if dib.len() < 40 {
        return None;
    }
    let header = u32::from_le_bytes([dib[0], dib[1], dib[2], dib[3]]) as usize;
    let depth = u16::from_le_bytes([dib[14], dib[15]]);
    let mut colours = u32::from_le_bytes([dib[32], dib[33], dib[34], dib[35]]) as usize;
    // Zero means "as many as the depth allows" for the small depths, and
    // means none at all for the large ones.
    if colours == 0 && depth <= 8 {
        colours = 1 << depth;
    }
    // `BI_BITFIELDS` puts three channel masks between the small header and
    // the pixels -- four with `BI_ALPHABITFIELDS` -- and `biClrUsed` does not
    // count them. The big headers carry their masks inside themselves, so
    // this is only ever the forty-byte one.
    let compression = u32::from_le_bytes([dib[16], dib[17], dib[18], dib[19]]);
    let masks = match (header <= 40, compression) {
        (true, 3) => 12,
        (true, 6) => 16,
        _ => 0,
    };
    let pixels_at = 14 + header + masks + colours * 4;
    let whole = 14 + dib.len();

    let mut file = Vec::with_capacity(whole);
    file.extend_from_slice(b"BM");
    file.extend_from_slice(&(whole as u32).to_le_bytes());
    file.extend_from_slice(&0u16.to_le_bytes());
    file.extend_from_slice(&0u16.to_le_bytes());
    file.extend_from_slice(&(pixels_at as u32).to_le_bytes());
    file.extend_from_slice(dib);
    Some(file)
}

/// Whatever a clipboard handle is holding, as bytes.
///
/// # Safety
/// The clipboard must be open and `handle` must have come off it.
#[cfg(windows)]
unsafe fn bytes_of(handle: windows::Win32::Foundation::HANDLE) -> Option<Vec<u8>> {
    use windows::Win32::Foundation::HGLOBAL;
    use windows::Win32::System::Memory::{GlobalLock, GlobalSize, GlobalUnlock};

    let held = HGLOBAL(handle.0);
    let many = unsafe { GlobalSize(held) };
    if many == 0 {
        return None;
    }
    let at = unsafe { GlobalLock(held) } as *const u8;
    if at.is_null() {
        return None;
    }
    let bytes = unsafe { std::slice::from_raw_parts(at, many) }.to_vec();
    let _ = unsafe { GlobalUnlock(held) };
    Some(bytes)
}

/// The text on the clipboard.
///
/// # Safety
/// The clipboard must be open.
#[cfg(windows)]
unsafe fn words() -> Option<String> {
    use windows::Win32::Foundation::HGLOBAL;
    use windows::Win32::System::DataExchange::GetClipboardData;
    use windows::Win32::System::Memory::{GlobalLock, GlobalUnlock};
    use windows::Win32::System::Ole::CF_UNICODETEXT;

    let handle = unsafe { GetClipboardData(CF_UNICODETEXT.0 as u32) }.ok()?;
    let held = HGLOBAL(handle.0);
    let at = unsafe { GlobalLock(held) } as *const u16;
    if at.is_null() {
        return None;
    }
    // Counted to its terminator, because the handle's size is rounded up to an
    // allocation and says nothing about the text in it.
    let mut many = 0;
    while unsafe { *at.add(many) } != 0 {
        many += 1;
    }
    let said = String::from_utf16_lossy(unsafe { std::slice::from_raw_parts(at, many) });
    let _ = unsafe { GlobalUnlock(held) };
    Some(said)
}

#[cfg(windows)]
fn shut() {
    let _ = unsafe { windows::Win32::System::DataExchange::CloseClipboard() };
}

/// What the system is holding, if it is holding text.
#[cfg(windows)]
pub fn read() -> Option<String> {
    use windows::Win32::Foundation::HGLOBAL;
    use windows::Win32::System::DataExchange::{GetClipboardData, OpenClipboard};
    use windows::Win32::System::Memory::{GlobalLock, GlobalUnlock};
    use windows::Win32::System::Ole::CF_UNICODETEXT;

    // Every path from here has to close it again: the clipboard is one
    // system-wide lock, and a window that opens it and returns leaves every
    // other application unable to copy anything.
    unsafe { OpenClipboard(None) }.ok()?;
    let said = unsafe {
        let Ok(handle) = GetClipboardData(CF_UNICODETEXT.0 as u32) else {
            // Nothing on it, or nothing on it as text -- a copied picture or a
            // copied file both look like this.
            return close(None);
        };
        let held = HGLOBAL(handle.0);
        let at = GlobalLock(held) as *const u16;
        if at.is_null() {
            return close(None);
        }
        // Counted to its terminator, because the handle's size is rounded up
        // to an allocation and says nothing about the text in it.
        let mut many = 0;
        while *at.add(many) != 0 {
            many += 1;
        }
        let said = String::from_utf16_lossy(std::slice::from_raw_parts(at, many));
        let _ = GlobalUnlock(held);
        said
    };
    close(Some(said))
}

/// Closes the clipboard and hands back whatever was read from it.
#[cfg(windows)]
fn close(said: Option<String>) -> Option<String> {
    let _ = unsafe { windows::Win32::System::DataExchange::CloseClipboard() };
    said
}

/// Puts text on the system clipboard.
#[cfg(windows)]
pub fn write(text: &str) {
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::System::DataExchange::{
        CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData,
    };
    use windows::Win32::System::Memory::{
        GLOBAL_ALLOC_FLAGS, GlobalAlloc, GlobalLock, GlobalUnlock,
    };
    use windows::Win32::System::Ole::CF_UNICODETEXT;

    /// `GMEM_MOVEABLE`, which is the only kind of handle the clipboard takes.
    const MOVEABLE: GLOBAL_ALLOC_FLAGS = GLOBAL_ALLOC_FLAGS(0x0002);

    let wide: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
    if unsafe { OpenClipboard(None) }.is_err() {
        return;
    }
    unsafe {
        let _ = EmptyClipboard();
        let Ok(held) = GlobalAlloc(MOVEABLE, wide.len() * size_of::<u16>()) else {
            let _ = CloseClipboard();
            return;
        };
        let at = GlobalLock(held) as *mut u16;
        if at.is_null() {
            let _ = windows::Win32::Foundation::GlobalFree(Some(held));
            let _ = CloseClipboard();
            return;
        }
        std::ptr::copy_nonoverlapping(wide.as_ptr(), at, wide.len());
        let _ = GlobalUnlock(held);
        // The system takes the handle on success and frees it in its own time,
        // so it must not be freed here -- and must be freed here if it refused
        // it, or the window leaks a copy of every copy.
        match SetClipboardData(CF_UNICODETEXT.0 as u32, Some(HANDLE(held.0))) {
            Ok(_) => {}
            Err(_) => {
                let _ = windows::Win32::Foundation::GlobalFree(Some(held));
            }
        }
        let _ = CloseClipboard();
    }
}

/// Nothing to read anywhere else, which leaves the window's own copy of the
/// text as the whole of its clipboard -- as it was everywhere before this.
#[cfg(not(windows))]
pub fn read() -> Option<String> {
    None
}

#[cfg(not(windows))]
pub fn held() -> Option<Held> {
    None
}

#[cfg(not(windows))]
pub fn write(_text: &str) {}

#[cfg(all(test, windows))]
mod tests {
    /// Text put on the clipboard comes back off it.
    ///
    /// Ignored by default, and not because it is slow: it takes over the
    /// machine's clipboard, and a test suite that throws away whatever the
    /// person running it had just copied is a test suite nobody runs twice.
    ///
    ///     cargo test -p matterless-view clipboard -- --ignored
    /// Builds the smallest bitmap that is worth converting: one row of two
    /// pixels, top-down, thirty-two bits deep, with the alpha left at zero as
    /// a screen grab leaves it.
    #[cfg(test)]
    fn a_grab(compression: u32, masks: &[u32]) -> Vec<u8> {
        let mut dib = vec![0u8; 40];
        dib[0..4].copy_from_slice(&40u32.to_le_bytes());
        dib[4..8].copy_from_slice(&2i32.to_le_bytes());
        // Negative: the rows run down the way a screen does.
        dib[8..12].copy_from_slice(&(-1i32).to_le_bytes());
        dib[12..14].copy_from_slice(&1u16.to_le_bytes());
        dib[14..16].copy_from_slice(&32u16.to_le_bytes());
        dib[16..20].copy_from_slice(&compression.to_le_bytes());
        for mask in masks {
            dib.extend_from_slice(&mask.to_le_bytes());
        }
        // Blue, green, red, alpha -- and the alpha never written.
        dib.extend_from_slice(&[0x20, 0x40, 0x60, 0x00]);
        dib.extend_from_slice(&[0x80, 0x90, 0xA0, 0x00]);
        dib
    }

    /// A screen grab is not an empty pane of glass.
    ///
    /// Print Screen leaves a bitmap thirty-two bits deep whose alpha byte was
    /// never written. Encoded as it stands, every pixel is transparent and
    /// the picture that arrives is nothing at all -- so an alpha channel that
    /// is zero everywhere is read as no alpha channel.
    #[test]
    fn a_grab_with_alpha_never_written_is_not_sent_blank() {
        let file = super::as_a_file(&a_grab(0, &[])).expect("a file");
        let png = super::as_a_png(&file).expect("a picture");
        let back = image::load_from_memory_with_format(&png, image::ImageFormat::Png)
            .expect("a picture that reads back")
            .into_rgba8();

        assert_eq!(back.dimensions(), (2, 1));
        assert!(
            back.pixels().all(|pixel| pixel.0[3] == 255),
            "the picture came back transparent"
        );
        assert_eq!(
            back.pixels().map(|pixel| pixel.0[0]).collect::<Vec<_>>(),
            vec![0x60, 0xA0],
            "the colours did not survive, so the rows or the channels are turned around"
        );
    }

    /// The channel masks sit between the small header and the pixels, and
    /// `biClrUsed` does not count them.
    ///
    /// Thirty-two bits deep says "no colour table", so the old sum put the
    /// pixels twelve bytes early and read the masks as the first pixels: the
    /// picture arrived shifted rather than refused, which is the kind of
    /// wrong that gets shipped.
    #[test]
    fn the_masks_before_the_pixels_are_not_read_as_pixels() {
        let masks = [0x00FF_0000, 0x0000_FF00, 0x0000_00FF];
        let file = super::as_a_file(&a_grab(3, &masks)).expect("a file");
        assert_eq!(
            u32::from_le_bytes([file[10], file[11], file[12], file[13]]),
            14 + 40 + 12,
            "the pixels start after the masks"
        );
        assert_eq!(
            super::as_a_file(&a_grab(0, &[]))
                .map(|plain| u32::from_le_bytes([plain[10], plain[11], plain[12], plain[13]])),
            Some(14 + 40),
            "and start right after the header when there are none"
        );
    }

    /// A bitmap needs a file header, and the offset in it has to clear the
    /// colour table or the picture arrives shifted.
    #[test]
    fn a_bitmap_is_given_the_header_that_makes_it_a_file() {
        // Twenty-four bits a pixel: no colour table, whatever the count says.
        let mut dib = vec![0u8; 40 + 12];
        dib[0..4].copy_from_slice(&40u32.to_le_bytes());
        dib[14..16].copy_from_slice(&24u16.to_le_bytes());

        let file = super::as_a_file(&dib).expect("a file");
        assert_eq!(&file[0..2], b"BM");
        assert_eq!(
            u32::from_le_bytes([file[2], file[3], file[4], file[5]]) as usize,
            file.len(),
            "the length it claims is the length it is"
        );
        assert_eq!(
            u32::from_le_bytes([file[10], file[11], file[12], file[13]]),
            54,
            "the pixels start after the two headers and no table"
        );
        assert_eq!(&file[14..], &dib[..], "and the bitmap follows it whole");
    }

    /// Eight bits a pixel with the count left at zero means a table of 256,
    /// which the pixels start after.
    #[test]
    fn a_colour_table_is_counted_even_when_it_is_not_stated() {
        let mut dib = vec![0u8; 40 + 256 * 4 + 4];
        dib[0..4].copy_from_slice(&40u32.to_le_bytes());
        dib[14..16].copy_from_slice(&8u16.to_le_bytes());
        // `biClrUsed` left at zero, which is what a screenshot tool sends.

        let file = super::as_a_file(&dib).expect("a file");
        assert_eq!(
            u32::from_le_bytes([file[10], file[11], file[12], file[13]]) as usize,
            14 + 40 + 256 * 4
        );
    }

    /// A stated count is believed over the depth's own.
    #[test]
    fn a_stated_colour_count_is_the_one_used() {
        let mut dib = vec![0u8; 40 + 2 * 4 + 4];
        dib[0..4].copy_from_slice(&40u32.to_le_bytes());
        dib[14..16].copy_from_slice(&8u16.to_le_bytes());
        dib[32..36].copy_from_slice(&2u32.to_le_bytes());

        let file = super::as_a_file(&dib).expect("a file");
        assert_eq!(
            u32::from_le_bytes([file[10], file[11], file[12], file[13]]) as usize,
            14 + 40 + 2 * 4
        );
    }

    /// Too short to hold a header is refused rather than read past.
    #[test]
    fn something_too_short_to_be_a_bitmap_is_refused() {
        assert_eq!(super::as_a_file(&[0u8; 12]), None);
        assert_eq!(super::as_a_file(&[]), None);
    }

    #[test]
    #[ignore = "takes over the machine's clipboard"]
    fn the_clipboard_carries_text_both_ways() {
        let said = "a stack trace, or a permalink";
        super::write(said);
        assert_eq!(super::read().as_deref(), Some(said));
        // Including the empty one, which is what a reader who cleared the box
        // and copied it puts there.
        super::write("");
        assert_eq!(super::read().as_deref(), Some(""));
    }
}
