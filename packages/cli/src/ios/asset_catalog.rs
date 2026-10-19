//! The app's icon in the shell's asset catalog, `Assets.car`: the home
//! screen draws the icon from there, and App Store validation requires it
//! there. Apple doesn't document the format. What OmniBuild relies on was
//! read from what actool writes (Xcode 26 and 27 write the same), checked
//! against Timac's notes on the CAR format and against assetutil, Xcode's
//! tool for listing what a catalog holds:
//!
//! - The catalog is a BOM store (bom.rs). FACETKEYS maps each asset's name
//!   to its attributes, among them an identifier. RENDITIONS maps
//!   rendition keys, whose attributes KEYFORMAT lists, to renditions: each
//!   image of an asset, per size and device. A rendition's key carries its
//!   asset's identifier.
//! - A rendition is a CSI header, its TLVs, then its data, all of it
//!   little-endian. An app icon has a bitmap per size, and a list of its
//!   sizes (`SISM`), which stays as it is since the sizes don't change.
//! - A bitmap's data is `MLEC`: flags (bit 0, it's in chunks; bit 1, every
//!   pixel is opaque), the compression, and the number of chunks. Each
//!   chunk is `KCBC`, two zeros, its number of rows and its length, then
//!   its rows, compressed on their own. actool cuts a bitmap into three
//!   chunks of a third of its rows each, and a fourth for any left over.
//! - Pixels are premultiplied BGRA, which `CoreUI` calls ARGB, top row
//!   first, with no padding.
//!
//! OmniBuild doesn't write a catalog of its own. The shell's, which actool
//! compiled, already has the icon in every size iOS needs, and anything
//! else the shell uses. So only the pixels of the icon's bitmaps change,
//! each at its own size, and every key and value actool chose stays, as
//! App Store validation knows them.

use anyhow::{Context, Result, bail};
use image::{Rgba, RgbaImage};

use super::bom::Bom;
use crate::bytes::Reader;
use crate::icon;

/// The rendition key attribute that holds the identifier of the asset a
/// rendition is of (kCRThemeIdentifierName).
const IDENTIFIER: u32 = 17;
/// A rendition's CSI header, which its TLVs follow.
const CSI_HEADER_SIZE: usize = 184;
/// Where the CSI header has the length of the rendition's data.
const DATA_LENGTH_AT: usize = 180;
/// Premultiplied BGRA, `CoreUI`'s ARGB, as the CSI header spells it.
const ARGB: &[u8] = b"BGRA";
/// A bitmap: its tag, its flags, and its compression.
const BITMAP: &[u8] = b"MLEC";
const IN_CHUNKS: u32 = 1;
const OPAQUE: u32 = 2;
const LZFSE: u32 = 4;
/// The list of an icon's sizes.
const SIZES: &[u8] = b"SISM";

/// The catalog `car`, with `icon` in place of each bitmap of the icon
/// named `name`.
pub fn set_icon(car: &[u8], name: &str, icon: &RgbaImage) -> Result<Vec<u8>> {
    let mut bom = Bom::parse(car)?;
    let position = identifier_position(&bom).context("its rendition keys have no identifier")?;
    let identifier = asset_identifier(&bom, name)?;
    let mut replaced = 0;
    for (key, value) in bom.tree("RENDITIONS")? {
        if Some(Reader::at(bom.block(key), position * 2).u16()) != identifier {
            continue;
        }
        let rendition = Rendition::parse(bom.block(value));
        if rendition.data == SIZES {
            continue;
        }
        if rendition.data != BITMAP || rendition.pixel_format != ARGB || rendition.width == 0 {
            bail!(
                "OmniBuild can't replace {} in its {name}: it isn't an ARGB bitmap of its own. Xcode packs the smaller sizes of an icon in with other images, which an icon of a single size doesn't have",
                rendition.file
            );
        }
        let pixels = icon::sized(icon, rendition.width, rendition.height);
        let replacement = rendition.with_bitmap(&bitmap(&pixels));
        bom.set_block(value, replacement);
        replaced += 1;
    }
    if replaced == 0 {
        bail!("it has no icon named {name}");
    }
    Ok(bom.to_bytes())
}

/// Where a rendition key has the identifier: KEYFORMAT lists the keys'
/// attributes in order, after its tag, its version and their count.
fn identifier_position(bom: &Bom) -> Option<usize> {
    let format = bom
        .var("KEYFORMAT")
        .map_or(&[][..], |block| bom.block(block));
    let mut format = Reader::at(format, 8);
    let count = format.u32() as usize;
    format
        .bytes(count.saturating_mul(4))
        .as_chunks::<4>()
        .0
        .iter()
        .position(|attribute| *attribute == IDENTIFIER.to_le_bytes())
}

/// The identifier of the asset named `name`, if the catalog has one:
/// FACETKEYS gives a hot spot, the number of attributes, then each
/// attribute's ID and value.
fn asset_identifier(bom: &Bom, name: &str) -> Result<Option<u16>> {
    Ok(bom
        .tree("FACETKEYS")?
        .into_iter()
        .find(|&(key, _)| bom.block(key) == name.as_bytes())
        .and_then(|(_, attributes)| {
            let mut attributes = Reader::at(bom.block(attributes), 4);
            let count = attributes.u16();
            (0..count)
                .map(|_| (attributes.u16(), attributes.u16()))
                .find(|&(attribute, _)| u32::from(attribute) == IDENTIFIER)
                .map(|(_, identifier)| identifier)
        }))
}

/// What OmniBuild reads of a rendition. A truncated one reads as empty
/// values, which aren't a bitmap OmniBuild replaces.
struct Rendition<'a> {
    bytes: &'a [u8],
    width: u32,
    height: u32,
    pixel_format: &'a [u8],
    /// The file actool compiled it from, such as `AppIcon.png`.
    file: String,
    /// Where its data starts, after the CSI header and the TLVs.
    data_at: usize,
    /// The data's tag.
    data: &'a [u8],
}

impl<'a> Rendition<'a> {
    fn parse(bytes: &'a [u8]) -> Self {
        // After the tag ("ISTC"), the version and the flags.
        let mut csi = Reader::at(bytes, 12);
        let (width, height) = (csi.u32(), csi.u32());
        csi.skip(4); // The scale, in hundredths.
        let pixel_format = csi.bytes(4);
        csi.skip(12); // The colour space, a modification time and the layout.
        let file = String::from_utf8_lossy(csi.bytes(128))
            .trim_end_matches('\0')
            .to_owned();
        let tlvs = csi.u32() as usize;
        let data_at = CSI_HEADER_SIZE.saturating_add(tlvs);
        Self {
            bytes,
            width,
            height,
            pixel_format,
            file,
            data_at,
            data: Reader::at(bytes, data_at).bytes(4),
        }
    }

    /// The rendition with other data, of a bitmap of the same size. The
    /// CSI header and the TLVs stay: they describe its size, which hasn't
    /// changed.
    fn with_bitmap(&self, data: &[u8]) -> Vec<u8> {
        let mut rendition = self.bytes[..self.data_at].to_vec();
        rendition[DATA_LENGTH_AT..CSI_HEADER_SIZE]
            .copy_from_slice(&u32_of(data.len()).to_le_bytes());
        rendition.extend(data);
        rendition
    }
}

/// A bitmap's data, as actool writes it: in chunks, LZFSE-compressed.
/// Whether every pixel is opaque is in its flags, which the App Store's
/// check that an icon has no transparency reads.
fn bitmap(pixels: &RgbaImage) -> Vec<u8> {
    let (width, height) = pixels.dimensions();
    let opaque = pixels.pixels().all(|pixel| pixel[3] == u8::MAX);
    let bgra: Vec<u8> = pixels
        .pixels()
        .flat_map(|&Rgba([red, green, blue, alpha])| {
            [
                premultiplied(blue, alpha),
                premultiplied(green, alpha),
                premultiplied(red, alpha),
                alpha,
            ]
        })
        .collect();
    let row = width as usize * 4;
    let rows_per_chunk = (height / 3).max(1) as usize;
    let chunks: Vec<&[u8]> = bgra.chunks(rows_per_chunk * row).collect();

    let flags = if opaque {
        IN_CHUNKS | OPAQUE
    } else {
        IN_CHUNKS
    };
    let mut data = BITMAP.to_vec();
    for field in [flags, LZFSE, u32_of(chunks.len())] {
        data.extend(field.to_le_bytes());
    }
    for chunk in chunks {
        let mut compressed = Vec::new();
        lzfse_rust::encode_bytes(chunk, &mut compressed)
            .expect("compressing into memory can't fail");
        data.extend(b"KCBC");
        for field in [0, 0, u32_of(chunk.len() / row), u32_of(compressed.len())] {
            data.extend(field.to_le_bytes());
        }
        data.extend(compressed);
    }
    data
}

/// A colour channel multiplied by its pixel's alpha, rounded.
fn premultiplied(channel: u8, alpha: u8) -> u8 {
    u8::try_from((u16::from(channel) * u16::from(alpha) + 127) / 255)
        .expect("a byte times a byte, over 255, is a byte")
}

fn u32_of(value: usize) -> u32 {
    u32::try_from(value).expect("asset catalogs are under 4 GiB")
}

#[cfg(test)]
pub mod tests {
    use image::{Rgba, RgbaImage};

    use super::set_icon;
    use crate::bytes::Reader;
    use crate::ios::bom::Bom;
    use crate::ios::bom::tests::{node, shell_catalog, store, tree};

    /// The block number of the shell's icon bitmap, the rendition whose
    /// data starts with `MLEC`.
    fn bitmap_block(bom: &Bom) -> u32 {
        bom.tree("RENDITIONS")
            .unwrap()
            .into_iter()
            .map(|(_, value)| value)
            .find(|&value| bom.block(value).windows(4).any(|tag| tag == b"MLEC"))
            .unwrap()
    }

    /// A bitmap rendition's CSI header and TLVs, its flags, each chunk's
    /// number of rows, and its pixels, decompressed.
    pub fn decode_bitmap(rendition: &[u8]) -> (Vec<u8>, u32, Vec<u32>, Vec<u8>) {
        let tlvs = Reader::at(rendition, 168).u32() as usize;
        let mut data = Reader::at(rendition, 184 + tlvs);
        assert_eq!(data.bytes(4), b"MLEC");
        let (flags, compression, chunks) = (data.u32(), data.u32(), data.u32());
        assert_eq!(compression, 4, "LZFSE");
        let (mut rows, mut pixels) = (Vec::new(), Vec::new());
        for _ in 0..chunks {
            assert_eq!(data.bytes(4), b"KCBC");
            assert_eq!((data.u32(), data.u32()), (0, 0));
            rows.push(data.u32());
            let length = data.u32() as usize;
            lzfse_rust::decode_bytes(data.bytes(length), &mut pixels).unwrap();
        }
        assert!(!data.overrun());
        assert_eq!(data.bytes(1), b"", "nothing after the chunks");
        (rendition[..184 + tlvs].to_vec(), flags, rows, pixels)
    }

    /// The icon's bitmap in a catalog, decoded.
    pub fn icon_bitmap(car: &[u8]) -> (Vec<u8>, u32, Vec<u32>, Vec<u8>) {
        let bom = Bom::parse(car).unwrap();
        decode_bitmap(bom.block(bitmap_block(&bom)))
    }

    #[test]
    fn gives_each_bitmap_of_the_icon_the_apps_pixels() {
        let shell = shell_catalog();
        let icon = RgbaImage::from_pixel(512, 512, Rgba([0xe8, 0x59, 0x0c, 0xff]));

        let car = set_icon(&shell, "AppIcon", &icon).unwrap();

        let (head, flags, rows, pixels) = icon_bitmap(&car);
        let (shell_head, shell_flags, shell_rows, _) = icon_bitmap(&shell);
        // In chunks, opaque, and cut as actool cut the shell's 1024×1024.
        assert_eq!((flags, &rows), (3, &vec![341, 341, 341, 1]));
        assert_eq!((shell_flags, &shell_rows), (flags, &rows));
        assert_eq!(pixels, [0x0c, 0x59, 0xe8, 0xff].repeat(1024 * 1024));
        // The CSI header and TLVs are the shell's, but for the data's length.
        assert_eq!(head[..180], shell_head[..180]);
        assert_eq!(head[184..], shell_head[184..]);
        // Every other block is as it was, the list of sizes among them.
        let (bom, shell) = (Bom::parse(&car).unwrap(), Bom::parse(&shell).unwrap());
        let replaced = bitmap_block(&bom);
        for block in (0..256).filter(|&block| block != replaced) {
            assert_eq!(bom.block(block), shell.block(block), "block {block}");
        }
    }

    #[test]
    fn premultiplies_a_translucent_icon_and_says_it_isnt_opaque() {
        let icon = RgbaImage::from_pixel(1024, 1024, Rgba([200, 100, 50, 128]));

        let car = set_icon(&shell_catalog(), "AppIcon", &icon).unwrap();

        let (_, flags, _, pixels) = icon_bitmap(&car);
        assert_eq!(flags, 1);
        // As actool writes the same pixel.
        assert_eq!(pixels[..4], [25, 50, 100, 128]);
    }

    #[test]
    fn reports_a_catalog_it_cant_give_the_icon() {
        let icon = RgbaImage::new(1, 1);
        let mut key_format = b"tmfk".to_vec();
        for field in [0u32, 1, 17] {
            key_format.extend(field.to_le_bytes());
        }
        let mut grey = Bom::parse(&shell_catalog()).unwrap();
        let block = bitmap_block(&grey);
        let mut rendition = grey.block(block).to_vec();
        rendition[24..28].copy_from_slice(b" 8AG");
        grey.set_block(block, rendition);
        let cases = [
            (b"not a catalog".to_vec(), "it isn't a BOM store"),
            (
                store(&[], &[]).to_bytes(),
                "its rendition keys have no identifier",
            ),
            (
                store(&[&key_format], &[("KEYFORMAT", 1)]).to_bytes(),
                "it has no FACETKEYS",
            ),
            (
                store(
                    &[&key_format, &tree(3), &node(true, &[])],
                    &[("KEYFORMAT", 1), ("FACETKEYS", 2)],
                )
                .to_bytes(),
                "it has no RENDITIONS",
            ),
            (
                grey.to_bytes(),
                "OmniBuild can't replace AppIcon.png in its AppIcon: it isn't an ARGB bitmap of its own. ",
            ),
        ];

        for (car, message) in cases {
            let error = set_icon(&car, "AppIcon", &icon).unwrap_err();

            assert!(error.to_string().starts_with(message), "{error}");
        }
        assert_eq!(
            set_icon(&shell_catalog(), "Missing", &icon)
                .unwrap_err()
                .to_string(),
            "it has no icon named Missing"
        );
    }
}
