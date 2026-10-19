//! The app's icon, in each size the shell has its icon in, on either
//! platform.

use std::fs;
use std::io::Cursor;
use std::path::Path;

use anyhow::{Context, Result, bail};
use image::imageops::{self, FilterType};
use image::{ImageFormat, RgbaImage};

use crate::bytes::Reader;

const PNG_SIGNATURE: &[u8] = b"\x89PNG\r\n\x1a\n";

/// Reads the app's icon: a square PNG.
pub fn load(path: &Path) -> Result<RgbaImage> {
    let icon = fs::read(path)
        .map_err(anyhow::Error::from)
        .and_then(|png| Ok(image::load_from_memory_with_format(&png, ImageFormat::Png)?))
        .with_context(|| format!("couldn't read the icon {} as a PNG", path.display()))?
        .into_rgba8();
    if icon.width() != icon.height() {
        bail!(
            "the icon {} isn't square: it's {}×{}",
            path.display(),
            icon.width(),
            icon.height()
        );
    }
    Ok(icon)
}

/// The icon at `width`×`height`, the size of an icon of the shell's it
/// replaces. Each of those is one density's or one device's size, so the
/// app's icon is drawn as sharp as the shell's was, with no density
/// arithmetic.
pub fn sized(icon: &RgbaImage, width: u32, height: u32) -> RgbaImage {
    imageops::resize(icon, width, height, FilterType::Lanczos3)
}

/// The icon as a PNG the size of `shell_icon`, the PNG it replaces. A
/// standard PNG, even where the shell's is Apple's variant, which iOS reads
/// too.
pub fn resized(icon: &RgbaImage, shell_icon: &[u8]) -> Result<Vec<u8>> {
    let (width, height) = dimensions(shell_icon)
        .context("it isn't a PNG, and OmniBuild only replaces PNG icons so far")?;
    let mut png = Vec::new();
    sized(icon, width, height)
        .write_to(&mut Cursor::new(&mut png), ImageFormat::Png)
        .expect("encoding a PNG into memory can't fail");
    Ok(png)
}

/// A PNG's size, from its header (`IHDR`). Only the header is read: Xcode
/// writes an iOS build's PNGs in Apple's variant (`CgBI`), which standard
/// decoders refuse, with a chunk of its own before the header.
fn dimensions(png: &[u8]) -> Option<(u32, u32)> {
    let mut chunks = Reader::at(png.strip_prefix(PNG_SIGNATURE)?, 0);
    for _ in 0..2 {
        let length = chunks.u32_be() as usize;
        if chunks.bytes(4) == b"IHDR" {
            let size = (chunks.u32_be(), chunks.u32_be());
            return (!chunks.overrun()).then_some(size);
        }
        // The chunk's data and its CRC.
        chunks.skip(length.saturating_add(4));
    }
    None
}

#[cfg(test)]
pub mod tests {
    use std::fs;
    use std::io::Cursor;

    use image::{ImageFormat, Rgba, RgbaImage};

    use super::{PNG_SIGNATURE, load, resized};

    /// A PNG of one colour.
    pub fn solid_png(width: u32, height: u32, colour: [u8; 4]) -> Vec<u8> {
        let mut png = Vec::new();
        RgbaImage::from_pixel(width, height, Rgba(colour))
            .write_to(&mut Cursor::new(&mut png), ImageFormat::Png)
            .unwrap();
        png
    }

    #[test]
    fn resizes_the_icon_to_the_shells() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("icon.png");
        fs::write(&path, solid_png(512, 512, [0xe8, 0x59, 0x0c, 0xff])).unwrap();

        let png = resized(&load(&path).unwrap(), &solid_png(192, 192, [0; 4])).unwrap();

        let image = image::load_from_memory(&png).unwrap().into_rgba8();
        assert_eq!(image.dimensions(), (192, 192));
        assert_eq!(image.get_pixel(96, 96), &Rgba([0xe8, 0x59, 0x0c, 0xff]));
    }

    /// A PNG in Apple's variant, as Xcode writes an iOS build's: a `CgBI`
    /// chunk, then the header, cut short after it.
    fn cgbi_png(width: u32, height: u32) -> Vec<u8> {
        let mut png = PNG_SIGNATURE.to_vec();
        png.extend([0, 0, 0, 4]);
        png.extend(b"CgBI\x50\x00\x20\x06");
        png.extend([0; 4]);
        png.extend([0, 0, 0, 13]);
        png.extend(b"IHDR");
        png.extend(width.to_be_bytes());
        png.extend(height.to_be_bytes());
        png
    }

    #[test]
    fn resizes_the_icon_to_the_size_of_apples_png_variant() {
        let icon = RgbaImage::from_pixel(64, 64, Rgba([0xe8, 0x59, 0x0c, 0xff]));

        let png = resized(&icon, &cgbi_png(120, 120)).unwrap();

        let image = image::load_from_memory(&png).unwrap().into_rgba8();
        assert_eq!(image.dimensions(), (120, 120));
    }

    #[test]
    fn rejects_a_png_without_a_header_where_it_belongs() {
        let mut two_other_chunks = PNG_SIGNATURE.to_vec();
        for _ in 0..2 {
            two_other_chunks.extend([0, 0, 0, 0]);
            two_other_chunks.extend(b"tEXt");
            two_other_chunks.extend([0; 4]);
        }
        let cut_short = cgbi_png(120, 120);

        for png in [two_other_chunks, cut_short[..cut_short.len() - 2].to_vec()] {
            assert!(resized(&RgbaImage::new(1, 1), &png).is_err());
        }
    }

    #[test]
    fn rejects_an_icon_that_isnt_a_square_png() {
        let dir = tempfile::tempdir().unwrap();
        let (text, wide) = (dir.path().join("icon.txt"), dir.path().join("wide.png"));
        fs::write(&text, "not a PNG").unwrap();
        fs::write(&wide, solid_png(2, 1, [0; 4])).unwrap();

        let missing = load(&dir.path().join("missing.png")).unwrap_err();
        let not_png = load(&text).unwrap_err();
        let not_square = load(&wide).unwrap_err();

        assert!(missing.to_string().starts_with("couldn't read the icon "));
        assert!(not_png.to_string().ends_with("icon.txt as a PNG"));
        assert!(
            not_square
                .to_string()
                .ends_with("wide.png isn't square: it's 2×1")
        );
        assert_eq!(
            resized(&RgbaImage::new(1, 1), b"<adaptive-icon/>")
                .unwrap_err()
                .to_string(),
            "it isn't a PNG, and OmniBuild only replaces PNG icons so far"
        );
    }
}
