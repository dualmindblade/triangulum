//! The photo map: an egui overlay opened with T. Every screenshot in
//! `interchange/` becomes a marker on a minimap of the planet (synthesized
//! from the baked rasters — no extra data files); markers and the photo
//! list select each other, a preview shows the shot, and "Teleport" commits
//! to the photo's exact pose — optionally restoring its time of day from
//! the JSON sidecar. Photos can be deleted (bulk, with confirmation); they
//! move to `interchange/trash/`, never straight to oblivion.
//!
//! egui paints through a small custom wgpu backend (`EguiPaint`) because
//! egui-wgpu pins a different wgpu major than the renderer; the paint side
//! of egui is just textured triangles and one texture manager, so we own
//! those ~200 lines and keep the renderer's wgpu.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use egui::epaint::{ImageDelta, Primitive};
use egui::{ClippedPrimitive, Color32, TextureId, TexturesDelta};
use glam::DVec3;

use crate::planet::{face_from_dir, ground_tint, Planet};

// ------------------------------------------------------------- photo index

/// One screenshot with a known pose. `day_time_s` comes from the sidecar
/// (present on every shot taken after 2026-07-08); older filename-only
/// shots still get position/view from the name.
pub struct Photo {
    pub path: PathBuf,
    pub name: String,
    pub lat: f64,
    pub lon: f64,
    pub alt_km: f64,
    pub yaw_deg: f64,
    pub pitch_deg: f64,
    pub day_time_s: Option<f64>,
}

fn parse_filename(name: &str) -> Option<(f64, f64, f64, f64, f64)> {
    // shot_lat4.990_lon-29.403_alt0.047km_yaw37_pitch-29.png
    let grab = |key: &str, until: &str| -> Option<f64> {
        let s = name.split(key).nth(1)?;
        let s = s.split(until).next()?;
        s.parse().ok()
    };
    Some((
        grab("_lat", "_lon")?,
        grab("_lon", "_alt")?,
        grab("_alt", "km")?,
        grab("_yaw", "_pitch")?,
        grab("_pitch", ".png")?,
    ))
}

/// Scan the interchange dir (top level only — harness run output lives in
/// subdirectories and is not the player's photo roll).
pub fn scan_photos(interchange: &Path) -> Vec<Photo> {
    let mut out = Vec::new();
    let Ok(rd) = std::fs::read_dir(interchange) else {
        return out;
    };
    for e in rd.flatten() {
        let path = e.path();
        let name = e.file_name().to_string_lossy().to_string();
        if !name.starts_with("shot_") || !name.ends_with(".png") {
            continue;
        }
        let sidecar = path.with_extension("json");
        let mut photo: Option<Photo> = None;
        if let Ok(raw) = std::fs::read_to_string(&sidecar)
            && let Ok(js) = serde_json::from_str::<serde_json::Value>(&raw)
        {
            let f = |k: &str| js.get(k).and_then(|v| v.as_f64());
            if let (Some(lat), Some(lon)) = (f("lat_deg"), f("lon_deg")) {
                photo = Some(Photo {
                    path: path.clone(),
                    name: name.clone(),
                    lat,
                    lon,
                    alt_km: f("alt_km").unwrap_or(0.3),
                    yaw_deg: f("yaw_deg").unwrap_or(0.0),
                    pitch_deg: f("pitch_deg").unwrap_or(-20.0),
                    day_time_s: f("day_cycle_time_s"),
                });
            }
        }
        if photo.is_none()
            && let Some((lat, lon, alt, yaw, pitch)) = parse_filename(&name)
        {
            photo = Some(Photo {
                path: path.clone(),
                name: name.clone(),
                lat,
                lon,
                alt_km: alt,
                yaw_deg: yaw,
                pitch_deg: pitch,
                day_time_s: None,
            });
        }
        if let Some(p) = photo {
            out.push(p);
        }
    }
    // newest first: the shot you just took is the one you want
    out.sort_by(|a, b| {
        let m = |p: &Photo| std::fs::metadata(&p.path).and_then(|m| m.modified()).ok();
        m(b).cmp(&m(a))
    });
    out
}

// ---------------------------------------------------------------- minimap

/// Equirectangular minimap from the baked rasters (bilinear elevation +
/// koppen): ocean by depth, land by biome tint shaded with elevation.
/// ~0.5 Mpx of raster reads — built once, on first open.
fn build_minimap(planet: &Planet, w: usize, h: usize) -> egui::ColorImage {
    let mut px = vec![Color32::BLACK; w * h];
    for y in 0..h {
        let lat = (90.0 - 180.0 * (y as f64 + 0.5) / h as f64).to_radians();
        for x in 0..w {
            let lon = (-180.0 + 360.0 * (x as f64 + 0.5) / w as f64).to_radians();
            let dir = DVec3::new(lat.cos() * lon.cos(), lat.cos() * lon.sin(), lat.sin());
            let (f, u, v) = face_from_dir(dir);
            let e = planet.elevation(f, u, v) as f64;
            let k = planet.koppen(f, u, v);
            let t = planet.temp(f, u, v);
            let c = if k == 255 {
                // sea: deep navy to shelf teal
                let d = (-e / 4.0).clamp(0.0, 1.0) as f32;
                [
                    0.10 + (0.02 - 0.10) * d,
                    0.32 + (0.08 - 0.32) * d,
                    0.42 + (0.22 - 0.42) * d,
                ]
            } else {
                let g = ground_tint(k);
                // shade land by elevation so ranges read on the map
                let l = (e / 4.5).clamp(0.0, 1.0) as f32;
                let snow = if t < -9.0 { 0.75f32 } else { 0.0 };
                [
                    g[0] + (0.93 - g[0]) * l.max(snow),
                    g[1] + (0.90 - g[1]) * l.max(snow),
                    g[2] + (0.88 - g[2]) * l.max(snow),
                ]
            };
            px[y * w + x] = Color32::from_rgb(
                (c[0].clamp(0.0, 1.0).powf(1.0 / 2.2) * 255.0) as u8,
                (c[1].clamp(0.0, 1.0).powf(1.0 / 2.2) * 255.0) as u8,
                (c[2].clamp(0.0, 1.0).powf(1.0 / 2.2) * 255.0) as u8,
            );
        }
    }
    egui::ColorImage { size: [w, h], source_size: egui::Vec2::new(w as f32, h as f32), pixels: px }
}

// ------------------------------------------------------------- popup state

/// What the popup asks the app to do when the player commits.
pub struct TeleportAction {
    pub lat: f64,
    pub lon: f64,
    pub alt_km: Option<f64>,
    pub yaw_deg: Option<f64>,
    pub pitch_deg: Option<f64>,
    /// Some(seconds into the day cycle) when "restore time of day" is on
    /// and the photo recorded it.
    pub day_time_s: Option<f64>,
}

pub struct PhotoMap {
    pub open: bool,
    interchange: PathBuf,
    photos: Vec<Photo>,
    map_tex: Option<egui::TextureHandle>,
    preview: Option<(usize, egui::TextureHandle)>,
    selected: Option<usize>,
    checked: HashSet<usize>,
    custom_dest: Option<(f64, f64)>,
    confirm_delete: bool,
    restore_time: bool,
    coord_input: String,
    scroll_to_selected: bool,
    status: String,
}

impl PhotoMap {
    pub fn new(interchange: PathBuf) -> Self {
        Self {
            open: false,
            interchange,
            photos: Vec::new(),
            map_tex: None,
            preview: None,
            selected: None,
            checked: HashSet::new(),
            custom_dest: None,
            confirm_delete: false,
            restore_time: false,
            coord_input: String::new(),
            scroll_to_selected: false,
            status: String::new(),
        }
    }

    pub fn toggle(&mut self) {
        self.open = !self.open;
        if self.open {
            self.photos = scan_photos(&self.interchange);
            self.selected = None;
            self.preview = None;
            self.checked.clear();
            self.custom_dest = None;
            self.confirm_delete = false;
            self.status = format!("{} photos", self.photos.len());
        }
    }

    fn load_preview(&mut self, ctx: &egui::Context, idx: usize) {
        if self.preview.as_ref().is_some_and(|(i, _)| *i == idx) {
            return;
        }
        let Some(photo) = self.photos.get(idx) else { return };
        let Ok(raw) = std::fs::read(&photo.path) else { return };
        let Ok(mut dec) = png_dims_and_rgba(&raw) else { return };
        // downscale to <=560 px wide for the preview texture
        let max_w = 560usize;
        if dec.0[0] > max_w {
            let step = dec.0[0].div_ceil(max_w);
            let (nw, nh) = (dec.0[0] / step, dec.0[1] / step);
            let mut small = Vec::with_capacity(nw * nh);
            for y in 0..nh {
                for x in 0..nw {
                    small.push(dec.1[y * step * dec.0[0] + x * step]);
                }
            }
            dec = ([nw, nh], small);
        }
        let img = egui::ColorImage {
            size: dec.0,
            source_size: egui::Vec2::new(dec.0[0] as f32, dec.0[1] as f32),
            pixels: dec.1,
        };
        let tex = ctx.load_texture(format!("preview{idx}"), img, Default::default());
        self.preview = Some((idx, tex));
    }

    fn trash_pair_paths(trash: &Path, png: &Path) -> Option<(PathBuf, PathBuf)> {
        let stem = png.file_stem()?.to_string_lossy();
        let ext = png.extension().and_then(|e| e.to_str()).unwrap_or("png");
        for suffix in 0usize.. {
            let base = if suffix == 0 {
                stem.to_string()
            } else {
                format!("{stem}-{suffix}")
            };
            let png_dest = trash.join(format!("{base}.{ext}"));
            let json_dest = trash.join(format!("{base}.json"));
            if !png_dest.exists() && !json_dest.exists() {
                return Some((png_dest, json_dest));
            }
        }
        None
    }

    fn delete_checked(&mut self) {
        let trash = self.interchange.join("trash");
        if let Err(e) = std::fs::create_dir_all(&trash) {
            self.status = format!("trash unavailable: {e}");
            return;
        }
        let mut moved_files = 0usize;
        let mut failed_files = 0usize;
        for &i in &self.checked {
            if let Some(p) = self.photos.get(i) {
                let Some((png_dest, json_dest)) = Self::trash_pair_paths(&trash, &p.path) else {
                    failed_files += 1;
                    continue;
                };
                if std::fs::rename(&p.path, &png_dest).is_err() {
                    failed_files += 1;
                    continue;
                }
                moved_files += 1;
                let sc = p.path.with_extension("json");
                if sc.exists() {
                    match std::fs::rename(&sc, &json_dest) {
                        Ok(()) => moved_files += 1,
                        Err(_) => {
                            failed_files += 1;
                            moved_files -= 1;
                            if std::fs::rename(&png_dest, &p.path).is_err() {
                                failed_files += 1;
                            }
                        }
                    }
                }
            }
        }
        self.status = if failed_files == 0 {
            format!("moved {moved_files} file(s) to interchange/trash/")
        } else {
            format!("moved {moved_files} file(s); {failed_files} file failure(s)")
        };
        self.checked.clear();
        self.selected = None;
        self.preview = None;
        self.photos = scan_photos(&self.interchange);
    }

    /// Build the popup for this frame. Returns a teleport action when the
    /// player commits (the caller closes the popup by our `open` flag).
    pub fn ui(&mut self, ctx: &egui::Context, planet: &Planet) -> Option<TeleportAction> {
        if !self.open {
            return None;
        }
        if self.map_tex.is_none() {
            let img = build_minimap(planet, 1024, 512);
            self.map_tex =
                Some(ctx.load_texture("planet-minimap", img, egui::TextureOptions::LINEAR));
        }
        let mut action: Option<TeleportAction> = None;
        let screen = ctx.content_rect();
        egui::Window::new("Photo map — teleport")
            .collapsible(false)
            .resizable(true)
            .default_size(egui::vec2(screen.width() * 0.86, screen.height() * 0.84))
            .pivot(egui::Align2::CENTER_CENTER)
            .default_pos(screen.center())
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label(&self.status);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label("Esc closes · click map = set destination · click marker/list = pick photo");
                    });
                });
                ui.separator();
                let list_w = 340.0;
                ui.horizontal_top(|ui| {
                    // ---------------- left: map + preview ----------------
                    ui.vertical(|ui| {
                        let avail = ui.available_width() - list_w;
                        let map_w = avail.max(320.0);
                        let map_h = map_w * 0.5;
                        let (rect, resp) = ui.allocate_exact_size(
                            egui::vec2(map_w, map_h),
                            egui::Sense::click(),
                        );
                        if let Some(tex) = &self.map_tex {
                            ui.painter().image(
                                tex.id(),
                                rect,
                                egui::Rect::from_min_max(
                                    egui::pos2(0.0, 0.0),
                                    egui::pos2(1.0, 1.0),
                                ),
                                Color32::WHITE,
                            );
                        }
                        let project = |lat: f64, lon: f64| -> egui::Pos2 {
                            egui::pos2(
                                rect.left() + ((lon + 180.0) / 360.0) as f32 * rect.width(),
                                rect.top() + ((90.0 - lat) / 180.0) as f32 * rect.height(),
                            )
                        };
                        // markers
                        for (i, p) in self.photos.iter().enumerate() {
                            let pos = project(p.lat, p.lon);
                            let sel = self.selected == Some(i);
                            let checkedc = self.checked.contains(&i);
                            let fill = if sel {
                                Color32::from_rgb(255, 230, 90)
                            } else if checkedc {
                                Color32::from_rgb(255, 120, 90)
                            } else {
                                Color32::from_rgb(80, 200, 255)
                            };
                            ui.painter().circle_filled(pos, if sel { 6.0 } else { 4.0 }, fill);
                            ui.painter().circle_stroke(
                                pos,
                                if sel { 8.0 } else { 5.5 },
                                egui::Stroke::new(1.5, Color32::from_black_alpha(160)),
                            );
                        }
                        if let Some((la, lo)) = self.custom_dest {
                            let pos = project(la, lo);
                            let s = 7.0;
                            let st = egui::Stroke::new(2.0, Color32::from_rgb(255, 90, 90));
                            ui.painter()
                                .line_segment([pos - egui::vec2(s, 0.0), pos + egui::vec2(s, 0.0)], st);
                            ui.painter()
                                .line_segment([pos - egui::vec2(0.0, s), pos + egui::vec2(0.0, s)], st);
                        }
                        // clicks: nearest marker within 10 px, else custom dest
                        if resp.clicked()
                            && let Some(click) = resp.interact_pointer_pos()
                        {
                            let mut best: Option<(usize, f32)> = None;
                            for (i, p) in self.photos.iter().enumerate() {
                                let d = project(p.lat, p.lon).distance(click);
                                if d < 10.0 && best.is_none_or(|(_, bd)| d < bd) {
                                    best = Some((i, d));
                                }
                            }
                            match best {
                                Some((i, _)) => {
                                    self.selected = Some(i);
                                    self.custom_dest = None;
                                    self.scroll_to_selected = true;
                                }
                                None => {
                                    let lon = -180.0
                                        + 360.0 * ((click.x - rect.left()) / rect.width()) as f64;
                                    let lat = 90.0
                                        - 180.0 * ((click.y - rect.top()) / rect.height()) as f64;
                                    self.custom_dest = Some((lat, lon));
                                    self.selected = None;
                                }
                            }
                        }
                        // hover label, painted beside the cursor
                        if let Some(hp) = resp.hover_pos() {
                            for p in &self.photos {
                                if project(p.lat, p.lon).distance(hp) < 10.0 {
                                    let font = egui::FontId::proportional(12.0);
                                    let galley = ui.painter().layout_no_wrap(
                                        p.name.clone(),
                                        font,
                                        Color32::WHITE,
                                    );
                                    let at = hp + egui::vec2(14.0, -8.0);
                                    let bg = egui::Rect::from_min_size(at, galley.size())
                                        .expand(4.0);
                                    ui.painter().rect_filled(
                                        bg,
                                        4.0,
                                        Color32::from_black_alpha(200),
                                    );
                                    ui.painter().galley(at, galley, Color32::WHITE);
                                    break;
                                }
                            }
                        }
                        // preview of the selected photo
                        if let Some(sel) = self.selected {
                            self.load_preview(ui.ctx(), sel);
                            if let Some((pi, tex)) = &self.preview
                                && *pi == sel
                            {
                                let size = tex.size_vec2();
                                let scale = (map_w / size.x).min(220.0 / size.y).min(1.0);
                                ui.add_space(6.0);
                                ui.image((tex.id(), size * scale));
                            }
                        }
                    });
                    ui.separator();
                    // ---------------- right: the photo list ----------------
                    ui.vertical(|ui| {
                        ui.set_width(list_w);
                        ui.horizontal(|ui| {
                            if ui.button("Select all").clicked() {
                                self.checked = (0..self.photos.len()).collect();
                            }
                            if ui.button("Clear").clicked() {
                                self.checked.clear();
                            }
                            let n = self.checked.len();
                            ui.add_enabled_ui(n > 0, |ui| {
                                if ui
                                    .button(format!("Delete {n}…"))
                                    .on_hover_text("moves to interchange/trash/")
                                    .clicked()
                                {
                                    self.confirm_delete = true;
                                }
                            });
                        });
                        ui.separator();
                        let row_h = 34.0;
                        egui::ScrollArea::vertical().max_height(
                            ui.available_height() - 96.0,
                        ).show_rows(
                            ui,
                            row_h,
                            self.photos.len(),
                            |ui, range| {
                                for i in range {
                                    let p = &self.photos[i];
                                    let sel = self.selected == Some(i);
                                    ui.horizontal(|ui| {
                                        let mut ck = self.checked.contains(&i);
                                        if ui.checkbox(&mut ck, "").changed() {
                                            if ck {
                                                self.checked.insert(i);
                                            } else {
                                                self.checked.remove(&i);
                                            }
                                        }
                                        let label = format!(
                                            "{:.3} {:.3}  alt {:.0} m{}",
                                            p.lat,
                                            p.lon,
                                            p.alt_km * 1000.0,
                                            if p.day_time_s.is_some() { "  ⏱" } else { "" },
                                        );
                                        let r = ui.selectable_label(sel, label);
                                        if sel && self.scroll_to_selected {
                                            r.scroll_to_me(Some(egui::Align::Center));
                                        }
                                        if r.clicked() {
                                            self.selected = Some(i);
                                            self.custom_dest = None;
                                        }
                                        r.on_hover_text(&p.name);
                                    });
                                }
                                self.scroll_to_selected = false;
                            },
                        );
                        ui.separator();
                        // manual coordinates, like the old title-bar prompt
                        ui.horizontal(|ui| {
                            ui.label("lat lon [alt km]:");
                            ui.text_edit_singleline(&mut self.coord_input);
                        });
                        ui.checkbox(
                            &mut self.restore_time,
                            "Restore photo's time of day",
                        )
                        .on_hover_text(
                            "teleporting to a photo also rewinds the day/night cycle \
                             to the moment it was taken (sidecar shots only)",
                        );
                        let dest = self.destination();
                        ui.add_enabled_ui(dest.is_some(), |ui| {
                            let label = match (&self.selected, &self.custom_dest) {
                                (Some(_), _) => "Teleport to photo",
                                (None, Some(_)) => "Teleport to map point",
                                _ if !self.coord_input.trim().is_empty() => "Teleport to coordinates",
                                _ => "Teleport",
                            };
                            if ui.button(label).clicked() {
                                action = dest;
                            }
                        });
                    });
                });
            });
        // delete confirmation modal
        if self.confirm_delete {
            let n = self.checked.len();
            egui::Window::new("Delete photos?")
                .collapsible(false)
                .resizable(false)
                .pivot(egui::Align2::CENTER_CENTER)
                .default_pos(screen.center())
                .show(ctx, |ui| {
                    ui.label(format!(
                        "Move {n} photo{} (and sidecars) to interchange/trash/?",
                        if n == 1 { "" } else { "s" }
                    ));
                    ui.horizontal(|ui| {
                        if ui.button("Delete").clicked() {
                            self.delete_checked();
                            self.confirm_delete = false;
                        }
                        if ui.button("Cancel").clicked() {
                            self.confirm_delete = false;
                        }
                    });
                });
        }
        if action.is_some() {
            self.open = false;
        }
        action
    }

    fn destination(&self) -> Option<TeleportAction> {
        if let Some(i) = self.selected {
            let p = self.photos.get(i)?;
            return Some(TeleportAction {
                lat: p.lat,
                lon: p.lon,
                alt_km: Some(p.alt_km.max(0.0025)),
                yaw_deg: Some(p.yaw_deg),
                pitch_deg: Some(p.pitch_deg),
                day_time_s: if self.restore_time { p.day_time_s } else { None },
            });
        }
        if let Some((lat, lon)) = self.custom_dest {
            return Some(TeleportAction {
                lat,
                lon,
                alt_km: None,
                yaw_deg: None,
                pitch_deg: None,
                day_time_s: None,
            });
        }
        // manual "lat lon [alt]" text, the old prompt's grammar — strict:
        // exactly 2 or 3 tokens, every one a finite number ("NaN" parses as
        // a valid f64 and would poison the camera), latitude in range
        let toks: Vec<&str> = self.coord_input.split_whitespace().collect();
        if toks.len() == 2 || toks.len() == 3 {
            let parts: Vec<f64> = toks
                .iter()
                .filter_map(|t| t.parse().ok())
                .filter(|v: &f64| v.is_finite())
                .collect();
            if parts.len() == toks.len() && parts[0].abs() <= 90.0 {
                return Some(TeleportAction {
                    lat: parts[0],
                    lon: parts[1],
                    alt_km: parts.get(2).copied().filter(|a| *a > 0.0),
                    yaw_deg: None,
                    pitch_deg: None,
                    day_time_s: None,
                });
            }
        }
        None
    }
}

/// Decode a PNG into egui Color32 pixels (RGB/RGBA/gray supported).
fn png_dims_and_rgba(raw: &[u8]) -> anyhow::Result<([usize; 2], Vec<Color32>)> {
    let decoder = png::Decoder::new(std::io::Cursor::new(raw));
    let mut reader = decoder.read_info()?;
    let size = reader
        .output_buffer_size()
        .ok_or_else(|| anyhow::anyhow!("png output size overflow"))?;
    let mut buf = vec![0u8; size];
    let info = reader.next_frame(&mut buf)?;
    let (w, h) = (info.width as usize, info.height as usize);
    let mut px = Vec::with_capacity(w * h);
    match info.color_type {
        png::ColorType::Rgba => {
            for c in buf[..w * h * 4].chunks_exact(4) {
                px.push(Color32::from_rgba_unmultiplied(c[0], c[1], c[2], c[3]));
            }
        }
        png::ColorType::Rgb => {
            for c in buf[..w * h * 3].chunks_exact(3) {
                px.push(Color32::from_rgb(c[0], c[1], c[2]));
            }
        }
        png::ColorType::Grayscale => {
            for &g in &buf[..w * h] {
                px.push(Color32::from_gray(g));
            }
        }
        other => anyhow::bail!("unsupported png color type {other:?}"),
    }
    Ok(([w, h], px))
}

// ----------------------------------------------------- egui paint (wgpu 30)

/// Minimal egui renderer against the viewer's own wgpu version. egui's
/// output is clipped, textured, vertex-colored triangles in physical pixel
/// space; this uploads them and draws with premultiplied-alpha blending
/// onto the already-rendered frame.
pub struct EguiPaint {
    pipeline: wgpu::RenderPipeline,
    sampler: wgpu::Sampler,
    bind_layout: wgpu::BindGroupLayout,
    textures: std::collections::HashMap<TextureId, (wgpu::Texture, wgpu::BindGroup)>,
    vbuf: Option<wgpu::Buffer>,
    ibuf: Option<wgpu::Buffer>,
    uniform: wgpu::Buffer,
    uniform_bind: wgpu::BindGroup,
}

const EGUI_SHADER: &str = r#"
struct Screen { size_px: vec2<f32>, _pad: vec2<f32> };
@group(0) @binding(0) var<uniform> screen: Screen;
@group(1) @binding(0) var tex: texture_2d<f32>;
@group(1) @binding(1) var samp: sampler;

struct VsIn {
    @location(0) pos: vec2<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) color: vec4<f32>, // sRGB 0-1, premultiplied coverage semantics
};
struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
};

fn srgb_to_linear(c: vec3<f32>) -> vec3<f32> {
    let lo = c / 12.92;
    let hi = pow((c + vec3<f32>(0.055)) / 1.055, vec3<f32>(2.4));
    return select(hi, lo, c < vec3<f32>(0.04045));
}

@vertex
fn vs_main(in: VsIn) -> VsOut {
    var out: VsOut;
    out.pos = vec4<f32>(
        in.pos.x / screen.size_px.x * 2.0 - 1.0,
        1.0 - in.pos.y / screen.size_px.y * 2.0,
        0.0,
        1.0,
    );
    out.uv = in.uv;
    out.color = vec4<f32>(srgb_to_linear(in.color.rgb), in.color.a);
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let t = textureSample(tex, samp, in.uv);
    // egui vertex colors and textures are ALREADY premultiplied — do not
    // multiply rgb by alpha again (double-premultiply darkens translucent
    // panels and fringes text edges)
    return in.color * t;
}
"#;

impl EguiPaint {
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("egui"),
            source: wgpu::ShaderSource::Wgsl(EGUI_SHADER.into()),
        });
        let uniform_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("egui-uniform"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let bind_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("egui-tex"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        multisampled: false,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("egui"),
            bind_group_layouts: &[Some(&uniform_layout), Some(&bind_layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("egui"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: 20,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &[
                        wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Float32x2,
                            offset: 0,
                            shader_location: 0,
                        },
                        wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Float32x2,
                            offset: 8,
                            shader_location: 1,
                        },
                        wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Unorm8x4,
                            offset: 16,
                            shader_location: 2,
                        },
                    ],
                })],
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState {
                        color: wgpu::BlendComponent {
                            src_factor: wgpu::BlendFactor::One,
                            dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                            operation: wgpu::BlendOperation::Add,
                        },
                        alpha: wgpu::BlendComponent {
                            src_factor: wgpu::BlendFactor::One,
                            dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                            operation: wgpu::BlendOperation::Add,
                        },
                    }),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("egui"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("egui-screen"),
            size: 16,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let uniform_bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("egui-screen"),
            layout: &uniform_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform.as_entire_binding(),
            }],
        });
        Self {
            pipeline,
            sampler,
            bind_layout,
            textures: Default::default(),
            vbuf: None,
            ibuf: None,
            uniform,
            uniform_bind,
        }
    }

    fn apply_texture(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        id: TextureId,
        delta: &ImageDelta,
    ) {
        let size = delta.image.size();
        let pixels: Vec<u8> = match &delta.image {
            egui::ImageData::Color(img) => {
                img.pixels.iter().flat_map(|c| c.to_array()).collect()
            }
        };
        let whole = delta.pos.is_none();
        if whole {
            let tex = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("egui-tex"),
                size: wgpu::Extent3d {
                    width: size[0] as u32,
                    height: size[1] as u32,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8UnormSrgb,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });
            let view = tex.create_view(&Default::default());
            let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("egui-tex"),
                layout: &self.bind_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(&self.sampler),
                    },
                ],
            });
            self.textures.insert(id, (tex, bind));
        }
        if let Some((tex, _)) = self.textures.get(&id) {
            let (x, y) = delta.pos.map_or((0, 0), |p| (p[0] as u32, p[1] as u32));
            queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: tex,
                    mip_level: 0,
                    origin: wgpu::Origin3d { x, y, z: 0 },
                    aspect: wgpu::TextureAspect::All,
                },
                &pixels,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(4 * size[0] as u32),
                    rows_per_image: None,
                },
                wgpu::Extent3d {
                    width: size[0] as u32,
                    height: size[1] as u32,
                    depth_or_array_layers: 1,
                },
            );
        }
    }

    /// Paint one egui frame onto `view` (which already holds the scene).
    pub fn paint(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        view: &wgpu::TextureView,
        size_px: (u32, u32),
        pixels_per_point: f32,
        primitives: &[ClippedPrimitive],
        deltas: &TexturesDelta,
    ) {
        for (id, delta) in &deltas.set {
            self.apply_texture(device, queue, *id, delta);
        }
        // flatten meshes into one vertex/index upload
        let mut verts: Vec<u8> = Vec::new();
        let mut idxs: Vec<u32> = Vec::new();
        let mut draws = Vec::new(); // (clip, tex, index range, base vertex)
        let mut vcount = 0u32;
        for cp in primitives {
            let Primitive::Mesh(mesh) = &cp.primitive else { continue };
            let istart = idxs.len() as u32;
            idxs.extend(mesh.indices.iter().map(|&i| i + vcount));
            for v in &mesh.vertices {
                verts.extend_from_slice(&v.pos.x.to_le_bytes());
                verts.extend_from_slice(&v.pos.y.to_le_bytes());
                verts.extend_from_slice(&v.uv.x.to_le_bytes());
                verts.extend_from_slice(&v.uv.y.to_le_bytes());
                verts.extend_from_slice(&v.color.to_array());
            }
            vcount += mesh.vertices.len() as u32;
            draws.push((cp.clip_rect, mesh.texture_id, istart..idxs.len() as u32));
        }
        if draws.is_empty() {
            return;
        }
        let vbytes: &[u8] = &verts;
        let ibytes: &[u8] = bytemuck::cast_slice(&idxs);
        let need_v = vbytes.len() as u64;
        let need_i = ibytes.len() as u64;
        if self.vbuf.as_ref().is_none_or(|b| b.size() < need_v) {
            self.vbuf = Some(device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("egui-v"),
                size: need_v.next_power_of_two().max(1 << 16),
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }));
        }
        if self.ibuf.as_ref().is_none_or(|b| b.size() < need_i) {
            self.ibuf = Some(device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("egui-i"),
                size: need_i.next_power_of_two().max(1 << 16),
                usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }));
        }
        let (vbuf, ibuf) = (self.vbuf.as_ref().unwrap(), self.ibuf.as_ref().unwrap());
        queue.write_buffer(vbuf, 0, vbytes);
        queue.write_buffer(ibuf, 0, ibytes);
        let logical = [
            size_px.0 as f32 / pixels_per_point,
            size_px.1 as f32 / pixels_per_point,
            0.0,
            0.0,
        ];
        queue.write_buffer(&self.uniform, 0, bytemuck::cast_slice(&logical));
        let mut enc =
            device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("egui") });
        {
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("egui"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &self.uniform_bind, &[]);
            pass.set_vertex_buffer(0, vbuf.slice(..));
            pass.set_index_buffer(ibuf.slice(..), wgpu::IndexFormat::Uint32);
            for (clip, tex_id, range) in draws {
                let Some((_, bind)) = self.textures.get(&tex_id) else { continue };
                // clip rect: logical points -> physical pixels, clamped
                let cx = (clip.min.x * pixels_per_point).max(0.0) as u32;
                let cy = (clip.min.y * pixels_per_point).max(0.0) as u32;
                let cx1 = ((clip.max.x * pixels_per_point) as u32).min(size_px.0);
                let cy1 = ((clip.max.y * pixels_per_point) as u32).min(size_px.1);
                if cx1 <= cx || cy1 <= cy {
                    continue;
                }
                pass.set_scissor_rect(cx, cy, cx1 - cx, cy1 - cy);
                pass.set_bind_group(1, bind, &[]);
                pass.draw_indexed(range, 0, 0..1);
            }
        }
        queue.submit([enc.finish()]);
        for id in &deltas.free {
            self.textures.remove(id);
        }
    }
}
