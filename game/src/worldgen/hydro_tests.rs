//! Generated water: containment (no water walls), nothing perched, rest,
//! and agreement between LODs, swept over many lakes and long river
//! stretches of the real world (needs the bake with `hydro.bin`).
//!
//! The rule checked, cell by cell at LOD 0 (see `hydro.rs`): for a water
//! cell A with level L (1..255) and each horizontal neighbour B in the same
//! layer, B is solid, or B's level (0 for air) is at least L - 1 (at rest),
//! or the drop is designated: A's and B's columns both lie in river
//! channels and B is downstream of A (same link further along, or a link
//! A's water reaches by following receivers), and B's column holds water at
//! or below that layer. The cell under a water cell is solid or full water.

use super::hydro::Hydro;
use super::*;
use crate::grid::FastMap;
use rayon::prelude::*;
use std::sync::OnceLock;

fn world_gen() -> &'static Generator {
    static G: OnceLock<Generator> = OnceLock::new();
    G.get_or_init(|| {
        // HYDRO_VSCALE=0.125 runs the water tests at 1/8 (H 4096, sea 1536).
        let g = match std::env::var("HYDRO_VSCALE").ok().and_then(|s| s.parse::<f64>().ok()) {
            Some(vs) => super::tests::generator_pub(Some(vs), 4096, 1536),
            None => super::tests::test_generator(),
        };
        assert!(g.hydro.is_some(), "the bake has no hydro.bin: run python scripts/bake_rivers_v2.py");
        g
    })
}

fn hydro() -> &'static Hydro {
    world_gen().hydro.as_deref().unwrap()
}

fn registry() -> &'static Registry {
    static R: OnceLock<Registry> = OnceLock::new();
    R.get_or_init(|| Registry::load(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/blocks.json")).unwrap())
}

type Col = (u8, u32, u32);

/// Generated chunks at one LOD around a set of columns (voxel columns of
/// that LOD), with lookups across chunk borders and face edges.
struct Region {
    lod: u8,
    chunks: FastMap<ChunkKey, Chunk>,
}

impl Region {
    /// Generate every chunk the columns (plus a one-voxel margin) need,
    /// over the heights their ground and water span (sampled), padded.
    fn build(g: &Generator, lod: u8, cols: &[Col]) -> Region {
        let p = &g.planet;
        let vb = 1u64 << lod;
        let voxel_m = p.voxel_m(lod);
        let per = (CHUNK as u64) * vb; // blocks per chunk edge
        // Chunk footprints touched, margin included.
        let mut fps: FastMap<(u8, u32, u32), ()> = FastMap::default();
        let n_vox = (p.n as u64 / vb) as i64;
        for &(f, i, j) in cols {
            for di in -1..=1i64 {
                for dj in -1..=1i64 {
                    let (ff, ii, jj) = wrap_voxel(p, lod, f, i as i64 + di, j as i64 + dj, n_vox);
                    fps.insert((ff, (ii as u64 * vb / per) as u32, (jj as u64 * vb / per) as u32), ());
                }
            }
        }
        let fps: Vec<(u8, u32, u32)> = fps.into_keys().collect();
        // Height range per footprint from a 9 x 9 sample of columns.
        let keys: Vec<ChunkKey> = fps
            .par_iter()
            .flat_map_iter(|&(f, cx, cy)| {
                let (mut lo, mut hi) = (f64::MAX, f64::MIN);
                for a in 0..17 {
                    for b in 0..17 {
                        let i = cx as f64 * per as f64 + (a as f64 + 0.5) / 17.0 * per as f64;
                        let j = cy as f64 * per as f64 + (b as f64 + 0.5) / 17.0 * per as f64;
                        let c = g.column(p.dir(f, i, j), voxel_m);
                        let h = c.height_m as f64;
                        lo = lo.min(h);
                        hi = hi.max(h);
                        if c.water_m.is_finite() {
                            lo = lo.min(c.water_m);
                            hi = hi.max(c.water_m);
                        }
                    }
                }
                let pad = 16.0 * p.block_size + 3.0 * voxel_m;
                // Below: also the tallest fall and its plunge pool, which a
                // sample can straddle.
                let pad_lo = pad + 28.0 * p.block_size;
                let k_lo = ((lo - pad_lo) / p.block_size + p.sea_level as f64).max(0.0);
                let k_hi = ((hi + pad) / p.block_size + p.sea_level as f64).min(p.height as f64 - 1.0);
                let (z0, z1) = ((k_lo as u64 / per) as u32, (k_hi as u64 / per) as u32);
                (z0..=z1).map(move |cz| ChunkKey::neisor(f, lod, cx, cy, cz))
            })
            .collect();
        let chunks: FastMap<ChunkKey, Chunk> = keys.par_iter().map(|&k| (k, g.generate(k).chunk)).collect();
        Region { lod, chunks }
    }

    /// (block id, water level) of voxel (i, j, k) of face `f` at this LOD
    /// (i, j may step one past a face edge); None if not generated.
    fn cell(&self, p: &Planet, f: u8, i: i64, j: i64, k: i64) -> Option<(BlockId, u8)> {
        if k < 0 {
            return Some((BlockId::MAX, 0)); // below the world: treat as solid
        }
        let vb = 1u64 << self.lod;
        let n_vox = (p.n as u64 / vb) as i64;
        let (f, i, j) = wrap_voxel(p, self.lod, f, i, j, n_vox);
        let key = ChunkKey::neisor(f, self.lod, i / CHUNK as u32, j / CHUNK as u32, (k as u64 / CHUNK as u64) as u32);
        let c = self.chunks.get(&key)?;
        let (x, y, z) = ((i % CHUNK as u32) as usize, (j % CHUNK as u32) as usize, (k as u64 % CHUNK as u64) as usize);
        Some((c.get(x, y, z), c.water_level(x, y, z)))
    }

    /// Voxel layer range generated for a column.
    fn k_range(&self, p: &Planet, f: u8, i: u32, j: u32) -> Option<(i64, i64)> {
        let per = CHUNK as u32;
        let (cx, cy) = (i / per, j / per);
        let (mut lo, mut hi) = (u32::MAX, 0u32);
        for cz in 0..p.chunks_per_column(self.lod) {
            if self.chunks.contains_key(&ChunkKey::neisor(f, self.lod, cx, cy, cz)) {
                lo = lo.min(cz);
                hi = hi.max(cz);
            }
        }
        (lo <= hi).then(|| (lo as i64 * CHUNK as i64, (hi as i64 + 1) * CHUNK as i64 - 1))
    }
}

/// Voxel (i, j) of face `f` at a LOD, carried across face edges.
fn wrap_voxel(p: &Planet, lod: u8, f: u8, i: i64, j: i64, n_vox: i64) -> (u8, u32, u32) {
    if (0..n_vox).contains(&i) && (0..n_vox).contains(&j) {
        return (f, i as u32, j as u32);
    }
    let vb = (1u64 << lod) as f64;
    let (ff, wi, wj) = p.wrap(f, (i as f64 + 0.5) * vb, (j as f64 + 0.5) * vb);
    let m = n_vox as f64 - 1.0;
    (ff, (wi / vb).floor().clamp(0.0, m) as u32, (wj / vb).floor().clamp(0.0, m) as u32)
}

#[derive(Default, Debug)]
struct Report {
    columns: u64,
    water_cells: u64,
    rest_pairs: u64,
    drops: u64,
    walls: Vec<String>,
    perched: Vec<String>,
    missing: u64,
}

impl Report {
    fn merge(mut self, o: Report) -> Report {
        self.columns += o.columns;
        self.water_cells += o.water_cells;
        self.rest_pairs += o.rest_pairs;
        self.drops += o.drops;
        self.missing += o.missing;
        self.walls.extend(o.walls);
        self.perched.extend(o.perched);
        self
    }
}

/// Whether a drop from column a to column b (LOD 0) is designated.
fn designated(g: &Generator, a: DVec3, b: DVec3) -> bool {
    let h = hydro();
    let ca = g.channels_at(a, g.planet.block_size);
    let cb = g.channels_at(b, g.planet.block_size);
    ca.iter().any(|&(sa, xa)| cb.iter().any(|&(sb, xb)| (sa == sb && xb > xa) || (sa != sb && h.downstream(sa, sb, 12))))
}

/// Check every water cell of the given LOD-0 columns.
fn check(g: &Generator, region: &Region, cols: &[Col]) -> Report {
    let p = &g.planet;
    let reg = registry();
    cols.par_iter()
        .map(|&(f, i, j)| {
            let mut r = Report { columns: 1, ..Default::default() };
            let Some((k0, k1)) = region.k_range(p, f, i, j) else {
                r.missing += 1;
                return r;
            };
            let solid = |id: BlockId| id == BlockId::MAX || reg.is_solid(id);
            let liquid = |id: BlockId| id != BlockId::MAX && reg.is_liquid(id);
            // A frozen surface (ice over water) counts as water below a drop.
            let ice = reg.id("ice").unwrap();
            for k in k0..=k1 {
                let Some((id, la)) = region.cell(p, f, i as i64, j as i64, k) else { continue };
                if !liquid(id) {
                    continue;
                }
                r.water_cells += 1;
                let here = || {
                    let d = p.dir(f, i as f64 + 0.5, j as f64 + 0.5);
                    let (lat, lon) = Planet::lat_lon_deg(d);
                    format!("face {f} i {i} j {j} k {k} (lat {lat:.5} lon {lon:.5})")
                };
                // Under it: solid or full water.
                match region.cell(p, f, i as i64, j as i64, k - 1) {
                    Some((b, lb)) if solid(b) || (liquid(b) && lb == 255) => {}
                    Some((b, lb)) => r.perched.push(format!("{}: level {la} over {} level {lb}", here(), if liquid(b) { "water" } else { "air" })),
                    None => r.missing += 1,
                }
                for (di, dj) in [(-1i64, 0i64), (1, 0), (0, -1), (0, 1)] {
                    let (ni, nj) = (i as i64 + di, j as i64 + dj);
                    let Some((nb, lb)) = region.cell(p, f, ni, nj, k) else {
                        r.missing += 1;
                        continue;
                    };
                    if solid(nb) {
                        continue;
                    }
                    let lb = if liquid(nb) { lb } else { 0 };
                    if lb as i32 + 1 >= la as i32 {
                        r.rest_pairs += 1;
                        continue;
                    }
                    // A drop: is B's column wet at or below this layer?
                    let mut wet_below = lb > 0;
                    let mut kk = k - 1;
                    while !wet_below && kk >= k - 64 {
                        match region.cell(p, f, ni, nj, kk) {
                            Some((c, _)) if liquid(c) || c == ice => wet_below = true,
                            Some((c, _)) if solid(c) => break,
                            Some(_) => {}
                            None => break,
                        }
                        kk -= 1;
                    }
                    let (bf, bi, bj) = p.wrap_cell(f, ni, nj);
                    let a_dir = p.dir(f, i as f64 + 0.5, j as f64 + 0.5);
                    let b_dir = p.dir(bf, bi as f64 + 0.5, bj as f64 + 0.5);
                    if wet_below && designated(g, a_dir, b_dir) {
                        r.drops += 1;
                    } else {
                        let ca = g.channels_at(a_dir, p.block_size);
                        let cb = g.channels_at(b_dir, p.block_size);
                        let wa = g.column(a_dir, p.block_size);
                        let wb = g.column(b_dir, p.block_size);
                        r.walls.push(format!(
                            "{}: level {la} next to {} level {lb} at ({di},{dj}); B wet below {wet_below}; A {:?} W {:.3} h {:.2} ch {:?}; B {:?} W {:.3} h {:.2} ch {:?}",
                            here(),
                            if liquid(nb) { "water" } else { "air" },
                            wa.water_kind,
                            wa.water_m,
                            wa.height_m,
                            ca,
                            wb.water_kind,
                            wb.water_m,
                            wb.height_m,
                            cb
                        ));
                    }
                }
            }
            r
        })
        .reduce(Report::default, Report::merge)
}

/// LOD-0 columns within `r` blocks of a lattice point.
fn square(p: &Planet, dir: DVec3, r: i64, out: &mut FastMap<Col, ()>) {
    let (f, i, j) = p.lattice_of_dir(dir);
    let (i, j) = (i.floor() as i64, j.floor() as i64);
    for di in -r..=r {
        for dj in -r..=r {
            out.insert(p.wrap_cell(f, i + di, j + dj), ());
        }
    }
}

/// A point on lake `k`'s shore (field = zone) between one of its cells and
/// the nearest field cell outside it, if any.
fn shore_point(k: i32, nth: usize) -> Option<DVec3> {
    let h = hydro();
    let inside: Vec<DVec3> = h.field.iter().filter(|c| c.lake == k).map(|c| c.pos).collect();
    if inside.is_empty() {
        return None;
    }
    // nth: 0 = first cell, 1 = the cell farthest from it.
    let a = if nth == 0 { inside[0] } else { *inside.iter().max_by(|x, y| (**x - inside[0]).length().total_cmp(&(**y - inside[0]).length()))? };
    let out = h.field.iter().filter(|c| c.lake != k).min_by(|x, y| (x.pos - a).length().total_cmp(&(y.pos - a).length()))?.pos;
    let field = |d: DVec3| {
        let c = h.cands(d, 0.0);
        let l = h.lake_at(d, &c.cells, 1.0);
        l.field_of(k)
    };
    let (mut lo, mut hi) = (0.0f64, 1.0f64); // lo inside, hi outside
    if field(a) <= h.lake_zone || field(out) > h.lake_zone {
        return None;
    }
    for _ in 0..40 {
        let m = 0.5 * (lo + hi);
        if field((a * (1.0 - m) + out * m).normalize()) > h.lake_zone {
            lo = m;
        } else {
            hi = m;
        }
    }
    Some((a * (1.0 - lo) + out * lo).normalize())
}

/// Columns along `links` river links: everything within the channel, its
/// banks and a margin of the course.
fn along(links: &[u32], out: &mut FastMap<Col, ()>) {
    let g = world_gen();
    let h = hydro();
    let p = &g.planet;
    for &k in links {
        let s = &h.segs[k as usize];
        let r = (s.hw + 2.0 * p.block_size + 8.0).ceil() as i64;
        let steps = (s.len / 6.0).ceil() as usize;
        for t in 0..=steps {
            let x = s.len * t as f64 / steps as f64;
            let i = s.cum.partition_point(|&c| c <= x).clamp(1, s.pts.len() - 1);
            let (a, b) = (s.pts[i - 1], s.pts[i]);
            let u = ((x - s.cum[i - 1]) / (s.cum[i] - s.cum[i - 1]).max(1e-9)).clamp(0.0, 1.0);
            square(p, (a * (1.0 - u) + b * u).normalize(), r, out);
        }
    }
}

/// A river path: `n` links downstream from link `k`.
fn path(k: u32, n: usize) -> Vec<u32> {
    let h = hydro();
    let mut v = vec![k];
    let mut cur = k;
    for _ in 1..n {
        let next = h.node_seg[h.segs[cur as usize].b as usize];
        if next < 0 {
            break;
        }
        cur = next as u32;
        v.push(cur);
    }
    v
}

/// The sites of the sweep: lake shores and river stretches (mouths, falls,
/// lake inflows and outflows, face-edge crossings, the largest rivers, and
/// a seeded random sample).
fn sweep_sites() -> (Vec<Col>, Vec<Col>, String) {
    let g = world_gen();
    let h = hydro();
    let p = &g.planet;
    let mut lakes: FastMap<Col, ()> = FastMap::default();
    let mut n_shore = 0;
    let shore_pts: Vec<DVec3> = (0..h.lakes.len() as i32).into_par_iter().flat_map_iter(|k| (0..2).filter_map(move |n| shore_point(k, n))).collect();
    for d in &shore_pts {
        square(p, *d, 24, &mut lakes);
        n_shore += 1;
    }
    let mut rivers: FastMap<Col, ()> = FastMap::default();
    let mut links: Vec<u32> = Vec::new();
    let ns = h.segs.len() as u32;
    // The largest rivers, four links each, ending wherever they go.
    let mut by_flow: Vec<u32> = (0..ns).collect();
    by_flow.sort_by(|a, b| h.segs[*b as usize].flow.total_cmp(&h.segs[*a as usize].flow).then(a.cmp(b)));
    for &k in by_flow.iter().take(12) {
        links.extend(path(k, 4));
    }
    // Mouths: the last three links before the sea.
    let mouths: Vec<u32> = (0..ns).filter(|&k| h.node_kind[h.segs[k as usize].b as usize] == 1).collect();
    for &k in mouths.iter().step_by((mouths.len() / 25).max(1)) {
        // back up two links along the main stem
        let mut start = k;
        for _ in 0..2 {
            if let Some(up) = (0..ns).filter(|&u| h.segs[u as usize].b == h.segs[start as usize].a).max_by(|a, b| h.segs[*a as usize].flow.total_cmp(&h.segs[*b as usize].flow)) {
                start = up;
            }
        }
        links.extend(path(start, 3));
    }
    // Steepest links (falls and cascades at this scale), with neighbours.
    let mut steep: Vec<u32> = (0..ns).collect();
    steep.sort_by(|a, b| {
        let (sa, sb) = (&h.segs[*a as usize], &h.segs[*b as usize]);
        (sb.w_a - sb.w_b).total_cmp(&(sa.w_a - sa.w_b)).then(a.cmp(b))
    });
    for &k in steep.iter().take(30) {
        links.extend(path(k, 2));
    }
    // Lake inflows and outflows.
    let inflows: Vec<u32> = (0..ns).filter(|&k| h.segs[k as usize].lake_b >= 0 && h.segs[k as usize].lake_a < 0).collect();
    let outflows: Vec<u32> = (0..ns).filter(|&k| h.segs[k as usize].lake_a >= 0 && h.segs[k as usize].lake_b < 0).collect();
    for &k in inflows.iter().step_by((inflows.len() / 20).max(1)).chain(outflows.iter().step_by((outflows.len() / 20).max(1))) {
        links.extend(path(k, 2));
    }
    // Face-edge crossings.
    let crossing: Vec<u32> = (0..ns)
        .filter(|&k| {
            let s = &h.segs[k as usize];
            p.lattice_of_dir(s.pts[0]).0 != p.lattice_of_dir(*s.pts.last().unwrap()).0
        })
        .collect();
    for &k in crossing.iter().step_by((crossing.len() / 12).max(1)) {
        links.push(k);
    }
    // A seeded random sample of long stretches.
    for t in 0..16 {
        let k = (noise::hash01(t, 5, 1, 0xF10) * ns as f64) as u32;
        links.extend(path(k.min(ns - 1), 6));
    }
    links.sort_unstable();
    links.dedup();
    along(&links, &mut rivers);
    let length_km: f64 = links.iter().map(|&k| h.segs[k as usize].len).sum::<f64>() / 1000.0;
    let note = format!(
        "{} lake shore windows ({} lakes), {} river links ({:.1} km of course: {} mouths, {} inflows, {} outflows, {} face-edge crossings in the network)",
        n_shore,
        h.lakes.len(),
        links.len(),
        length_km,
        mouths.len(),
        inflows.len(),
        outflows.len(),
        crossing.len()
    );
    let mut l: Vec<Col> = lakes.into_keys().collect();
    let mut r: Vec<Col> = rivers.into_keys().collect();
    l.sort_unstable();
    r.sort_unstable();
    (l, r, note)
}

fn report(name: &str, rep: &Report) {
    println!(
        "{name}: {} columns, {} water cells, {} neighbour pairs at rest, {} designated drops, {} walls, {} perched, {} cells not generated",
        rep.columns,
        rep.water_cells,
        rep.rest_pairs,
        rep.drops,
        rep.walls.len(),
        rep.perched.len(),
        rep.missing
    );
    for w in rep.walls.iter().take(15) {
        println!("  wall: {w}");
    }
    for w in rep.perched.iter().take(10) {
        println!("  perched: {w}");
    }
}

/// The gate: no water walls, nothing perched, everywhere the sweep reaches.
#[test]
fn water_is_contained() {
    let g = world_gen();
    let t0 = std::time::Instant::now();
    let (lakes, rivers, note) = sweep_sites();
    println!("sweep: {note}; sites found in {:.1} s", t0.elapsed().as_secs_f64());
    let mut all = Vec::new();
    for (name, cols) in [("lake shores", &lakes), ("river stretches", &rivers)] {
        let t = std::time::Instant::now();
        let region = Region::build(g, 0, cols);
        let built = t.elapsed().as_secs_f64();
        let rep = check(g, &region, cols);
        println!("{name}: {} chunks generated in {built:.1} s, checked in {:.1} s", region.chunks.len(), t.elapsed().as_secs_f64() - built);
        report(name, &rep);
        all.push((name, rep));
    }
    for (name, rep) in &all {
        assert!(rep.water_cells > 10_000, "{name}: the sweep found almost no water ({})", rep.water_cells);
        assert_eq!(rep.missing, 0, "{name}: cells outside the generated region");
        assert!(rep.walls.is_empty(), "{name}: {} water walls, e.g. {}", rep.walls.len(), rep.walls[0]);
        assert!(rep.perched.is_empty(), "{name}: {} perched water cells, e.g. {}", rep.perched.len(), rep.perched[0]);
    }
}

/// Water surface and depth of a generated column (m): the top of its
/// highest water cell (partial levels counted at LOD 0; a frozen surface,
/// ice over water or ice alone where the water lay, counts as water) and
/// how far down the water and ice reach; None if dry.
fn surface_of(region: &Region, p: &Planet, reg: &Registry, f: u8, i: u32, j: u32) -> Option<(f64, f64)> {
    let (k0, k1) = region.k_range(p, f, i, j)?;
    let v = (1u64 << region.lod) as f64;
    let ice = reg.id("ice").unwrap();
    let wet = |id: BlockId| id != BlockId::MAX && (reg.is_liquid(id) || id == ice);
    for k in (k0..=k1).rev() {
        let (id, l) = region.cell(p, f, i as i64, j as i64, k)?;
        if wet(id) {
            let frac = if region.lod == 0 && reg.is_liquid(id) { l as f64 / 255.0 } else { 1.0 };
            let top = ((k as f64 + frac) * v - p.sea_level as f64) * p.block_size;
            let mut kb = k;
            while kb > k0 && region.cell(p, f, i as i64, j as i64, kb - 1).is_some_and(|(c, _)| wet(c)) {
                kb -= 1;
            }
            let bottom = (kb as f64 * v - p.sea_level as f64) * p.block_size;
            return Some((top, top - bottom));
        }
    }
    None
}

/// Coarse LODs agree with LOD 0 about the water: where a coarse voxel
/// column holds water, its surface is within half a voxel (plus a block) of
/// the LOD-0 surfaces under it; where most of the LOD-0 columns under a
/// coarse column hold water deeper than a voxel, so does the coarse one;
/// and a coarse wet column always has wet LOD-0 columns within a voxel.
/// Lake shores and river stretches, LODs 1-3.
#[test]
fn water_agrees_across_lods() {
    let g = world_gen();
    let h = hydro();
    let p = &g.planet;
    let reg = registry();
    let mut fine: FastMap<Col, ()> = FastMap::default();
    for k in (0..h.lakes.len() as i32).step_by(5) {
        if let Some(d) = shore_point(k, 0) {
            square(p, d, 40, &mut fine);
        }
    }
    let ns = h.segs.len() as u32;
    let links: Vec<u32> = (0..40).map(|t| (noise::hash01(t, 7, 7, 0x10D) * ns as f64) as u32 % ns).collect();
    along(&links, &mut fine);
    let fine: Vec<Col> = fine.into_keys().collect();
    let r0 = Region::build(g, 0, &fine);
    // LOD-0 surfaces of every generated column, once.
    let mut all0: Vec<Col> = Vec::new();
    for k in r0.chunks.keys() {
        if k.cz == r0.chunks.keys().filter(|o| o.face == k.face && o.cx == k.cx && o.cy == k.cy).map(|o| o.cz).min().unwrap() {
            for a in 0..CHUNK as u32 {
                for b in 0..CHUNK as u32 {
                    all0.push((k.face, k.cx * CHUNK as u32 + a, k.cy * CHUNK as u32 + b));
                }
            }
        }
    }
    let surf0: FastMap<Col, Option<(f64, f64)>> = all0.par_iter().map(|&(f, i, j)| ((f, i, j), surface_of(&r0, p, reg, f, i, j))).collect();
    let mut lines = Vec::new();
    for lod in 1..=3u8 {
        let v = 1u32 << lod;
        let mut coarse: FastMap<Col, ()> = FastMap::default();
        for &(f, i, j) in &fine {
            coarse.insert((f, i / v, j / v), ());
        }
        let coarse: Vec<Col> = coarse.into_keys().collect();
        let rc = Region::build(g, lod, &coarse);
        let (mut wet, mut surf_bad, mut miss_bad, mut orphan, mut deep_cols, mut checked) = (0u64, Vec::new(), Vec::new(), Vec::new(), 0u64, 0u64);
        for &(f, ci, cj) in &coarse {
            // The LOD-0 columns under this coarse column, and a voxel around them.
            let mut under = Vec::new();
            let mut around = Vec::new();
            let mut complete = true;
            for a in -(v as i64)..(2 * v as i64) {
                for b in -(v as i64)..(2 * v as i64) {
                    let (ff, ii, jj) = p.wrap_cell(f, (ci * v) as i64 + a, (cj * v) as i64 + b);
                    let Some(&s) = surf0.get(&(ff, ii, jj)) else {
                        complete = false;
                        continue;
                    };
                    if (0..v as i64).contains(&a) && (0..v as i64).contains(&b) {
                        under.push((s, ff, ii, jj));
                    }
                    around.push(s);
                }
            }
            if !complete || under.len() < (v * v) as usize {
                continue;
            }
            checked += 1;
            let c = surface_of(&rc, p, reg, f, ci, cj).map(|x| x.0);
            let fine_wet: Vec<f64> = under.iter().filter_map(|u| u.0.map(|x| x.0)).collect();
            let vm = p.voxel_m(lod);
            let here = || {
                let (lat, lon) = Planet::lat_lon_deg(p.dir(f, ((ci * v) as f64 + 0.5 * v as f64) * 1.0, ((cj * v) as f64 + 0.5 * v as f64) * 1.0));
                format!("LOD {lod} face {f} ({ci}, {cj}) lat {lat:.5} lon {lon:.5}")
            };
            if let Some(cs) = c {
                wet += 1;
                let near: Vec<f64> = around.iter().filter_map(|s| s.map(|x| x.0)).collect();
                if near.is_empty() {
                    orphan.push(format!("{}: coarse water at {cs:.2} m with no LOD-0 water within a voxel", here()));
                } else if !fine_wet.is_empty() {
                    // The coarse surface is one LOD 0 has within a voxel (a
                    // fall's step may sit a voxel apart between LODs), to
                    // half a voxel plus a block.
                    let (lo, hi) = near.iter().fold((f64::MAX, f64::MIN), |a, &x| (a.0.min(x), a.1.max(x)));
                    let tol = 0.5 * vm + p.block_size + 1e-6;
                    if cs < lo - tol || cs > hi + tol {
                        let mean = fine_wet.iter().sum::<f64>() / fine_wet.len() as f64;
                        surf_bad.push(format!("{}: coarse surface {cs:.2} m, LOD-0 {lo:.2}..{hi:.2} m within a voxel (mean under it {mean:.2} m)", here()));
                    }
                }
            }
            // Every LOD-0 column under it holds water at least three quarters
            // of a voxel deep: the coarse column must hold water too.
            let deep = under.iter().filter(|u| u.0.is_some_and(|x| x.1 >= 0.75 * vm)).count();
            if deep == under.len() {
                deep_cols += 1;
                if c.is_none() {
                    miss_bad.push(format!("{}: all {} LOD-0 columns hold water over 3/4 of a voxel deep, coarse dry", here(), under.len()));
                }
            }
        }
        let line = format!(
            "LOD {lod}: {checked} coarse columns checked ({} chunks), {wet} wet, {deep_cols} over deep LOD-0 water; {} surface disagreements, {} deep water missing, {} coarse water with no LOD-0 water near",
            rc.chunks.len(),
            surf_bad.len(),
            miss_bad.len(),
            orphan.len()
        );
        println!("{line}");
        for e in surf_bad.iter().chain(&miss_bad).chain(&orphan).take(8) {
            println!("  {e}");
        }
        assert!(wet > 1000, "LOD {lod}: almost no coarse water ({wet})");
        lines.push((lod, surf_bad.len(), miss_bad.len(), orphan.len()));
    }
    for (lod, s, m, o) in lines {
        assert_eq!((s, m, o), (0, 0, 0), "LOD {lod}: coarse water disagrees with LOD 0 (surface, missing, orphan)");
    }
}

/// Not a check: generation cost per chunk before and after the hydrology,
/// on the same chunk keys in the same binary ("before" = the generator with
/// no hydro.bin, i.e. the old dry valleys along the discharge field).
/// Single-threaded, median and mean over each set. Run in release with
/// --ignored --nocapture; timing wants the machine to itself.
#[test]
#[ignore]
fn profile_hydro_chunks() {
    use std::hint::black_box;
    use std::time::Instant;
    let g = world_gen();
    let h = hydro();
    let p = &g.planet;
    let mut before = super::tests::test_generator();
    before.hydro = None;
    before.lf.valleys = true;
    // Surface chunk containing a direction at a LOD.
    let key_at = |d: DVec3, lod: u8| {
        let (f, i, j) = p.lattice_of_dir(d);
        let e = (CHUNK as f64) * (1u64 << lod) as f64;
        let c = g.column(d, p.voxel_m(lod));
        let top = (c.height_m as f64).max(if c.water_m.is_finite() { c.water_m } else { f64::MIN });
        let k = p.sea_level as f64 + top / p.block_size;
        ChunkKey::neisor(f, lod, (i / e) as u32, (j / e) as u32, (k / e).max(0.0) as u32)
    };
    let ns = h.segs.len() as u32;
    let rivers: Vec<DVec3> = (0..200).map(|t| {
        let k = (noise::hash01(t, 9, 2, 0xC05) * ns as f64) as u32 % ns;
        let s = &h.segs[k as usize];
        s.point(s.len * noise::hash01(t, 9, 3, 0xC05))
    }).collect();
    let lakes: Vec<DVec3> = (0..h.lakes.len() as i32).step_by((h.lakes.len() / 200).max(1)).filter_map(|k| shore_point(k, 0)).collect();
    let land: Vec<DVec3> = super::tests::dirs_pub(4000, 31).into_iter().filter(|d| g.bake.sample(*d).ocean < 0.5).take(200).collect();
    let hills = crate::engine::gnomonic_dir(crate::engine::HILLS.0, crate::engine::HILLS.1, crate::engine::HILLS.2);
    let time = |gn: &Generator, keys: &[ChunkKey]| -> (f64, f64) {
        for k in keys.iter().take(5) {
            black_box(gn.generate(*k));
        }
        let mut v: Vec<f64> = keys
            .iter()
            .map(|k| {
                let t = Instant::now();
                black_box(gn.generate(*k));
                t.elapsed().as_secs_f64() * 1e3
            })
            .collect();
        v.sort_by(f64::total_cmp);
        (v[v.len() / 2], v.iter().sum::<f64>() / v.len() as f64)
    };
    println!("generation per chunk, ms (median / mean), single thread; before = no hydrology (dry valleys), after = lakes and rivers");
    for (name, dirs, lods) in [("random land", &land, vec![0u8, 2, 4]), ("river courses", &rivers, vec![0, 2, 4]), ("lake shores", &lakes, vec![0, 2, 4]), ("hills anchor", &vec![hills; 20], vec![0, 1, 2, 3, 4, 5])] {
        for &lod in &lods {
            let keys: Vec<ChunkKey> = dirs.iter().map(|d| key_at(*d, lod)).collect();
            let (bm, ba) = time(&before, &keys);
            let (am, aa) = time(g, &keys);
            println!("  {name:14} LOD {lod}: before {bm:6.2} / {ba:6.2}   after {am:6.2} / {aa:6.2}   ({} chunks)", keys.len());
        }
    }
}

/// Not a check: where the hydrology's per-column time goes on river chunks.
#[test]
#[ignore]
fn profile_hydro_parts() {
    use std::hint::black_box;
    use std::time::Instant;
    let g = world_gen();
    let h = hydro();
    let p = &g.planet;
    let mut before = super::tests::test_generator();
    before.hydro = None;
    before.lf.valleys = true;
    let ns = h.segs.len() as u32;
    let mut dirs = Vec::new();
    for t in 0..60 {
        let k = (noise::hash01(t, 9, 2, 0xC05) * ns as f64) as u32 % ns;
        let s = &h.segs[k as usize];
        let c = s.point(s.len * noise::hash01(t, 9, 3, 0xC05));
        let (f, i, j) = p.lattice_of_dir(c);
        for a in 0..32 {
            for b in 0..32 {
                dirs.push(p.dir(f, (i / 32.0).floor() * 32.0 + a as f64 + 0.5, (j / 32.0).floor() * 32.0 + b as f64 + 0.5));
            }
        }
    }
    let n = dirs.len() as f64;
    let t = Instant::now();
    for d in &dirs {
        black_box(before.column(*d, 1.0));
    }
    let col_before = t.elapsed().as_nanos() as f64 / n;
    let t = Instant::now();
    for d in &dirs {
        black_box(g.column(*d, 1.0));
    }
    let col_after = t.elapsed().as_nanos() as f64 / n;
    // Per-chunk candidates (as chunk_columns finds them), then the parts.
    let cands: Vec<Cands> = (0..60).map(|c| h.cands(dirs[c * 1024 + 528], 27.0)).collect();
    let t = Instant::now();
    for c in 0..60 {
        black_box(h.cands(dirs[c * 1024 + 528], 27.0));
    }
    let cands_us = t.elapsed().as_micros() as f64 / 60.0;
    let mut rivers = Vec::new();
    let t = Instant::now();
    for (i, d) in dirs.iter().enumerate() {
        h.rivers_at(*d, &cands[i / 1024], 1.0, &mut rivers);
        black_box(&rivers);
    }
    let rivers_ns = t.elapsed().as_nanos() as f64 / n;
    let t = Instant::now();
    for (i, d) in dirs.iter().enumerate() {
        black_box(h.lake_at(*d, &cands[i / 1024].cells, 1.0));
    }
    let lake_ns = t.elapsed().as_nanos() as f64 / n;
    let mut total_r = 0usize;
    let t = Instant::now();
    for (i, d) in dirs.iter().enumerate() {
        h.rivers_at(*d, &cands[i / 1024], 1.0, &mut rivers);
        total_r += rivers.len();
        let l = h.lake_at(*d, &cands[i / 1024].cells, 1.0);
        black_box(h.shape(*d, 5.0, &l, &rivers, 0.0, 0.5, 1.0, 0));
    }
    let shape_all_ns = t.elapsed().as_nanos() as f64 / n;
    let avg_segs = cands.iter().map(|c| c.segs.len()).sum::<usize>() as f64 / 60.0;
    let avg_cells = cands.iter().map(|c| c.cells.len()).sum::<usize>() as f64 / 60.0;
    println!("river chunks (LOD 0): column before {col_before:.0} ns, after {col_after:.0} ns; cands {cands_us:.1} us per chunk ({avg_segs:.1} links, {avg_cells:.1} field cells); rivers_at {rivers_ns:.0} ns, lake_at {lake_ns:.0} ns, rivers+lake+shape {shape_all_ns:.0} ns per column; {:.2} links in reach per column", total_r as f64 / n);
    let keys: Vec<ChunkKey> = (0..60)
        .map(|c| {
            let d = dirs[c * 1024 + 528];
            let (f, i, j) = p.lattice_of_dir(d);
            let col = g.column(d, 1.0);
            let k = p.sea_level as f64 + (col.height_m as f64) / p.block_size;
            ChunkKey::neisor(f, 0, (i / 32.0) as u32, (j / 32.0) as u32, (k / 32.0) as u32)
        })
        .collect();
    for (name, gn) in [("before", &before), ("after", g)] {
        let t = Instant::now();
        for k in &keys {
            black_box(gn.chunk_columns(*k));
        }
        let cols_us = t.elapsed().as_micros() as f64 / 60.0;
        let t = Instant::now();
        for k in &keys {
            black_box(gn.generate(*k));
        }
        let gen_us = t.elapsed().as_micros() as f64 / 60.0;
        println!("  {name}: chunk_columns {cols_us:.0} us, generate {gen_us:.0} us");
    }
}

/// Not a check: find evidence sites (a lake shore, a river reach, the
/// tallest waterfall, the largest river mouth, a river crossing a cube-face
/// edge) and write `out/hydro-shots.play`, which shoots each from the
/// ground and from 300 m.
#[test]
#[ignore]
fn hydro_places() {
    let g = world_gen();
    let h = hydro();
    let p = &g.planet;
    let r = p.radius;
    let ns = h.segs.len() as u32;
    let heading = |at: DVec3, v: DVec3| {
        let (north, east, _) = crate::camera::Camera::local_frame(at);
        v.dot(east).atan2(v.dot(north)).to_degrees()
    };
    // Move `d` metres from `at` along tangent `v`.
    let step = |at: DVec3, v: DVec3, d: f64| (at * r + v * d).normalize();
    // One script per site (so each run can start there and be lit by day).
    let scripts: std::cell::RefCell<Vec<(String, String, f64, f64)>> = std::cell::RefCell::new(Vec::new());
    let shot = |name: &str, eye: DVec3, alt: f64, head: f64, pitch: f64, note: &str| {
        let (lat, lon) = Planet::lat_lon_deg(eye);
        let site = name.trim_end_matches("_ground").trim_end_matches("_300m").to_string();
        let text = format!("# {note}\nat {lat:.6} {lon:.6} {alt:.2}\nlook {head:.2} {pitch:.1}\nshot {name}\n");
        let mut v = scripts.borrow_mut();
        match v.iter_mut().find(|e| e.0 == site) {
            Some(e) => e.1 += &text,
            None => v.push((site, format!("# M2-hydro evidence, written by `cargo test --release hydro_places -- --ignored`.\nmode fly\n{text}"), lat, lon)),
        }
        println!("{name}: lat {lat:.5} lon {lon:.5} alt {alt:.1} heading {head:.1} pitch {pitch} ({note})");
    };
    let tangent = |k: u32, s: f64| {
        let seg = &h.segs[k as usize];
        let a = seg.point((s - 2.0).max(0.0));
        let b = seg.point((s + 2.0).min(seg.len));
        let t = b - a;
        let at = seg.point(s);
        (t - at * t.dot(at)).normalize()
    };
    let surface = |d: DVec3| g.ground_height_m(d);
    // Pair of shots looking along `v` at `at`: ground (from `back` metres
    // behind, `side` to the side) and 300 m (from 350 m behind).
    let pair = |name: &str, at: DVec3, v: DVec3, back: f64, side: f64, pitch: f64, note: &str| {
        let up = at;
        let lateral = v.cross(up).normalize();
        // Stand on dry ground: step further sideways until the column is dry.
        let mut eye = step(step(at, v, -back), lateral, side);
        for t in 0..20 {
            if !g.column(eye, p.block_size).wet() {
                break;
            }
            eye = step(step(at, v, -back), lateral, side + 4.0 * (t + 1) as f64);
        }
        let alt = surface(eye).max(surface(at)) + 2.5;
        let look = (at * r - eye * r).normalize();
        shot(&format!("{name}_ground"), eye, alt, heading(eye, look), pitch, note);
        let eye3 = step(at, v, -350.0);
        shot(&format!("{name}_300m"), eye3, surface(eye3).max(surface(at)) + 300.0, heading(eye3, (at * r - eye3 * r).normalize()), -38.0, note);
    };
    // 1. Lake shore: the largest lake's shore, looking across the water.
    let mut by_size: Vec<(usize, i32)> = (0..h.lakes.len() as i32).map(|k| (h.field.iter().filter(|c| c.lake == k).count(), k)).collect();
    by_size.sort_by(|a, b| b.cmp(a));
    let lake = by_size[0].1;
    let shore = shore_point(lake, 1).or_else(|| shore_point(lake, 0)).unwrap();
    let inside = h.field.iter().filter(|c| c.lake == lake).min_by(|a, b| (a.pos - shore).length().total_cmp(&(b.pos - shore).length())).unwrap().pos;
    let v = {
        let t = inside - shore;
        (t - shore * t.dot(shore)).normalize()
    };
    pair("lake_shore", shore, v, 30.0, 0.0, -6.0, &format!("lake {lake} ({} field cells), level {:.2} m", by_size[0].0, h.lakes[lake as usize].w));
    // 2. River reach: the largest river away from lakes and the sea, mid-link.
    let mut by_flow: Vec<u32> = (0..ns).filter(|&k| {
        let s = &h.segs[k as usize];
        s.lake_a < 0 && s.lake_b < 0 && h.node_kind[s.b as usize] == 0 && h.node_kind[s.a as usize] == 0 && h.profile(k, p.block_size).steps == 0
    }).collect();
    by_flow.sort_by(|a, b| h.segs[*b as usize].flow.total_cmp(&h.segs[*a as usize].flow));
    let k = by_flow[0];
    let s = &h.segs[k as usize];
    let at = s.point(0.5 * s.len);
    pair("river_reach", at, tangent(k, 0.5 * s.len), 45.0, s.hw + 6.0, -8.0, &format!("link {k}, {:.0} m3/s, half-width {:.1} m, W {:.2}", s.flow, s.hw, h.w_water(k, &h.profile(k, p.block_size), 0.5 * s.len)));
    // 3. The tallest waterfall at this scale: look upstream at it from below.
    let falls = (0..ns).filter_map(|k| {
        let pr = h.profile(k, p.block_size);
        (pr.falls && pr.steps > 0).then_some((pr.delta, k, pr))
    });
    let (delta, k, pr) = falls.max_by(|a, b| a.0.total_cmp(&b.0)).expect("no waterfalls at this scale");
    // The first fall's water step, by bisection around its unjittered place.
    let s_mid = pr.s0 + pr.r_a + pr.lm * 0.5 / pr.steps as f64;
    let (mut lo, mut hi) = (s_mid - 0.2 * pr.lm / pr.steps as f64, s_mid + 0.2 * pr.lm / pr.steps as f64);
    let w_top = h.w_water(k, &pr, lo);
    for _ in 0..50 {
        let m = 0.5 * (lo + hi);
        if h.w_water(k, &pr, m) > w_top - 0.5 * delta { lo = m } else { hi = m }
    }
    let s_f = lo;
    let at = h.segs[k as usize].point(s_f);
    let v = -tangent(k, s_f);
    let below = h.segs[k as usize].point((s_f + 30.0).min(h.segs[k as usize].len));
    let lateral = v.cross(at).normalize();
    let eye = step(below, lateral, h.segs[k as usize].hw * 0.3);
    let alt = surface(eye) + 2.5;
    shot("waterfall_ground", eye, alt, heading(eye, (at * r - eye * r).normalize()), 6.0, &format!("link {k}, fall {delta:.1} m of {} steps, flow {:.0} m3/s", pr.steps, h.segs[k as usize].flow));
    let eye3 = step(at, v, -350.0);
    shot("waterfall_300m", eye3, surface(at) + 300.0, heading(eye3, (at * r - eye3 * r).normalize()), -38.0, "the same fall from 300 m");
    // From the side, above the lip, looking across at the fall and its gorge.
    let _lip = h.segs[k as usize].point(s_f + 12.0);
    let eye_s = step(step(at, v, -15.0), lateral, 40.0);
    shot("waterfall_side", eye_s, surface(at).max(surface(eye_s)) + 6.0, heading(eye_s, (at * r - eye_s * r).normalize()), -12.0, "the same fall from the side, above the lip");
    // 4. The largest river mouth: from upstream, looking out to sea.
    let mut mouths: Vec<u32> = (0..ns).filter(|&k| h.node_kind[h.segs[k as usize].b as usize] == 1).collect();
    mouths.sort_by(|a, b| h.segs[*b as usize].flow.total_cmp(&h.segs[*a as usize].flow));
    let k = mouths[0];
    let s = &h.segs[k as usize];
    // The coast: where the column becomes sea along the link.
    let coast = (0..200).map(|t| s.len * t as f64 / 200.0).find(|&x| g.column(s.point(x), p.block_size).ocean).unwrap_or(0.5 * s.len);
    pair("river_mouth", s.point((coast - 100.0).max(0.0)), tangent(k, (coast - 100.0).max(0.0)), 150.0, s.hw + 6.0, -3.0, &format!("link {k}, {:.0} m3/s, looking down the estuary to the coast", s.flow));
    // 5. A river crossing a cube-face edge (the largest that does).
    let mut crossing: Vec<u32> = (0..ns).filter(|&k| {
        let s = &h.segs[k as usize];
        p.lattice_of_dir(s.pts[0]).0 != p.lattice_of_dir(*s.pts.last().unwrap()).0 && s.lake_a < 0 && s.lake_b < 0
    }).collect();
    crossing.sort_by(|a, b| h.segs[*b as usize].flow.total_cmp(&h.segs[*a as usize].flow));
    if let Some(&k) = crossing.first() {
        let s = &h.segs[k as usize];
        let f0 = p.lattice_of_dir(s.pts[0]).0;
        let (mut lo, mut hi) = (0.0, s.len);
        for _ in 0..40 {
            let m = 0.5 * (lo + hi);
            if p.lattice_of_dir(s.point(m)).0 == f0 { lo = m } else { hi = m }
        }
        pair("face_edge", s.point(lo), tangent(k, lo), 40.0, s.hw + 6.0, -8.0, &format!("link {k}, {:.0} m3/s, faces {} -> {}", s.flow, f0, p.lattice_of_dir(*s.pts.last().unwrap()).0));
    }
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/out/hydro-shots");
    std::fs::create_dir_all(dir).unwrap();
    let mut list = String::new();
    for (site, text, lat, lon) in scripts.into_inner() {
        std::fs::write(format!("{dir}/{site}.play"), text).unwrap();
        list += &format!("{site} {lat:.6} {lon:.6}\n");
    }
    std::fs::write(format!("{dir}/sites.txt"), &list).unwrap();
    println!("wrote {dir}/*.play and sites.txt:\n{list}");
}

/// Not a check: columns inside lake LAKE (env, default the largest) where
/// something other than the lake sets the surface, and the ground's shape.
#[test]
#[ignore]
fn hydro_lake_survey() {
    let g = world_gen();
    let h = hydro();
    let p = &g.planet;
    let lake: i32 = std::env::var("LAKE").ok().and_then(|s| s.parse().ok()).unwrap_or_else(|| {
        (0..h.lakes.len() as i32).max_by_key(|&k| h.field.iter().filter(|c| c.lake == k).count()).unwrap()
    });
    let cells: Vec<DVec3> = h.field.iter().filter(|c| c.lake == lake).map(|c| c.pos).collect();
    let wl = h.lakes[lake as usize].w;
    let (mut n, mut river_wins, mut higher, mut bumps) = (0, 0, 0, 0);
    let mut examples = Vec::new();
    for (ci, c) in cells.iter().enumerate().step_by(3) {
        let (f, i, j) = p.lattice_of_dir(*c);
        for a in -40..40 {
            for b in -40..40 {
                let d = p.dir(f, i + 4.0 * a as f64, j + 4.0 * b as f64);
                let cands = h.cands(d, 0.0);
                let vox: f64 = std::env::var("VOXEL").ok().and_then(|s| s.parse().ok()).unwrap_or(1.0);
                let l = h.lake_at(d, &cands.cells, vox);
                if l.field_of(lake) <= h.lake_zone {
                    continue;
                }
                n += 1;
                let col = g.column(d, vox);
                if col.water_kind == WaterKind::River {
                    river_wins += 1;
                }
                if col.water_m > wl + 1.0 / 255.0 {
                    higher += 1;
                    if examples.len() < 6 {
                        let (lat, lon) = Planet::lat_lon_deg(d);
                        examples.push(format!("lat {lat:.5} lon {lon:.5}: W {:.3} vs lake {wl:.3}, kind {:?}, channels {:?}", col.water_m, col.water_kind, g.channels_at(d, 1.0)));
                    }
                }
                if col.height_m as f64 > wl - 0.25 {
                    bumps += 1;
                }
            }
        }
        if ci > 300 {
            break;
        }
    }
    println!("lake {lake} (level {wl:.3}): {n} zone columns sampled; river sets the surface in {river_wins}, surface above the lake in {higher}, ground within 0.25 m of the surface in {bumps}");
    // Along the lake's own links: ground on the course vs 25 m beside it.
    let vox: f64 = std::env::var("VOXEL").ok().and_then(|s| s.parse().ok()).unwrap_or(1.0);
    let mut shown = 0;
    for (k, s) in h.segs.iter().enumerate().filter(|(_, s)| s.lake_a == lake && s.lake_b == lake).take(3) {
        let (lat, lon) = Planet::lat_lon_deg(s.point(0.5 * s.len));
        println!("  internal link {k}: midpoint lat {lat:.5} lon {lon:.5}, len {:.0}", s.len);
    }
    for (k, s) in h.segs.iter().enumerate() {
        if s.lake_a != lake && s.lake_b != lake {
            continue;
        }
        for t in 1..8 {
            let x = s.len * t as f64 / 8.0;
            let on = s.point(x);
            let a = s.point((x - 2.0).max(0.0));
            let b = s.point((x + 2.0).min(s.len));
            let side = (b - a).cross(on).normalize();
            let off = (on * p.radius + side * 25.0).normalize();
            let (c0, c1) = (g.column(on, vox), g.column(off, vox));
            if shown < 12 && (c0.height_m - c1.height_m).abs() > 0.4 {
                let (lat, lon) = Planet::lat_lon_deg(on);
                println!("  link {k} ({} -> {}) s {x:.0}: on course h {:.2} W {:.3} {:?}; 25 m aside h {:.2} W {:.3} {:?}  (lat {lat:.5} lon {lon:.5})", s.lake_a, s.lake_b, c0.height_m, c0.water_m, c0.water_kind, c1.height_m, c1.water_m, c1.water_kind);
                shown += 1;
            }
        }
    }
    for e in examples {
        println!("  {e}");
    }
}

/// The gate over far more of the river network (about a tenth of all
/// links, every link with a fall at this scale and every face-edge
/// crossing): too slow for every run (minutes), run it before merging.
#[test]
#[ignore]
fn water_is_contained_wide() {
    let g = world_gen();
    let h = hydro();
    let p = &g.planet;
    let ns = h.segs.len() as u32;
    let mut links: Vec<u32> = (0..ns).step_by(10).collect();
    links.extend((0..ns).filter(|&k| {
        let pr = h.profile(k, p.block_size);
        let s = &h.segs[k as usize];
        (pr.falls && pr.steps > 0) || p.lattice_of_dir(s.pts[0]).0 != p.lattice_of_dir(*s.pts.last().unwrap()).0
    }));
    links.sort_unstable();
    links.dedup();
    let km: f64 = links.iter().map(|&k| h.segs[k as usize].len).sum::<f64>() / 1000.0;
    let mut total = Report::default();
    for (n, batch) in links.chunks(300).enumerate() {
        let mut cols: FastMap<Col, ()> = FastMap::default();
        along(batch, &mut cols);
        let cols: Vec<Col> = cols.into_keys().collect();
        let region = Region::build(g, 0, &cols);
        let rep = check(g, &region, &cols);
        println!("batch {n}: {} links, {} chunks, {} water cells, {} drops, {} walls, {} perched", batch.len(), region.chunks.len(), rep.water_cells, rep.drops, rep.walls.len(), rep.perched.len());
        total = total.merge(rep);
    }
    println!("wide sweep: {} links ({km:.0} km of course)", links.len());
    report("wide river sweep", &total);
    assert_eq!(total.missing, 0);
    assert!(total.walls.is_empty(), "{} water walls, e.g. {}", total.walls.len(), total.walls[0]);
    assert!(total.perched.is_empty(), "{} perched, e.g. {}", total.perched.len(), total.perched[0]);
}

/// Not a check: how much of the river network is at rest, stepped, or has
/// falls at this scale (HYDRO_VSCALE to change it).
#[test]
#[ignore]
fn hydro_stats() {
    let g = world_gen();
    let h = hydro();
    let p = &g.planet;
    let (mut smooth, mut stepped, mut falls, mut n_falls, mut fall_m, mut tallest) = (0.0, 0.0, 0.0, 0u32, 0.0, 0.0f64);
    for k in 0..h.segs.len() as u32 {
        let s = &h.segs[k as usize];
        let pr = h.profile(k, p.block_size);
        let km = s.len / 1000.0;
        if pr.steps == 0 {
            smooth += km;
        } else if pr.falls {
            falls += km;
            n_falls += pr.steps;
            fall_m += pr.delta * pr.steps as f64;
            tallest = tallest.max(pr.delta);
        } else {
            stepped += km;
        }
    }
    let total = smooth + stepped + falls;
    let t = std::time::Instant::now();
    let rebuilt = Hydro::new(g.bake.hydro().unwrap(), p.radius, p.block_size, g.vscale, g.seed);
    println!("building the model from hydro.bin: {:.2} s ({} links)", t.elapsed().as_secs_f64(), rebuilt.segs.len());
    println!(
        "vertical scale 1/{:.1}: {} links, {total:.0} km of river: {:.1}% smooth and at rest, {:.1}% pools with cascades (<= 1 block), {:.1}% with waterfalls; {n_falls} falls, mean {:.1} m, tallest {tallest:.1} m; {} lakes",
        1.0 / g.vscale,
        h.segs.len(),
        100.0 * smooth / total,
        100.0 * stepped / total,
        100.0 * falls / total,
        fall_m / n_falls.max(1) as f64,
        h.lakes.len()
    );
}

/// Not a check: print the generated cells and the water model around
/// HYDRO_PROBE="face i j" (LOD 0), and the links of HYDRO_LINKS="a,b,...".
#[test]
#[ignore]
fn hydro_probe() {
    let g = world_gen();
    let h = hydro();
    let p = &g.planet;
    let reg = registry();
    if let Ok(links) = std::env::var("HYDRO_LINKS") {
        for k in links.split(',').filter_map(|s| s.trim().parse::<u32>().ok()) {
            let s = &h.segs[k as usize];
            let pr = h.profile(k, p.block_size);
            println!(
                "link {k}: nodes {} -> {} (kinds {} -> {}, lakes {} -> {}), len {:.1} m, W {:.3} -> {:.3}, flow {:.0}, hw {:.1} depth {:.1} hf {:.1}; profile {:?}; next link {}",
                s.a, s.b, h.node_kind[s.a as usize], h.node_kind[s.b as usize], s.lake_a, s.lake_b, s.len, s.w_a, s.w_b, s.flow, s.hw, s.depth, s.hf, pr, h.node_seg[s.b as usize]
            );
            let ups: Vec<u32> = (0..h.segs.len() as u32).filter(|&u| h.segs[u as usize].b == s.a).collect();
            println!("  upstream links {ups:?}; links ending at the same node {:?}", (0..h.segs.len() as u32).filter(|&u| h.segs[u as usize].b == s.b).collect::<Vec<_>>());
        }
    }
    let Ok(at) = std::env::var("HYDRO_PROBE") else { return };
    let v: Vec<i64> = at.split_whitespace().filter_map(|s| s.parse().ok()).collect();
    let (f, i, j) = (v[0] as u8, v[1], v[2]);
    let mut cols = Vec::new();
    for di in -2..=2 {
        for dj in -2..=2 {
            cols.push(p.wrap_cell(f, i + di, j + dj));
        }
    }
    let region = Region::build(g, 0, &cols);
    for (di, dj) in [(0, 0), (-1, 0), (1, 0), (0, -1), (0, 1)] {
        let (ff, ii, jj) = p.wrap_cell(f, i + di, j + dj);
        let d = p.dir(ff, ii as f64 + 0.5, jj as f64 + 0.5);
        let c = g.column(d, p.block_size);
        let ch = g.channels_at(d, p.block_size);
        let cands = h.cands(d, 0.0);
        let mut rivers = Vec::new();
        h.rivers_at(d, &cands, p.block_size, &mut rivers);
        let lake = h.lake_at(d, &cands.cells, p.block_size);
        println!("({di},{dj}) face {ff} i {ii} j {jj}: h {:.3} W {:.4} {:?} channels {ch:?} lake {lake:?}", c.height_m, c.water_m, c.water_kind);
        for r in &rivers {
            let pr = h.profile(r.seg, p.block_size);
            println!("   link {} d {:.2} s {:.2}: water {:.4} bed {:.4}", r.seg, r.d, r.s, h.w_water(r.seg, &pr, r.s), h.w_bed(r.seg, &pr, r.s));
        }
        let (k0, k1) = region.k_range(p, ff, ii, jj).unwrap();
        let top = (p.sea_level as f64 + c.height_m.max(c.water_m.max(-1e9) as f32) as f64) as i64;
        let mut line = String::new();
        for k in (top - 4).max(k0)..=(top + 3).min(k1) {
            let (id, l) = region.cell(p, ff, ii as i64, jj as i64, k).unwrap();
            line += &format!(" k{k}:{}{}", reg.def(id).name, if reg.is_liquid(id) { format!("/{l}") } else { String::new() });
        }
        println!("   cells:{line}");
    }
}
