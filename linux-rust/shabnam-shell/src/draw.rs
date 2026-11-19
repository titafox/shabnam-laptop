//! Software drawing into a wl_shm ARGB8888 buffer (premultiplied, bytes B G R A): rounded rectangles,
//! small shapes, text with a font fallback chain, and pictures.
//! `Canvas` methods take logical (surface) coordinates and multiply by the output scale themselves.

use ab_glyph::{point, Font, FontRef, Glyph, GlyphId, PxScale, VariableFont};
use std::path::{Path, PathBuf};

/// Straight (not premultiplied) R, G, B, A.
pub type Color = [u8; 4];

/// 0xAARRGGBB, the notation the original Android theme uses.
pub const fn argb(v: u32) -> Color {
    [(v >> 16) as u8, (v >> 8) as u8, v as u8, (v >> 24) as u8]
}

pub struct Canvas<'a> {
    /// Size in buffer pixels.
    pub w: i32,
    pub h: i32,
    pub px: &'a mut [u8],
    /// Buffer pixels per logical unit.
    pub s: f32,
    /// Nothing is drawn outside this rectangle (buffer pixels: x0, y0, x1, y1).
    clip: [i32; 4],
}

impl<'a> Canvas<'a> {
    pub fn new(px: &'a mut [u8], w: i32, h: i32, s: f32) -> Canvas<'a> {
        Canvas { w, h, px, s, clip: [0, 0, w, h] }
    }

    pub fn clear(&mut self) {
        self.px.fill(0);
    }

    /// Limits drawing to a logical rectangle; `None` lifts the limit.
    pub fn set_clip(&mut self, rect: Option<(f32, f32, f32, f32)>) {
        self.clip = match rect {
            None => [0, 0, self.w, self.h],
            Some((x, y, w, h)) => [
                ((x * self.s).round() as i32).max(0),
                ((y * self.s).round() as i32).max(0),
                (((x + w) * self.s).round() as i32).min(self.w),
                (((y + h) * self.s).round() as i32).min(self.h),
            ],
        };
    }

    fn blend(&mut self, x: i32, y: i32, c: Color, coverage: f32) {
        if x < self.clip[0] || y < self.clip[1] || x >= self.clip[2] || y >= self.clip[3] {
            return;
        }
        let a = (c[3] as f32 / 255.0) * coverage.clamp(0.0, 1.0);
        if a <= 0.0 {
            return;
        }
        let i = ((y * self.w + x) * 4) as usize;
        let d = &mut self.px[i..i + 4];
        let mix = |src: u8, dst: u8| (src as f32 * a + dst as f32 * (1.0 - a) + 0.5) as u8;
        d[0] = mix(c[2], d[0]);
        d[1] = mix(c[1], d[1]);
        d[2] = mix(c[0], d[2]);
        d[3] = mix(255, d[3]);
    }

    /// Anti-aliased rounded rectangle; edges are snapped to whole pixels so they stay crisp.
    pub fn rrect(&mut self, x: f32, y: f32, w: f32, h: f32, radius: f32, c: Color) {
        let (x0, y0) = ((x * self.s).round(), (y * self.s).round());
        let (x1, y1) = (((x + w) * self.s).round().max(x0 + 1.0), ((y + h) * self.s).round().max(y0 + 1.0));
        let (hw, hh) = ((x1 - x0) / 2.0, (y1 - y0) / 2.0);
        let r = (radius * self.s).min(hw).min(hh).max(0.0);
        let (cx, cy) = (x0 + hw, y0 + hh);
        for py in (y0 as i32).max(self.clip[1])..(y1 as i32).min(self.clip[3]) {
            for px in (x0 as i32).max(self.clip[0])..(x1 as i32).min(self.clip[2]) {
                // Signed distance from the pixel centre to the outline (negative inside).
                let qx = (px as f32 + 0.5 - cx).abs() - (hw - r);
                let qy = (py as f32 + 0.5 - cy).abs() - (hh - r);
                let dist = if qx > 0.0 && qy > 0.0 { (qx * qx + qy * qy).sqrt() - r } else { qx.max(qy) - r };
                if dist < 0.5 {
                    self.blend(px, py, c, 0.5 - dist);
                }
            }
        }
    }

    pub fn circle(&mut self, cx: f32, cy: f32, r: f32, c: Color) {
        self.rrect(cx - r, cy - r, 2.0 * r, 2.0 * r, r, c);
    }

    /// Any small shape: `inside` is asked about points given in logical units relative to the
    /// box's top-left corner (so 0..w, 0..h); edges are smoothed by 4x4 sampling.
    pub fn shape(&mut self, x: f32, y: f32, w: f32, h: f32, c: Color, inside: impl Fn(f32, f32) -> bool) {
        let s = self.s;
        for py in (y * s).floor() as i32..((y + h) * s).ceil() as i32 {
            for px in (x * s).floor() as i32..((x + w) * s).ceil() as i32 {
                let mut hits = 0;
                for sy in 0..4 {
                    for sx in 0..4 {
                        let lx = (px as f32 + (sx as f32 + 0.5) / 4.0) / s - x;
                        let ly = (py as f32 + (sy as f32 + 0.5) / 4.0) / s - y;
                        if lx >= 0.0 && ly >= 0.0 && lx <= w && ly <= h && inside(lx, ly) {
                            hits += 1;
                        }
                    }
                }
                if hits > 0 {
                    self.blend(px, py, c, hits as f32 / 16.0);
                }
            }
        }
    }

    /// Draws a picture (already sized in buffer pixels) with its top-left corner at logical (x, y).
    pub fn image(&mut self, x: f32, y: f32, img: &Image) {
        let (x, y) = ((x * self.s).round() as i32, (y * self.s).round() as i32);
        for iy in 0..img.h as i32 {
            for ix in 0..img.w as i32 {
                let (tx, ty) = (x + ix, y + iy);
                if tx < self.clip[0] || ty < self.clip[1] || tx >= self.clip[2] || ty >= self.clip[3] {
                    continue;
                }
                let s = &img.rgba[((iy as u32 * img.w + ix as u32) * 4) as usize..][..4];
                if s[3] == 0 {
                    continue;
                }
                let inv = 255 - s[3] as u32;
                let i = ((ty * self.w + tx) * 4) as usize;
                let d = &mut self.px[i..i + 4];
                let over = |src: u8, dst: u8| (src as u32 + (dst as u32 * inv + 127) / 255).min(255) as u8;
                d[0] = over(s[2], d[0]);
                d[1] = over(s[1], d[1]);
                d[2] = over(s[0], d[2]);
                d[3] = over(s[3], d[3]);
            }
        }
    }

    /// One line of text starting at logical `x`, vertically centred on `mid_y`; `size` is the
    /// logical em size. Returns the logical width.
    pub fn text(&mut self, fonts: &mut Fonts, text: &str, x: f32, mid_y: f32, size: f32, c: Color) -> f32 {
        let px_size = size * self.s;
        let baseline = (mid_y * self.s + fonts.baseline_offset(px_size)).round();
        fonts.draw(self, text, x * self.s, baseline, px_size, c) / self.s
    }

    /// Like `text`, but centred on `cx` and shortened with an ellipsis to fit `max_w`.
    pub fn text_centered(&mut self, fonts: &mut Fonts, text: &str, cx: f32, mid_y: f32, size: f32, max_w: f32, c: Color) {
        let shown = fonts.fit(text, size, max_w);
        let width = fonts.width(&shown, size);
        self.text(fonts, &shown, cx - width / 2.0, mid_y, size, c);
    }
}

/// No picture file larger than this is read.
pub const MAX_FILE: u64 = 4 * 1024 * 1024;
/// No bitmap wider or taller than this is decoded.
pub const MAX_SIDE: u32 = 2048;

/// The contents of a picture file, provided it is an ordinary file (not a pipe or a device, which
/// could make the read hang or never end) of at most `MAX_FILE` bytes.
pub fn read_capped(path: &Path) -> Option<Vec<u8>> {
    use std::io::Read;
    use std::os::unix::fs::OpenOptionsExt;
    let before = std::fs::metadata(path).ok()?;
    if !before.file_type().is_file() || before.len() > MAX_FILE {
        return None;
    }
    // Checked again on the opened file: the name could have been pointed elsewhere meanwhile.
    // O_NONBLOCK keeps the open itself from waiting should it now be a pipe.
    let file = std::fs::OpenOptions::new().read(true).custom_flags(libc::O_NONBLOCK).open(path).ok()?;
    let meta = file.metadata().ok()?;
    if !meta.file_type().is_file() || meta.len() > MAX_FILE {
        return None;
    }
    let mut data = Vec::with_capacity(meta.len() as usize);
    file.take(MAX_FILE + 1).read_to_end(&mut data).ok()?;
    (data.len() as u64 <= MAX_FILE).then_some(data)
}

/// Premultiplied RGBA picture.
pub struct Image {
    pub w: u32,
    pub h: u32,
    pub rgba: Vec<u8>,
}

impl Image {
    /// Reads and decodes a PNG file, within the limits of `read_capped` and `from_png`.
    pub fn load_png(path: &str) -> Option<Image> {
        Image::from_png(&read_capped(Path::new(path))?)
    }

    /// Decodes PNG data. Pictures larger than `MAX_SIDE` in either direction are refused before
    /// any memory is set aside for their pixels.
    pub fn from_png(data: &[u8]) -> Option<Image> {
        let mut decoder = png::Decoder::new(data);
        decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
        let mut reader = decoder.read_info().ok()?;
        let (width, height) = (reader.info().width, reader.info().height);
        if width == 0 || height == 0 || width > MAX_SIDE || height > MAX_SIDE {
            return None;
        }
        let mut buf = vec![0; reader.output_buffer_size()];
        let info = reader.next_frame(&mut buf).ok()?;
        let n = (info.width * info.height) as usize;
        let mut rgba = Vec::with_capacity(n * 4);
        let pre = |v: u8, a: u8| ((v as u32 * a as u32 + 127) / 255) as u8;
        match info.color_type {
            png::ColorType::Rgba => buf[..n * 4].chunks_exact(4).for_each(|p| {
                rgba.extend([pre(p[0], p[3]), pre(p[1], p[3]), pre(p[2], p[3]), p[3]]);
            }),
            png::ColorType::Rgb => buf[..n * 3].chunks_exact(3).for_each(|p| rgba.extend([p[0], p[1], p[2], 255])),
            png::ColorType::GrayscaleAlpha => buf[..n * 2].chunks_exact(2).for_each(|p| {
                let v = pre(p[0], p[1]);
                rgba.extend([v, v, v, p[1]]);
            }),
            png::ColorType::Grayscale => buf[..n].iter().for_each(|&v| rgba.extend([v, v, v, 255])),
            _ => return None,
        }
        Some(Image { w: info.width, h: info.height, rgba })
    }

    /// Scales to fit inside `max_w` x `max_h` pixels, keeping the proportions.
    pub fn fitted(&self, max_w: u32, max_h: u32) -> Image {
        let by_height = (self.w as u64 * max_h as u64 + self.h as u64 / 2) / self.h.max(1) as u64;
        if by_height <= max_w as u64 {
            self.scaled((by_height as u32).max(1), max_h)
        } else {
            let h = (self.h as u64 * max_w as u64 + self.w as u64 / 2) / self.w.max(1) as u64;
            self.scaled(max_w, (h as u32).max(1))
        }
    }

    /// Scales to exactly `w` x `h` pixels by averaging the source pixels each target pixel
    /// covers (right for shrinking; when enlarging it repeats pixels).
    pub fn scaled(&self, w: u32, h: u32) -> Image {
        let (w, h) = (w.max(1), h.max(1));
        if self.w == 0 || self.h == 0 {
            return Image { w, h, rgba: vec![0; (w * h * 4) as usize] };
        }
        let mut rgba = Vec::with_capacity((w * h * 4) as usize);
        for y in 0..h {
            let (y0, y1) = (y * self.h / h, ((y + 1) * self.h / h).max(y * self.h / h + 1).min(self.h));
            for x in 0..w {
                let (x0, x1) = (x * self.w / w, ((x + 1) * self.w / w).max(x * self.w / w + 1).min(self.w));
                let mut sum = [0u32; 4];
                for sy in y0..y1 {
                    for sx in x0..x1 {
                        let p = &self.rgba[((sy * self.w + sx) * 4) as usize..][..4];
                        (0..4).for_each(|k| sum[k] += p[k] as u32);
                    }
                }
                let count = (y1 - y0) * (x1 - x0);
                rgba.extend(sum.map(|s| ((s + count / 2) / count) as u8));
            }
        }
        Image { w, h, rgba }
    }
}

/// Latin / Cyrillic text face, first one found wins.
const BASE_FONTS: &[&str] = &[
    "/usr/share/fonts/TTF/DejaVuSans.ttf",
    "/usr/share/fonts/dejavu/DejaVuSans.ttf",
    "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
    "/usr/share/fonts/roboto/Roboto-Regular.ttf",
    "/usr/share/fonts/roboto/Roboto[ital,wdth,wght].ttf",
    "/usr/share/fonts/noto/NotoSans-Regular.ttf",
    "/usr/share/fonts/liberation/LiberationSans-Regular.ttf",
];

/// File-name fragments (lower case) of fonts with Chinese glyphs, most wanted first.
const CJK_HINTS: &[&str] = &[
    "notosanscjksc-regular",
    "notosanscjk-regular",
    "notosanssc-regular",
    "notosanscjk",
    "notosanssc",
    "sourcehansanssc-regular",
    "sourcehansans",
    "notoserifcjk-regular",
    "notoserifcjk",
    "wqy-microhei",
    "wqy-zenhei",
    "wqy",
    "droidsansfallback",
    "cjk",
];

const FONT_ROOTS: &[&str] = &["/usr/share/fonts", "/usr/local/share/fonts"];

pub struct Fonts {
    faces: Vec<FontRef<'static>>,
    /// The fallback fonts are looked for only when a character first needs them.
    fallbacks_loaded: bool,
}

struct Placed {
    face: usize,
    id: GlyphId,
    x: f32,
}

/// Maps a font file and keeps it for the life of the process (pages are read only when touched,
/// which matters for the 20+ MB Chinese fonts).
fn map_file(path: &Path) -> Option<&'static [u8]> {
    let file = std::fs::File::open(path).ok()?;
    let map = unsafe { memmap2::Mmap::map(&file) }.ok()?;
    Some(&Box::leak(Box::new(map))[..])
}

/// In a .ttc holding several regional variants, the index of the Simplified Chinese one.
fn preferred_index(data: &[u8]) -> u32 {
    let count = ttf_parser::fonts_in_collection(data).unwrap_or(1);
    for i in 0..count {
        let Ok(face) = ttf_parser::Face::parse(data, i) else { continue };
        let is_sc = face.names().into_iter().any(|n| {
            n.name_id == ttf_parser::name_id::FAMILY
                && n.to_string().is_some_and(|s| s.ends_with(" SC") || s.contains(" SC ") || s.contains("Simplified"))
        });
        if is_sc {
            return i;
        }
    }
    0
}

fn open_face(path: &Path) -> Option<FontRef<'static>> {
    let data = map_file(path)?;
    let mut font = FontRef::try_from_slice_and_index(data, preferred_index(data)).ok()?;
    font.set_variation(b"wght", 400.0); // variable fonts may default to a thin weight
    Some(font)
}

fn walk(dir: &Path, depth: u32, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if depth < 4 {
                walk(&path, depth + 1, out);
            }
        } else if matches!(
            path.extension().and_then(|e| e.to_str()).map(|e| e.to_ascii_lowercase()).as_deref(),
            Some("ttf" | "otf" | "ttc" | "otc")
        ) {
            out.push(path);
        }
    }
}

impl Fonts {
    pub fn load() -> Fonts {
        let mut faces: Vec<_> = BASE_FONTS.iter().filter_map(|p| open_face(Path::new(p))).take(1).collect();
        let mut fallbacks_loaded = false;
        if faces.is_empty() {
            // No known Latin font: take whatever is installed.
            let mut fonts = Fonts { faces, fallbacks_loaded: true };
            fonts.load_fallbacks(true);
            faces = fonts.faces;
            fallbacks_loaded = true;
        }
        Fonts { faces, fallbacks_loaded }
    }

    fn load_fallbacks(&mut self, any: bool) {
        let mut files = Vec::new();
        FONT_ROOTS.iter().for_each(|root| walk(Path::new(root), 0, &mut files));
        files.sort();
        let lower: Vec<String> =
            files.iter().map(|f| f.file_name().unwrap_or_default().to_string_lossy().to_lowercase()).collect();
        // The best Chinese font only: one is enough, and each extra face costs a lookup per miss.
        let cjk = CJK_HINTS.iter().find_map(|hint| lower.iter().position(|name| name.contains(hint)));
        if let Some(font) = cjk.and_then(|i| open_face(&files[i])) {
            self.faces.push(font);
        }
        if any && self.faces.is_empty() {
            self.faces.extend(files.iter().filter_map(|f| open_face(f)).take(1));
        }
    }

    fn face_for(&mut self, c: char) -> Option<(usize, GlyphId)> {
        let find = |faces: &[FontRef<'static>]| {
            faces.iter().enumerate().map(|(i, f)| (i, f.glyph_id(c))).find(|(_, id)| id.0 != 0)
        };
        let found = find(&self.faces);
        if found.is_none() && !self.fallbacks_loaded && !c.is_ascii() {
            self.fallbacks_loaded = true;
            self.load_fallbacks(false);
            return find(&self.faces);
        }
        found
    }

    /// True when every character has a glyph in some loaded font.
    pub fn can_show(&mut self, text: &str) -> bool {
        text.chars().all(|c| c.is_whitespace() || self.face_for(c).is_some())
    }

    fn em_factor(&self, face: usize, size: f32) -> f32 {
        size / self.faces[face].units_per_em().unwrap_or(1000.0)
    }

    fn layout(&mut self, text: &str, size: f32) -> (Vec<Placed>, f32) {
        let mut placed = Vec::new();
        let mut x = 0.0;
        if self.faces.is_empty() {
            return (placed, x);
        }
        for c in text.chars() {
            let (face, id) = self.face_for(c).unwrap_or((0, GlyphId(0)));
            placed.push(Placed { face, id, x });
            x += self.faces[face].h_advance_unscaled(id) * self.em_factor(face, size);
        }
        (placed, x)
    }

    pub fn width(&mut self, text: &str, size: f32) -> f32 {
        self.layout(text, size).1
    }

    /// Offset from the vertical middle of a line to the baseline, for text of em size `size`.
    pub fn baseline_offset(&self, size: f32) -> f32 {
        match self.faces.first() {
            Some(f) => (f.ascent_unscaled() + f.descent_unscaled()) / 2.0 * self.em_factor(0, size),
            None => size * 0.35,
        }
    }

    /// Shortens `text` with an ellipsis so that it fits in `max_width`.
    pub fn fit(&mut self, text: &str, size: f32, max_width: f32) -> String {
        if self.width(text, size) <= max_width {
            return text.to_string();
        }
        let mut chars: Vec<char> = text.chars().collect();
        while !chars.is_empty() {
            chars.pop();
            let candidate: String = chars.iter().collect::<String>().trim_end().to_string() + "…";
            if self.width(&candidate, size) <= max_width {
                return candidate;
            }
        }
        String::new()
    }

    /// Draws one line with its left end at `x` and its baseline at `baseline`; returns its width.
    /// Everything here is in buffer pixels, `size` being the em size.
    pub fn draw(&mut self, canvas: &mut Canvas, text: &str, x: f32, baseline: f32, size: f32, color: Color) -> f32 {
        let (placed, width) = self.layout(text, size);
        for g in placed {
            let font = &self.faces[g.face];
            // ab_glyph scales by line height (ascent - descent), not by em.
            let scale = PxScale::from(font.height_unscaled() * self.em_factor(g.face, size));
            let glyph = Glyph { id: g.id, scale, position: point(x + g.x, baseline) };
            if let Some(outline) = font.outline_glyph(glyph) {
                let bounds = outline.px_bounds();
                let (ox, oy) = (bounds.min.x as i32, bounds.min.y as i32);
                outline.draw(|gx, gy, coverage| canvas.blend(ox + gx as i32, oy + gy as i32, color, coverage));
            }
        }
        width
    }
}
