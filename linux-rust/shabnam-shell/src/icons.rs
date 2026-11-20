//! Application icons as rounded-square tiles in the manner of macOS: found by name in the icon
//! directories or given as a file path (PNG or SVG). A picture that is a tile already is shown as
//! it is, a full square gets its corners rounded, a free-standing shape goes on a light plate,
//! and when there is no picture a coloured tile with the app's first letter stands in.

use crate::draw::{Canvas, Color, Fonts, Image};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;

const PLATE: Color = [0xf1, 0xf3, 0xf4, 0xff];
const LETTER_COLORS: [Color; 6] = [
    [0x12, 0xa5, 0xa2, 0xff],
    [0x42, 0x85, 0xf4, 0xff],
    [0xdb, 0x6e, 0x3c, 0xff],
    [0x8e, 0x5b, 0xd0, 0xff],
    [0x3c, 0x9a, 0x5f, 0xff],
    [0xc2, 0x52, 0x7a, 0xff],
];

pub struct Icons {
    /// Icon name -> best file found for it.
    index: HashMap<String, PathBuf>,
    /// (cache key, pixel size) -> finished tile.
    tiles: HashMap<(String, u32), Rc<Image>>,
}

fn theme_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if let Some(home) = std::env::var_os("HOME") {
        roots.push(Path::new(&home).join(".local/share/icons/hicolor"));
    }
    roots.push("/usr/local/share/icons/hicolor".into());
    roots.push("/usr/share/icons/hicolor".into());
    roots
}

/// How good a file is for a tile of some 50-70 pixels: a large bitmap first (cheapest to load and
/// sharp), then a vector drawing, then small bitmaps.
fn score(size_dir: &str, extension: &str) -> u32 {
    let pixels = size_dir.split('x').next().and_then(|n| n.parse::<u32>().ok());
    match (extension, pixels) {
        ("png", Some(n)) if n >= 96 => 3000u32.saturating_sub(n),
        ("png", Some(n)) if n >= 64 => 2000 + n,
        ("svg", _) => 1500,
        ("png", Some(n)) => n,
        ("png", None) => 1400,
        _ => 0,
    }
}

impl Icons {
    pub fn new() -> Icons {
        let mut best: HashMap<String, (u32, PathBuf)> = HashMap::new();
        let mut offer = |dir: &Path, size_dir: &str| {
            let Ok(entries) = std::fs::read_dir(dir) else { return };
            for entry in entries.flatten() {
                let path = entry.path();
                let (Some(stem), Some(ext)) = (path.file_stem().and_then(|s| s.to_str()), path.extension().and_then(|e| e.to_str())) else {
                    continue;
                };
                let points = score(size_dir, ext);
                if points > 0 && best.get(stem).map_or(true, |(p, _)| points > *p) {
                    best.insert(stem.to_string(), (points, path.clone()));
                }
            }
        };
        for root in theme_roots() {
            let Ok(sizes) = std::fs::read_dir(&root) else { continue };
            for size in sizes.flatten() {
                let name = size.file_name().to_string_lossy().into_owned();
                offer(&size.path().join("apps"), &name);
            }
        }
        offer(Path::new("/usr/share/pixmaps"), "");
        Icons { index: best.into_iter().map(|(name, (_, path))| (name, path)).collect(), tiles: HashMap::new() }
    }

    /// True when an icon file of this name was found.
    pub fn has(&self, name: &str) -> bool {
        plain_name(name) && self.index.contains_key(name)
    }

    /// Forgets the finished tiles: a tile without a picture shows the first letter of the
    /// app's name, and names change with the language.
    pub fn forget(&mut self) {
        self.tiles.clear();
    }

    /// The tile for an app, `px` buffer pixels across. `key` identifies the app in the cache,
    /// `label` supplies the fallback letter. `icon` is an icon name; it may be the path of a
    /// picture file only when `from_desktop_file` says it is the Icon= value of an installed
    /// desktop file. Anything a running program can choose (a window's app id) is only ever
    /// looked up by name among the installed icons, never opened as a path.
    pub fn tile(&mut self, key: &str, icon: &str, from_desktop_file: bool, label: &str, px: u32, fonts: &mut Fonts) -> Rc<Image> {
        let px = px.clamp(8, 512);
        if let Some(tile) = self.tiles.get(&(key.to_string(), px)) {
            return tile.clone();
        }
        let path = if from_desktop_file && icon.starts_with('/') {
            Some(PathBuf::from(icon))
        } else if plain_name(icon) {
            self.index.get(icon).cloned()
        } else {
            None
        };
        // A damaged icon file must never take the shell down with it.
        // A picture that came out empty (an SVG using something the renderer lacks) counts as none.
        let picture = path
            .and_then(|p| std::panic::catch_unwind(|| load(&p, px)).ok().flatten())
            .filter(|picture| picture.rgba.chunks_exact(4).any(|p| p[3] > 16));
        let tile = Rc::new(match picture {
            Some(picture) => square_tile(&picture, px),
            None => letter_tile(key, label, px, fonts),
        });
        self.tiles.insert((key.to_string(), px), tile.clone());
        tile
    }
}

/// An icon name that can only be a name: not empty, no directory part, not a hidden file.
fn plain_name(name: &str) -> bool {
    !name.is_empty() && name.len() <= 128 && !name.contains('/') && !name.starts_with('.') && !name.contains('\0')
}

fn load(path: &Path, px: u32) -> Option<Image> {
    let data = crate::draw::read_capped(path)?;
    if path.extension().and_then(|e| e.to_str()) != Some("svg") {
        return Image::from_png(&data);
    }
    let tree = resvg::usvg::Tree::from_data(&data, &resvg::usvg::Options::default()).ok()?;
    let size = tree.size();
    let scale = px as f32 / size.width().max(size.height()).max(1.0);
    let mut pixmap = resvg::tiny_skia::Pixmap::new(px, px)?;
    let (dx, dy) = ((px as f32 - size.width() * scale) / 2.0, (px as f32 - size.height() * scale) / 2.0);
    let transform = resvg::tiny_skia::Transform::from_scale(scale, scale).post_translate(dx, dy);
    resvg::render(&tree, transform, &mut pixmap.as_mut());
    Some(Image { w: px, h: px, rgba: pixmap.take() })
}

/// Canvas bytes are B G R A, pictures are R G B A.
fn to_image(mut bytes: Vec<u8>, px: u32) -> Image {
    bytes.chunks_exact_mut(4).for_each(|p| p.swap(0, 2));
    Image { w: px, h: px, rgba: bytes }
}

/// Corner radius of a tile as a share of its side (the look of macOS icons).
const CORNER: f32 = 0.22;

/// Multiplies the tile's pixels by a rounded-square mask.
fn mask_rounded(bytes: &mut [u8], px: u32) {
    let (half, r) = (px as f32 / 2.0, px as f32 * CORNER);
    for (i, p) in bytes.chunks_exact_mut(4).enumerate() {
        let (qx, qy) = (((i as u32 % px) as f32 + 0.5 - half).abs() - (half - r), ((i as u32 / px) as f32 + 0.5 - half).abs() - (half - r));
        let dist = if qx > 0.0 && qy > 0.0 { (qx * qx + qy * qy).sqrt() - r } else { qx.max(qy) - r };
        let keep = (0.5 - dist).clamp(0.0, 1.0);
        if keep < 1.0 {
            p.iter_mut().for_each(|v| *v = (*v as f32 * keep) as u8);
        }
    }
}

/// How an icon picture becomes a tile.
#[derive(PartialEq, Debug)]
enum Kind {
    /// Opaque to its corners: shown whole, its corners rounded off.
    Full,
    /// Already a tile of its own (a rounded square or the like with clear corners): shown as it is.
    Tile,
    /// A free-standing shape: put on a light rounded-square plate.
    Shape,
}

fn kind_of(picture: &Image) -> Kind {
    if picture.w == 0 || picture.h == 0 {
        return Kind::Shape;
    }
    let solid = |x: u32, y: u32| picture.rgba[((y * picture.w + x) * 4 + 3) as usize] > 250;
    let (right, bottom) = (picture.w - 1, picture.h - 1);
    if solid(0, 0) && solid(right, 0) && solid(0, bottom) && solid(right, bottom) {
        return Kind::Full;
    }
    // A picture that is mostly solid is a tile already; a glyph or logo covers far less.
    let (mut hits, mut total) = (0u32, 0u32);
    for gy in 0..24 {
        for gx in 0..24 {
            total += 1;
            hits += solid((gx * 2 + 1) * picture.w / 48, (gy * 2 + 1) * picture.h / 48) as u32;
        }
    }
    if hits * 100 >= total * 55 {
        Kind::Tile
    } else {
        Kind::Shape
    }
}

fn square_tile(picture: &Image, px: u32) -> Image {
    let mut bytes = vec![0u8; (px * px * 4) as usize];
    let mut canvas = Canvas::new(&mut bytes, px as i32, px as i32, 1.0);
    let size = px as f32;
    match kind_of(picture) {
        Kind::Full => {
            canvas.image(0.0, 0.0, &picture.scaled(px, px));
            mask_rounded(&mut bytes, px);
        }
        Kind::Tile => {
            let fitted = picture.fitted(px, px);
            canvas.image(((px - fitted.w) / 2) as f32, ((px - fitted.h) / 2) as f32, &fitted);
        }
        Kind::Shape => {
            canvas.rrect(0.0, 0.0, size, size, size * CORNER, PLATE);
            let inner = (size * 0.68).round() as u32;
            let fitted = picture.fitted(inner, inner);
            canvas.image(((px - fitted.w) / 2) as f32, ((px - fitted.h) / 2) as f32, &fitted);
        }
    }
    to_image(bytes, px)
}

fn letter_tile(key: &str, label: &str, px: u32, fonts: &mut Fonts) -> Image {
    let hash = key.bytes().fold(0u32, |h, b| h.wrapping_mul(31).wrapping_add(b as u32));
    let mut bytes = vec![0u8; (px * px * 4) as usize];
    let mut canvas = Canvas::new(&mut bytes, px as i32, px as i32, 1.0);
    let size = px as f32;
    canvas.rrect(0.0, 0.0, size, size, size * CORNER, LETTER_COLORS[hash as usize % LETTER_COLORS.len()]);
    let letter: String = label.chars().find(|c| c.is_alphanumeric()).map(|c| c.to_uppercase().collect()).unwrap_or_else(|| "?".into());
    canvas.text_centered(fonts, &letter, size / 2.0, size / 2.0, size * 0.46, size, [0xff, 0xff, 0xff, 0xff]);
    to_image(bytes, px)
}
