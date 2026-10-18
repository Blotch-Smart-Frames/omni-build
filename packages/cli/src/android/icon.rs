//! The app's icon, in each size the shell has its icon in.

use std::fs;
use std::io::Cursor;
use std::path::Path;

use anyhow::{Context, Result, bail};
use image::imageops::{self, FilterType};
use image::{ImageFormat, ImageReader, RgbaImage};

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

/// The icon as a PNG the size of `shell_icon`, the PNG it replaces. Each of
/// the shell's PNGs is one density's size, so the app's icon is drawn as
/// sharp as the shell's was, with no density arithmetic.
pub fn resized(icon: &RgbaImage, shell_icon: &[u8]) -> Result<Vec<u8>> {
    let (width, height) = ImageReader::with_format(Cursor::new(shell_icon), ImageFormat::Png)
        .into_dimensions()
        .ok()
        .context(
            "it isn't a PNG, and OmniBuild only replaces PNG icons so far, not adaptive ones",
        )?;
    let mut png = Vec::new();
    imageops::resize(icon, width, height, FilterType::Lanczos3)
        .write_to(&mut Cursor::new(&mut png), ImageFormat::Png)
        .expect("encoding a PNG into memory can't fail");
    Ok(png)
}

#[cfg(test)]
pub mod tests {
    use std::fs;
    use std::io::Cursor;

    use image::{ImageFormat, Rgba, RgbaImage};

    use super::{load, resized};

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
            "it isn't a PNG, and OmniBuild only replaces PNG icons so far, not adaptive ones"
        );
    }
}
