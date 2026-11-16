//! A PDF's pages drawn as pictures for the Files preview, as the WebView
//! shows them with pdf.js. The pages are drawn on the CPU by `hayro`, in
//! pure Rust, so they look the same on macOS and Windows; the first few
//! are drawn, and the rest are left to the system's application.
use crate::i18n::{fill, tr};

/// How many pages the preview draws.
pub const SHOWN: usize = 12;
/// Pixels per PDF point: sharp on a Retina screen without being slow.
pub const SCALE: f32 = 1.5;
/// The widest a drawn page may be, so an outsized page stays affordable.
const MAX_SIDE: f32 = 2400.;

/// Whether the preview draws this file's pages.
pub fn is_pdf(path: &str) -> bool {
    path.rsplit_once('.')
        .is_some_and(|(_, kind)| kind.eq_ignore_ascii_case("pdf"))
}

/// The pages drawn, as PNG, and how many the document has.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Drawn {
    pub pages: Vec<Vec<u8>>,
    pub total: usize,
}

/// Draws the first `limit` pages of `bytes` at `scale` (white paper).
pub fn draw(bytes: Vec<u8>, limit: usize, scale: f32) -> Result<Drawn, String> {
    use hayro::hayro_syntax::{LoadPdfError, Pdf};
    let pdf = Pdf::new(bytes).map_err(|error| match error {
        LoadPdfError::Decryption(_) => {
            tr("This PDF is protected; open it with the system's application.").to_owned()
        }
        LoadPdfError::Invalid => tr("This PDF could not be read.").to_owned(),
    })?;
    let pages = pdf.pages();
    let total = pages.len();
    let cache = hayro::RenderCache::new();
    let interpreter = hayro::hayro_interpret::InterpreterSettings::default();
    let render = hayro::RenderSettings::default();
    let mut drawn = Vec::new();
    for page in pages.iter().take(limit) {
        let (width, height) = page.render_dimensions();
        // An outsized page is drawn smaller, never larger than asked.
        let fit = (MAX_SIDE / width.max(height).max(1.)).min(scale);
        let pixmap = hayro::render(
            page,
            &cache,
            &interpreter,
            &render,
            &hayro::PixmapSettings {
                x_scale: fit,
                y_scale: fit,
                bg_color: hayro::vello_cpu::color::palette::css::WHITE,
            },
        );
        let png = pixmap
            .into_png()
            .map_err(|error| fill(tr("A page could not be drawn: {}"), &[&error]))?;
        drawn.push(png);
    }
    Ok(Drawn {
        pages: drawn,
        total,
    })
}

/// A PNG's width and height, from its header.
pub fn png_size(png: &[u8]) -> Option<(u32, u32)> {
    if png.get(1..4)? != b"PNG" {
        return None;
    }
    let number = |at: usize| Some(u32::from_be_bytes(png.get(at..at + 4)?.try_into().ok()?));
    Some((number(16)?, number(20)?))
}

/// The line over the pages: how many there are, and how many are shown.
pub fn note(drawn: &Drawn) -> String {
    match (drawn.total, drawn.pages.len()) {
        (1, _) => tr("1 page").to_owned(),
        (total, shown) if shown < total => fill(
            tr("The first {} of {} pages; open it with the system's application for the rest."),
            &[&shown, &total],
        ),
        (total, _) => fill(tr("{} pages"), &[&total]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A PDF of `pages` pages, each a black square on white, with offsets
    /// computed so the cross-reference table is right.
    fn pdf(pages: usize) -> Vec<u8> {
        let mut objects = vec![
            "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
            format!(
                "<< /Type /Pages /Kids [{}] /Count {pages} >>",
                (0..pages)
                    .map(|page| format!("{} 0 R", 3 + page * 2))
                    .collect::<Vec<_>>()
                    .join(" ")
            ),
        ];
        let content = "0 0 0 rg 20 20 60 60 re f";
        for page in 0..pages {
            objects.push(format!(
                "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 100 200] /Contents {} 0 R >>",
                4 + page * 2
            ));
            objects.push(format!(
                "<< /Length {} >>\nstream\n{content}\nendstream",
                content.len()
            ));
        }
        let mut out = b"%PDF-1.4\n".to_vec();
        let mut offsets = Vec::new();
        for (index, object) in objects.iter().enumerate() {
            offsets.push(out.len());
            out.extend(format!("{} 0 obj\n{object}\nendobj\n", index + 1).bytes());
        }
        let xref = out.len();
        out.extend(format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).bytes());
        for offset in offsets {
            out.extend(format!("{offset:010} 00000 n \n").bytes());
        }
        out.extend(
            format!(
                "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
                objects.len() + 1
            )
            .bytes(),
        );
        out
    }

    #[test]
    fn the_first_pages_are_drawn_at_the_scale_asked() {
        let drawn = draw(pdf(3), 2, 2.).unwrap();
        assert_eq!(drawn.total, 3);
        assert_eq!(drawn.pages.len(), 2);
        // A 100 × 200 point page at twice its size.
        assert_eq!(png_size(&drawn.pages[0]), Some((200, 400)));
        assert_eq!(png_size(b"not a png"), None);
        assert_eq!(
            note(&drawn),
            "The first 2 of 3 pages; open it with the system's application for the rest."
        );
        let all = draw(pdf(1), SHOWN, 1.).unwrap();
        assert_eq!(all.pages.len(), 1);
        assert_eq!(note(&all), "1 page");
        let two = draw(pdf(2), SHOWN, 1.).unwrap();
        assert_eq!(note(&two), "2 pages");
    }

    #[test]
    fn what_is_not_a_pdf_is_said_and_not_drawn() {
        assert!(is_pdf("papers/Draft.PDF"));
        assert!(!is_pdf("papers/notes.pdf.txt"));
        assert!(!is_pdf("pdf"));
        assert_eq!(
            draw(b"not a pdf at all".to_vec(), SHOWN, 1.).unwrap_err(),
            "This PDF could not be read."
        );
    }
}
