//! Block textures (M3b): pictures for block faces.
//!
//! A face's texture is a PNG in the `textures/` folder beside blocks.json
//! (`game/assets/textures/` for the shipped file). Every picture is kept at
//! one size, `texture_size` in blocks.json (16 x 16 by default, Andrew's
//! setting): a file of any other size is resampled when it is read, so an
//! imported photo and a 32 x 32 drawing both work. The pictures are the
//! layers of one GPU texture array with a full mip chain, built here in
//! linear light, so a face far away shows the picture's average colour
//! (the 1 x 1 level) and never shimmers.
//!
//! The shader tiles a texture once per block, at every LOD (a greedy quad
//! spans many blocks; a coarse voxel spans many blocks too), from texture
//! coordinates in blocks along the face. Faces without a texture keep
//! their flat colour exactly: the colour table says which faces sample
//! (render/shader.wgsl).
//!
//! `TextureSet` is the CPU side: which file is which layer, the pixels, and
//! the watches that reload a file edited in a paint program (polled a few
//! files a frame, so a long list never costs a frame spike).

use crate::look::FileWatch;
use crate::world::Registry;
use crate::world::blocks::{FACES_PER_BLOCK, FaceLayer};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// A face picture: `size` x `size` RGBA8, sRGB-encoded as in a PNG.
#[derive(Clone, Debug, PartialEq)]
pub struct Picture {
    pub size: u32,
    pub rgba: Vec<u8>,
}

/// sRGB byte to linear light.
pub fn to_linear(u: u8) -> f32 {
    let x = u as f32 / 255.0;
    if x <= 0.04045 { x / 12.92 } else { ((x + 0.055) / 1.055).powf(2.4) }
}

/// Linear light to an sRGB byte.
pub fn to_srgb(x: f32) -> u8 {
    let x = x.clamp(0.0, 1.0);
    let s = if x <= 0.003_130_8 { 12.92 * x } else { 1.055 * x.powf(1.0 / 2.4) - 0.055 };
    (s * 255.0 + 0.5).floor().clamp(0.0, 255.0) as u8
}

/// Decode a PNG of any size and colour type to RGBA8.
pub fn decode_png(bytes: &[u8]) -> Result<(u32, u32, Vec<u8>), String> {
    let mut decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    decoder.set_transformations(png::Transformations::normalize_to_color8());
    let mut reader = decoder.read_info().map_err(|e| format!("not a PNG I can read: {e}"))?;
    let mut buf = vec![0u8; reader.output_buffer_size().ok_or("the picture is too big")?];
    let info = reader.next_frame(&mut buf).map_err(|e| format!("broken PNG: {e}"))?;
    let (w, h) = (info.width, info.height);
    if w == 0 || h == 0 || w > 8192 || h > 8192 {
        return Err(format!("a {w} x {h} picture: up to 8192 x 8192 please"));
    }
    let px = (w * h) as usize;
    let data = &buf[..info.line_size * h as usize];
    let mut rgba = Vec::with_capacity(px * 4);
    for row in data.chunks_exact(info.line_size) {
        match info.color_type {
            png::ColorType::Rgba => rgba.extend_from_slice(&row[..w as usize * 4]),
            png::ColorType::Rgb => row[..w as usize * 3].chunks_exact(3).for_each(|p| rgba.extend_from_slice(&[p[0], p[1], p[2], 255])),
            png::ColorType::GrayscaleAlpha => row[..w as usize * 2].chunks_exact(2).for_each(|p| rgba.extend_from_slice(&[p[0], p[0], p[0], p[1]])),
            png::ColorType::Grayscale => row[..w as usize].iter().for_each(|&g| rgba.extend_from_slice(&[g, g, g, 255])),
            png::ColorType::Indexed => return Err("an indexed PNG the decoder did not expand".into()),
        }
    }
    Ok((w, h, rgba))
}

/// Average of a set of RGBA8 pixels with weights, in linear light; colour
/// weighted by alpha (a transparent pixel's colour counts for nothing).
fn mix(px: impl Iterator<Item = ([u8; 4], f32)>) -> [u8; 4] {
    let (mut c, mut a, mut w, mut cu) = ([0f32; 3], 0f32, 0f32, [0f32; 3]);
    for (p, wt) in px {
        let al = p[3] as f32 / 255.0;
        for i in 0..3 {
            let l = to_linear(p[i]);
            c[i] += l * al * wt;
            cu[i] += l * wt;
        }
        a += al * wt;
        w += wt;
    }
    if w <= 0.0 {
        return [0, 0, 0, 0];
    }
    let col = if a > 1e-6 { c.map(|x| x / a) } else { cu.map(|x| x / w) };
    [to_srgb(col[0]), to_srgb(col[1]), to_srgb(col[2]), ((a / w) * 255.0 + 0.5).floor().clamp(0.0, 255.0) as u8]
}

impl Picture {
    /// One colour all over (linear RGB), opaque.
    pub fn flat(size: u32, linear: [f32; 3]) -> Picture {
        let p = [to_srgb(linear[0]), to_srgb(linear[1]), to_srgb(linear[2]), 255];
        Picture { size, rgba: p.repeat((size * size) as usize) }
    }

    /// Any picture, cut to its middle square and resampled to `size`:
    /// pixel for pixel when it already is that size, each pixel repeated
    /// when it is smaller (pixel art stays crisp), and averaged over the
    /// pixels each one covers when it is larger (a photo shrinks smoothly).
    pub fn from_rgba(w: u32, h: u32, rgba: &[u8], size: u32) -> Picture {
        let n = w.min(h);
        let (x0, y0) = ((w - n) / 2, (h - n) / 2);
        let at = |x: u32, y: u32| -> [u8; 4] {
            let i = (((y0 + y) * w + x0 + x) * 4) as usize;
            [rgba[i], rgba[i + 1], rgba[i + 2], rgba[i + 3]]
        };
        let mut out = Vec::with_capacity((size * size * 4) as usize);
        for y in 0..size {
            for x in 0..size {
                let p = if n <= size {
                    at(x * n / size, y * n / size)
                } else {
                    // The source box this pixel covers, with partial edge pixels weighted.
                    let s = n as f64 / size as f64;
                    let (ax, bx, ay, by) = (x as f64 * s, (x + 1) as f64 * s, y as f64 * s, (y + 1) as f64 * s);
                    let cells = (ay.floor() as u32..(by.ceil() as u32).min(n)).flat_map(move |sy| (ax.floor() as u32..(bx.ceil() as u32).min(n)).map(move |sx| (sx, sy)));
                    mix(cells.map(|(sx, sy)| {
                        let wx = (bx.min(sx as f64 + 1.0) - ax.max(sx as f64)).max(0.0);
                        let wy = (by.min(sy as f64 + 1.0) - ay.max(sy as f64)).max(0.0);
                        (at(sx, sy), (wx * wy) as f32)
                    }))
                };
                out.extend_from_slice(&p);
            }
        }
        Picture { size, rgba: out }
    }

    /// Decode PNG bytes and resample to `size`.
    pub fn decode(bytes: &[u8], size: u32) -> Result<Picture, String> {
        let (w, h, rgba) = decode_png(bytes)?;
        Ok(Picture::from_rgba(w, h, &rgba, size))
    }

    /// A texture file, resampled to `size` and opaque: blocks are solid,
    /// so a transparent pixel shows its colour (and counts in the far
    /// average) like any other.
    pub fn load(path: &Path, size: u32) -> Result<Picture, String> {
        let bytes = std::fs::read(path).map_err(|e| format!("read {}: {e}", path.display()))?;
        Picture::decode(&bytes, size).map(Picture::opaque).map_err(|e| format!("{}: {e}", path.display()))
    }

    /// Every pixel at full alpha, its colour kept.
    pub fn opaque(mut self) -> Picture {
        self.rgba.chunks_exact_mut(4).for_each(|p| p[3] = 255);
        self
    }

    /// Composite over a colour (linear RGB) and make opaque: what an
    /// imported picture with see-through parts becomes on a solid block.
    pub fn over(mut self, under: [f32; 3]) -> Picture {
        for p in self.rgba.chunks_exact_mut(4) {
            let a = p[3] as f32 / 255.0;
            for i in 0..3 {
                p[i] = to_srgb(to_linear(p[i]) * a + under[i] * (1.0 - a));
            }
            p[3] = 255;
        }
        self
    }

    /// The same picture at another size.
    pub fn resized(&self, size: u32) -> Picture {
        if size == self.size { self.clone() } else { Picture::from_rgba(self.size, self.size, &self.rgba, size) }
    }

    /// PNG bytes (RGBA, 8 bits).
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::new();
        {
            let mut enc = png::Encoder::new(&mut out, self.size, self.size);
            enc.set_color(png::ColorType::Rgba);
            enc.set_depth(png::BitDepth::Eight);
            let mut w = enc.write_header().expect("png header");
            w.write_image_data(&self.rgba).expect("png data");
        }
        out
    }

    /// Write the picture as a PNG, beside the file and renamed over it.
    pub fn save(&self, path: &Path) -> Result<(), String> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("create {}: {e}", dir.display()))?;
        }
        let tmp = path.with_extension("png.saving");
        std::fs::write(&tmp, self.encode()).map_err(|e| format!("write {}: {e}", tmp.display()))?;
        std::fs::rename(&tmp, path).map_err(|e| format!("replace {}: {e}", path.display()))
    }

    pub fn get(&self, x: u32, y: u32) -> [u8; 4] {
        let i = ((y * self.size + x) * 4) as usize;
        [self.rgba[i], self.rgba[i + 1], self.rgba[i + 2], self.rgba[i + 3]]
    }

    pub fn set(&mut self, x: u32, y: u32, p: [u8; 4]) {
        let i = ((y * self.size + x) * 4) as usize;
        self.rgba[i..i + 4].copy_from_slice(&p);
    }

    /// Fill the 4-connected area of pixels the colour of (x, y) with `p`.
    /// Returns whether anything changed.
    pub fn fill(&mut self, x: u32, y: u32, p: [u8; 4]) -> bool {
        let from = self.get(x, y);
        if from == p {
            return false;
        }
        let mut stack = vec![(x, y)];
        while let Some((x, y)) = stack.pop() {
            if self.get(x, y) != from {
                continue;
            }
            self.set(x, y, p);
            if x > 0 {
                stack.push((x - 1, y));
            }
            if y > 0 {
                stack.push((x, y - 1));
            }
            if x + 1 < self.size {
                stack.push((x + 1, y));
            }
            if y + 1 < self.size {
                stack.push((x, y + 1));
            }
        }
        true
    }

    /// The average colour, linear RGB: what a face shows from far away.
    pub fn average(&self) -> [f32; 3] {
        let n = (self.size * self.size) as f32;
        let p = mix(self.rgba.chunks_exact(4).map(|p| ([p[0], p[1], p[2], p[3]], 1.0 / n)));
        [to_linear(p[0]), to_linear(p[1]), to_linear(p[2])]
    }

    /// Every mip level, the picture first and 1 x 1 last, each halving
    /// the one before by averaging 2 x 2 pixels in linear light.
    pub fn mips(&self) -> Vec<Vec<u8>> {
        let mut levels = vec![self.rgba.clone()];
        let mut s = self.size;
        while s > 1 {
            let prev = levels.last().unwrap();
            let h = s / 2;
            let at = |x: u32, y: u32| -> [u8; 4] {
                let i = ((y * s + x) * 4) as usize;
                [prev[i], prev[i + 1], prev[i + 2], prev[i + 3]]
            };
            let mut next = Vec::with_capacity((h * h * 4) as usize);
            for y in 0..h {
                for x in 0..h {
                    next.extend_from_slice(&mix([(2 * x, 2 * y), (2 * x + 1, 2 * y), (2 * x, 2 * y + 1), (2 * x + 1, 2 * y + 1)].into_iter().map(|(a, b)| (at(a, b), 0.25))));
                }
            }
            levels.push(next);
            s = h;
        }
        levels
    }
}

// ------------------------------------------------------------ the set

struct Layer {
    /// The file, relative to the textures folder, with forward slashes.
    file: String,
    picture: Picture,
    /// The picture's average colour (linear), kept with it.
    avg: [f32; 3],
    watch: FileWatch,
    /// Painted in the panel and not yet written (Save look writes it).
    unsaved: bool,
}

/// Which file is which layer of the GPU texture array, and the pixels.
pub struct TextureSet {
    pub size: u32,
    pub dir: PathBuf,
    layers: Vec<Layer>,
    /// Layers whose pixels changed since the renderer last took them.
    dirty: Vec<usize>,
    /// Referenced files that could not be read: (file, why), and a watch
    /// so they load when they appear.
    missing: Vec<(String, String, FileWatch)>,
    /// Round-robin position of the file poll.
    poll_at: usize,
}

/// The textures folder for a blocks file: `textures/` beside it.
pub fn dir_for(blocks_path: &Path) -> PathBuf {
    blocks_path.parent().unwrap_or(Path::new(".")).join("textures")
}

fn norm(file: &str) -> String {
    file.replace('\\', "/")
}

impl TextureSet {
    pub fn new(dir: PathBuf, size: u32) -> Self {
        Self { size, dir, layers: Vec::new(), dirty: Vec::new(), missing: Vec::new(), poll_at: 0 }
    }

    /// Every texture `registry` names, loaded, with the layers assigned
    /// into the registry (its colour table then says which faces sample).
    pub fn for_registry(registry: &Registry, dir: PathBuf) -> Self {
        let mut set = Self::new(dir, registry.texture_size());
        set.assign(registry);
        set.dirty.clear();
        set
    }

    pub fn path_of(&self, file: &str) -> PathBuf {
        self.dir.join(norm(file))
    }

    pub fn len(&self) -> usize {
        self.layers.len()
    }

    pub fn is_empty(&self) -> bool {
        self.layers.is_empty()
    }

    pub fn layer(&self, file: &str) -> Option<usize> {
        let f = norm(file);
        self.layers.iter().position(|l| l.file == f)
    }

    pub fn picture(&self, file: &str) -> Option<&Picture> {
        self.layer(file).map(|i| &self.layers[i].picture)
    }

    pub fn pictures(&self) -> Vec<&Picture> {
        self.layers.iter().map(|l| &l.picture).collect()
    }

    /// Does the file exist on disk (or is it painted, waiting to be saved)?
    pub fn known(&self, file: &str) -> bool {
        self.layer(file).is_some() || self.path_of(file).exists()
    }

    /// Files named but not readable, with the reason.
    pub fn missing(&self) -> Vec<(String, String)> {
        self.missing.iter().map(|(f, e, _)| (f.clone(), e.clone())).collect()
    }

    /// The layer for `file`, reading it from disk the first time.
    fn ensure(&mut self, file: &str) -> Option<usize> {
        let f = norm(file);
        if let Some(i) = self.layer(&f) {
            return Some(i);
        }
        let path = self.path_of(&f);
        match Picture::load(&path, self.size) {
            Ok(picture) => {
                self.missing.retain(|(m, _, _)| *m != f);
                let avg = picture.average();
                self.layers.push(Layer { file: f, picture, avg, watch: FileWatch::new(path), unsaved: false });
                self.dirty.push(self.layers.len() - 1);
                Some(self.layers.len() - 1)
            }
            Err(e) => {
                if !self.missing.iter().any(|(m, _, _)| *m == f) {
                    self.missing.push((f, e, FileWatch::new(path)));
                }
                None
            }
        }
    }

    /// Give every face `registry` textures a layer and tell the registry
    /// (layer and average colour per face). True when the registry's
    /// colour table changed.
    pub fn assign(&mut self, registry: &Registry) -> bool {
        let textures = registry.textures();
        let mut layers: Vec<[FaceLayer; FACES_PER_BLOCK]> = Vec::with_capacity(textures.len());
        for t in &textures {
            let mut l = [None; FACES_PER_BLOCK];
            for (f, file) in t.faces().into_iter().enumerate() {
                if let Some(i) = file.and_then(|file| self.ensure(file)) {
                    l[f] = Some((i as u32, self.layers[i].avg));
                }
            }
            layers.push(l);
        }
        // Files nobody names any more are not missing.
        let named: Vec<String> = textures.iter().flat_map(|t| t.faces().into_iter().flatten().map(norm).collect::<Vec<_>>()).collect();
        self.missing.retain(|(f, _, _)| named.contains(f));
        registry.set_layers(&layers)
    }

    /// Put a painted picture in a file's layer (adding the layer when the
    /// file is new). It shows at once and is written by `save`.
    pub fn set_picture(&mut self, file: &str, picture: Picture) -> usize {
        let f = norm(file);
        let picture = picture.resized(self.size);
        let i = match self.layer(&f) {
            Some(i) => {
                if self.layers[i].picture == picture {
                    return i;
                }
                self.layers[i].avg = picture.average();
                self.layers[i].picture = picture;
                i
            }
            None => {
                let path = self.path_of(&f);
                self.missing.retain(|(m, _, _)| *m != f);
                let avg = picture.average();
                self.layers.push(Layer { file: f, picture, avg, watch: FileWatch::new(path), unsaved: true });
                self.layers.len() - 1
            }
        };
        self.layers[i].unsaved = true;
        self.dirty.push(i);
        i
    }

    /// Files painted but not written.
    pub fn unsaved(&self) -> Vec<String> {
        self.layers.iter().filter(|l| l.unsaved).map(|l| l.file.clone()).collect()
    }

    /// Write every painted picture to its file.
    pub fn save(&mut self) -> Result<Vec<String>, String> {
        let mut written = Vec::new();
        for l in self.layers.iter_mut().filter(|l| l.unsaved) {
            let path = self.dir.join(&l.file);
            l.picture.save(&path)?;
            l.watch = FileWatch::new(path);
            l.unsaved = false;
            written.push(l.file.clone());
        }
        Ok(written)
    }

    /// Throw away painting: every unsaved picture goes back to its file
    /// (or to nothing, when it was never written).
    pub fn revert(&mut self) {
        for i in 0..self.layers.len() {
            if !self.layers[i].unsaved {
                continue;
            }
            let path = self.dir.join(&self.layers[i].file);
            self.layers[i].unsaved = false;
            if let Ok(p) = Picture::load(&path, self.size) {
                self.layers[i].avg = p.average();
                self.layers[i].picture = p;
                self.dirty.push(i);
            }
        }
    }

    /// Check up to `budget` files for changes on disk and reload those
    /// that changed (a file painted in the panel and changed on disk takes
    /// the disk's picture). Returns a note per file, and whether a
    /// missing file appeared (the caller then reassigns).
    pub fn poll(&mut self, budget: usize) -> (Vec<String>, bool) {
        let mut notes = Vec::new();
        let total = self.layers.len() + self.missing.len();
        let mut appeared = false;
        for _ in 0..budget.min(total) {
            let k = self.poll_at % total;
            self.poll_at = (self.poll_at + 1) % total.max(1);
            if k < self.layers.len() {
                if self.layers[k].watch.changed() {
                    let path = self.dir.join(&self.layers[k].file);
                    match Picture::load(&path, self.size) {
                        Ok(p) => {
                            if p != self.layers[k].picture {
                                self.layers[k].avg = p.average();
                                self.layers[k].picture = p;
                                self.layers[k].unsaved = false;
                                self.dirty.push(k);
                                notes.push(format!("{} changed on disk: applied.", self.layers[k].file));
                            }
                        }
                        Err(e) => notes.push(format!("{} changed but was not applied: {e}", self.layers[k].file)),
                    }
                }
            } else {
                let m = k - self.layers.len();
                if self.missing[m].2.changed() {
                    appeared = true;
                }
            }
        }
        (notes, appeared)
    }

    /// A new texture size: every picture resampled (from its file when it
    /// has one on disk and is not being painted).
    pub fn set_size(&mut self, size: u32) {
        if size == self.size {
            return;
        }
        self.size = size;
        for i in 0..self.layers.len() {
            let path = self.dir.join(&self.layers[i].file);
            let from_disk = if self.layers[i].unsaved { None } else { Picture::load(&path, size).ok() };
            self.layers[i].picture = from_disk.unwrap_or_else(|| self.layers[i].picture.resized(size));
            self.layers[i].avg = self.layers[i].picture.average();
        }
        self.dirty = (0..self.layers.len()).collect();
    }

    /// Layers whose pixels changed since the last call (for the renderer).
    pub fn take_dirty(&mut self) -> Vec<usize> {
        let mut d = std::mem::take(&mut self.dirty);
        d.sort_unstable();
        d.dedup();
        d
    }

    pub fn picture_at(&self, layer: usize) -> &Picture {
        &self.layers[layer].picture
    }

    pub fn file_at(&self, layer: usize) -> &str {
        &self.layers[layer].file
    }
}

// ------------------------------------------------------------ startup

/// What main hands every renderer: the block size (texture coordinates
/// are in blocks) and the pictures, layer by layer.
pub struct Installed {
    pub block_m: f64,
    pub size: u32,
    pub pictures: Vec<Picture>,
}

static INSTALLED: OnceLock<Installed> = OnceLock::new();

/// Installed once by main, like `look::install`, so the app, the bench,
/// the harness and the reels all draw the same textures.
pub fn install(i: Installed) {
    let _ = INSTALLED.set(i);
}

pub fn installed() -> Option<&'static Installed> {
    INSTALLED.get()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::blocks::BlockTextures;

    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("triangulum-textures-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("textures")).unwrap();
        dir
    }

    #[test]
    fn srgb_round_trip_and_flat_average() {
        for u in 0..=255u8 {
            assert_eq!(to_srgb(to_linear(u)), u);
        }
        let c = [0.3, 0.29, 0.28];
        let p = Picture::flat(16, c);
        let a = p.average();
        for i in 0..3 {
            assert!((a[i] - c[i]).abs() < 0.004, "{a:?}");
        }
        // Every mip level of a flat picture is the same colour; the last is 1 x 1.
        let mips = p.mips();
        assert_eq!(mips.len(), 5);
        assert_eq!(mips[4].len(), 4);
        assert_eq!(&mips[4][..], &p.rgba[..4]);
    }

    /// The far colour is the average in linear light: a black and white
    /// checkerboard is mid grey (188 in sRGB), not 128.
    #[test]
    fn mips_average_in_linear_light() {
        let mut p = Picture::flat(4, [0.0; 3]);
        for y in 0..4 {
            for x in 0..4 {
                if (x + y) % 2 == 0 {
                    p.set(x, y, [255, 255, 255, 255]);
                }
            }
        }
        let mips = p.mips();
        assert_eq!(mips.len(), 3);
        assert_eq!(mips[2], vec![188, 188, 188, 255]);
        assert!((p.average()[0] - 0.5).abs() < 0.01);
    }

    /// Import: a picture of any size becomes the setting's size. A 4 x 4
    /// is repeated pixel for pixel (crisp), a 64 x 32 is cut to its middle
    /// square and averaged down, and PNG bytes round-trip exactly.
    #[test]
    fn resampling_and_png_round_trip() {
        let mut small = Picture::flat(4, [0.0; 3]);
        small.set(1, 2, [200, 10, 20, 255]);
        let big = small.resized(16);
        assert_eq!(big.get(4, 8), [200, 10, 20, 255]);
        assert_eq!(big.get(7, 11), [200, 10, 20, 255]);
        assert_eq!(big.get(8, 8), [0, 0, 0, 255]);
        // 64 x 32: left quarter red, middle half green, right quarter blue.
        let (w, h) = (64u32, 32u32);
        let mut rgba = Vec::new();
        for _ in 0..h {
            for x in 0..w {
                rgba.extend_from_slice(if x < 16 { &[255, 0, 0, 255] } else if x < 48 { &[0, 255, 0, 255] } else { &[0, 0, 255, 255] });
            }
        }
        let p = Picture::from_rgba(w, h, &rgba, 16);
        assert_eq!(p.size, 16);
        assert!(p.rgba.chunks_exact(4).all(|q| q == [0, 255, 0, 255]), "the middle square is all green");
        let bytes = big.encode();
        assert_eq!(Picture::decode(&bytes, 16).unwrap(), big);
        assert!(Picture::decode(b"not a png", 16).is_err());
    }

    #[test]
    fn fill_stays_inside_its_colour() {
        let mut p = Picture::flat(8, [0.0; 3]);
        for i in 0..8 {
            p.set(3, i, [255, 255, 255, 255]); // a white wall down the middle
        }
        assert!(p.fill(0, 0, [255, 0, 0, 255]));
        assert_eq!(p.get(2, 7), [255, 0, 0, 255]);
        assert_eq!(p.get(3, 4), [255, 255, 255, 255]);
        assert_eq!(p.get(4, 0), [0, 0, 0, 255], "the far side of the wall is untouched");
        assert!(!p.fill(0, 0, [255, 0, 0, 255]), "filling with the same colour changes nothing");
    }

    /// M3b hot reload: a texture file changed on disk (as a paint program
    /// saves it) is picked up by the poll, its layer's pixels change and the
    /// layer is marked for upload; the registry's average colour follows on
    /// the next assign. A file that appears later is noticed too.
    #[test]
    fn hot_reload_of_a_texture_file() {
        let dir = scratch("reload");
        let text = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/blocks.json")).unwrap();
        let registry = Registry::parse(&text).unwrap();
        let stone = registry.id("stone").unwrap() as usize;
        let dirt = registry.id("dirt").unwrap() as usize;
        let mut t = registry.textures();
        t[stone] = BlockTextures { all: Some("stone.png".into()), ..Default::default() };
        t[dirt] = BlockTextures { all: Some("later.png".into()), ..Default::default() };
        registry.set_textures(&t);
        let path = dir.join("textures/stone.png");
        Picture::flat(16, [0.1, 0.1, 0.1]).save(&path).unwrap();
        let mut set = TextureSet::for_registry(&registry, dir.join("textures"));
        assert_eq!(set.len(), 1);
        assert_eq!(set.missing().len(), 1, "later.png is not there yet");
        let table = registry.colour_table();
        assert_eq!(table[stone * 3][3], 0.0, "stone samples layer 0");
        assert_eq!(table[dirt * 3][3], -1.0, "dirt stays flat while its file is missing");
        assert!(set.take_dirty().is_empty());
        assert!(set.poll(8).0.is_empty(), "nothing changed yet");

        // Paint it red in "another program", with a different size on disk.
        std::thread::sleep(std::time::Duration::from_millis(20));
        Picture::flat(32, [0.8, 0.05, 0.05]).save(&path).unwrap();
        let (notes, _) = set.poll(8);
        assert_eq!(notes.len(), 1, "{notes:?}");
        assert_eq!(set.take_dirty(), vec![0]);
        assert_eq!(set.picture("stone.png").unwrap().size, 16, "resampled to the setting");
        assert_eq!(set.picture("stone.png").unwrap().get(5, 5), [to_srgb(0.8), to_srgb(0.05), to_srgb(0.05), 255]);
        assert!(set.assign(&registry), "the average colour reaches the colour table");
        assert!((registry.colour_table()[stone * 3][0] - 0.8).abs() < 0.01);

        // The missing file appears.
        Picture::flat(16, [0.2, 0.15, 0.1]).save(&dir.join("textures/later.png")).unwrap();
        let mut appeared = false;
        for _ in 0..4 {
            appeared |= set.poll(8).1;
        }
        assert!(appeared);
        set.assign(&registry);
        assert_eq!(registry.colour_table()[dirt * 3][3], 1.0, "dirt now samples layer 1");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Painting, saving and reverting: a painted picture is unsaved until
    /// `save` writes it; `revert` throws painting away.
    #[test]
    fn paint_save_revert() {
        let dir = scratch("paint");
        let mut set = TextureSet::new(dir.join("textures"), 16);
        let mut p = Picture::flat(16, [0.5, 0.5, 0.5]);
        set.set_picture("brick.png", p.clone());
        assert_eq!(set.unsaved(), vec!["brick.png".to_string()]);
        assert!(!dir.join("textures/brick.png").exists());
        assert_eq!(set.save().unwrap(), vec!["brick.png".to_string()]);
        assert_eq!(Picture::load(&dir.join("textures/brick.png"), 16).unwrap(), p);
        p.set(0, 0, [255, 0, 0, 255]);
        set.set_picture("brick.png", p.clone());
        set.revert();
        assert_eq!(set.picture("brick.png").unwrap().get(0, 0), [to_srgb(0.5), to_srgb(0.5), to_srgb(0.5), 255]);
        assert!(set.unsaved().is_empty());
        // A new size resamples everything.
        set.set_size(32);
        assert_eq!(set.picture("brick.png").unwrap().size, 32);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
