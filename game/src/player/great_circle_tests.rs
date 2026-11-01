//! Holding a key follows a great circle, in fly and in walk, in any direction,
//! and the view goes with it: over a pole, and at any speed the wheel can reach.
//! Andrew's #10. Everything here is plain geometry: where the circle goes is
//! worked out from the start, by nothing the player's own code uses.

use super::*;
use std::f64::consts::{FRAC_PI_2, TAU};

const STONE: BlockId = 1;

/// Solid below `floor`, on every face.
struct Floor(i64);

impl Voxels for Floor {
    fn block(&self, _face: u8, _i: i64, _j: i64, k: i64) -> BlockId {
        if k < self.0 { STONE } else { 0 }
    }
    fn is_solid(&self, id: BlockId) -> bool {
        id != 0
    }
    fn is_liquid(&self, _id: BlockId) -> bool {
        false
    }
}

fn big() -> Planet {
    Planet::new(1.0, 2048, 512, 131072)
}

/// Small enough to walk round in a test.
fn small() -> Planet {
    Planet::new(1.0, 2048, 512, 8192)
}

fn at(lat: f64, lon: f64) -> DVec3 {
    Planet::dir_of_lat_lon_deg(lat, lon)
}

fn angle(a: DVec3, b: DVec3) -> f64 {
    // Good to the last bit at small angles, unlike a dot product.
    a.cross(b).length().atan2(a.dot(b))
}

const W: Input = Input { forward: true, back: false, left: false, right: false, jump: false, down: false, sprint: false };

const STARTS: [(f64, f64); 7] = [(0.0, 0.0), (35.0, 45.0), (-60.0, 100.0), (80.0, -150.0), (89.9, 10.0), (-85.8, -122.0), (0.0, 179.0)];
const HEADINGS: [f64; 6] = [0.0, 37.0, 90.0, 135.0, 200.0, 271.0];

/// Fly `turns` of a circumference from a start: every step stays on the circle
/// the start says, and the view stays in its plane; it ends where it should.
fn fly_circle(p: &Planet, lat: f64, lon: f64, heading: f64, speed: f64, turns: f64) {
    let dir = at(lat, lon);
    let mut pl = Player::new(dir * (p.radius + 500.0), heading, 0.0, Mode::Fly, 1.0);
    pl.fly_speed = speed;
    let first = pl.look_dir();
    let normal = dir.cross(first).normalize();
    let r = pl.pos.length();
    let steps = (turns * TAU * r / (speed * STEP)).round() as usize;
    for step in 0..steps {
        pl.step(&Floor(0), p, &W, STEP);
        let u = pl.pos.normalize();
        assert!(u.dot(normal).abs() < 1e-9, "({lat}, {lon}) heading {heading} at {speed} m/s, step {step}: {:.3} m off the circle's plane", u.dot(normal) * r);
        assert!(pl.look_dir().dot(normal).abs() < 1e-9, "({lat}, {lon}) heading {heading}, step {step}: the view left the plane by {:.6} deg", pl.look_dir().dot(normal).asin().to_degrees());
        assert!(pl.look_dir().dot(u).abs() < 1e-9, "level flight stays level: {}", pl.look_dir().dot(u));
    }
    assert!((pl.pos.length() - r).abs() < 1e-6, "the height held: {} to {}", r, pl.pos.length());
    let gone = steps as f64 * speed * STEP / r;
    let want = (dir * gone.cos() + first * gone.sin()) * r;
    let miss = pl.pos.distance(want);
    assert!(miss < 1.0 + 2e-7 * steps as f64, "({lat}, {lon}) heading {heading}: ended {miss:.3} m from where the circle says");
}

#[test]
fn flying_forward_goes_a_quarter_of_the_way_round_on_a_great_circle() {
    let p = big();
    for &(lat, lon) in &STARTS {
        for &h in &HEADINGS {
            fly_circle(&p, lat, lon, h, 5000.0, 0.25);
        }
    }
}

#[test]
fn flying_forward_goes_all_the_way_round_on_a_great_circle() {
    let p = big();
    for &(lat, lon) in &STARTS {
        for &h in &HEADINGS {
            fly_circle(&p, lat, lon, h, 2000.0, 1.0);
        }
    }
}

/// A steep flight is in the plane too: the pitch tips the view within it.
#[test]
fn a_pitched_flight_stays_in_the_plane() {
    let p = big();
    for &(lat, lon) in &STARTS {
        for &h in &HEADINGS {
            for pitch in [-30.0, 25.0] {
                let dir = at(lat, lon);
                let mut pl = Player::new(dir * (p.radius + 40_000.0), h, pitch, Mode::Fly, 1.0);
                pl.fly_speed = 300.0;
                let normal = dir.cross(pl.look_dir()).normalize();
                for step in 0..3000 {
                    pl.step(&Floor(0), &p, &W, STEP);
                    assert!(pl.pos.normalize().dot(normal).abs() < 1e-9 && pl.look_dir().dot(normal).abs() < 1e-9, "({lat}, {lon}) heading {h} pitch {pitch} step {step}");
                }
            }
        }
    }
}

/// Sideways is a great circle at right angles to the view, and the view keeps
/// looking along the axis of that circle: it does not turn at all.
#[test]
fn strafing_is_a_great_circle_too() {
    let p = big();
    for &(lat, lon) in &STARTS {
        for &h in &HEADINGS {
            for (input, sign) in [(Input { right: true, ..Default::default() }, 1.0), (Input { left: true, ..Default::default() }, -1.0)] {
                let dir = at(lat, lon);
                let mut pl = Player::new(dir * (p.radius + 500.0), h, 0.0, Mode::Fly, 1.0);
                pl.fly_speed = 5000.0;
                let view = pl.look_dir();
                let side = view.cross(dir).normalize() * sign;
                let r = pl.pos.length();
                let steps = (0.25 * TAU * r / (5000.0 * STEP)).round() as usize;
                for step in 0..steps {
                    pl.step(&Floor(0), &p, &input, STEP);
                    assert!(pl.pos.normalize().dot(view).abs() < 1e-9, "({lat}, {lon}) heading {h}: step {step} left the circle");
                    assert!(angle(pl.look_dir(), view) < 1e-9, "({lat}, {lon}) heading {h}: step {step} the view turned {} rad", angle(pl.look_dir(), view));
                }
                let gone = steps as f64 * 5000.0 * STEP / r;
                let want = (dir * gone.cos() + side * gone.sin()) * r;
                assert!(pl.pos.distance(want) < 3.0, "({lat}, {lon}) heading {h}: {} m off", pl.pos.distance(want));
            }
        }
    }
}

/// W and D together: a great circle at 45 degrees to the view.
#[test]
fn a_diagonal_is_a_great_circle_too() {
    let p = big();
    let input = Input { forward: true, right: true, ..Default::default() };
    for &(lat, lon) in &STARTS {
        let dir = at(lat, lon);
        let mut pl = Player::new(dir * (p.radius + 500.0), 30.0, 0.0, Mode::Fly, 1.0);
        pl.fly_speed = 5000.0;
        let view = pl.look_dir();
        let along = (view + view.cross(dir)).normalize();
        let normal = dir.cross(along).normalize();
        for step in 0..2000 {
            pl.step(&Floor(0), &p, &input, STEP);
            let u = pl.pos.normalize();
            assert!(u.dot(normal).abs() < 1e-9, "({lat}, {lon}) step {step}");
            assert!((angle(pl.look_dir(), normal.cross(u)) - std::f64::consts::FRAC_PI_4).abs() < 1e-9, "the view keeps its angle to the path");
        }
    }
}

/// Fly over the pole: the view turns by the motion's own angle and no more.
fn no_jump_over_a_pole(speed: f64, pole: f64, miss_deg: f64, lon: f64) {
    let p = big();
    // Start 0.5 degrees short of the pole, aimed to miss it by `miss_deg` on the side.
    let start = at(pole * 89.5, lon);
    let aim = at(pole * (90.0 - miss_deg), lon + 90.0);
    let mut pl = Player::new(start * (p.radius + 200.0), 0.0, 0.0, Mode::Fly, 1.0);
    pl.face(aim - start);
    pl.fly_speed = speed;
    let view0 = pl.look_dir();
    let normal = start.cross(view0).normalize();
    // A degree of arc: half a degree to the pole and half a degree beyond.
    let steps = (1f64.to_radians() * pl.pos.length() / (speed * STEP)).ceil() as usize;
    let mut nearest = f64::MAX;
    for step in 0..steps {
        let (u0, v0) = (pl.pos.normalize(), pl.look_dir());
        pl.step(&Floor(0), &p, &W, STEP);
        let (u1, v1) = (pl.pos.normalize(), pl.look_dir());
        let (moved, turned) = (angle(u0, u1), angle(v0, v1));
        assert!(turned <= moved * (1.0 + 1e-6) + 1e-12, "{speed} m/s, step {step}, {:.4} deg from the pole: the view turned {turned:.6e} rad for a move of {moved:.6e} rad", (90.0 - (u1.z * pole).asin().to_degrees()));
        assert!(v1.dot(normal).abs() < 1e-9, "{speed} m/s step {step}: the view left the plane");
        nearest = nearest.min(90.0 - (u1.z * pole).clamp(-1.0, 1.0).asin().to_degrees());
    }
    assert!(nearest <= miss_deg + (speed * STEP / pl.pos.length()).to_degrees(), "it passed the pole: {nearest} degrees off");
    let end = pl.pos.normalize();
    assert!(end.z * pole > 0.9999, "and came out the other side: lat {}", end.z.asin().to_degrees());
    assert!(angle(end, start) > 0.016, "it went a degree: {}", angle(end, start).to_degrees());
}

#[test]
fn flying_straight_over_a_pole_has_no_jump_at_20_m_s() {
    for pole in [1.0, -1.0] {
        for lon in [0.0, 77.0] {
            for miss in [0.0, 1e-7, 1e-4, 0.01] {
                no_jump_over_a_pole(20.0, pole, miss, lon);
            }
        }
    }
}

#[test]
fn flying_straight_over_a_pole_has_no_jump_at_5000_m_s() {
    for pole in [1.0, -1.0] {
        for lon in [0.0, 77.0] {
            for miss in [0.0, 1e-7, 1e-4, 0.01] {
                no_jump_over_a_pole(5000.0, pole, miss, lon);
            }
        }
    }
}

/// Exactly on the pole, where there is no north: the view is the view, a look
/// turns it, and the first step goes the way it faces.
#[test]
fn standing_exactly_on_a_pole_is_nothing_special() {
    let p = big();
    for sign in [1.0, -1.0] {
        let pos = DVec3::Z * sign * (p.radius + 100.0);
        let mut pl = Player::new(pos, 40.0, 0.0, Mode::Fly, 1.0);
        let view = pl.look_dir();
        assert!(view.is_finite() && view.dot(DVec3::Z).abs() < 1e-12, "{view}");
        pl.fly_speed = 20.0;
        pl.step(&Floor(0), &p, &W, STEP);
        assert!(angle(pl.pos - pos, view) < 1e-5, "it went the way it faced");
        assert!(angle(pl.look_dir(), view) < 1e-3, "and the view stayed put");
        assert!(pl.camera().heading().is_finite());
    }
}

/// `start`, walking on the small planet's floor, facing `heading`.
fn walker(p: &Planet, dir: DVec3, heading: f64) -> Player {
    let floor = 600;
    let mut pl = Player::new(dir * p.radius_at(floor as f64 + 0.001), heading, 0.0, Mode::Walk, 1.0);
    pl.on_ground = true;
    pl
}

fn walk_circle(p: &Planet, lat: f64, lon: f64, heading: f64, turns: f64) {
    let w = Floor(600);
    let dir = at(lat, lon);
    let mut pl = walker(p, dir, heading);
    let first = pl.look_dir();
    let normal = dir.cross(first).normalize();
    let r = pl.pos.length();
    let steps = (turns * TAU * r / (WALK_SPEED * STEP)).round() as usize;
    let mut worst = 0f64;
    for step in 0..steps {
        pl.step(&w, p, &W, STEP);
        let u = pl.pos.normalize();
        worst = worst.max(u.dot(normal).abs() * r);
        assert!(worst < 2.0, "({lat}, {lon}) heading {heading}, step {step} of {steps}: {worst:.3} m off the circle");
        assert!(pl.look_dir().dot(normal).abs() < 1e-3, "({lat}, {lon}) heading {heading}, step {step}: the view left the plane by {:.4} deg", pl.look_dir().dot(normal).asin().to_degrees());
        assert!(pl.on_ground, "step {step}: fell off a floor that goes everywhere");
    }
    let gone = angle(dir, pl.pos.normalize());
    let want = (turns * TAU).rem_euclid(TAU);
    let want = if want > std::f64::consts::PI { TAU - want } else { want };
    assert!((gone - want).abs() * r < 0.003 * turns * TAU * r + 1.0, "({lat}, {lon}) heading {heading}: went {:.1} m, the circle says {:.1} m", gone * r, want * r);
}

#[test]
fn walking_forward_goes_a_quarter_of_the_way_round_on_a_great_circle() {
    let p = small();
    for &(lat, lon) in &STARTS {
        for &h in &HEADINGS {
            walk_circle(&p, lat, lon, h, 0.25);
        }
    }
}

#[test]
fn walking_forward_goes_all_the_way_round_on_a_great_circle() {
    let p = small();
    for &(lat, lon) in &STARTS[..5] {
        for &h in &[37.0, 135.0] {
            walk_circle(&p, lat, lon, h, 1.0);
        }
    }
}

#[test]
fn walking_sideways_is_a_great_circle_too() {
    let p = small();
    let w = Floor(600);
    for &(lat, lon) in &STARTS {
        let dir = at(lat, lon);
        let mut pl = walker(&p, dir, 30.0);
        let view = pl.look_dir();
        let r = pl.pos.length();
        let steps = (0.25 * TAU * r / (WALK_SPEED * STEP)).round() as usize;
        for step in 0..steps {
            pl.step(&w, &p, &Input { right: true, ..Default::default() }, STEP);
            assert!((pl.pos.normalize().dot(view) * r).abs() < 2.0, "({lat}, {lon}) step {step}: left the circle by {:.3} m", pl.pos.normalize().dot(view) * r);
        }
    }
}

/// Walk over the pole on the floor: no jump in the view, and out the far side.
#[test]
fn walking_straight_over_a_pole_has_no_jump() {
    let p = small();
    let w = Floor(600);
    for pole in [1.0, -1.0] {
        for (lon, miss) in [(0.0, 0.0), (77.0, 0.0), (20.0, 0.02), (130.0, 0.001)] {
            let span: f64 = 0.3;
            let start = at(pole * (90.0 - span), lon);
            let aim = at(pole * (90.0 - miss), lon + 90.0);
            let mut pl = walker(&p, start, 0.0);
            pl.face(aim - start);
            let normal = start.cross(pl.look_dir()).normalize();
            let steps = (2.0 * span.to_radians() * pl.pos.length() / (WALK_SPEED * STEP)) as usize;
            for step in 0..steps {
                let (u0, v0) = (pl.pos.normalize(), pl.look_dir());
                pl.step(&w, &p, &W, STEP);
                let (u1, v1) = (pl.pos.normalize(), pl.look_dir());
                let (moved, turned) = (angle(u0, u1), angle(v0, v1));
                assert!(turned <= moved * 1.001 + 1e-9, "pole {pole} lon {lon} step {step}: the view turned {turned:.6e} for a move of {moved:.6e}");
                assert!(v1.dot(normal).abs() < 1e-3 && (u1.dot(normal) * pl.pos.length()).abs() < 0.5, "pole {pole} lon {lon} step {step}: off the circle");
                assert!(pl.on_ground);
            }
            let end = pl.pos.normalize();
            let far = (end * pole).z;
            assert!(far > 0.99 && angle(end, start) > 1.9 * span.to_radians(), "pole {pole} lon {lon}: lat {} after {steps} steps", end.z.asin().to_degrees());
        }
    }
}

/// The same floor as the world says it under `--powder levels`: every walking
/// step goes through `step_on_slopes`.
struct SlopedFloor(i64);

impl Voxels for SlopedFloor {
    fn block(&self, _face: u8, _i: i64, _j: i64, k: i64) -> BlockId {
        if k < self.0 { STONE } else { 0 }
    }
    fn is_solid(&self, id: BlockId) -> bool {
        id != 0
    }
    fn is_liquid(&self, _id: BlockId) -> bool {
        false
    }
    fn has_slopes(&self) -> bool {
        true
    }
}

#[test]
fn walking_under_powder_levels_is_a_great_circle_too() {
    let p = small();
    let w = SlopedFloor(600);
    for &(lat, lon) in &STARTS {
        let dir = at(lat, lon);
        let mut pl = walker(&p, dir, 37.0);
        let normal = dir.cross(pl.look_dir()).normalize();
        let r = pl.pos.length();
        let steps = (0.25 * TAU * r / (WALK_SPEED * STEP)).round() as usize;
        for step in 0..steps {
            pl.step(&w, &p, &W, STEP);
            assert!((pl.pos.normalize().dot(normal) * r).abs() < 2.0, "({lat}, {lon}) step {step}: {:.3} m off the circle", pl.pos.normalize().dot(normal) * r);
            assert!(pl.look_dir().dot(normal).abs() < 1e-3, "({lat}, {lon}) step {step}: the view left the plane");
        }
    }
}

/// Jumping as it goes: in the air or on the ground, the view goes with the body.
#[test]
fn the_view_is_carried_on_any_step_however_the_step_moved() {
    let p = small();
    let w = Floor(600);
    let mut pl = walker(&p, at(89.0, 30.0), 10.0);
    let first = pl.look_dir();
    let normal = pl.pos.normalize().cross(first).normalize();
    for step in 0..2000 {
        let jump = step % 90 == 0;
        pl.step(&w, &p, &Input { forward: true, jump, sprint: true, ..Default::default() }, STEP);
        assert!(pl.pos.normalize().dot(normal).abs() < 1e-5, "step {step}: {}", pl.pos.normalize().dot(normal));
    }
}

/// How far a look turns the view, measured against where the view would have
/// been carried without it.
fn turn_at(lat: f64, counts: f64) -> f64 {
    let p = big();
    let mut pl = Player::new(at(lat, 20.0) * (p.radius + 100.0), 50.0, 0.0, Mode::Fly, 1.0);
    let v0 = pl.look_dir();
    let right = pl.camera().right();
    pl.look(counts, 0.0);
    let v1 = pl.look_dir();
    assert!(v1.dot(right) > 0.0, "a look to the right goes right");
    angle(v0, v1)
}

#[test]
fn mouse_look_at_lat_89_99_turns_the_same_as_at_the_equator() {
    for counts in [0.001, 0.1, 1.0] {
        let e = turn_at(0.0, counts);
        for lat in [89.99, -89.99, 89.9999999, 90.0, -90.0] {
            let got = turn_at(lat, counts);
            assert!((got - e).abs() < 1e-12, "lat {lat}: {got} rad, {e} at the equator");
        }
    }
}

/// The same, on the move: at 16 m from the pole a heading swings round fast as
/// the longitude does, and that must not reach the turn the mouse asked for.
#[test]
fn mouse_look_on_the_move_near_a_pole_turns_by_what_the_mouse_said() {
    let p = big();
    for fly_right in [false, true] {
        let mut pl = Player::new(at(89.99, 20.0) * (p.radius + 100.0), 50.0, 0.0, Mode::Fly, 1.0);
        pl.fly_speed = 20.0;
        let input = Input { forward: !fly_right, right: fly_right, ..Default::default() };
        for step in 0..3000 {
            let (u0, v0) = (pl.pos.normalize(), pl.look_dir());
            pl.step(&Floor(0), &p, &input, STEP);
            let u1 = pl.pos.normalize();
            let moved = carried(u0, u1, v0);
            assert!(angle(moved, pl.look_dir()) < 1e-9, "step {step}: the view swung {} rad on its own", angle(moved, pl.look_dir()));
            let before = pl.look_dir();
            pl.look(0.002, 0.0);
            assert!((angle(before, pl.look_dir()) - 0.002).abs() < 1e-12, "step {step}");
        }
    }
}

/// A teleport keeps the compass heading: the frames are different, the
/// direction people read is not.
#[test]
fn a_teleport_faces_the_same_compass_heading_at_the_other_end() {
    for (from, to) in [((12.0, 30.0), (-40.0, 170.0)), ((89.0, 0.0), (-89.0, 90.0)), ((0.0, 0.0), (0.0, 180.0))] {
        for h in [0.0, 45.0, -120.0, 179.0] {
            let mut pl = Player::new(at(from.0, from.1) * 1e5, h, 10.0, Mode::Fly, 1.0);
            pl.place(at(to.0, to.1) * 2e5);
            let got = pl.heading().to_degrees();
            assert!((got - h).rem_euclid(360.0).min((h - got).rem_euclid(360.0)) < 1e-9, "{from:?} to {to:?}: heading {h} became {got}");
            assert!(pl.facing.dot(pl.pos.normalize()).abs() < 1e-12, "level at the new place");
            assert_eq!(pl.pitch, 10f64.to_radians());
        }
    }
}

#[test]
fn set_heading_and_heading_agree() {
    for lat in [0.0, 45.0, 89.0, -89.999] {
        for h in [0.0, 30.0, 90.0, -90.0, 179.0] {
            let mut pl = Player::new(at(lat, 33.0) * 1e5, 0.0, 0.0, Mode::Fly, 1.0);
            pl.set_heading(f64::to_radians(h));
            assert!((pl.heading().to_degrees() - h).abs() < 1e-9, "{lat}: {h} vs {}", pl.heading().to_degrees());
            let (north, east, _) = Camera::local_frame(pl.pos);
            let want = north * f64::to_radians(h).cos() + east * f64::to_radians(h).sin();
            assert!(angle(pl.look_dir(), want) < 1e-9, "{lat}: heading {h}: {}", angle(pl.look_dir(), want));
        }
    }
}

// ---- no limit on the wheel ---------------------------------------------------

/// Every speed there is, every direction, from the ground out and back: the
/// numbers stay numbers, nothing falls below `min_r`, nothing leaves the
/// universe, and a dive stops at the floor on the side it started from.
#[test]
fn any_speed_any_direction_stays_a_position() {
    let p = big();
    let min_r = p.bottom_radius() + 2.0;
    let inputs = [
        W,
        Input { back: true, ..Default::default() },
        Input { left: true, ..Default::default() },
        Input { right: true, sprint: true, ..Default::default() },
        Input { jump: true, ..Default::default() },
        Input { down: true, ..Default::default() },
        Input { forward: true, jump: true, sprint: true, ..Default::default() },
        Input { forward: true, down: true, right: true, ..Default::default() },
    ];
    for speed in [1e3, 1e6, 1e9, 1e12, 1e18, 1e30, 1e100, MAX_FLY_SPEED] {
        for pitch in [-88.0, -45.0, 0.0, 45.0, 88.0] {
            let mut pl = Player::new(at(35.0, 45.0) * (p.radius + 10.0), 30.0, pitch, Mode::Fly, 1.0);
            pl.fly_speed = speed;
            for step in 0..400 {
                let input = inputs[(step / 50) % inputs.len()];
                let u0 = pl.pos.normalize();
                pl.step(&Floor(0), &p, &input, STEP);
                let r = pl.pos.length();
                assert!(pl.pos.is_finite() && pl.vel.is_finite() && pl.facing.is_finite() && pl.look_dir().is_finite(), "{speed} m/s pitch {pitch} step {step}: {pl:?}");
                assert!(r >= min_r - 1e-6 && r <= MAX_RADIUS_M * (1.0 + 1e-9), "{speed} m/s pitch {pitch} step {step}: r = {r}");
                assert!((pl.facing.length() - 1.0).abs() < 1e-12 && pl.facing.dot(pl.pos.normalize()).abs() < 1e-9, "{speed} m/s pitch {pitch} step {step}: facing {}", pl.facing);
                assert!(angle(u0, pl.pos.normalize()) <= FRAC_PI_2 + 1e-9, "{speed} m/s pitch {pitch} step {step}: a step went past a quarter turn");
            }
        }
    }
}

/// Straight down at any speed, from anywhere: it stops at the bottom of the
/// world on this side, in one step or many. `pos += vel * dt` came out on the
/// far side of the planet when a step was longer than the planet is wide.
#[test]
fn a_huge_step_down_stops_at_the_floor_and_does_not_tunnel() {
    let p = big();
    let min_r = p.bottom_radius() + 2.0;
    let down = Input { down: true, ..Default::default() };
    for speed in [5000.0, 1e6, 1e7, 3e7, 1e9, 1e12, 1e30, MAX_FLY_SPEED] {
        for from in [p.radius + 1000.0, 3e5, 3.84e8, 1e12] {
            let steps = (from / (speed * STEP)).ceil() + 1.0;
            if steps > 1e7 {
                continue;
            }
            let start = at(20.0, -30.0);
            let mut pl = Player::new(start * from, 0.0, 0.0, Mode::Fly, 1.0);
            pl.fly_speed = speed;
            for _ in 0..steps as usize {
                pl.step(&Floor(0), &p, &down, STEP);
            }
            assert!(pl.pos.distance(start * min_r) < 1e-5, "{speed} m/s from {from}: ended at {} ({} m from where it should)", pl.pos, pl.pos.distance(start * min_r));
        }
    }
}

/// Flying at the view, not straight down: a dive at the ground at a thousand
/// times the planet's size a step still lands on this side of it.
#[test]
fn a_huge_dive_along_the_view_does_not_tunnel() {
    let p = big();
    let min_r = p.bottom_radius() + 2.0;
    for speed in [1e7, 1e8, 1e9, 1e12] {
        let start = at(-20.0, 60.0);
        let mut pl = Player::new(start * (p.radius + 2e5), 0.0, -88.0, Mode::Fly, 1.0);
        pl.fly_speed = speed;
        for step in 0..3 {
            let u0 = pl.pos.normalize();
            pl.step(&Floor(0), &p, &W, STEP);
            assert!(pl.pos.normalize().dot(u0) >= 0.0, "{speed} m/s step {step}: went through to the far side");
            assert!(pl.pos.length() >= min_r - 1e-6);
        }
    }
}

/// The walk, thrown in from orbit or from deep space: a switch to walk at any
/// height is a body falling, and nothing in the walk minds how far out it is.
#[test]
fn walking_from_far_out_is_a_body_falling() {
    let p = big();
    for r in [1e6, 3.84e8, 1e12, MAX_RADIUS_M] {
        let mut pl = Player::new(at(10.0, 10.0) * r, 0.0, 0.0, Mode::Walk, 1.0);
        for step in 0..200 {
            pl.step(&Floor(600), &p, &W, STEP);
            assert!(pl.pos.is_finite() && pl.vel.is_finite() && pl.facing.is_finite(), "r {r} step {step}: {pl:?}");
        }
        assert!(pl.pos.length() < r, "it falls");
    }
}

#[test]
fn nothing_can_be_picked_from_far_out() {
    let p = big();
    for r in [p.radius_at(2048.0) + 7.0, 3.84e8, 1e15, 1e25] {
        let t = std::time::Instant::now();
        assert!(pick(&Floor(600), &p, DVec3::X * r, DVec3::new(-1.0, 0.3, 0.1), REACH_M, false).is_none());
        assert!(t.elapsed().as_secs_f64() < 0.5, "a pick at {r} took {:?}", t.elapsed());
    }
}
