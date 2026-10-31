// Frozen pre-B3 picker, used only to measure the regression.
use super::*;

pub(super) fn overlaps(player: &Player, planet: &Planet, face: u8, cell: (i64, i64, i64)) -> bool {
    let f = Frame::at(planet, player.pos);
    let cell = if f.face != face {
        let centre = planet.dir(face, cell.0 as f64 + 0.5, cell.1 as f64 + 0.5) * planet.radius_at(cell.2 as f64 + 0.5);
        let fc = Frame::at(planet, centre);
        if fc.face != f.face { return false; }
        (fc.i.floor() as i64, fc.j.floor() as i64, fc.k.floor() as i64)
    } else { cell };
    let (bmin, bmax) = player.lattice_box(&f);
    let c0 = DVec3::new(cell.0 as f64, cell.1 as f64, cell.2 as f64);
    (0..3).all(|a| bmin[a] < c0[a] + 1.0 - EPS && bmax[a] > c0[a] + EPS)
}
/// March a ray through the lattice (Amanatides-Woo) for `reach_m` metres,
/// returning the first solid cell, or with `liquids` the first solid or
/// liquid cell past the eye's own (holding water: the bucket, M2b). Linear
/// in lattice space: over a few metres the lattice is straight to well under
/// a millimetre.
pub(super) fn legacy_pick(vox: &dyn Voxels, planet: &Planet, eye: DVec3, dir: DVec3, reach_m: f64, liquids: bool) -> Option<Hit> {
    let frame = Frame::at(planet, eye);
    let o = DVec3::new(frame.i, frame.j, frame.k);
    let d = frame.to_lattice(dir.normalize()); // lattice units per metre
    let mut cell = [o.x.floor() as i64, o.y.floor() as i64, o.z.floor() as i64];
    let mut step = [0i64; 3];
    let mut t_max = [f64::INFINITY; 3];
    let mut t_delta = [f64::INFINITY; 3];
    for a in 0..3 {
        if d[a] > 0.0 {
            step[a] = 1;
            t_max[a] = ((cell[a] + 1) as f64 - o[a]) / d[a];
            t_delta[a] = 1.0 / d[a];
        } else if d[a] < 0.0 {
            step[a] = -1;
            t_max[a] = (cell[a] as f64 - o[a]) / d[a];
            t_delta[a] = -1.0 / d[a];
        }
    }
    let mut t = 0.0;
    let mut normal = IVec3::ZERO;
    for step_n in 0..512 {
        let id = vox.block(frame.face, cell[0], cell[1], cell[2]);
        if vox.is_solid(id) || (liquids && step_n > 0 && vox.is_liquid(id)) {
            // The cell may lie past this face's edge (`vox.block` resolved it
            // onto the neighbour), so the block it names is that same cell
            // on its own face, never this face's clamped boundary column
            // (review 2 #6: the hit said the neighbour's material and
            // breaking changed the wrong block). Below layer 0 the lookup
            // returns the virtual bedrock floor: nothing to hit there.
            if cell[2] < 0 || cell[2] >= planet.height as i64 {
                return None;
            }
            let (bf, bi, bj) = planet.wrap_cell(frame.face, cell[0], cell[1]);
            let bp = BlockPos { face: bf, i: bi, j: bj, k: cell[2] as u32 };
            return Some(Hit { block: bp, id, face: frame.face, cell: (cell[0], cell[1], cell[2]), normal, t });
        }
        let a = if t_max[0] < t_max[1] { if t_max[0] < t_max[2] { 0 } else { 2 } } else if t_max[1] < t_max[2] { 1 } else { 2 };
        t = t_max[a];
        if t > reach_m {
            return None;
        }
        cell[a] += step[a];
        t_max[a] += t_delta[a];
        normal = IVec3::ZERO;
        normal[a] = -step[a] as i32;
    }
    None
}
