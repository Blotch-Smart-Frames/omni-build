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
//!   first. Each row is padded to a multiple of 32 bytes, its length in a
//!   TLV: a 1024-pixel row needs none, a 180-pixel one has 16 bytes more.
//! - An icon set with every size, as Xcode made them before Xcode 14, has
//!   its smaller sizes packed together into larger images, the renditions
//!   named `ZZZZPackedAsset-<scale>.<...>`. The icon's renditions for those
//!   sizes are links instead (`INLK`, in a TLV): a rectangle, measured
//!   from the bottom left as Core Graphics measures, in the packed image
//!   whose rendition key the link gives as
//!   (attribute, value) pairs, between two zeros, with any attribute it
//!   leaves out 0.
//!
//! OmniBuild doesn't write a catalog of its own. The shell's, which actool
//! compiled, already has the icon in every size iOS needs, and anything
//! else the shell uses. So only the pixels of the icon's bitmaps change,
//! each at its own size, and every key and value actool chose stays, as
//! App Store validation knows them. In a packed image, only the icon's
//! rectangles change: whatever else actool packed in with them stays.

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
/// The TLV that makes a rendition a link into a packed image.
const LINK: u32 = 0x3f2;
/// The TLV with the length of a bitmap's rows, padding included.
const BYTES_PER_ROW: u32 = 0x3ef;
const LINK_TAG: &[u8] = b"KLNI";

/// The catalog `car`, with `icon` in place of each bitmap of the icon
/// named `name`.
pub fn set_icon(car: &[u8], name: &str, icon: &RgbaImage) -> Result<Vec<u8>> {
    let mut bom = Bom::parse(car)?;
    let attributes = key_format(&bom);
    let position = attributes
        .iter()
        .position(|&attribute| attribute == IDENTIFIER)
        .context("its rendition keys have no identifier")?;
    let identifier = asset_identifier(&bom, name)?;
    let renditions = bom.tree("RENDITIONS")?;
    let mut replaced = 0;
    // The rectangles to paint in each packed image, by its rendition key.
    let mut links: Vec<(Vec<u16>, Link)> = Vec::new();
    for &(key, value) in &renditions {
        if Some(Reader::at(bom.block(key), position * 2).u16()) != identifier {
            continue;
        }
        let rendition = Rendition::parse(bom.block(value));
        if rendition.data == SIZES {
            continue;
        }
        if let Some((pairs, link)) = rendition.link() {
            let key = attributes
                .iter()
                .map(|attribute| {
                    pairs
                        .iter()
                        .find(|(paired, _)| u32::from(*paired) == *attribute)
                        .map_or(0, |&(_, value)| value)
                })
                .collect();
            links.push((key, link));
            replaced += 1;
            continue;
        }
        if rendition.data != BITMAP || rendition.pixel_format != ARGB || rendition.width == 0 {
            bail!(
                "OmniBuild can't replace {} in its {name}: it isn't an ARGB bitmap, or a link into one",
                rendition.file
            );
        }
        let pixels = premultiplied_bgra(&icon::sized(icon, rendition.width, rendition.height));
        let stride = rendition.bytes_per_row();
        let padded: Vec<u8> = pixels
            .chunks(rendition.width as usize * 4)
            .flat_map(|row| {
                let mut row = row.to_vec();
                row.resize(stride, 0);
                row
            })
            .collect();
        let replacement = rendition.with_bitmap(&encode(&padded, &rendition));
        bom.set_block(value, replacement);
        replaced += 1;
    }
    if replaced == 0 {
        bail!("it has no icon named {name}");
    }
    for &(key, value) in &renditions {
        let key: Vec<u16> = (0..attributes.len())
            .map(|index| Reader::at(bom.block(key), index * 2).u16())
            .collect();
        let rectangles: Vec<&Link> = links
            .iter()
            .filter(|(packed, _)| *packed == key)
            .map(|(_, link)| link)
            .collect();
        if rectangles.is_empty() {
            continue;
        }
        let packed = Rendition::parse(bom.block(value));
        let mut bgra = packed.pixels().with_context(|| {
            format!(
                "couldn't read {}, which has the icon's smaller sizes",
                packed.file
            )
        })?;
        let stride = packed.bytes_per_row();
        for link in &rectangles {
            if link.x + link.width > packed.width || link.y + link.height > packed.height {
                bail!("its icon's link into {} is outside it", packed.file);
            }
            let pixels = premultiplied_bgra(&icon::sized(icon, link.width, link.height));
            let row = link.width as usize * 4;
            // The rectangle's top row, from the top, as the bitmap's go.
            let top = (packed.height - link.y - link.height) as usize;
            for (y, source) in pixels.chunks(row).enumerate() {
                let at = (top + y) * stride + link.x as usize * 4;
                bgra[at..at + row].copy_from_slice(source);
            }
        }
        let replacement = packed.with_bitmap(&encode(&bgra, &packed));
        bom.set_block(value, replacement);
        links.retain(|(packed, _)| *packed != key);
    }
    if let Some((_, link)) = links.first() {
        bail!(
            "its {name} links to a packed image it doesn't have, for its {}×{} size",
            link.width,
            link.height
        );
    }
    Ok(bom.to_bytes())
}

/// The attributes rendition keys have, in order, as KEYFORMAT lists them
/// after its tag, its version and their count.
fn key_format(bom: &Bom) -> Vec<u32> {
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
        .map(|attribute| u32::from_le_bytes(*attribute))
        .collect()
}

/// A rectangle in a packed image, from its bottom left.
#[derive(Debug)]
struct Link {
    x: u32,
    y: u32,
    width: u32,
    height: u32,
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
    /// Its TLVs, each a type, a length and that many bytes.
    tlvs: &'a [u8],
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
            tlvs: Reader::at(bytes, CSI_HEADER_SIZE).bytes(tlvs),
            width,
            height,
            pixel_format,
            file,
            data_at,
            data: Reader::at(bytes, data_at).bytes(4),
        }
    }

    /// The value of its TLV of this type, if it has one.
    fn tlv(&self, wanted: u32) -> Option<&'a [u8]> {
        let mut tlvs = Reader::at(self.tlvs, 0);
        loop {
            let (kind, length) = (tlvs.u32(), tlvs.u32() as usize);
            let value = tlvs.bytes(length);
            if tlvs.overrun() {
                return None;
            }
            if kind == wanted {
                return Some(value);
            }
        }
    }

    /// The length of its bitmap's rows, padding included.
    fn bytes_per_row(&self) -> usize {
        self.tlv(BYTES_PER_ROW)
            .map_or(self.width as usize * 4, |value| {
                Reader::at(value, 0).u32() as usize
            })
    }

    /// Where the rendition links to, if it's a link into a packed image:
    /// the packed image's key, as (attribute, value) pairs, and the
    /// rectangle in it.
    fn link(&self) -> Option<(Vec<(u16, u16)>, Link)> {
        let mut link = Reader::at(self.tlv(LINK)?, 0);
        if link.bytes(4) != LINK_TAG {
            return None;
        }
        link.skip(4);
        let rectangle = Link {
            x: link.u32(),
            y: link.u32(),
            width: link.u32(),
            height: link.u32(),
        };
        link.skip(2); // The packed image's layout.
        let key = link.u16() as usize;
        // Between a zero before the pairs and one after them.
        let mut pairs = Reader::at(link.bytes(key), 2);
        let pairs: Vec<(u16, u16)> = (0..key.saturating_sub(4) / 4)
            .map(|_| (pairs.u16(), pairs.u16()))
            .collect();
        (!link.overrun()).then_some((pairs, rectangle))
    }

    /// The pixels of a bitmap, premultiplied BGRA, top row first, each row
    /// padded as it is.
    fn pixels(&self) -> Result<Vec<u8>> {
        let mut data = Reader::at(self.bytes, self.data_at);
        let (tag, _, compression, chunks) = (data.bytes(4), data.u32(), data.u32(), data.u32());
        if tag != BITMAP || self.pixel_format != ARGB {
            bail!("it isn't an ARGB bitmap");
        }
        if compression != LZFSE {
            bail!("it's compressed in a way OmniBuild doesn't read ({compression})");
        }
        let mut pixels = Vec::new();
        for _ in 0..chunks {
            if data.bytes(4) != b"KCBC" {
                bail!("it's corrupt or cut short");
            }
            data.skip(12);
            let length = data.u32() as usize;
            lzfse_rust::decode_bytes(data.bytes(length), &mut pixels)
                .map_err(|_| anyhow::anyhow!("it's corrupt or cut short"))?;
        }
        if data.overrun() || pixels.len() != self.bytes_per_row() * self.height as usize {
            bail!("it's corrupt or cut short");
        }
        Ok(pixels)
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

/// The image's pixels, premultiplied BGRA.
fn premultiplied_bgra(pixels: &RgbaImage) -> Vec<u8> {
    pixels
        .pixels()
        .flat_map(|&Rgba([red, green, blue, alpha])| {
            [
                premultiplied(blue, alpha),
                premultiplied(green, alpha),
                premultiplied(red, alpha),
                alpha,
            ]
        })
        .collect()
}

/// A bitmap's data, as actool writes it, from its pixels, premultiplied
/// BGRA, in rows as long as the rendition's: in chunks, LZFSE-compressed.
/// Whether every pixel is opaque is in its flags, which the App Store's
/// check that an icon has no transparency reads: the padding doesn't count.
fn encode(bgra: &[u8], rendition: &Rendition) -> Vec<u8> {
    let (width, height) = (rendition.width, rendition.height);
    let row = rendition.bytes_per_row();
    let opaque = bgra.chunks(row).all(|row| {
        row[..width as usize * 4]
            .chunks(4)
            .all(|pixel| pixel[3] == u8::MAX)
    });
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

    use super::{Link, Rendition, key_format, set_icon};
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

    /// A catalog actool compiled from an icon set with every size, each a
    /// square of its own colour, its size in pixels in the red channel:
    /// see tests/fixtures/README.md.
    const PACKED: &[u8] = include_bytes!("../../tests/fixtures/packed-icons.car");

    /// Each link in a catalog, with the packed image it's into, decoded,
    /// and the length of that image's rows.
    fn links(car: &[u8]) -> Vec<(Link, Vec<u8>, usize)> {
        let bom = Bom::parse(car).unwrap();
        let attributes = key_format(&bom);
        let renditions = bom.tree("RENDITIONS").unwrap();
        let key = |key: u32| -> Vec<u16> {
            (0..attributes.len())
                .map(|index| Reader::at(bom.block(key), index * 2).u16())
                .collect()
        };
        renditions
            .iter()
            .filter_map(|&(_, value)| Rendition::parse(bom.block(value)).link())
            .map(|(pairs, link)| {
                let wanted: Vec<u16> = attributes
                    .iter()
                    .map(|&attribute| {
                        pairs
                            .iter()
                            .find(|(paired, _)| u32::from(*paired) == attribute)
                            .map_or(0, |&(_, value)| value)
                    })
                    .collect();
                let &(_, packed) = renditions
                    .iter()
                    .find(|&&(packed, _)| key(packed) == wanted)
                    .unwrap();
                let packed = Rendition::parse(bom.block(packed));
                (link, packed.pixels().unwrap(), packed.bytes_per_row())
            })
            .collect()
    }

    /// A packed image's pixel, BGRA, in rows of `stride` bytes.
    fn pixel(pixels: &[u8], stride: usize, x: u32, y: u32) -> [u8; 4] {
        let at = y as usize * stride + x as usize * 4;
        pixels[at..at + 4].try_into().unwrap()
    }

    #[test]
    fn finds_the_packed_sizes_where_actool_put_them() {
        let links = links(PACKED);

        assert_eq!(links.len(), 15);
        for (link, pixels, stride) in &links {
            // Its corners: the rows go from the top, and the rectangle from
            // the bottom.
            let rows = u32::try_from(pixels.len() / stride).unwrap();
            let top = rows - link.y - link.height;
            for (x, y) in [
                (link.x, top),
                (link.x + link.width - 1, top + link.height - 1),
            ] {
                let size = u8::try_from(link.width % 256).unwrap();
                assert_eq!(
                    pixel(pixels, *stride, x, y),
                    [0xeb, 0x6f, size, 0xff],
                    "{link:?}"
                );
            }
        }
    }

    #[test]
    fn paints_the_icon_into_the_packed_sizes() {
        // Red on top, blue below.
        let icon = RgbaImage::from_fn(512, 512, |_, y| {
            if y < 256 {
                Rgba([0xff, 0, 0, 0xff])
            } else {
                Rgba([0, 0, 0xff, 0xff])
            }
        });

        let car = set_icon(PACKED, "AppIcon", &icon).unwrap();

        let (before, after) = (links(PACKED), links(&car));
        // Rows from the top; rectangles from the bottom.
        let top = |link: &Link, pixels: &[u8], stride: usize| {
            u32::try_from(pixels.len() / stride).unwrap() - link.y - link.height
        };
        for (link, pixels, stride) in &after {
            let top = top(link, pixels, *stride);
            assert_eq!(
                pixel(pixels, *stride, link.x + 1, top + 1),
                [0, 0, 0xff, 0xff]
            );
            assert_eq!(
                pixel(pixels, *stride, link.x + 1, top + link.height - 2),
                [0xff, 0, 0, 0xff]
            );
        }
        // Outside the icon's rectangles, each packed image is as it was.
        let mut images: Vec<(&Vec<u8>, &Vec<u8>, usize)> = Vec::new();
        for ((_, pixels, stride), (_, shell, _)) in after.iter().zip(&before) {
            if !images.iter().any(|(seen, _, _)| *seen == pixels) {
                images.push((pixels, shell, *stride));
            }
        }
        assert_eq!(images.len(), 5);
        for (pixels, shell, stride) in images {
            let rectangles: Vec<(u32, &Link)> = after
                .iter()
                .filter(|(_, other, _)| other == pixels)
                .map(|(link, _, _)| (top(link, pixels, stride), link))
                .collect();
            let inside = |x: u32, y: u32| {
                rectangles.iter().any(|(top, link)| {
                    (link.x..link.x + link.width).contains(&x)
                        && (*top..top + link.height).contains(&y)
                })
            };
            assert_eq!(pixels.len(), shell.len());
            let (rows, columns) = (pixels.len() / stride, stride / 4);
            for y in 0..u32::try_from(rows).unwrap() {
                for x in (0..u32::try_from(columns).unwrap()).filter(|&x| !inside(x, y)) {
                    assert_eq!(pixel(pixels, stride, x, y), pixel(shell, stride, x, y));
                }
            }
        }
        // The 1024×1024 icon, which isn't packed, too.
        let bom = Bom::parse(&car).unwrap();
        let large = bom
            .tree("RENDITIONS")
            .unwrap()
            .into_iter()
            .map(|(_, value)| Rendition::parse(bom.block(value)))
            .find(|rendition| rendition.width == 1024)
            .unwrap();
        assert_eq!(&large.pixels().unwrap()[..4], [0, 0, 0xff, 0xff]);
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
                "OmniBuild can't replace AppIcon.png in its AppIcon: it isn't an ARGB bitmap, or a link into one",
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

    /// The packed catalog with `change` made to each rendition `which`
    /// picks.
    fn packed_with(which: impl Fn(&Rendition) -> bool, change: impl Fn(&mut Vec<u8>)) -> Vec<u8> {
        let mut bom = Bom::parse(PACKED).unwrap();
        for (_, value) in bom.tree("RENDITIONS").unwrap() {
            if which(&Rendition::parse(bom.block(value))) {
                let mut rendition = bom.block(value).to_vec();
                change(&mut rendition);
                bom.set_block(value, rendition);
            }
        }
        bom.to_bytes()
    }

    /// Where the bytes are in the rendition.
    fn find(rendition: &[u8], bytes: &[u8]) -> usize {
        rendition
            .windows(bytes.len())
            .position(|window| window == bytes)
            .unwrap()
    }

    #[test]
    fn reports_packed_sizes_it_cant_give_the_icon() {
        let icon = RgbaImage::from_pixel(64, 64, Rgba([0xe8, 0x59, 0x0c, 0xff]));
        let packed = |rendition: &Rendition| rendition.file.starts_with("ZZZZPackedAsset");
        let small = |rendition: &Rendition| rendition.file == "icon-40.png";
        // The packed image's bitmap: its compression, its first chunk's
        // tag, and its first chunk's compressed data.
        let bitmap = |at: usize, value: &[u8]| {
            let value = value.to_vec();
            packed_with(packed, move |rendition| {
                let start = find(rendition, b"MLEC") + at;
                rendition[start..start + value.len()].copy_from_slice(&value);
            })
        };
        // The 40×40 link's rectangle, its key, and its tag.
        let link = |at: usize, value: &[u8]| {
            let value = value.to_vec();
            packed_with(small, move |rendition| {
                let start = find(rendition, b"KLNI") + at;
                rendition[start..start + value.len()].copy_from_slice(&value);
            })
        };
        let cases = [
            (
                bitmap(8, &1u32.to_le_bytes()),
                "couldn't read ZZZZPackedAsset-2.1.0-gamut0, which has the icon's smaller sizes: it's compressed in a way OmniBuild doesn't read (1)",
            ),
            (
                bitmap(16, b"XXXX"),
                "couldn't read ZZZZPackedAsset-2.1.0-gamut0, which has the icon's smaller sizes: it's corrupt or cut short",
            ),
            (
                bitmap(36, b"XXXX"),
                "couldn't read ZZZZPackedAsset-2.1.0-gamut0, which has the icon's smaller sizes: it's corrupt or cut short",
            ),
            (
                // Taller than its rows.
                packed_with(packed, |rendition| rendition[16] += 1),
                "couldn't read ZZZZPackedAsset-2.1.0-gamut0, which has the icon's smaller sizes: it's corrupt or cut short",
            ),
            (
                packed_with(packed, |rendition| {
                    rendition[24..28].copy_from_slice(b" 8AG");
                }),
                "couldn't read ZZZZPackedAsset-2.1.0-gamut0, which has the icon's smaller sizes: it isn't an ARGB bitmap",
            ),
            (
                link(8, &1000u32.to_le_bytes()),
                "its icon's link into ZZZZPackedAsset-2.1.0-gamut0 is outside it",
            ),
            (
                // The value of the packed image's name, attribute 1.
                link(32, &99u16.to_le_bytes()),
                "its AppIcon links to a packed image it doesn't have, for its 40×40 size",
            ),
            (
                link(0, b"XXXX"),
                "OmniBuild can't replace icon-40.png in its AppIcon: it isn't an ARGB bitmap, or a link into one",
            ),
        ];

        for (car, message) in cases {
            let error = set_icon(&car, "AppIcon", &icon).unwrap_err();

            assert_eq!(format!("{error:#}"), message);
        }
    }
}
