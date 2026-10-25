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
        let g = super::tests::test_generator();
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
                for a in 0..9 {
                    for b in 0..9 {
                        let i = cx as f64 * per as f64 + (a as f64 + 0.5) / 9.0 * per as f64;
                        let j = cy as f64 * per as f64 + (b as f64 + 0.5) / 9.0 * per as f64;
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
                let pad = 10.0 * p.block_size + 3.0 * voxel_m;
                let k_lo = ((lo - pad) / p.block_size + p.sea_level as f64).max(0.0);
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
        h.rivers_at(d, &cands.segs, &mut rivers);
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
