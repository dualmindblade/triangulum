//! The windowed app's input (M2c): window events in, step-positioned
//! inputs out. Nothing here touches the game between steps. Key presses,
//! clicks, mouse motion and the wheel are queued as they arrive and handed
//! to `Session::apply` at the next fixed step, in the order they arrived;
//! the held movement keys are sampled once per step. So the game sees the
//! same inputs at the same steps whether the window draws at 30 or 144
//! frames a second (review 2 #16), and a replay of those inputs is exact.
//!
//! Two details keep fast input from depending on the frame rate: a key
//! pressed and released between two steps still counts as held for one
//! step (at 30 fps a quick tap of Space would otherwise fall between
//! frames and never jump), and mouse motion is drawn ahead of the step
//! (`camera`), so looking around stays smooth above 60 fps without moving
//! the player before the step it belongs to.

use crate::camera::Camera;
use crate::player::{Input, Player, STEP};
use crate::session::{Action, Button, LOOK_RAD_PER_COUNT, PITCH_MAX, Session};
use std::collections::HashSet;
use winit::event::MouseButton;
use winit::keyboard::KeyCode;

/// At most this many steps per frame; a frame later than that drops the
/// backlog rather than spiral (the steps still happen in order; only wall
/// time is lost).
pub const MAX_STEPS_PER_FRAME: usize = 8;

#[derive(Default)]
pub struct Controls {
    held: HashSet<KeyCode>,
    /// Movement keys pressed since the last step.
    tapped: HashSet<KeyCode>,
    pending: Vec<Action>,
    /// Fixed-step accumulator (s).
    acc: f64,
    /// The place last chosen in the T window (index into `Engine::presets`).
    place: usize,
}

/// Where a key goes (§17.1): an open window owns all input.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Route {
    /// Nobody: an open window has it, or it is a repeat.
    Window,
    /// The game's and the app's own keys, as ever (every release comes here,
    /// so nothing sticks down).
    Game,
    OpenMenu,
    CloseMenu,
    /// Esc with the T window open: back out one layer.
    MenuBack,
}

/// Decide where a key goes, with egui already told about it. `menu_open`: the
/// T window is open. `typing`: a text field has the keyboard.
pub fn route_key(code: KeyCode, pressed: bool, repeat: bool, menu_open: bool, typing: bool) -> Route {
    if !pressed {
        return Route::Game;
    }
    if menu_open {
        return match code {
            _ if repeat => Route::Window,
            KeyCode::Escape => Route::MenuBack,
            KeyCode::KeyT if !typing => Route::CloseMenu,
            _ => Route::Window,
        };
    }
    match code {
        KeyCode::KeyT if repeat => Route::Window,
        KeyCode::KeyT if !typing => Route::OpenMenu,
        _ => Route::Game,
    }
}

fn is_movement(code: KeyCode) -> bool {
    matches!(code, KeyCode::KeyW | KeyCode::KeyA | KeyCode::KeyS | KeyCode::KeyD | KeyCode::Space | KeyCode::ControlLeft | KeyCode::ControlRight | KeyCode::ShiftLeft | KeyCode::ShiftRight)
}

impl Controls {
    /// The place last chosen in the T window.
    pub fn place(&self) -> usize {
        self.place
    }

    pub fn set_place(&mut self, i: usize) {
        self.place = i;
    }

    /// Queue an input for the next step, as a key press does (the T window's
    /// teleports and places).
    pub fn queue(&mut self, action: Action) {
        self.pending.push(action);
    }

    /// A window took over the input: let go of the mouse button and every key
    /// (their release events may go to it).
    pub fn release_all(&mut self) {
        self.pending.push(Action::Release);
        self.held.clear();
        self.tapped.clear();
    }

    /// A key went down or up. Returns true on the first press of a key
    /// (not an auto-repeat), so the app can act on its own keys (Esc, Q, V,
    /// H, F10). Game keys queue their action here.
    pub fn key(&mut self, code: KeyCode, pressed: bool) -> bool {
        if !pressed {
            self.held.remove(&code);
            return false;
        }
        let first = self.held.insert(code);
        if !first {
            return false;
        }
        if is_movement(code) {
            self.tapped.insert(code);
        }
        let action = match code {
            KeyCode::KeyG => Some(Action::ToggleMode),
            KeyCode::KeyR => Some(Action::Ground),
            KeyCode::KeyF => Some(Action::ToggleWater),
            KeyCode::BracketLeft => Some(Action::Time(false)),
            KeyCode::BracketRight => Some(Action::Time(true)),
            KeyCode::Digit1 => Some(Action::Select(0)),
            KeyCode::Digit2 => Some(Action::Select(1)),
            KeyCode::Digit3 => Some(Action::Select(2)),
            KeyCode::Digit4 => Some(Action::Select(3)),
            KeyCode::Digit5 => Some(Action::Select(4)),
            KeyCode::Digit6 => Some(Action::Select(5)),
            KeyCode::Digit7 => Some(Action::Select(6)),
            KeyCode::Digit8 => Some(Action::Select(7)),
            KeyCode::Digit9 => Some(Action::Select(8)),
            _ => None,
        };
        if let Some(a) = action {
            self.pending.push(a);
        }
        true
    }

    /// A mouse button while the mouse is captured (the app uses the first
    /// click to capture it; that click is not game input).
    pub fn button(&mut self, button: MouseButton, pressed: bool) {
        let b = match button {
            MouseButton::Left => Button::Break,
            MouseButton::Right => Button::Place,
            _ => return,
        };
        self.pending.push(if pressed { Action::Press(b) } else { Action::Release });
    }

    /// Raw mouse motion while captured. Motion between two other inputs is
    /// summed into one look, so the order of looks and clicks is kept.
    pub fn motion(&mut self, dx: f64, dy: f64) {
        if let Some(Action::Look(x, y)) = self.pending.last_mut() {
            *x += dx;
            *y += dy;
        } else {
            self.pending.push(Action::Look(dx, dy));
        }
    }

    /// The wheel, in lines.
    pub fn scroll(&mut self, lines: f64) {
        self.pending.push(Action::Scroll(lines));
    }

    /// The window lost focus: release the mouse button and every key (their
    /// release events will not arrive).
    pub fn focus_lost(&mut self) {
        self.pending.push(Action::Release);
        self.held.clear();
    }

    /// The keys held for the next step.
    pub fn input(&self) -> Input {
        let k = |c: KeyCode| self.held.contains(&c) || self.tapped.contains(&c);
        Input {
            forward: k(KeyCode::KeyW),
            back: k(KeyCode::KeyS),
            left: k(KeyCode::KeyA),
            right: k(KeyCode::KeyD),
            jump: k(KeyCode::Space),
            down: k(KeyCode::ControlLeft) || k(KeyCode::ControlRight),
            sprint: k(KeyCode::ShiftLeft) || k(KeyCode::ShiftRight),
        }
    }

    /// One fixed step: the queued inputs in order, then the step.
    pub fn step(&mut self, session: &mut Session) {
        for a in std::mem::take(&mut self.pending) {
            if let Err(e) = session.apply(a) {
                eprintln!("{e}");
            }
        }
        let input = self.input();
        session.step(&input, STEP);
        self.tapped.clear();
    }

    /// A frame `dt` seconds after the last: run the steps it owes. Returns
    /// how many ran.
    pub fn frame(&mut self, session: &mut Session, dt: f64) -> usize {
        self.acc += dt.min(0.25);
        let mut steps = 0;
        while self.acc >= STEP && steps < MAX_STEPS_PER_FRAME {
            self.step(session);
            self.acc -= STEP;
            steps += 1;
        }
        if steps == MAX_STEPS_PER_FRAME {
            self.acc = 0.0;
        }
        steps
    }

    /// The camera to draw: the player's, with mouse motion not yet stepped
    /// already applied (drawing only; the step applies it for real).
    pub fn camera(&self, player: &Player) -> Camera {
        let mut cam = player.camera();
        let (mut h, mut p) = (cam.heading, cam.pitch);
        for a in &self.pending {
            if let Action::Look(dx, dy) = a {
                h += dx * LOOK_RAD_PER_COUNT;
                p = (p - dy * LOOK_RAD_PER_COUNT).clamp(-PITCH_MAX, PITCH_MAX);
            }
        }
        cam.heading = h;
        cam.pitch = p;
        cam
    }
}
