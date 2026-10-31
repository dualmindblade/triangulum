//! B7: walking on sloped powder. The collision follows the surface the mesher
//! draws: the box stands on the highest point of the slope under it. Falling
//! through is the one thing that may not happen, at any speed, from any side.

use super::*;
use crate::grid::BlockPos;
use crate::world::tests::{hills_ground, test_world};
use crate::world::{BlockId, World};

/// A stone plate in the sky with a cone of sand on it, `h0` units high at
/// `c`, `fall` units a cell, set straight into the world.
fn cone(w: &World, c: BlockPos, h0: i64, fall: i64, r: i64) {
    let (stone, sand) = (w.generator.blocks.stone, w.generator.blocks.sand);
    w.edits.set_slopes(true);
    let mut writes: Vec<(BlockPos, BlockId, Option<u8>)> = Vec::new();
    for di in -r..=r {
        for dj in -r..=r {
            let at = |k: u32| w.resolve_block(c.face, c.i as i64 + di, c.j as i64 + dj, k as i64).unwrap();
            writes.push((at(c.k - 1), stone, None));
            let mut left = (h0 - fall * ((di * di + dj * dj) as f64).sqrt().round() as i64).max(0);
            let mut k = c.k;
            while left > 0 {
                let u = left.min(255);
                writes.push((at(k), sand, Some(if u == 255 { 0 } else { u as u8 })));
                left -= u;
                k += 1;
            }
        }
    }
    w.commit_levels(&writes);
}

/// A body on the plate at (di, dj) from `c`, facing along +i.
fn walker(w: &World, c: BlockPos, di: f64, dj: f64) -> Player {
    let p = &w.planet;
    let pos = p.pos(c.face, c.i as f64 + 0.5 + di, c.j as f64 + 0.5 + dj, c.k as f64 + 1e-5);
    let mut b = Player::new(pos, 0.0, 0.0, Mode::Walk, p.block_size);
    let f = Frame::at(p, pos);
    let (north, east, _) = Camera::local_frame(pos);
    b.heading = f.ei.dot(east).atan2(f.ei.dot(north));
    b.on_ground = true;
    b
}

const FORWARD: Input = Input { forward: true, back: false, left: false, right: false, jump: false, down: false, sprint: false };

/// How far the feet stand over the surface under the box (negative: in it).
fn clearance(w: &World, b: &Player) -> Option<f64> {
    let f = Frame::at(&w.planet, b.pos);
    slope_under(w, &f, b.lattice_box(&f)).map(|top| f.k - top)
}

#[test]
fn a_body_walks_up_a_pile_and_down_the_other_side_on_the_surface() {
    let w = test_world();
    let g = hills_ground(&w);
    let c = BlockPos { k: g.k + 24, ..g };
    cone(&w, c, 900, 170, 8);
    let p = w.planet;
    for dj in [0.0, 0.31, 1.7, -2.4] {
        let mut b = walker(&w, c, -7.5, dj);
        let start = Frame::at(&p, b.pos).i;
        let (mut high, mut last, mut most, mut air) = (0f64, Frame::at(&p, b.pos).k, 0f64, 0);
        for step in 0..260 {
            b.step(&w, &p, &FORWARD, STEP);
            let f = Frame::at(&p, b.pos);
            if f.i > c.i as f64 + 7.5 {
                break;
            }
            high = high.max(f.k);
            most = most.max((f.k - last).abs());
            last = f.k;
            air += !b.on_ground as u32;
            if let Some(gap) = clearance(&w, &b) {
                assert!(gap >= -1e-3, "step {step}: {gap} into the slope at {:.2}", f.i);
                assert!(gap < 0.5, "step {step}: {gap} over the slope at {:.2}", f.i);
            }
            assert!(b.vel.length() < 8.0, "step {step}: {}", b.vel);
        }
        let f = Frame::at(&p, b.pos);
        let across = f.i - start;
        println!("dj {dj}: {across:.2} cells across, up {:.2}, steepest change {most:.3} a step, {air} steps off the ground", high - c.k as f64);
        assert!(across > 14.0, "it got across the pile: {across}");
        assert!(high - c.k as f64 > if dj == 0.0 { 2.5 } else { 1.0 }, "it went up it: {}", high - c.k as f64);
        assert!((f.k - c.k as f64).abs() < 1e-3, "and is back on the plate: {}", f.k);
        assert!(most < 0.2, "no step is a jump: {most}");
        assert!(air < 6, "it stays on the ground: {air} steps off it");
    }
}

/// Over a cube edge the lattice turns under the body, and the slope goes on.
#[test]
fn a_body_walks_over_a_pile_on_a_cube_edge() {
    let w = test_world();
    let n = w.planet.n;
    let p = w.planet;
    let c = BlockPos { face: 0, i: n - 3, j: n / 2, k: 1500 };
    cone(&w, c, 900, 170, 8);
    let mut b = walker(&w, c, -7.5, 0.3);
    let start = b.pos;
    let (mut high, mut crossed) = (0f64, false);
    for step in 0..200 {
        b.step(&w, &p, &FORWARD, STEP);
        let f = Frame::at(&p, b.pos);
        crossed |= f.face != c.face;
        high = high.max(f.k);
        if let Some(gap) = clearance(&w, &b) {
            assert!(gap >= -1e-3, "step {step}: {gap} into the slope on face {}", f.face);
        }
        assert!(f.k >= c.k as f64 - 1e-3, "step {step}: through the plate");
        // Face to face the heading is the same world direction only for a
        // moment: keep it pointing onward.
        let onward = (b.pos - start).normalize();
        let (north, east, up) = Camera::local_frame(b.pos);
        let t = (onward - up * onward.dot(up)).normalize();
        b.heading = t.dot(east).atan2(t.dot(north));
    }
    assert!(crossed, "it reached the other face");
    assert!(high - c.k as f64 > 2.5, "up the pile: {}", high - c.k as f64);
}

/// Dropped from high enough to reach terminal speed, anywhere on the pile: it
/// stops on the surface. At 70 m/s a step is more than a cell.
#[test]
fn a_body_falling_at_terminal_speed_stops_on_the_surface() {
    let w = test_world();
    let g = hills_ground(&w);
    let c = BlockPos { k: g.k + 24, ..g };
    cone(&w, c, 900, 170, 8);
    let p = w.planet;
    for (di, dj) in [(0.0, 0.0), (0.4, 0.6), (1.5, 2.2), (-3.3, 1.1), (4.9, -0.1), (-3.5, -2.5)] {
        let mut b = walker(&w, c, di, dj);
        let d = b.pos.normalize();
        b.pos = d * p.radius_at(c.k as f64 + 12.0);
        b.vel = -d * 70.0;
        b.on_ground = false;
        for step in 0..60 {
            b.step(&w, &p, &Input::default(), STEP);
            if let Some(gap) = clearance(&w, &b) {
                assert!(gap >= -1e-3, "({di}, {dj}) step {step}: {gap} into the slope");
            }
            assert!(Frame::at(&p, b.pos).k >= c.k as f64 - 1e-3, "({di}, {dj}) step {step}: through to the plate");
        }
        assert!(b.on_ground, "({di}, {dj}) it came to rest");
        assert!(b.vel.length() < 1e-6, "({di}, {dj}) at rest: {}", b.vel);
        let gap = clearance(&w, &b).expect("over the pile");
        assert!(gap.abs() < 1e-3, "({di}, {dj}): {gap}");
    }
}

#[test]
fn a_body_stands_jumps_and_sneaks_on_a_slope() {
    let w = test_world();
    let g = hills_ground(&w);
    let c = BlockPos { k: g.k + 24, ..g };
    cone(&w, c, 900, 170, 8);
    let p = w.planet;
    let mut b = walker(&w, c, -3.0, 0.2);
    for _ in 0..60 {
        b.step(&w, &p, &Input::default(), STEP);
    }
    assert!(b.on_ground);
    let rest = b.pos;
    for _ in 0..60 {
        b.step(&w, &p, &Input::default(), STEP);
    }
    assert!((b.pos - rest).length() < 1e-6, "it does not creep: {}", (b.pos - rest).length());
    // A jump leaves the surface and comes back to it.
    let k0 = Frame::at(&p, b.pos).k;
    b.step(&w, &p, &Input { jump: true, ..Input::default() }, STEP);
    assert!(!b.on_ground || b.vel.dot(b.pos.normalize()) > 1.0, "the jump took");
    let mut high = 0f64;
    for _ in 0..50 {
        b.step(&w, &p, &Input::default(), STEP);
        high = high.max(Frame::at(&p, b.pos).k - k0);
    }
    assert!(high > 0.9, "a jump is a jump on a slope: {high}");
    assert!(b.on_ground);
    assert!((Frame::at(&p, b.pos).k - k0).abs() < 1e-3);
    // Sneaking: slower, and not stuck, though there is no edge to hold.
    let x0 = Frame::at(&p, b.pos).i;
    for _ in 0..60 {
        b.step(&w, &p, &Input { forward: true, down: true, ..Input::default() }, STEP);
    }
    let moved = Frame::at(&p, b.pos).i - x0;
    assert!(moved > 0.8 && moved < 1.6, "a second of sneaking: {moved}");
}

/// A body walks into the edge of a pile that stands on a shelf a full block
/// high: nothing lifts it up the shelf, and the slope does not either.
#[test]
fn a_shelf_is_a_wall_whatever_stands_on_it() {
    let w = test_world();
    let g = hills_ground(&w);
    let c = BlockPos { k: g.k + 24, ..g };
    let p = w.planet;
    let (stone, sand) = (w.generator.blocks.stone, w.generator.blocks.sand);
    w.edits.set_slopes(true);
    let mut writes: Vec<(BlockPos, BlockId, Option<u8>)> = Vec::new();
    for di in -8i64..=8 {
        for dj in -4i64..=4 {
            let (i, j) = ((c.i as i64 + di) as u32, (c.j as i64 + dj) as u32);
            writes.push((BlockPos { i, j, k: c.k - 1, ..c }, stone, None));
            if di >= 0 {
                writes.push((BlockPos { i, j, k: c.k, ..c }, stone, None));
                writes.push((BlockPos { i, j, k: c.k + 1, ..c }, sand, Some(180)));
            }
        }
    }
    w.commit_levels(&writes);
    let mut b = walker(&w, c, -3.0, 0.0);
    for _ in 0..120 {
        b.step(&w, &p, &FORWARD, STEP);
    }
    let f = Frame::at(&p, b.pos);
    assert!(f.i < c.i as f64 + 0.5, "stopped at the shelf: {}", f.i - c.i as f64);
    assert!((f.k - c.k as f64).abs() < 1e-3, "on the plate: {}", f.k - c.k as f64);
}

/// Under a stone shelf with sand on top a body walks through as if it were
/// not there: a slope over its head does not lift it.
#[test]
fn a_slope_overhead_is_not_the_ground() {
    let w = test_world();
    let g = hills_ground(&w);
    let c = BlockPos { k: g.k + 24, ..g };
    let p = w.planet;
    let (stone, sand) = (w.generator.blocks.stone, w.generator.blocks.sand);
    w.edits.set_slopes(true);
    let mut writes: Vec<(BlockPos, BlockId, Option<u8>)> = Vec::new();
    for di in -8i64..=8 {
        for dj in -4i64..=4 {
            let (i, j) = ((c.i as i64 + di) as u32, (c.j as i64 + dj) as u32);
            writes.push((BlockPos { i, j, k: c.k - 1, ..c }, stone, None));
            if di.abs() < 4 {
                writes.push((BlockPos { i, j, k: c.k + 2, ..c }, stone, None));
                writes.push((BlockPos { i, j, k: c.k + 3, ..c }, sand, Some(100)));
            }
        }
    }
    w.commit_levels(&writes);
    let mut b = walker(&w, c, -7.0, 0.0);
    for step in 0..180 {
        b.step(&w, &p, &FORWARD, STEP);
        assert!((Frame::at(&p, b.pos).k - c.k as f64).abs() < 1e-3, "step {step}: {}", Frame::at(&p, b.pos).k);
    }
    assert!(Frame::at(&p, b.pos).i > c.i as f64 + 4.0, "it walked through");
}

/// The ray stops where the eye sees the surface. A ray over a low partial
/// cell passes on to what is behind it; a ray into it finds it.
#[test]
fn picking_finds_the_surface_not_the_cube() {
    let w = test_world();
    let g = hills_ground(&w);
    let c = BlockPos { k: g.k + 24, ..g };
    let p = w.planet;
    let (stone, sand) = (w.generator.blocks.stone, w.generator.blocks.sand);
    w.edits.set_slopes(true);
    let mut writes: Vec<(BlockPos, BlockId, Option<u8>)> = Vec::new();
    for di in -6i64..=6 {
        for dj in -6i64..=6 {
            let (i, j) = ((c.i as i64 + di) as u32, (c.j as i64 + dj) as u32);
            writes.push((BlockPos { i, j, k: c.k - 1, ..c }, stone, None));
        }
    }
    // A low partial cell with a stone post behind it.
    writes.push((c, sand, Some(60)));
    let post = BlockPos { i: c.i + 2, ..c };
    writes.push((post, stone, None));
    w.commit_levels(&writes);
    let cell = |i: f64, k: f64| p.pos(c.face, c.i as f64 + i, c.j as f64 + 0.5, c.k as f64 + k);
    // Level, at 0.6 of a layer: above the cell's 0.235 at its middle, inside its cube.
    let eye = cell(-2.0, 0.6);
    let dir = (cell(2.5, 0.6) - eye).normalize();
    let hit = pick(&w, &p, eye, dir, 6.0, false).expect("a hit");
    assert_eq!(hit.block, post, "the ray passes over the slope to the post");
    // Down onto the middle of the cell: it hits the sand.
    let eye = cell(0.5, 3.0);
    let dir = (cell(0.5, 0.0) - eye).normalize();
    let hit = pick(&w, &p, eye, dir, 6.0, false).expect("a hit");
    assert_eq!(hit.block, c, "the ray finds the sand");
    assert_eq!(hit.id, sand);
    // With the switch off the cube is the cell, as it always was.
    w.edits.set_slopes(false);
    let eye = cell(-2.0, 0.6);
    let dir = (cell(2.5, 0.6) - eye).normalize();
    assert_eq!(pick(&w, &p, eye, dir, 6.0, false).expect("a hit").block, c, "off, a partial cell would be its cube");
}

/// Off, the walking step is the walking step: a body on a whole-block sand
/// stair goes the way it always did, with partial levels ignored.
#[test]
fn with_the_switch_off_a_cell_is_its_cube() {
    let w = test_world();
    let g = hills_ground(&w);
    let c = BlockPos { k: g.k + 24, ..g };
    cone(&w, c, 900, 170, 8);
    w.edits.set_slopes(false);
    let p = w.planet;
    let mut b = walker(&w, c, -7.5, 0.0);
    for _ in 0..120 {
        b.step(&w, &p, &FORWARD, STEP);
    }
    let f = Frame::at(&p, b.pos);
    assert!(f.i < c.i as f64 - 2.0, "a cube's side stops it: {}", f.i - c.i as f64);
    assert!(b.box_is_free(&w, &p), "and the box is in no cube");
}
