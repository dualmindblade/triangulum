//! The annotator (B6): draw on a picture and say what's wrong in a sentence.
//! The T window opens it on a photo, and F9 opens it on the frame just grabbed.
//!
//! Marks are data, in the picture's own pixels (origin top left), so a reader
//! can say exactly which pixels were meant: `annotations` in the sidecar. The
//! original PNG is never touched. A flattened copy, `<stem>_annotated.png`, is
//! written beside it so nobody needs a tool to see them. One function paints a
//! mark on screen and into that copy (`paint_mark`), and the copy goes through
//! egui's own tessellator into a small software rasteriser (`flatten`), so it
//! is what was on screen, on any thread, with no GPU.
//!
//! The pen reads the raw pointer events, every one of them, not egui's drag
//! gesture: a stroke starts on the pixel that was pressed, in the pass that saw
//! the press, with no dead zone before it, and a fast stroke keeps all its
//! points between two frames.
//!
//! `Gate` is what keeps the game out of it. F9 stops the steps; while the
//! annotator is open the game sees no press, click, motion or wheel, and every
//! release, so what was held at F9 comes back as it should and nothing sticks.

use crate::controls::Controls;
use crate::session::Session;
use egui::{Color32, Id, Key, Modifiers, Pos2, Rect, Stroke, pos2, vec2};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::path::Path;
use winit::event::MouseButton;
use winit::keyboard::KeyCode;

/// The colours, strong enough to read on any terrain.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Ink {
    Red,
    Yellow,
    Green,
    Blue,
    White,
}

impl Ink {
    pub const ALL: [Ink; 5] = [Ink::Red, Ink::Yellow, Ink::Green, Ink::Blue, Ink::White];

    pub fn colour(self) -> Color32 {
        match self {
            Ink::Red => Color32::from_rgb(255, 44, 44),
            Ink::Yellow => Color32::from_rgb(255, 225, 30),
            Ink::Green => Color32::from_rgb(40, 235, 90),
            Ink::Blue => Color32::from_rgb(40, 190, 255),
            Ink::White => Color32::WHITE,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Ink::Red => "red",
            Ink::Yellow => "yellow",
            Ink::Green => "green",
            Ink::Blue => "blue",
            Ink::White => "white",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tool {
    Pen,
    Arrow,
    Box,
    Text,
}

impl Tool {
    pub const ALL: [Tool; 4] = [Tool::Pen, Tool::Arrow, Tool::Box, Tool::Text];

    pub fn name(self) -> &'static str {
        match self {
            Tool::Pen => "Pen",
            Tool::Arrow => "Arrow",
            Tool::Box => "Box",
            Tool::Text => "Text",
        }
    }
}

/// One mark, in the picture's pixels. The sidecar's `annotations.marks`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "tool", rename_all = "lowercase")]
pub enum Mark {
    Pen { colour: Ink, points: Vec<[f64; 2]> },
    Arrow { colour: Ink, from: [f64; 2], to: [f64; 2] },
    Box { colour: Ink, from: [f64; 2], to: [f64; 2] },
    Text { colour: Ink, at: [f64; 2], text: String },
}

impl Mark {
    /// In words, for someone who can't see the picture.
    pub fn describe(&self) -> String {
        let p = |a: &[f64; 2]| format!("({:.0}, {:.0})", a[0], a[1]);
        match self {
            Mark::Pen { colour, points } => {
                let (mut lo, mut hi) = ([f64::MAX; 2], [f64::MIN; 2]);
                for q in points {
                    lo = [lo[0].min(q[0]), lo[1].min(q[1])];
                    hi = [hi[0].max(q[0]), hi[1].max(q[1])];
                }
                format!("{} pen line within {} to {}", colour.name(), p(&lo), p(&hi))
            }
            Mark::Arrow { colour, from, to } => format!("{} arrow from {} pointing at {}", colour.name(), p(from), p(to)),
            Mark::Box { colour, from, to } => format!("{} box from {} to {}", colour.name(), p(&[from[0].min(to[0]), from[1].min(to[1])]), p(&[from[0].max(to[0]), from[1].max(to[1])])),
            Mark::Text { colour, at, text } => format!("{} text {text:?} at {}", colour.name(), p(at)),
        }
    }
}

/// The marks as a sidecar holds them.
pub fn marks_json(marks: &[Mark], width: u32, height: u32) -> Value {
    json!({ "image": [width, height], "marks": marks })
}

/// The marks in a sidecar; a sidecar with none, or some we can't read, has none.
pub fn marks_of(side: &Value) -> Vec<Mark> {
    serde_json::from_value(side["annotations"]["marks"].clone()).unwrap_or_default()
}

// ------------------------------------------------------------------ paint

/// Line weight and text size for a picture this tall, in its pixels: what an
/// artist's marker looks like on a 1080p frame, the same on any picture.
fn weight(image_h: f32) -> f32 {
    (image_h / 216.0).max(2.0)
}

fn text_size(image_h: f32) -> f32 {
    (image_h / 34.0).max(12.0)
}

/// From the picture's pixels to the painter's points.
#[derive(Clone, Copy)]
struct Place {
    origin: Pos2,
    k: f32,
    image_w: f32,
    image_h: f32,
}

impl Place {
    fn at(&self, p: [f64; 2]) -> Pos2 {
        self.origin + vec2(p[0] as f32, p[1] as f32) * self.k
    }
}

/// One mark, on a screen or into the flattened picture. A dark edge under each
/// so it reads on snow and on rock.
fn paint_mark(paint: &egui::Painter, mark: &Mark, place: Place) {
    let w = weight(place.image_h) * place.k;
    let edge = (w * 0.45).max(1.0);
    let dark = Color32::from_black_alpha(170);
    match mark {
        Mark::Pen { colour, points } => {
            let pts: Vec<Pos2> = points.iter().map(|p| place.at(*p)).collect();
            if pts.len() == 1 {
                paint.circle_filled(pts[0], w * 0.5 + edge, dark);
                paint.circle_filled(pts[0], w * 0.5, colour.colour());
            } else {
                paint.add(egui::Shape::line(pts.clone(), Stroke::new(w + edge * 2.0, dark)));
                paint.add(egui::Shape::line(pts, Stroke::new(w, colour.colour())));
            }
        }
        Mark::Arrow { colour, from, to } => {
            let (a, b) = (place.at(*from), place.at(*to));
            let d = b - a;
            let len = d.length();
            if len < 1.0 {
                return;
            }
            let u = d / len;
            let head = (w * 4.5).min(len * 0.6);
            let n = u.rot90() * head * 0.55;
            let base = b - u * head;
            let tri = vec![b, base + n, base - n];
            paint.line_segment([a, base], Stroke::new(w + edge * 2.0, dark));
            paint.add(egui::Shape::convex_polygon(tri.clone(), Color32::TRANSPARENT, Stroke::new(edge * 2.0, dark)));
            paint.line_segment([a, base], Stroke::new(w, colour.colour()));
            paint.add(egui::Shape::convex_polygon(tri, colour.colour(), Stroke::NONE));
        }
        Mark::Box { colour, from, to } => {
            let r = Rect::from_two_pos(place.at(*from), place.at(*to));
            paint.rect_stroke(r, 0.0, Stroke::new(w + edge * 2.0, dark), egui::StrokeKind::Middle);
            paint.rect_stroke(r, 0.0, Stroke::new(w, colour.colour()), egui::StrokeKind::Middle);
        }
        Mark::Text { colour, at, text } => {
            let size = text_size(place.image_h) * place.k;
            let room = ((place.image_w - at[0] as f32) * place.k - size * 0.5).max(size * 4.0);
            let g = paint.layout(text.clone(), egui::FontId::proportional(size), colour.colour(), room);
            let p = place.at(*at);
            paint.rect_filled(Rect::from_min_size(p, g.size()).expand(size * 0.2), size * 0.2, Color32::from_black_alpha(190));
            paint.galley(p, g, colour.colour());
        }
    }
}

/// The picture with the marks on it, by the code that drew them on screen.
/// Thread-safe, needs no GPU, and leaves `rgba` alone.
pub fn flatten(rgba: &[u8], width: u32, height: u32, marks: &[Mark]) -> Vec<u8> {
    let mut out = rgba.to_vec();
    if marks.is_empty() || width == 0 || height == 0 {
        return out;
    }
    let (w, h) = (width as f32, height as f32);
    let ctx = egui::Context::default();
    let raw = egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(w, h))), ..Default::default() };
    let place = Place { origin: Pos2::ZERO, k: 1.0, image_w: w, image_h: h };
    let full = ctx.run_ui(raw, |ui| {
        let paint = ui.ctx().layer_painter(egui::LayerId::new(egui::Order::Foreground, Id::new("flatten")));
        for m in marks {
            paint_mark(&paint, m, place);
        }
    });
    let atlas = full.textures_delta.set.iter().find_map(|(_, d)| match (&d.image, d.pos) {
        (egui::ImageData::Color(c), None) => Some(c.clone()),
        _ => None,
    });
    let Some(atlas) = atlas else { return out };
    for cp in ctx.tessellate(full.shapes, full.pixels_per_point) {
        if let egui::epaint::Primitive::Mesh(mesh) = &cp.primitive {
            for t in mesh.indices.chunks_exact(3) {
                triangle(&mut out, width as usize, height as usize, cp.clip_rect, &atlas, [&mesh.vertices[t[0] as usize], &mesh.vertices[t[1] as usize], &mesh.vertices[t[2] as usize]]);
            }
        }
    }
    out
}

/// One of egui's triangles into an RGBA picture: premultiplied vertex colours,
/// the font atlas, blended over what is there.
fn triangle(dst: &mut [u8], w: usize, h: usize, clip: Rect, atlas: &egui::ColorImage, v: [&egui::epaint::Vertex; 3]) {
    let cross = |a: Pos2, b: Pos2, c: Pos2| (b.x - a.x) * (c.y - a.y) - (b.y - a.y) * (c.x - a.x);
    let area = cross(v[0].pos, v[1].pos, v[2].pos);
    if area.abs() < 1e-6 {
        return;
    }
    let lo = |f: fn(&Pos2) -> f32| v.iter().map(|x| f(&x.pos)).fold(f32::MAX, f32::min);
    let hi = |f: fn(&Pos2) -> f32| v.iter().map(|x| f(&x.pos)).fold(f32::MIN, f32::max);
    let x0 = (lo(|p| p.x).max(clip.min.x).floor().max(0.0)) as usize;
    let x1 = (hi(|p| p.x).min(clip.max.x).ceil().min(w as f32)) as usize;
    let y0 = (lo(|p| p.y).max(clip.min.y).floor().max(0.0)) as usize;
    let y1 = (hi(|p| p.y).min(clip.max.y).ceil().min(h as f32)) as usize;
    let c: [[f32; 4]; 3] = [0, 1, 2].map(|i| v[i].color.to_array().map(|x| x as f32));
    let flat_uv = v[0].uv == v[1].uv && v[1].uv == v[2].uv;
    let texel = |u: f32, q: f32| {
        let x = ((u * atlas.size[0] as f32) as usize).min(atlas.size[0] - 1);
        let y = ((q * atlas.size[1] as f32) as usize).min(atlas.size[1] - 1);
        atlas.pixels[y * atlas.size[0] + x].to_array().map(|t| t as f32 / 255.0)
    };
    let flat = texel(v[0].uv.x, v[0].uv.y);
    for y in y0..y1 {
        for x in x0..x1 {
            let p = pos2(x as f32 + 0.5, y as f32 + 0.5);
            let (b0, b1, b2) = (cross(v[1].pos, v[2].pos, p) / area, cross(v[2].pos, v[0].pos, p) / area, cross(v[0].pos, v[1].pos, p) / area);
            if b0 < 0.0 || b1 < 0.0 || b2 < 0.0 {
                continue;
            }
            let t = if flat_uv { flat } else { texel(b0 * v[0].uv.x + b1 * v[1].uv.x + b2 * v[2].uv.x, b0 * v[0].uv.y + b1 * v[1].uv.y + b2 * v[2].uv.y) };
            let src: [f32; 4] = [0, 1, 2, 3].map(|i| (b0 * c[0][i] + b1 * c[1][i] + b2 * c[2][i]) * t[i]);
            let px = &mut dst[(y * w + x) * 4..(y * w + x) * 4 + 4];
            let keep = 1.0 - src[3] / 255.0;
            for i in 0..3 {
                px[i] = (src[i] + px[i] as f32 * keep).round().clamp(0.0, 255.0) as u8;
            }
        }
    }
}

/// Save marks and a comment on a photo that is on disk: the sidecar's
/// `annotations` and `notes`, and `<stem>_annotated.png`. The PNG itself is only
/// read.
pub fn save_to_photo(png: &Path, marks: &[Mark], comment: &str) -> Result<(), String> {
    let (w, h, rgba) = crate::render::read_png(&png.to_string_lossy())?;
    let flat = crate::photos::annotated_of(png);
    if marks.is_empty() {
        match std::fs::remove_file(&flat) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(format!("remove {}: {e}", flat.display())),
        }
    } else {
        crate::photos::write_png_whole(&flat, w, h, &flatten(&rgba, w, h, marks))?;
    }
    crate::photos::edit_sidecar(png, |m| {
        m.insert("annotations".into(), marks_json(marks, w, h));
        m.insert("notes".into(), Value::String(comment.to_string()));
        if marks.is_empty() {
            m.remove("annotated");
        } else {
            m.insert("annotated".into(), Value::String(flat.file_name().map_or(String::new(), |n| n.to_string_lossy().into_owned())));
        }
    })
}

// --------------------------------------------------------------- the gate

/// The game's side of an open annotator (F9). Steps stop; presses, clicks,
/// motion and the wheel go no further; a release always gets through, or a key
/// held at F9 and let go while drawing would stay down, and F9 itself would be
/// dead (it is still "held" in `Controls`). A key held at F9 and held all the
/// way through is still held: the game carries on where it stopped, with the
/// same accumulator and the same queue.
#[derive(Default)]
pub struct Gate {
    open: bool,
    /// Left and right, as the game last heard them.
    down: [bool; 2],
}

fn button_index(b: MouseButton) -> Option<usize> {
    match b {
        MouseButton::Left => Some(0),
        MouseButton::Right => Some(1),
        _ => None,
    }
}

impl Gate {
    pub fn is_open(&self) -> bool {
        self.open
    }

    pub fn open(&mut self) {
        self.open = true;
    }

    pub fn close(&mut self) {
        self.open = false;
    }

    /// A key. True on the first press of a key, as `Controls::key`.
    pub fn key(&self, c: &mut Controls, code: KeyCode, pressed: bool) -> bool {
        if self.open && pressed { false } else { c.key(code, pressed) }
    }

    /// A mouse button the game may see. Inside the annotator, only the release
    /// of a button the game saw go down.
    pub fn button(&mut self, c: &mut Controls, button: MouseButton, pressed: bool) {
        let i = button_index(button);
        if self.open {
            if !pressed && i.is_some_and(|i| std::mem::take(&mut self.down[i])) {
                c.button(button, false);
            }
            return;
        }
        if let Some(i) = i {
            self.down[i] = pressed;
        }
        c.button(button, pressed);
    }

    pub fn motion(&self, c: &mut Controls, dx: f64, dy: f64) {
        if !self.open {
            c.motion(dx, dy);
        }
    }

    pub fn scroll(&self, c: &mut Controls, lines: f64) {
        if !self.open {
            c.scroll(lines);
        }
    }

    /// A frame: its steps, or none while the annotator is open. The time spent
    /// in there is not owed afterwards: nothing accumulates.
    pub fn frame(&self, c: &mut Controls, session: &mut Session, dt: f64) -> usize {
        if self.open { 0 } else { c.frame(session, dt) }
    }
}

// ---------------------------------------------------------------- the panel

/// How the annotator was left.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    Save,
    Cancel,
}

/// A text mark being typed.
struct Draft {
    at: [f64; 2],
    text: String,
}

pub struct Annotator {
    title: String,
    /// A line under the tools: what this is and what the game is doing.
    context: String,
    save_label: String,
    size: [f32; 2],
    picture: Option<egui::ColorImage>,
    texture: Option<egui::TextureHandle>,
    pub marks: Vec<Mark>,
    pub comment: String,
    pub tool: Tool,
    pub ink: Ink,
    /// The mark under the pointer.
    drawing: Option<Mark>,
    draft: Option<Draft>,
    /// Esc pressed once with work to lose.
    armed: bool,
    pub spots: HashMap<String, Rect>,
}

/// An RGBA picture for the screen.
pub fn colour_image(rgba: &[u8], w: u32, h: u32) -> egui::ColorImage {
    let pixels = rgba.chunks_exact(4).map(|p| Color32::from_rgb(p[0], p[1], p[2])).collect();
    egui::ColorImage { size: [w as usize, h as usize], source_size: vec2(w as f32, h as f32), pixels }
}

fn tenth(x: f32) -> f64 {
    (x as f64 * 10.0).round() / 10.0
}

impl Annotator {
    pub fn new(title: impl Into<String>, context: impl Into<String>, save_label: impl Into<String>, width: u32, height: u32) -> Self {
        Self { title: title.into(), context: context.into(), save_label: save_label.into(), size: [width as f32, height as f32], picture: None, texture: None, marks: Vec::new(), comment: String::new(), tool: Tool::Pen, ink: Ink::Red, drawing: None, draft: None, armed: false, spots: HashMap::new() }
    }

    /// The picture, whenever it arrives.
    pub fn set_picture(&mut self, image: egui::ColorImage) {
        self.size = [image.size[0] as f32, image.size[1] as f32];
        self.picture = Some(image);
    }

    pub fn picture_size(&self) -> (u32, u32) {
        (self.size[0] as u32, self.size[1] as u32)
    }

    pub fn has_picture(&self) -> bool {
        self.picture.is_some() || self.texture.is_some()
    }

    /// The mark under the pointer now, if any.
    pub fn drawing(&self) -> Option<&Mark> {
        self.drawing.as_ref()
    }

    fn typing_mark(&self) -> bool {
        self.draft.is_some()
    }

    fn undo(&mut self) {
        if self.draft.take().is_none() && self.drawing.take().is_none() {
            self.marks.pop();
        }
        self.armed = false;
    }

    /// Finish the text being typed: it becomes a mark, unless it's empty.
    fn commit_draft(&mut self) {
        if let Some(d) = self.draft.take()
            && !d.text.trim().is_empty()
        {
            self.marks.push(Mark::Text { colour: self.ink, at: d.at, text: d.text.trim().to_string() });
        }
    }

    fn press(&mut self, p: [f64; 2]) {
        self.armed = false;
        self.commit_draft();
        let colour = self.ink;
        match self.tool {
            Tool::Pen => self.drawing = Some(Mark::Pen { colour, points: vec![p] }),
            Tool::Arrow => self.drawing = Some(Mark::Arrow { colour, from: p, to: p }),
            Tool::Box => self.drawing = Some(Mark::Box { colour, from: p, to: p }),
            Tool::Text => self.draft = Some(Draft { at: p, text: String::new() }),
        }
    }

    fn drag(&mut self, p: [f64; 2]) {
        match &mut self.drawing {
            Some(Mark::Pen { points, .. }) => {
                if points.last().is_none_or(|q| (q[0] - p[0]).hypot(q[1] - p[1]) >= 0.5) {
                    points.push(p);
                }
            }
            Some(Mark::Arrow { to, .. } | Mark::Box { to, .. }) => *to = p,
            _ => {}
        }
    }

    fn release(&mut self) {
        let Some(m) = self.drawing.take() else { return };
        // A click with the box or the arrow isn't a mark.
        let keep = match &m {
            Mark::Arrow { from, to, .. } | Mark::Box { from, to, .. } => (from[0] - to[0]).abs().max((from[1] - to[1]).abs()) >= 4.0,
            _ => true,
        };
        if keep {
            self.marks.push(m);
        }
    }

    /// Every pointer event of this pass, in order: the stroke is painted in the
    /// same pass, and a fast one keeps all its points.
    fn pointer(&mut self, ctx: &egui::Context, canvas: Rect) {
        let k = canvas.width() / self.size[0];
        let [w, h] = self.size;
        let to_image = |p: Pos2| [tenth(((p.x - canvas.min.x) / k).clamp(0.0, w)), tenth(((p.y - canvas.min.y) / k).clamp(0.0, h))];
        let events = ctx.input(|i| i.events.clone());
        // The field of a text mark being typed, not of the last one.
        if self.draft.is_none() {
            self.spots.remove("draft");
        }
        let over_draft = self.spots.get("draft").copied();
        for e in events {
            match e {
                egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: true, .. } if canvas.contains(pos) && self.drawing.is_none() && !over_draft.is_some_and(|r| r.contains(pos)) => self.press(to_image(pos)),
                egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: false, .. } if self.drawing.is_some() => {
                    self.drag(to_image(pos));
                    self.release();
                }
                egui::Event::PointerMoved(pos) if self.drawing.is_some() => self.drag(to_image(pos)),
                egui::Event::PointerGone | egui::Event::WindowFocused(false) => self.release(),
                _ => {}
            }
        }
    }

    /// One pass. Returns how it was left, once. On `Save` everything typed is
    /// in `marks` and `comment`.
    pub fn ui(&mut self, ctx: &egui::Context) -> Option<Outcome> {
        if let Some(image) = self.picture.take() {
            self.texture = Some(ctx.load_texture("annotate", image, egui::TextureOptions::LINEAR));
        }
        // Keys come first, ahead of the text boxes that would act on them.
        let has_marks = !self.marks.is_empty() || self.drawing.is_some() || self.draft.is_some();
        let (undo, enter, esc) = ctx.input_mut(|i| {
            // Shift+Enter is the comment's new line.
            let shifted = i.events.iter().any(|e| matches!(e, egui::Event::Key { key: Key::Enter, pressed: true, modifiers, .. } if modifiers.shift));
            (has_marks && i.consume_key(Modifiers::COMMAND, Key::Z), !shifted && i.consume_key(Modifiers::NONE, Key::Enter), i.consume_key(Modifiers::NONE, Key::Escape))
        });
        let mut out = None;
        if undo {
            self.undo();
        }
        if enter {
            if self.draft.is_some() {
                self.commit_draft();
            } else {
                out = Some(Outcome::Save);
            }
        }
        if esc {
            if self.draft.take().is_none() && self.drawing.take().is_none() {
                if self.armed || (self.marks.is_empty() && self.comment.trim().is_empty()) {
                    out = Some(Outcome::Cancel);
                } else {
                    self.armed = true;
                }
            }
        }
        let screen = ctx.content_rect();
        ctx.layer_painter(egui::LayerId::background()).rect_filled(screen, 0.0, Color32::from_gray(14));
        let fit = vec2(screen.width() * 0.97, (screen.height() - 44.0).max(240.0));
        egui::Window::new(self.title.as_str()).id(Id::new("annotator")).collapsible(false).resizable(false).movable(false).fixed_size(fit).pivot(egui::Align2::CENTER_CENTER).fixed_pos(screen.center()).show(ctx, |ui| {
            self.ui_tools(ui);
            ui.separator();
            let foot = 118.0;
            let room = vec2(ui.available_width(), (ui.available_height() - foot).max(80.0));
            let k = (room.x / self.size[0]).min(room.y / self.size[1]).min(3.0);
            let size = vec2(self.size[0], self.size[1]) * k;
            let (space, _) = ui.allocate_exact_size(room, egui::Sense::hover());
            let canvas = Rect::from_center_size(space.center(), size);
            let resp = ui.interact(canvas, ui.id().with("canvas"), egui::Sense::drag());
            self.spots.insert("canvas".into(), canvas);
            self.pointer(ui.ctx(), canvas);
            if resp.hovered() {
                ui.ctx().set_cursor_icon(if self.tool == Tool::Text { egui::CursorIcon::Text } else { egui::CursorIcon::Crosshair });
            }
            let paint = ui.painter_at(canvas);
            match &self.texture {
                Some(t) => {
                    paint.image(t.id(), canvas, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
                }
                None => {
                    paint.rect_filled(canvas, 0.0, Color32::from_gray(30));
                    paint.text(canvas.center(), egui::Align2::CENTER_CENTER, "Loading the picture...", egui::FontId::proportional(20.0), Color32::WHITE);
                }
            }
            let place = Place { origin: canvas.min, k, image_w: self.size[0], image_h: self.size[1] };
            for m in self.marks.iter().chain(&self.drawing) {
                paint_mark(&paint, m, place);
            }
            self.ui_foot(ui, &mut out);
            self.ui_draft(ui.ctx(), place);
        });
        if out == Some(Outcome::Save) {
            self.release();
            self.commit_draft();
        }
        out
    }

    fn ui_tools(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            for t in Tool::ALL {
                let r = ui.selectable_label(self.tool == t, egui::RichText::new(t.name()).strong());
                self.spots.insert(format!("tool-{}", t.name().to_lowercase()), r.rect);
                if r.clicked() {
                    self.commit_draft();
                    self.tool = t;
                }
            }
            ui.separator();
            for i in Ink::ALL {
                let (r, resp) = ui.allocate_exact_size(vec2(26.0, 26.0), egui::Sense::click());
                self.spots.insert(format!("ink-{}", i.name()), r);
                let on = self.ink == i;
                ui.painter().rect_filled(r.shrink(3.0), 3.0, i.colour());
                ui.painter().rect_stroke(r.shrink(3.0), 3.0, if on { Stroke::new(3.0, Color32::WHITE) } else { Stroke::new(1.0, Color32::from_gray(90)) }, egui::StrokeKind::Outside);
                if resp.on_hover_text(i.name()).clicked() {
                    self.ink = i;
                }
            }
            ui.separator();
            let r = ui.add_enabled(!self.marks.is_empty() || self.draft.is_some(), egui::Button::new("Undo"));
            self.spots.insert("undo".into(), r.rect);
            if r.clicked() {
                self.undo();
            }
        });
        // A row of its own: beside the tools it runs into them on a small window.
        let r = ui.label(egui::RichText::new(&self.context).small());
        self.spots.insert("context".into(), r.rect);
    }

    fn ui_foot(&mut self, ui: &mut egui::Ui, out: &mut Option<Outcome>) {
        let id = Id::new("annotator-comment");
        let r = ui.add(egui::TextEdit::multiline(&mut self.comment).id(id).hint_text("What's wrong? One sentence is enough.").desired_rows(2).desired_width(f32::INFINITY).lock_focus(true).return_key(egui::KeyboardShortcut::new(Modifiers::SHIFT, Key::Enter)));
        self.spots.insert("comment".into(), r.rect);
        if r.changed() {
            self.armed = false;
        }
        // The comment has the keyboard unless a text mark is being typed, so
        // typing just works after any stroke.
        if !self.typing_mark() {
            ui.ctx().memory_mut(|m| m.request_focus(id));
        }
        ui.horizontal(|ui| {
            let r = ui.button(egui::RichText::new(&self.save_label).strong());
            self.spots.insert("annot-save".into(), r.rect);
            if r.clicked() {
                *out = Some(Outcome::Save);
            }
            let r = ui.button("Cancel");
            self.spots.insert("annot-cancel".into(), r.rect);
            if r.clicked() {
                *out = Some(Outcome::Cancel);
            }
            if self.armed {
                ui.colored_label(Color32::from_rgb(255, 170, 120), "Esc again throws this away.");
            }
        });
        ui.label(egui::RichText::new("Drag on the picture to draw; with Text, click and type. Otherwise typing goes in the comment. Ctrl+Z undoes a mark. Enter saves, Shift+Enter is a new line, Esc cancels.").small().weak());
    }

    /// The text mark being typed, in its own place on the picture.
    fn ui_draft(&mut self, ctx: &egui::Context, place: Place) {
        let colour = self.ink.colour();
        let room = self.spots["canvas"].right() - 8.0;
        let Some(d) = &mut self.draft else { return };
        let at = place.at(d.at);
        let size = text_size(place.image_h) * place.k;
        let id = Id::new("annotator-draft");
        let resp = egui::Area::new(Id::new("annotator-draft-area")).order(egui::Order::Foreground).fixed_pos(at).show(ctx, |ui| {
            ui.add(egui::TextEdit::singleline(&mut d.text).id(id).font(egui::FontId::proportional(size)).text_color(colour).hint_text("type here").desired_width((room - at.x).max(size * 4.0)).background_color(Color32::from_black_alpha(190)))
        });
        // Held every pass: the release of the click that opened it lands
        // outside it, and egui would take the keys away.
        ctx.memory_mut(|m| m.request_focus(id));
        self.spots.insert("draft".into(), resp.inner.rect);
    }
}

/// How long a pass of the annotator takes with this many marks (ms): for the
/// findings and the test that keeps it quick.
#[cfg(test)]
pub(crate) fn pass_ms(a: &mut Annotator, ctx: &egui::Context, raw: egui::RawInput) -> f64 {
    let t = std::time::Instant::now();
    let full = ctx.run_ui(raw, |ui| {
        a.ui(ui.ctx());
    });
    let _ = ctx.tessellate(full.shapes, full.pixels_per_point);
    t.elapsed().as_secs_f64() * 1e3
}

#[cfg(test)]
#[path = "annotate_tests.rs"]
mod tests;
