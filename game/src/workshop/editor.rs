//! The pixel editor (M3b): a block face's texture as a grid of pixels in
//! the look panel, with a pencil, a fill, a colour picker (the eyedropper:
//! also a right click), a colour chooser and undo. It edits a working copy
//! of a `Picture`; the workshop decides which file the copy goes to and
//! shows every change on the blocks in the world at once.

use crate::render::textures::Picture;
use egui::{Color32, Pos2, Rect, Sense, Stroke, Vec2};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tool {
    Pencil,
    Fill,
    Pick,
}

impl Tool {
    pub fn label(self) -> &'static str {
        match self {
            Tool::Pencil => "Pencil",
            Tool::Fill => "Fill",
            Tool::Pick => "Pick colour",
        }
    }
    fn help(self) -> &'static str {
        match self {
            Tool::Pencil => "Click or drag to paint pixels.",
            Tool::Fill => "Click to fill the patch of one colour you click on.",
            Tool::Pick => "Click a pixel to take its colour (a right click does this with any tool).",
        }
    }
}

const UNDO_MAX: usize = 100;
const RECENT_MAX: usize = 12;

pub struct PixelEditor {
    /// The picture being painted (the workshop puts it here and takes it
    /// back after every change).
    pub picture: Option<Picture>,
    undo: Vec<Picture>,
    pub tool: Tool,
    pub colour: Color32,
    recent: Vec<Color32>,
    /// The press under way: its tool, the last pixel it painted, and
    /// whether its undo step was kept yet.
    press: Option<Press>,
}

#[derive(Clone, Copy)]
struct Press {
    tool: Tool,
    last: Option<(u32, u32)>,
    undo_kept: bool,
}

impl Default for PixelEditor {
    fn default() -> Self {
        Self { picture: None, undo: Vec::new(), tool: Tool::Pencil, colour: Color32::from_rgb(120, 70, 40), recent: Vec::new(), press: None }
    }
}

/// The pixels on the line from `a` to `b` (Bresenham), so a fast drag
/// leaves no gaps.
fn line(a: (u32, u32), b: (u32, u32)) -> Vec<(u32, u32)> {
    let (mut x0, mut y0, x1, y1) = (a.0 as i64, a.1 as i64, b.0 as i64, b.1 as i64);
    let (dx, dy) = ((x1 - x0).abs(), -(y1 - y0).abs());
    let (sx, sy) = (if x0 < x1 { 1 } else { -1 }, if y0 < y1 { 1 } else { -1 });
    let mut err = dx + dy;
    let mut out = Vec::new();
    loop {
        out.push((x0 as u32, y0 as u32));
        if x0 == x1 && y0 == y1 {
            return out;
        }
        let e2 = 2 * err;
        if e2 >= dy {
            err += dy;
            x0 += sx;
        }
        if e2 <= dx {
            err += dx;
            y0 += sy;
        }
    }
}

fn rgba(c: Color32) -> [u8; 4] {
    [c.r(), c.g(), c.b(), 255]
}

impl PixelEditor {
    /// Start on another picture (another block or face): undo starts over.
    pub fn open(&mut self, picture: Option<Picture>) {
        self.picture = picture;
        self.undo.clear();
        self.press = None;
    }

    /// Is a press (a stroke) under way? The workshop leaves the picture
    /// alone until it ends.
    pub fn painting(&self) -> bool {
        self.press.is_some()
    }

    /// Keep the picture before a change, for undo.
    pub fn push_undo(&mut self) {
        if let Some(p) = &self.picture {
            if self.undo.last() != Some(p) {
                self.undo.push(p.clone());
                if self.undo.len() > UNDO_MAX {
                    self.undo.remove(0);
                }
            }
        }
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    /// Step back one change. True when the picture changed.
    pub fn undo(&mut self) -> bool {
        match self.undo.pop() {
            Some(p) => {
                self.picture = Some(p);
                true
            }
            None => false,
        }
    }

    /// Replace the whole picture as one undoable change (Import, Start
    /// from the block's colour).
    pub fn replace(&mut self, picture: Picture) {
        self.push_undo();
        self.picture = Some(picture);
    }

    fn remember(&mut self, c: Color32) {
        self.recent.retain(|r| *r != c);
        self.recent.insert(0, c);
        self.recent.truncate(RECENT_MAX);
    }

    /// The pixel grid, `side` points square. Returns true when the picture
    /// changed this frame.
    pub fn canvas(&mut self, ui: &mut egui::Ui, side: f32) -> bool {
        let Some(size) = self.picture.as_ref().map(|p| p.size) else { return false };
        let (resp, painter) = ui.allocate_painter(Vec2::splat(side), Sense::click_and_drag());
        let rect = resp.rect;
        let cell = side / size as f32;
        let cell_at = |pos: Pos2| -> Option<(u32, u32)> {
            let d = (pos - rect.min) / cell;
            (d.x >= 0.0 && d.y >= 0.0 && d.x < size as f32 && d.y < size as f32).then(|| (d.x as u32, d.y as u32))
        };
        let mut changed = false;
        let down = ui.input(|i| i.pointer.primary_down());
        if resp.is_pointer_button_down_on() && down {
            if let Some(c) = resp.interact_pointer_pos().and_then(cell_at) {
                let first = self.press.is_none();
                let mut press = self.press.unwrap_or(Press { tool: self.tool, last: None, undo_kept: false });
                match press.tool {
                    Tool::Pencil => {
                        let p = rgba(self.colour);
                        let cells = match press.last {
                            Some(l) => line(l, c),
                            None => vec![c],
                        };
                        let pic = self.picture.as_ref().unwrap();
                        if cells.iter().any(|&(x, y)| pic.get(x, y) != p) {
                            if !press.undo_kept {
                                self.push_undo();
                                press.undo_kept = true;
                            }
                            let pic = self.picture.as_mut().unwrap();
                            for (x, y) in cells {
                                pic.set(x, y, p);
                            }
                            changed = true;
                        }
                        if first {
                            self.remember(self.colour);
                        }
                    }
                    Tool::Fill if first => {
                        let before = self.picture.clone();
                        if self.picture.as_mut().unwrap().fill(c.0, c.1, rgba(self.colour)) {
                            if let Some(b) = before {
                                self.undo.push(b);
                            }
                            changed = true;
                            self.remember(self.colour);
                        }
                    }
                    Tool::Pick if first => {
                        let p = self.picture.as_ref().unwrap().get(c.0, c.1);
                        self.colour = Color32::from_rgb(p[0], p[1], p[2]);
                        self.tool = Tool::Pencil;
                    }
                    _ => {}
                }
                press.last = Some(c);
                self.press = Some(press);
            }
        } else if !down {
            self.press = None;
        }
        // A right click takes a pixel's colour with any tool.
        if resp.secondary_clicked() {
            if let Some(c) = resp.interact_pointer_pos().and_then(cell_at) {
                let p = self.picture.as_ref().unwrap().get(c.0, c.1);
                self.colour = Color32::from_rgb(p[0], p[1], p[2]);
            }
        }
        // The pixels, a faint grid, the pixel under the pointer.
        let pic = self.picture.as_ref().unwrap();
        for y in 0..size {
            for x in 0..size {
                let p = pic.get(x, y);
                let r = Rect::from_min_size(rect.min + Vec2::new(x as f32 * cell, y as f32 * cell), Vec2::splat(cell));
                painter.rect_filled(r, 0.0, Color32::from_rgb(p[0], p[1], p[2]));
            }
        }
        if cell >= 6.0 {
            let grid = Stroke::new(1.0, Color32::from_black_alpha(40));
            for i in 1..size {
                let t = i as f32 * cell;
                painter.line_segment([rect.min + Vec2::new(t, 0.0), rect.min + Vec2::new(t, side)], grid);
                painter.line_segment([rect.min + Vec2::new(0.0, t), rect.min + Vec2::new(side, t)], grid);
            }
        }
        painter.rect_stroke(rect, 0.0, Stroke::new(1.0, Color32::from_gray(160)), egui::StrokeKind::Outside);
        if let Some(c) = resp.hover_pos().and_then(cell_at) {
            let r = Rect::from_min_size(rect.min + Vec2::new(c.0 as f32 * cell, c.1 as f32 * cell), Vec2::splat(cell));
            painter.rect_stroke(r, 0.0, Stroke::new(2.0, Color32::WHITE), egui::StrokeKind::Inside);
        }
        let resp = resp.on_hover_text(self.tool.help());
        let _ = resp;
        changed
    }

    /// The tool buttons, undo, the colour chooser and recent colours.
    /// Returns true when undo changed the picture.
    pub fn tools(&mut self, ui: &mut egui::Ui) -> bool {
        let mut changed = false;
        ui.horizontal(|ui| {
            for t in [Tool::Pencil, Tool::Fill, Tool::Pick] {
                if ui.selectable_label(self.tool == t, t.label()).on_hover_text(t.help()).clicked() {
                    self.tool = t;
                }
            }
            ui.separator();
            let undo = ui.input(|i| i.modifiers.command && i.key_pressed(egui::Key::Z));
            if (ui.add_enabled(self.can_undo(), egui::Button::new("Undo")).on_hover_text("Take back the last change (Ctrl+Z).").clicked() || undo) && self.undo() {
                changed = true;
            }
        });
        ui.horizontal_wrapped(|ui| {
            ui.label("Colour");
            egui::color_picker::color_edit_button_srgba(ui, &mut self.colour, egui::color_picker::Alpha::Opaque).on_hover_text("Choose the colour to paint with.");
            for c in self.recent.clone() {
                let (r, resp) = ui.allocate_exact_size(Vec2::splat(18.0), Sense::click());
                ui.painter().rect_filled(r, 2.0, c);
                if c == self.colour {
                    ui.painter().rect_stroke(r, 2.0, Stroke::new(2.0, Color32::WHITE), egui::StrokeKind::Outside);
                }
                if resp.on_hover_text("A colour you used: click to paint with it again.").clicked() {
                    self.colour = c;
                }
            }
        });
        changed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_have_no_gaps() {
        let l = line((0, 0), (5, 2));
        assert_eq!(l.first(), Some(&(0, 0)));
        assert_eq!(l.last(), Some(&(5, 2)));
        for w in l.windows(2) {
            assert!(w[0].0.abs_diff(w[1].0) <= 1 && w[0].1.abs_diff(w[1].1) <= 1);
        }
        assert_eq!(line((3, 3), (3, 3)), vec![(3, 3)]);
    }

    #[test]
    fn undo_steps_back() {
        let mut e = PixelEditor::default();
        e.open(Some(Picture::flat(4, [0.5; 3])));
        let start = e.picture.clone().unwrap();
        let mut p = start.clone();
        p.set(1, 1, [255, 0, 0, 255]);
        e.replace(p.clone());
        assert!(e.can_undo());
        assert!(e.undo());
        assert_eq!(e.picture.as_ref(), Some(&start));
        assert!(!e.undo());
    }
}
