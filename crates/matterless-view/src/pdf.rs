//! A PDF's pages, as pictures for the viewer.
//!
//! Drawn by the renderer Windows has carried since 10 (`Windows.Data.Pdf`),
//! so nothing comes with the app to do it. A page at a time, fitted to the
//! window: the viewer shows a page as it shows a picture, and turning one is
//! rendering the next.
//!
//! The last document opened stays open. Turning a page is then a render and
//! nothing else, where opening it again read and parsed the whole file.

/// A page as the viewer wants it: its size, its pixels, and how many pages
/// the document has.
pub struct Page {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
    pub pages: usize,
}

/// Whether this document is the one held open, so its bytes need not be read.
pub fn holds(file_id: &str) -> bool {
    #[cfg(windows)]
    return held()
        .as_ref()
        .is_some_and(|(id, _)| id.as_str() == file_id);
    #[cfg(not(windows))]
    {
        let _ = file_id;
        false
    }
}

/// Renders page `page` of the document `file_id`, as large as fits `within`.
///
/// `bytes` is the file, for a document that is not the one held open; `None`
/// renders the one that is.
pub fn render(
    file_id: &str,
    bytes: Option<&[u8]>,
    page: usize,
    within: (u32, u32),
) -> Result<Page, String> {
    #[cfg(windows)]
    return rendered(file_id, bytes, page, within).map_err(|why| why.message().to_string());
    #[cfg(not(windows))]
    {
        let _ = (file_id, bytes, page, within);
        Err("PDFs are only shown on Windows".to_string())
    }
}

#[cfg(windows)]
fn held() -> std::sync::MutexGuard<'static, Option<(String, windows::Data::Pdf::PdfDocument)>> {
    static HELD: std::sync::Mutex<Option<(String, windows::Data::Pdf::PdfDocument)>> =
        std::sync::Mutex::new(None);
    HELD.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// How many pixels the renderer gives for each one asked of it, in
/// thousandths. It takes a size in device-independent pixels and scales it by
/// the system's DPI, whatever this process's own awareness is: asked for 945
/// pixels across at 180%, it gave 1701. Learned from what it gives back.
#[cfg(windows)]
static SCALED: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(1000);

#[cfg(windows)]
fn rendered(
    file_id: &str,
    bytes: Option<&[u8]>,
    page: usize,
    within: (u32, u32),
) -> windows::core::Result<Page> {
    use windows::Data::Pdf::{PdfDocument, PdfPageRenderOptions};
    use windows::Graphics::Imaging::BitmapEncoder;
    use windows::Storage::Streams::{DataReader, DataWriter, InMemoryRandomAccessStream};

    let mut held = held();
    let document = match (bytes, held.as_ref()) {
        (None, Some((id, document))) if id.as_str() == file_id => document.clone(),
        (Some(bytes), _) => {
            let stream = InMemoryRandomAccessStream::new()?;
            let writer = DataWriter::CreateDataWriter(&stream)?;
            writer.WriteBytes(bytes)?;
            writer.StoreAsync()?.get()?;
            writer.DetachStream()?;
            stream.Seek(0)?;
            let document = PdfDocument::LoadFromStreamAsync(&stream)?.get()?;
            *held = Some((file_id.to_string(), document.clone()));
            document
        }
        _ => {
            return Err(windows::core::Error::from_hresult(
                windows::Win32::Foundation::E_INVALIDARG,
            ));
        }
    };
    drop(held);

    let pages = document.PageCount()? as usize;
    let one = document.GetPage(page.min(pages.saturating_sub(1)) as u32)?;
    // Enlarged as well as shrunk, unlike a picture: a page is drawn, not
    // sampled, and is as sharp at the window's size as at its own.
    let size = one.Size()?;
    let scale = (within.0.max(1) as f32 / size.Width.max(1.0))
        .min(within.1.max(1) as f32 / size.Height.max(1.0));
    let width = ((size.Width * scale) as u32).max(1);
    let height = ((size.Height * scale) as u32).max(1);
    let scaled = SCALED.load(std::sync::atomic::Ordering::Relaxed).max(1);
    let asked = (
        (width as u64 * 1000 / scaled as u64).max(1) as u32,
        (height as u64 * 1000 / scaled as u64).max(1) as u32,
    );
    let options = PdfPageRenderOptions::new()?;
    options.SetDestinationWidth(asked.0)?;
    options.SetDestinationHeight(asked.1)?;
    // A bitmap rather than the PNG it gives by default: nothing to compress
    // only to decompress again a moment later.
    options.SetBitmapEncoderId(BitmapEncoder::BmpEncoderId()?)?;
    let out = InMemoryRandomAccessStream::new()?;
    one.RenderWithOptionsToStreamAsync(&out, &options)?.get()?;
    let length = out.Size()? as u32;
    let reader = DataReader::CreateDataReader(&out.GetInputStreamAt(0)?)?;
    reader.LoadAsync(length)?.get()?;
    let mut encoded = vec![0u8; length as usize];
    reader.ReadBytes(&mut encoded)?;

    let decoded =
        image::load_from_memory_with_format(&encoded, image::ImageFormat::Bmp).map_err(|why| {
            windows::core::Error::new(windows::Win32::Foundation::E_FAIL, why.to_string())
        })?;
    let mut rgba = decoded.to_rgba8();
    // Not the size wanted, so the DPI is not the one last seen: learned for
    // the next page, and this one brought to size.
    if rgba.width().abs_diff(width) > 2 || rgba.height().abs_diff(height) > 2 {
        SCALED.store(
            (rgba.width() as u64 * 1000 / asked.0 as u64).max(1) as u32,
            std::sync::atomic::Ordering::Relaxed,
        );
        rgba = image::imageops::resize(&rgba, width, height, image::imageops::FilterType::Triangle);
    }
    // A page is opaque, whatever the bitmap's fourth channel says.
    for pixel in rgba.pixels_mut() {
        pixel.0[3] = 255;
    }
    Ok(Page {
        width: rgba.width(),
        height: rgba.height(),
        rgba: rgba.into_raw(),
        pages,
    })
}

#[cfg(all(test, windows))]
mod tests {
    /// A PDF of `pages` US Letter pages, the first with a black square in the
    /// middle of an otherwise white page, its cross-references counted out.
    fn letters(pages: usize) -> Vec<u8> {
        let mut objects = vec![
            "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
            format!(
                "<< /Type /Pages /Kids [{}] /Count {pages} >>",
                (0..pages)
                    .map(|at| format!("{} 0 R", 4 + at))
                    .collect::<Vec<_>>()
                    .join(" ")
            ),
            "<< /Length 30 >>\nstream\n0 g 256 346 100 100 re f\nendstream".to_string(),
        ];
        for at in 0..pages {
            let contents = if at == 0 { " /Contents 3 0 R" } else { "" };
            objects.push(format!(
                "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792]{contents} >>"
            ));
        }
        let mut pdf = b"%PDF-1.4\n".to_vec();
        let mut offsets = Vec::new();
        for (at, object) in objects.iter().enumerate() {
            offsets.push(pdf.len());
            pdf.extend(format!("{} 0 obj\n{object}\nendobj\n", at + 1).bytes());
        }
        let xref = pdf.len();
        pdf.extend(format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).bytes());
        for offset in offsets {
            pdf.extend(format!("{offset:010} 00000 n \n").bytes());
        }
        pdf.extend(
            format!(
                "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
                objects.len() + 1
            )
            .bytes(),
        );
        pdf
    }

    /// A page comes back as large as fits, its shape kept, opaque, with what
    /// is drawn on it where it was drawn; and the next page of the same
    /// document is rendered without its bytes.
    #[test]
    fn a_page_is_rendered_to_fit_and_the_document_stays_open() {
        let pdf = letters(2);
        let page = super::render("test-letters", Some(&pdf), 0, (1224, 1224)).expect("renders");
        assert_eq!(page.pages, 2);
        assert_eq!(
            (page.width, page.height),
            (945, 1224),
            "fitted, and enlarged"
        );
        assert_eq!(page.rgba.len() as u32, page.width * page.height * 4);
        let at = |x: u32, y: u32| {
            let index = ((y * page.width + x) * 4) as usize;
            page.rgba[index..index + 4].to_vec()
        };
        assert_eq!(at(10, 10), [255, 255, 255, 255], "white paper");
        assert_eq!(
            at(page.width / 2, page.height / 2),
            [0, 0, 0, 255],
            "the square in the middle"
        );

        assert!(super::holds("test-letters"));
        let second = super::render("test-letters", None, 1, (1224, 1224)).expect("renders");
        assert_eq!(second.pages, 2);
        assert_eq!((second.width, second.height), (945, 1224));
        assert_eq!(
            second.rgba[((second.height / 2 * second.width + second.width / 2) * 4) as usize],
            255,
            "the second page is blank"
        );
    }

    #[test]
    fn what_is_not_a_pdf_says_so() {
        assert!(super::render("test-not", Some(b"hello"), 0, (100, 100)).is_err());
    }
}
