//! The look panel's Blocks section (M3a colours, M3b the rest): every
//! block's colours, adding a block, a block's name, kind and hotbar key,
//! the hotbar, and the texture editor.

use super::{Request, Workshop};
use crate::render::textures::Picture;
use crate::world::BlockId;
use crate::world::blocks::{self, BlockTextures, FACE_NAMES};
use egui::{Color32, RichText};
use std::path::PathBuf;
use std::sync::mpsc;

const FACE_CHOICES: [(usize, &str); 4] = [(3, "every face"), (0, "top"), (1, "sides"), (2, "bottom")];

fn faces_of(face: usize) -> Vec<usize> {
    if face >= 3 { vec![0, 1, 2] } else { vec![face] }
}

fn weak(text: impl Into<String>) -> RichText {
    RichText::new(text.into()).weak()
}

impl Workshop {
    pub(super) fn ui_blocks(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        self.ui_add(ui);
        ui.add_space(4.0);
        self.ui_hotbar(ui);
        ui.add_space(6.0);
        ui.label(weak("Each block's colour. Tick \"faces\" to give the top, the sides and the bottom their own colours. Click a block's name to rename it, change how it behaves, or paint its texture."));
        let following = self.look.water.far_ocean_follows_water;
        egui::Grid::new("blocks").num_columns(6).striped(true).spacing([10.0, 6.0]).show(ui, |ui| {
            ui.label(RichText::new("block").strong());
            ui.label(RichText::new("colour").strong());
            ui.label(RichText::new("faces").strong());
            ui.label(RichText::new("top").strong());
            ui.label(RichText::new("sides").strong());
            ui.label(RichText::new("bottom").strong());
            ui.end_row();
            for id in 1..self.blocks.len() {
                let d = &self.blocks[id];
                if d.name.starts_with("unused_") {
                    continue;
                }
                let (label, hidden) = (d.label(), d.hidden);
                let locked = following && Some(id as BlockId) == self.deep_ocean;
                let default = self.colours_default[id];
                let mut changed = false;
                ui.horizontal(|ui| {
                    let tex = self.textures[id].faces();
                    if let Some(file) = tex[0].or(tex[1]).map(str::to_string) {
                        if let Some(p) = self.texset.picture(&file).cloned() {
                            let t = self.thumb(&ctx, &file, &p);
                            ui.image((t, egui::vec2(18.0, 18.0)));
                        }
                    }
                    let text = if hidden { RichText::new(format!("{label} (hidden)")).weak() } else { RichText::new(label) };
                    let selected = self.bui.selected == Some(id as BlockId);
                    let r = ui.selectable_label(selected, text);
                    let r = if locked { r.on_hover_text("The sea seen from far away. It follows the water colour; untick that in Water to set it here.") } else { r.on_hover_text("Rename it, change how it behaves, paint its texture") };
                    if r.clicked() {
                        self.select(if selected { None } else { Some(id as BlockId) });
                    }
                });
                let c = &mut self.colours[id];
                ui.add_enabled_ui(!locked, |ui| {
                    changed |= super::pick(ui, &mut c.color).changed();
                });
                let mut faces = c.per_face();
                if ui.add_enabled(!locked, egui::Checkbox::without_text(&mut faces)).changed() {
                    if faces {
                        let [t, s, b] = c.faces();
                        (c.top, c.side, c.bottom) = (Some(t), Some(s), Some(b));
                    } else {
                        (c.top, c.side, c.bottom) = (None, None, None);
                    }
                    changed = true;
                }
                if c.per_face() {
                    for f in [&mut c.top, &mut c.side, &mut c.bottom] {
                        let mut v = f.unwrap_or(c.color);
                        if super::pick(ui, &mut v).changed() {
                            *f = Some(v);
                            changed = true;
                        }
                    }
                } else {
                    ui.label("");
                    ui.label("");
                    ui.label("");
                }
                if !locked && *c != default && ui.small_button("reset").on_hover_text("Back to this block's shipped colours").clicked() {
                    *c = default;
                    changed = true;
                }
                ui.end_row();
                self.colours_dirty |= changed;
            }
        });
        if ui.button("Reset all block colours to default").clicked() {
            self.colours = self.colours_default.clone();
            self.colours_dirty = true;
        }
        if let Some(id) = self.bui.selected.filter(|&id| (id as usize) < self.blocks.len()) {
            ui.separator();
            self.ui_block(ui, &ctx, id);
        }
    }

    pub(super) fn select(&mut self, id: Option<BlockId>) {
        self.bui.selected = id;
        self.bui.confirm_falls = None;
        self.bui.editing = None;
        self.bui.editor.open(None);
        if let Some(id) = id {
            self.bui.rename = self.blocks[id as usize].name.clone();
            self.bui.scroll_to = true;
        }
    }

    // ------------------------------------------------------- adding a block

    fn ui_add(&mut self, ui: &mut egui::Ui) {
        if ui.button(if self.bui.add_open { "Close the new block" } else { "Add a block..." }).on_hover_text("Make a new block: a name, a colour, solid or falling like sand.").clicked() {
            self.bui.add_open = !self.bui.add_open;
        }
        if !self.bui.add_open {
            return;
        }
        egui::Frame::group(ui.style()).show(ui, |ui| {
            let name = blocks::name_from_label(&self.bui.add_label);
            egui::Grid::new("add-block").num_columns(2).spacing([14.0, 8.0]).show(ui, |ui| {
                ui.label("Name");
                ui.vertical(|ui| {
                    ui.add(egui::TextEdit::singleline(&mut self.bui.add_label).hint_text("red brick").desired_width(220.0));
                    if !name.is_empty() {
                        ui.label(weak(format!("in blocks.json: \"{name}\"")));
                    }
                });
                ui.end_row();
                ui.label("Colour").on_hover_text("Its colour until you paint a texture (and far away, where a texture shows its average colour).");
                super::pick(ui, &mut self.bui.add_colour);
                ui.end_row();
                ui.label("Kind");
                ui.horizontal(|ui| {
                    ui.radio_value(&mut self.bui.add_falls, false, "solid").on_hover_text("Stays where you put it, like stone.");
                    ui.radio_value(&mut self.bui.add_falls, true, "falls like sand").on_hover_text("Falls when there's nothing under it, like sand and gravel.");
                });
                ui.end_row();
            });
            ui.label(weak(
                "Every block you add is solid: you can stand on it and it hides what's behind it. See-through blocks (glass, leaves) and a second liquid aren't in the game yet: blocks are drawn solid, and the water rules only know water.",
            ));
            let problem = if name.is_empty() {
                Some("Give it a name.".to_string())
            } else if let Err(e) = blocks::check_name(&name) {
                Some(e)
            } else if self.registry.id(&name).is_ok() {
                Some(format!("There is already a block called {name}."))
            } else {
                None
            };
            if let Some(p) = problem.as_ref().filter(|_| !self.bui.add_label.trim().is_empty()) {
                ui.colored_label(Color32::from_rgb(255, 160, 110), p);
            }
            if let Some(e) = &self.bui.add_error {
                ui.colored_label(Color32::from_rgb(255, 120, 100), e);
            }
            let add = ui.add_enabled(problem.is_none(), egui::Button::new(RichText::new("Add block").strong())).on_hover_text("Adds it to blocks.json now. Blocks are never deleted (a world may hold them): hide one from the hotbar if you stop wanting it.");
            if add.clicked() {
                self.requests.push(Request::Add { name, colour: self.bui.add_colour, falls: self.bui.add_falls });
            }
            ui.label(weak("It is saved to blocks.json straight away. Its colours and texture are saved with Save look, like any other block's."));
        });
    }

    // ------------------------------------------------------- the hotbar

    fn ui_hotbar(&mut self, ui: &mut egui::Ui) {
        ui.label(RichText::new("Hotbar: what keys 1 to 9 hold").strong()).on_hover_text("Saved in blocks.json at once. The play scripts (--play) keep the default hotbar.");
        let offered: Vec<(BlockId, String)> = self.blocks.iter().filter(|d| d.id != 0 && !d.hidden && !d.name.starts_with("unused_") && (d.solid || d.liquid)).map(|d| (d.id, d.label())).collect();
        let label = |id: BlockId| self.blocks.get(id as usize).map_or_else(|| format!("block {id}"), |d| d.label());
        let mut hb = self.hotbar.clone();
        let mut changed = false;
        let slots = (hb.len() + 1).min(9);
        egui::Grid::new("hotbar").num_columns(6).spacing([8.0, 6.0]).show(ui, |ui| {
            for slot in 0..slots {
                ui.label(RichText::new(format!("{}", slot + 1)).monospace().strong());
                let current = hb.get(slot).copied();
                let text = current.map_or_else(|| "(add a block)".to_string(), &label);
                egui::ComboBox::from_id_salt(("hotbar", slot)).selected_text(text).width(130.0).show_ui(ui, |ui| {
                    for (id, name) in &offered {
                        if ui.selectable_label(current == Some(*id), name).clicked() {
                            if slot < hb.len() {
                                hb[slot] = *id;
                            } else {
                                hb.push(*id);
                            }
                            changed = true;
                        }
                    }
                });
                if slot % 3 == 2 {
                    ui.end_row();
                }
            }
        });
        if changed {
            self.requests.push(Request::Hotbar(Some(hb)));
        }
        if ui.small_button("Back to the default hotbar").on_hover_text("The first eight solid blocks and water, as the game ships.").clicked() {
            self.requests.push(Request::Hotbar(None));
        }
    }

    // ------------------------------------------------------- one block

    fn ui_block(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, id: BlockId) {
        let d = self.blocks[id as usize].clone();
        let heading = ui.heading(format!("{} (block {id})", d.label()));
        if std::mem::take(&mut self.bui.scroll_to) {
            heading.scroll_to_me(Some(egui::Align::Min));
        }
        let shipped = self.fixed.contains(&id);
        egui::Grid::new("block-details").num_columns(2).spacing([14.0, 8.0]).show(ui, |ui| {
            ui.label("Name");
            if shipped {
                ui.label(format!("{} (fixed)", d.name)).on_hover_text("The world generator (and the scripts) find this block by its name, so it keeps it. Blocks you add can be renamed.");
            } else {
                ui.horizontal(|ui| {
                    ui.add(egui::TextEdit::singleline(&mut self.bui.rename).desired_width(180.0));
                    let new = blocks::name_from_label(&self.bui.rename);
                    let ok = !new.is_empty() && new != d.name && blocks::check_name(&new).is_ok() && self.registry.id(&new).map_or(true, |o| o == id);
                    if ui.add_enabled(ok, egui::Button::new("Rename")).on_hover_text("Saved to blocks.json at once. The old name is kept there (\"was\"), so saves and replays that know it still work.").clicked() {
                        self.requests.push(Request::Rename(id, new));
                    }
                });
            }
            ui.end_row();
            if !d.was.is_empty() {
                ui.label("");
                ui.label(weak(format!("was called {}", d.was.join(", "))));
                ui.end_row();
            }
            ui.label("Kind");
            if d.solid {
                ui.horizontal(|ui| {
                    let mut falls = self.bui.confirm_falls.filter(|c| c.0 == id).map_or(d.gravity, |c| c.1);
                    ui.radio_value(&mut falls, false, "solid").on_hover_text("Stays where it is, like stone.");
                    ui.radio_value(&mut falls, true, "falls like sand").on_hover_text("Falls when there's nothing under it, like sand and gravel.");
                    if falls != d.gravity {
                        self.bui.confirm_falls = Some((id, falls));
                    } else if self.bui.confirm_falls.is_some_and(|c| c.0 == id) {
                        self.bui.confirm_falls = None;
                    }
                });
            } else {
                ui.label(weak(if d.liquid { "a liquid (the water rules)" } else { "nothing: air" }));
            }
            ui.end_row();
            if id != 0 {
                ui.label("Hotbar");
                ui.horizontal(|ui| {
                    let mut hidden = d.hidden;
                    if ui.checkbox(&mut hidden, "hidden").on_hover_text("Blocks are never deleted: a world may hold them. Hide one you don't want to pick any more; it stays in the world.").changed() {
                        self.requests.push(Request::Hidden(id, hidden));
                    }
                    ui.separator();
                    let on = self.hotbar.iter().position(|&h| h == id);
                    ui.label(on.map_or("not on a key".to_string(), |k| format!("on key {}", k + 1)));
                    let n = (self.hotbar.len() + 1).min(9);
                    self.bui.hotbar_slot = self.bui.hotbar_slot.min(n - 1);
                    egui::ComboBox::from_id_salt("put-on-key").selected_text(format!("key {}", self.bui.hotbar_slot + 1)).width(70.0).show_ui(ui, |ui| {
                        for k in 0..n {
                            ui.selectable_value(&mut self.bui.hotbar_slot, k, format!("key {}", k + 1));
                        }
                    });
                    if ui.button("Put it there").clicked() {
                        let mut hb = self.hotbar.clone();
                        if self.bui.hotbar_slot < hb.len() {
                            hb[self.bui.hotbar_slot] = id;
                        } else {
                            hb.push(id);
                        }
                        self.requests.push(Request::Hotbar(Some(hb)));
                    }
                });
                ui.end_row();
            }
        });
        if let Some((cid, falls)) = self.bui.confirm_falls.filter(|c| c.0 == id) {
            let name = d.label();
            let text = if falls {
                format!(
                    "{name} will fall like sand. That changes the game's rules: your world and its saves keep working, {name} already in the world falls only when something next to it changes (dig under it), and replays recorded before now will show a difference when played back. Saved to blocks.json at once."
                )
            } else {
                format!("{name} will stay put instead of falling. That changes the game's rules: your world and its saves keep working, and replays recorded before now will show a difference when played back. Saved to blocks.json at once.")
            };
            ui.colored_label(Color32::from_rgb(255, 190, 90), text);
            ui.horizontal(|ui| {
                if ui.button(if falls { "Yes, make it fall" } else { "Yes, make it stay put" }).clicked() {
                    self.requests.push(Request::Falls(cid, falls));
                    self.bui.confirm_falls = None;
                }
                if ui.button("Cancel").clicked() {
                    self.bui.confirm_falls = None;
                }
            });
        }
        if id != 0 && !d.liquid {
            ui.add_space(6.0);
            self.ui_texture(ui, ctx, id);
        }
    }

    // ------------------------------------------------------- textures

    /// The file each chosen face shows now, when they agree (every face:
    /// the sides' when they don't).
    pub(super) fn shown_file(&self, id: BlockId, face: usize) -> Option<String> {
        let cur = self.textures[id as usize].faces();
        cur[if face >= 3 { 1 } else { face }].map(str::to_string)
    }

    /// The file to paint the chosen faces into: the one they show, when
    /// only they show it and it isn't an example; otherwise a new file
    /// named after the block (copy on write: an example, or a picture
    /// another block or face uses, is never changed by painting here).
    pub(super) fn own_file(&self, id: BlockId, face: usize) -> String {
        let faces = faces_of(face);
        let cur: Vec<Option<String>> = self.textures[id as usize].faces().iter().map(|f| f.map(str::to_string)).collect();
        if let Some(f) = &cur[faces[0]] {
            let all_same = faces.iter().all(|&g| cur[g].as_ref() == Some(f));
            let others_here = (0..3).filter(|g| !faces.contains(g)).any(|g| cur[g].as_ref() == Some(f));
            let elsewhere = self.textures.iter().enumerate().any(|(j, t)| j != id as usize && t.faces().contains(&Some(f.as_str())));
            if all_same && !others_here && !elsewhere && !f.starts_with("examples/") {
                return f.clone();
            }
        }
        let name = &self.blocks[id as usize].name;
        let base = if faces.len() == 3 { name.clone() } else { format!("{name}_{}", FACE_NAMES[faces[0]]) };
        let used = |f: &str| self.texset.known(f) || self.textures.iter().any(|t| t.faces().contains(&Some(f)));
        (1..).map(|n| if n == 1 { format!("{base}.png") } else { format!("{base}_{n}.png") }).find(|f| !used(f)).unwrap()
    }

    /// Point the chosen faces at `file` (None: back to the flat colour).
    pub(super) fn set_faces(&mut self, id: BlockId, face: usize, file: Option<String>) {
        let t = &mut self.textures[id as usize];
        if face >= 3 {
            *t = BlockTextures { all: file, ..Default::default() };
        } else {
            t.set_face(face, file);
        }
        self.textures_dirty = true;
    }

    /// The editor's picture goes to the chosen faces: into their own file
    /// (made when needed), shown in the world at once, written by Save look.
    pub(super) fn commit_paint(&mut self, id: BlockId, face: usize) {
        let Some(pic) = self.bui.editor.picture.clone() else { return };
        let file = self.own_file(id, face);
        if let Err(e) = self.texset.set_picture(&file, pic) {
            self.note(e);
            return;
        }
        if self.shown_file(id, face).as_deref() != Some(file.as_str()) || (face >= 3 && self.textures[id as usize].per_face()) {
            self.set_faces(id, face, Some(file.clone()));
        }
        self.bui.editing = Some(file);
    }

    /// Show the chosen faces' picture in the editor (unless a stroke is
    /// under way); a file changed on disk shows too.
    pub(super) fn sync_editor(&mut self, id: BlockId, face: usize) {
        if self.bui.editor.painting() {
            return;
        }
        let file = self.shown_file(id, face);
        let want = file.as_deref().and_then(|f| self.texset.picture(f)).cloned();
        if self.bui.editing != file {
            self.bui.editor.open(want);
            self.bui.editing = file;
        } else if self.bui.editor.picture != want {
            self.bui.editor.picture = want;
        }
    }

    fn ui_texture(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, id: BlockId) {
        let size = self.texset.size;
        ui.label(RichText::new("Texture").strong());
        ui.label(weak(format!(
            "A {size} x {size} picture on the block's faces, tiled once per block. A face without one shows its colour; far away a texture shows its average colour."
        )));
        ui.horizontal(|ui| {
            ui.label("Paint");
            for (f, name) in FACE_CHOICES {
                if ui.selectable_label(self.bui.face == f, name).clicked() && self.bui.face != f {
                    self.bui.face = f;
                    self.bui.editing = None;
                    self.bui.editor.open(None);
                }
            }
        });
        let face = self.bui.face;
        if face >= 3 && self.textures[id as usize].per_face() {
            ui.label(weak("The faces have pictures of their own: painting here gives every face this one."));
        }
        self.sync_editor(id, face);
        let file = self.shown_file(id, face);
        if self.bui.editor.picture.is_some() {
            ui.horizontal(|ui| {
                let changed = self.bui.editor.canvas(ui, 336.0);
                if changed {
                    self.commit_paint(id, face);
                }
                ui.vertical(|ui| {
                    ui.label(weak("on the block"));
                    // The three faces as they are drawn now.
                    let tex = self.textures[id as usize].faces().map(|f| f.map(str::to_string));
                    for (f, t) in tex.iter().enumerate() {
                        ui.horizontal(|ui| {
                            match t.as_ref().and_then(|t| self.texset.picture(t).cloned().map(|p| (t.clone(), p))) {
                                Some((t, p)) => {
                                    let h = self.thumb(ctx, &t, &p);
                                    ui.image((h, egui::vec2(48.0, 48.0)));
                                }
                                None => {
                                    let c = self.colours[id as usize].faces()[f];
                                    let (r, _) = ui.allocate_exact_size(egui::vec2(48.0, 48.0), egui::Sense::hover());
                                    ui.painter().rect_filled(r, 0.0, super::srgb32(c));
                                }
                            }
                            ui.label(FACE_NAMES[f]);
                        });
                    }
                    if let Some(f) = &file {
                        ui.label(weak(format!("file: textures/{f}")));
                    }
                });
            });
            if self.bui.editor.tools(ui) {
                self.commit_paint(id, face);
            }
        } else {
            ui.label(weak(if face >= 3 { "No texture: the block shows its colours." } else { "No texture on this face: it shows its colour." }));
        }
        ui.horizontal_wrapped(|ui| {
            if ui.button("Start from the block's colour").on_hover_text("A picture of the face's colour to paint on.").clicked() {
                let c = self.colours[id as usize].faces()[faces_of(face)[0]];
                self.bui.editor.replace(Picture::flat(size, c));
                self.commit_paint(id, face);
            }
            let waiting = self.bui.import.is_some() || self.bui.decoding.is_some();
            if ui.add_enabled(!waiting, egui::Button::new("Import PNG...")).on_hover_text("Any PNG picture: it is cut to a square and resized to the texture size.").clicked() {
                self.start_import((id, face));
            }
            if file.is_some() && ui.button("Remove texture").on_hover_text("Back to the flat colour (the picture's file stays in the textures folder).").clicked() {
                self.set_faces(id, face, None);
                self.bui.editing = None;
                self.bui.editor.open(None);
            }
        });
        if self.bui.import.is_some() {
            ui.label(weak("Choose a picture in the window that opened..."));
        }
        ui.label(weak("You can also drop a PNG file on the game window."));
        if !self.examples.is_empty() {
            ui.label(weak("Try an example (placeholders made to show how textures look, not art):"));
            ui.horizontal_wrapped(|ui| {
                for (file, pic) in self.examples.clone() {
                    let h = self.thumb(ctx, &file, &pic);
                    let name = file.trim_start_matches("examples/").trim_end_matches(".png").replace('_', " ");
                    if ui.add(egui::Button::image(egui::Image::new((h, egui::vec2(40.0, 40.0))))).on_hover_text(format!("Use the {name} example on these faces (painting on it makes your own copy)")).clicked() {
                        self.set_faces(id, face, Some(file.clone()));
                        self.bui.editing = None;
                    }
                }
            });
        }
        self.poll_import();
    }

    // ------------------------------------------------------- importing

    fn start_import(&mut self, target: (BlockId, usize)) {
        #[cfg(windows)]
        {
            let (tx, rx) = mpsc::channel();
            // The dialog needs a full path to open in the textures folder.
            let dir = std::path::absolute(&self.texset.dir).unwrap_or_else(|_| self.texset.dir.clone());
            std::thread::spawn(move || {
                let r = rfd::FileDialog::new().set_title("Import a picture for the block").add_filter("PNG picture", &["png"]).set_directory(dir).pick_file();
                let _ = tx.send(r);
            });
            self.bui.import = Some((target, rx));
        }
        #[cfg(not(windows))]
        {
            let _ = target;
            let _: Option<mpsc::Receiver<Option<PathBuf>>> = None;
            self.note("Drop a PNG file on the game window to import it.");
        }
    }

    pub(super) fn poll_import(&mut self) {
        if let Some((target, path, job)) = self.bui.decoding.take() {
            match job.poll() {
                None => self.bui.decoding = Some((target, path, job)),
                Some(result) => self.finish_import(target, &path, result),
            }
        }
        let Some((target, rx)) = &self.bui.import else { return };
        let target = *target;
        match rx.try_recv() {
            Ok(Some(path)) => {
                self.bui.import = None;
                self.import_file(target, &path);
            }
            Ok(None) | Err(mpsc::TryRecvError::Disconnected) => self.bui.import = None,
            Err(mpsc::TryRecvError::Empty) => {}
        }
    }

    /// Files dropped on the window go to the face being edited.
    pub(super) fn take_drops(&mut self, ctx: &egui::Context) {
        let files: Vec<PathBuf> = ctx.input(|i| i.raw.dropped_files.iter().filter_map(|f| f.path.clone()).collect());
        let Some(path) = files.last() else { return };
        match self.bui.selected {
            Some(id) if id != 0 => self.import_file((id, self.bui.face), path),
            _ => self.note("To import a picture, click a block's name in Blocks first, then drop the PNG on the window."),
        }
    }

    /// A picture file onto a block's faces: resized to the texture size,
    /// see-through parts filled with the face's colour, one undo step.
    pub(super) fn import_file(&mut self, target: (BlockId, usize), path: &std::path::Path) {
        let (id, face) = target;
        if id as usize >= self.blocks.len() {
            return;
        }
        let under = self.colours[id as usize].faces()[faces_of(face)[0]];
        let job = crate::render::textures::PictureJob::start(path.to_path_buf(), self.texset.size, Some(under));
        self.bui.decoding = Some((target, path.to_path_buf(), job));
    }

    fn finish_import(&mut self, (id, face): (BlockId, usize), path: &std::path::Path, result: Result<(u32, u32, Picture), String>) {
        match result {
            Ok((w, h, pic)) => {
                if self.bui.selected != Some(id) || self.bui.face != face {
                    self.select(Some(id));
                    self.bui.face = face;
                }
                self.sync_editor(id, face);
                self.bui.editor.replace(pic.resized(self.texset.size));
                self.commit_paint(id, face);
                let size = self.texset.size;
                self.note(format!("Imported {} ({w} x {h}{}) onto {}'s {}.", path.display(), if w == h && w == size { String::new() } else { format!(", made {size} x {size}") }, self.blocks[id as usize].label(), FACE_CHOICES.iter().find(|c| c.0 == face).map_or("faces", |c| c.1)));
            }
            Err(e) => self.note(format!("Could not import {}: {e}", path.display())),
        }
    }
}
