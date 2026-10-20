//! The app's icon in an Android shell. Before Android 8.0, a launcher shows
//! a bitmap, one per density. From 8.0 on, it shows an adaptive icon, if the
//! shell has one: XML naming a background and a foreground layer, and from
//! Android 13 a monochrome one, which the launcher puts together in its own
//! shape. Android Studio's templates make both, the bitmaps as WebP and the
//! layers as vector drawables.
//!
//! The bitmaps get the app's icon, each at its size. The layers get what
//! the app config gives (`android.adaptiveIcon`), or the app's icon whole,
//! inside the launcher's shape. A layer that's a vector, or a colour, can't
//! hold an image, so its resource is pointed at a PNG of the whole layer
//! instead, and the vector goes: bundletool refuses an app bundle with a
//! file its resource table doesn't name.

use anyhow::{Context, Result, bail};
use image::{Rgba, RgbaImage};

use super::Layout;
use super::binary_xml::BinaryXml;
use super::manifest::{Document, Value as Attribute};
use super::proto_xml::ProtoXml;
use super::resources::{Resources, Value};
use crate::config::{AdaptiveIcon, App, Background};
use crate::icon;
use crate::zip::{Entry, Method};

/// android:drawable, the attribute an adaptive icon's layer names its
/// drawable with.
const DRAWABLE: u32 = 0x0101_0199;
/// The size of a layer OmniBuild draws in place of a vector or a colour:
/// 108dp at xxxhdpi, the highest density. Its resource has no density, so
/// Android scales it to each screen's, from as sharp as it gets.
const LAYER_SIZE: u32 = 432;

/// An adaptive icon's layers, by the element that names each.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Layer {
    Foreground,
    Background,
    Monochrome,
}

impl Layer {
    fn of(element: &str) -> Option<Self> {
        match element {
            "foreground" => Some(Self::Foreground),
            "background" => Some(Self::Background),
            "monochrome" => Some(Self::Monochrome),
            _ => None,
        }
    }
}

/// What a layer is filled with.
enum Fill {
    Image(RgbaImage),
    Color([u8; 3]),
}

impl Fill {
    /// The fill as an image `width`×`height`.
    fn image(&self, width: u32, height: u32) -> RgbaImage {
        match self {
            Self::Image(image) => icon::sized(image, width, height),
            Self::Color([red, green, blue]) => {
                RgbaImage::from_pixel(width, height, Rgba([*red, *green, *blue, 0xff]))
            }
        }
    }
}

/// Gives the app its icon in each of the shell's icon resources (`icons`):
/// the application's icon and round icon.
pub fn set(
    entries: &mut Vec<Entry>,
    layout: &Layout,
    resources: &mut dyn Resources,
    icons: &[u32],
    app: &App,
) -> Result<()> {
    let Some(path) = &app.icon else {
        return Ok(());
    };
    let icon = icon::load(path)?;
    // The bitmaps, and the adaptive icons, with the layers they name.
    let mut bitmaps = Vec::new();
    let mut adaptive_icons = Vec::new();
    let mut layers: Vec<(u32, Layer)> = Vec::new();
    for &id in icons {
        let values = resources
            .values(id)
            .with_context(|| format!("couldn't find the shell's icon in {}", layout.resources))?;
        for value in values {
            match value {
                Value::File(file) if is_xml(&file) => {
                    let xml = document(entries, layout, &file)?;
                    layers.extend(layers_of(xml.as_ref(), &file)?);
                    adaptive_icons.push(file);
                }
                Value::File(file) => bitmaps.push(file),
                _ => bail!("the shell's icon {id:#010x} isn't a file in its package"),
            }
        }
    }
    bitmaps.sort();
    bitmaps.dedup();
    for file in bitmaps {
        replace(entries, layout, &file, |bitmap| {
            icon::resized(&icon, bitmap)
        })?;
    }
    adaptive_icons.sort();
    adaptive_icons.dedup();
    if adaptive_icons.is_empty() {
        return Ok(());
    }

    let config = app.android.adaptive_icon.as_ref();
    let fills = fills(&icon, config)?;
    layers.sort_unstable();
    layers.dedup();
    // A drawable two layers share takes the first's fill: the template's
    // monochrome layer is its foreground's drawable.
    let mut done: Vec<u32> = Vec::new();
    let mut displaced = Vec::new();
    for &(drawable, layer) in &layers {
        let shares_with = |other: Layer| layers.contains(&(drawable, other));
        if layer == Layer::Monochrome
            && shares_with(Layer::Foreground)
            && config.is_some_and(|config| config.monochrome.is_some())
        {
            bail!(
                "the shell's adaptive icon draws its foreground and monochrome layers with one drawable, so they can't differ: leave out the monochrome layer"
            );
        }
        if done.contains(&drawable) {
            continue;
        }
        done.push(drawable);
        if let Some(fill) = &fills[layer as usize] {
            displaced.extend(set_layer(entries, layout, resources, drawable, fill)?);
        }
    }
    let named = resources.files()?;
    displaced.retain(|file| !named.contains(file));
    entries.retain(|entry| {
        entry
            .name
            .strip_prefix(layout.base)
            .is_none_or(|file| !displaced.iter().any(|displaced| displaced == file))
    });
    // Without a monochrome layer of the app's, the shell's goes, and
    // Android tints the icon its own way.
    if fills[Layer::Monochrome as usize].is_none() {
        let read = "the adaptive icon was read once already";
        for file in &adaptive_icons {
            let mut xml = document(entries, layout, file).expect(read);
            xml.remove_children("monochrome", &|_| true);
            let bytes = xml.to_bytes();
            replace(entries, layout, file, |_| Ok(bytes.clone())).expect(read);
        }
    }
    Ok(())
}

/// Each layer's fill, in `Layer`'s order: the app config's, or without
/// one, the app's icon whole as the background, nothing in front of it,
/// and no monochrome layer.
fn fills(icon: &RgbaImage, config: Option<&AdaptiveIcon>) -> Result<[Option<Fill>; 3]> {
    let Some(config) = config else {
        return Ok([
            Some(Fill::Image(RgbaImage::new(LAYER_SIZE, LAYER_SIZE))),
            Some(Fill::Image(icon::fitted(icon, LAYER_SIZE))),
            None,
        ]);
    };
    let foreground = icon::load(&config.foreground)?;
    let background = match &config.background {
        Background::Color(color) => Fill::Color(*color),
        Background::Image(path) => Fill::Image(icon::load(path)?),
    };
    let monochrome = match &config.monochrome {
        Some(path) => icon::load(path)?,
        None => foreground.clone(),
    };
    Ok([
        Some(Fill::Image(foreground)),
        Some(background),
        Some(Fill::Image(monochrome)),
    ])
}

/// The layers an adaptive icon names: each drawable, by its resource ID.
fn layers_of(xml: &dyn Document, file: &str) -> Result<Vec<(u32, Layer)>> {
    let layers: Vec<(u32, Layer)> = xml
        .attributes()
        .iter()
        .filter_map(
            |attribute| match (attribute.resource_id, &attribute.value) {
                (Some(DRAWABLE), Attribute::Reference(id)) => {
                    Layer::of(&attribute.element).map(|layer| (*id, layer))
                }
                _ => None,
            },
        )
        .collect();
    if ![Layer::Foreground, Layer::Background]
        .iter()
        .all(|layer| layers.iter().any(|(_, named)| named == layer))
    {
        bail!(
            "OmniBuild can't give the app its icon in the shell's {file}: it isn't an adaptive icon whose foreground and background each name a drawable"
        );
    }
    Ok(layers)
}

/// Fills the drawable `drawable` with `fill`. Bitmaps, one per density, as
/// Android Studio makes a layer from an image, are each redrawn at their
/// size; a colour stays a colour, given one; and anything else becomes a
/// PNG of the whole layer. Returns the files the drawable no longer names.
fn set_layer(
    entries: &mut Vec<Entry>,
    layout: &Layout,
    resources: &mut dyn Resources,
    drawable: u32,
    fill: &Fill,
) -> Result<Vec<String>> {
    let values = resources.values(drawable)?;
    let bitmaps: Vec<&str> = values
        .iter()
        .filter_map(|value| match value {
            Value::File(path) if !is_xml(path) => Some(path.as_str()),
            _ => None,
        })
        .collect();
    let colors = values.iter().all(|value| matches!(value, Value::Color(_)));
    if !values.is_empty() && bitmaps.len() == values.len() {
        for path in bitmaps {
            replace(entries, layout, path, |bitmap| {
                let image = fill.image(LAYER_SIZE, LAYER_SIZE);
                icon::resized(&image, bitmap)
            })?;
        }
        Ok(Vec::new())
    } else if let (Fill::Color([red, green, blue]), true) = (fill, colors) {
        let argb = u32::from_be_bytes([0xff, *red, *green, *blue]);
        resources
            .set_color(drawable, argb)
            .expect("its values were read once already");
        Ok(Vec::new())
    } else {
        let path = format!("res/drawable/omnibuild_{drawable:08x}.png");
        let png = icon::png(&fill.image(LAYER_SIZE, LAYER_SIZE));
        entries.push(Entry::new(
            &format!("{}{path}", layout.base),
            Method::Stored,
            &png,
        ));
        resources
            .set_file(drawable, &path)
            .expect("its values were read once already");
        Ok(values
            .into_iter()
            .filter_map(|value| match value {
                Value::File(displaced) => Some(displaced),
                _ => None,
            })
            .collect())
    }
}

/// Whether a file's compiled XML, as aapt2 names them, rather than an
/// image.
fn is_xml(path: &str) -> bool {
    path.rsplit_once('.')
        .is_some_and(|(_, extension)| extension.eq_ignore_ascii_case("xml"))
}

/// The compiled XML file `file`, from the package: binary XML in an APK,
/// protobuf in an AAB.
fn document(entries: &[Entry], layout: &Layout, file: &str) -> Result<Box<dyn Document>> {
    let name = format!("{}{file}", layout.base);
    let contents = entries
        .iter()
        .find(|entry| entry.name == name)
        .with_context(|| format!("the shell has no {name}, which its icon names"))?
        .contents()?;
    let xml: Result<Box<dyn Document>> = if layout.bundle {
        ProtoXml::parse(&contents).map(|xml| Box::new(xml) as Box<dyn Document>)
    } else {
        BinaryXml::parse(&contents).map(|xml| Box::new(xml) as Box<dyn Document>)
    };
    xml.with_context(|| format!("couldn't read the shell's {name}"))
}

/// Replaces the file's contents with what `change` makes of them.
fn replace(
    entries: &mut [Entry],
    layout: &Layout,
    file: &str,
    change: impl FnOnce(&[u8]) -> Result<Vec<u8>>,
) -> Result<()> {
    let name = format!("{}{file}", layout.base);
    let entry = entries
        .iter_mut()
        .find(|entry| entry.name == name)
        .with_context(|| format!("the shell has no {name}, which its icon names"))?;
    let contents = entry
        .contents()
        .and_then(|contents| change(&contents))
        .with_context(|| format!("couldn't replace the shell's icon {name}"))?;
    *entry = entry.with_contents(&contents);
    Ok(())
}
