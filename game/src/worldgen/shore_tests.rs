//! Shores: lakes and rivers at every LOD, what shows through the water, and
//! the shape of the coast, measured on the real generator (needs the bake).
//! The `shore_*` tests marked ignored are measuring tools: they print numbers
//! and write maps to SHORE_OUT (default out/shores).

use super::hydro_tests::{Col, Region, hydro, path, registry, world_gen};
use super::*;
use crate::grid::FastMap;
use crate::worldgen::inspect::{chart_dir, write_png};
use rayon::prelude::*;

fn out_dir() -> String {
    let d = std::env::var("SHORE_OUT").unwrap_or_else(|_| concat!(env!("CARGO_MANIFEST_DIR"), "/out/shores").to_string());
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn world() -> Generator {
    if std::env::var("SHORE_HALF").is_ok() {
        return super::tests::half_metre_generator();
    }
    match std::env::var("HYDRO_VSCALE").ok().and_then(|s| s.parse::<f64>().ok()) {
        Some(vs) => super::tests::generator_pub(Some(vs), 4096, 1536),
        None => super::tests::test_generator(),
    }
}

/// One field of a plan-view grid: h, W, kind, mask.
struct Plan {
    spacing: f64,
    base: Vec<f32>,
    ocean: Vec<f32>,
    h: Vec<f32>,
    w: Vec<f32>,
    kind: Vec<u8>,
}

fn plan(g: &Generator, centre: DVec3, px: usize, spacing: f64, voxel_m: f64) -> Plan {
    let face = g.bake.sample(centre).face;
    let (u0, v0) = bake::chart_uv(face, centre);
    let r = g.planet.radius;
    let half = (px as f64 - 1.0) * 0.5;
    let rows: Vec<Vec<(f32, f32, f32, f32, u8)>> = (0..px)
        .into_par_iter()
        .map(|y| {
            (0..px)
                .map(|x| {
                    let u = u0 + (x as f64 - half) * spacing / r;
                    let v = v0 + (half - y as f64) * spacing / r;
                    let dir = chart_dir(face, u, v);
                    let s = g.bake.sample(dir);
                    let c = g.column(dir, voxel_m);
                    let kind = match c.water_kind {
                        WaterKind::None => 0,
                        WaterKind::Ocean => 1,
                        WaterKind::Lake => 2,
                        WaterKind::River => 3,
                    };
                    ((s.elev_km * 1000.0 * g.vscale) as f32, s.ocean as f32, c.height_m, c.water_m as f32, kind)
                })
                .collect()
        })
        .collect();
    let mut p = Plan { spacing, base: vec![], ocean: vec![], h: vec![], w: vec![], kind: vec![] };
    for row in rows {
        for (b, o, h, w, k) in row {
            p.base.push(b);
            p.ocean.push(o);
            p.h.push(h);
            p.w.push(w);
            p.kind.push(k);
        }
    }
    p
}

/// Diverging colour for a signed height (blue below 0, tan above), `range` m full scale.
fn ramp(v: f32, range: f32) -> [u8; 3] {
    let t = (v / range).clamp(-1.0, 1.0);
    if t < 0.0 {
        let a = -t;
        [(240.0 * (1.0 - a)) as u8, (240.0 * (1.0 - 0.7 * a)) as u8, 255]
    } else {
        [255, (240.0 * (1.0 - 0.6 * t)) as u8, (240.0 * (1.0 - t)) as u8]
    }
}

fn put(path: &str, px: usize, f: impl Fn(usize) -> [u8; 3]) {
    let mut rgba = vec![255u8; px * px * 4];
    for i in 0..px * px {
        let c = f(i);
        rgba[i * 4..i * 4 + 3].copy_from_slice(&c);
    }
    write_png(path, px as u32, px as u32, &rgba);
}

/// Not a check: base, mask and final height maps around SHORE_AT="lat lon"
/// (SHORE_SIZE km, SHORE_PX pixels).
#[test]
#[ignore]
fn shore_fields() {
    let g = world();
    let at: Vec<f64> = std::env::var("SHORE_AT").unwrap_or("3.05 -15.16".into()).split_whitespace().filter_map(|s| s.parse().ok()).collect();
    let size: f64 = std::env::var("SHORE_SIZE").ok().and_then(|s| s.parse().ok()).unwrap_or(24.0) * 1000.0;
    let px: usize = std::env::var("SHORE_PX").ok().and_then(|s| s.parse().ok()).unwrap_or(800);
    let voxel: f64 = std::env::var("SHORE_VOXEL").ok().and_then(|s| s.parse().ok()).unwrap_or(g.planet.block_size);
    let name = std::env::var("SHORE_NAME").unwrap_or("site".into());
    let spacing = size / px as f64;
    let d = Planet::dir_of_lat_lon_deg(at[0], at[1]);
    let p = plan(&g, d, px, spacing, voxel);
    let out = out_dir();
    let range: f32 = std::env::var("SHORE_RANGE").ok().and_then(|s| s.parse().ok()).unwrap_or(6.0);
    put(&format!("{out}/{name}_base.png"), px, |i| ramp(p.base[i], range));
    put(&format!("{out}/{name}_ocean.png"), px, |i| {
        let v = (p.ocean[i] * 255.0) as u8;
        [v, v, v]
    });
    put(&format!("{out}/{name}_h.png"), px, |i| ramp(p.h[i], range));
    put(&format!("{out}/{name}_wet.png"), px, |i| {
        if p.w[i] > p.h[i] {
            match p.kind[i] {
                1 => [40, 90, 200],
                2 => [40, 180, 200],
                _ => [200, 40, 200],
            }
        } else if p.w[i].is_finite() && p.kind[i] != 0 {
            [230, 200, 120]
        } else if p.h[i] < 0.0 {
            [200, 60, 60]
        } else {
            [90, 160, 80]
        }
    });
    let (mut lo, mut hi) = (f32::MAX, f32::MIN);
    for v in &p.base {
        lo = lo.min(*v);
        hi = hi.max(*v);
    }
    let wet = (0..px * px).filter(|&i| p.w[i] > p.h[i]).count() as f64 / (px * px) as f64;
    let mask = p.ocean.iter().filter(|v| **v > 0.0).count() as f64 / (px * px) as f64;
    let half = p.ocean.iter().filter(|v| **v > 0.5).count() as f64 / (px * px) as f64;
    let flip = (0..px * px).filter(|&i| (p.base[i] < 0.0) != (p.h[i] < 0.0)).count() as f64 / (px * px) as f64;
    println!("{name}: spacing {:.1} m, base {lo:.2}..{hi:.2} m, wet {:.3}, mask>0 {mask:.3}, mask>0.5 {half:.3}, sign(h) differs from sign(base) on {flip:.3}", p.spacing, wet);
}

// ---------------------------------------------------------------------------
// #2: wet columns, at every LOD, on the chunks the game really builds
// ---------------------------------------------------------------------------

const K_LAKE: usize = 0;
const K_RIVER: usize = 1;
const K_SEA: usize = 2;
const KINDS: [&str; 3] = ["lake", "river", "sea"];

#[derive(Default, Clone, Debug)]
struct Tally {
    /// Wet columns by kind, and how many of them hold no water at all.
    wet: [u64; 3],
    lost: [u64; 3],
    /// Lost, and their ground is at or above the rounded surface.
    rounded: [u64; 3],
    /// Lost for any other reason.
    other: [u64; 3],
    /// Water in a column whose ground stands above its surface (or that has none).
    dry_water: u64,
    /// Top water voxels with air beside them, by kind, and the air's depth (voxels).
    walls: [u64; 3],
    wall_max: u32,
    /// Water voxels over air.
    perched: u64,
    examples: Vec<String>,
}

impl Tally {
    fn merge(mut self, o: Tally) -> Tally {
        for k in 0..3 {
            self.wet[k] += o.wet[k];
            self.lost[k] += o.lost[k];
            self.rounded[k] += o.rounded[k];
            self.other[k] += o.other[k];
            self.walls[k] += o.walls[k];
        }
        self.dry_water += o.dry_water;
        self.perched += o.perched;
        self.wall_max = self.wall_max.max(o.wall_max);
        if self.examples.len() < 6 {
            self.examples.extend(o.examples);
        }
        self
    }
}

/// Footprints (face, cx, cy) at a LOD that hold these directions.
fn footprints_of(g: &Generator, lod: u8, dirs: &[DVec3]) -> Vec<(u8, u32, u32)> {
    let p = &g.planet;
    let e = CHUNK as f64 * (1u64 << lod) as f64;
    let mut m: FastMap<(u8, u32, u32), ()> = FastMap::default();
    for d in dirs {
        let (f, i, j) = p.lattice_of_dir(*d);
        m.insert((f, (i / e) as u32, (j / e) as u32), ());
    }
    let mut v: Vec<_> = m.into_keys().collect();
    v.sort_unstable();
    v
}

/// Whether a block is the surface of a water body.
fn is_wet(g: &Generator, reg: &Registry, id: BlockId) -> bool {
    id != BlockId::MAX && (reg.is_liquid(id) || id == g.blocks.ice || id == g.blocks.deep_ocean)
}

/// Every column of these footprints: is its water there, at this LOD?
fn survey(g: &Generator, lod: u8, fps: &[(u8, u32, u32)]) -> Tally {
    let p = &g.planet;
    let reg = registry();
    let per = CHUNK as u32;
    // The perimeter is enough: `build` takes every footprint it touches, a
    // voxel's margin included, and walking all 1,024 columns of thousands of
    // LOD-0 footprints spends minutes in its hash map.
    let cols: Vec<Col> = fps
        .iter()
        .flat_map(|&(f, cx, cy)| {
            (0..per).flat_map(move |t| {
                let (x0, y0) = (cx * per, cy * per);
                [(f, x0 + t, y0), (f, x0 + t, y0 + per - 1), (f, x0, y0 + t), (f, x0 + per - 1, y0 + t)]
            })
        })
        .collect();
    let region = Region::build(g, lod, &cols);
    let vm = p.voxel_m(lod);
    fps.par_iter()
        .map(|&(f, cx, cy)| {
            let mut t = Tally::default();
            let key = ChunkKey::neisor(f, lod, cx, cy, 0);
            let columns = g.chunk_columns(key);
            for x in 0..per {
                for y in 0..per {
                    let c = columns[((x + 1) as usize) * PAD + (y + 1) as usize];
                    let (i, j) = (cx * per + x, cy * per + y);
                    let Some((k0, k1)) = region.k_range(p, f, i, j) else { continue };
                    let kind = match c.water_kind {
                        WaterKind::Lake => K_LAKE,
                        WaterKind::River => K_RIVER,
                        _ => K_SEA,
                    };
                    let mut top = None;
                    for k in (k0..=k1).rev() {
                        if let Some((id, _)) = region.cell(p, f, i as i64, j as i64, k)
                            && is_wet(g, reg, id)
                        {
                            top = Some(k);
                            break;
                        }
                    }
                    if c.wet() {
                        t.wet[kind] += 1;
                        if top.is_none() {
                            t.lost[kind] += 1;
                            let w_r = (c.water_m / vm).round() * vm;
                            if c.height_m as f64 >= w_r {
                                t.rounded[kind] += 1;
                            } else {
                                t.other[kind] += 1;
                            }
                            if t.examples.len() < 2 {
                                let vb = (1u64 << lod) as f64;
                                let (lat, lon) = Planet::lat_lon_deg(p.dir(f, (i as f64 + 0.5) * vb, (j as f64 + 0.5) * vb));
                                t.examples.push(format!("LOD {lod} {}: lat {lat:.5} lon {lon:.5} h {:.2} W {:.3} snapped {w_r:.1}", KINDS[kind], c.height_m, c.water_m));
                            }
                        }
                    } else if top.is_some() {
                        t.dry_water += 1;
                    }
                    let Some(k) = top else { continue };
                    // Under the top water: solid or more water, never air.
                    if let Some((b, _)) = region.cell(p, f, i as i64, j as i64, k - 1)
                        && b != BlockId::MAX
                        && !reg.is_solid(b)
                        && !is_wet(g, reg, b)
                    {
                        t.perched += 1;
                    }
                    // Beside it: air here means water stands in the open.
                    let mut wall = 0u32;
                    for (di, dj) in [(-1i64, 0i64), (1, 0), (0, -1), (0, 1)] {
                        let (ni, nj) = (i as i64 + di, j as i64 + dj);
                        let mut drop = 0u32;
                        let mut kk = k;
                        while let Some((id, _)) = region.cell(p, f, ni, nj, kk) {
                            if id == BlockId::MAX || reg.is_solid(id) || is_wet(g, reg, id) {
                                break;
                            }
                            drop += 1;
                            kk -= 1;
                        }
                        wall = wall.max(drop);
                    }
                    if wall > 0 {
                        t.walls[if c.wet() { kind } else { K_SEA }] += 1;
                        t.wall_max = t.wall_max.max(wall);
                    }
                }
            }
            t
        })
        .reduce(Tally::default, Tally::merge)
}

/// Lakes and rivers to survey: every `lake_step`th lake's field cells, and
/// two links downstream from every `link_step`th link.
fn water_sites(lake_step: usize, link_step: usize) -> (Vec<DVec3>, Vec<DVec3>) {
    let h = hydro();
    let lakes: Vec<DVec3> = h.field.iter().filter(|c| c.lake >= 0 && c.lake as usize % lake_step == 0).map(|c| c.pos).collect();
    let mut rivers = Vec::new();
    for k in (0..h.segs.len()).step_by(link_step) {
        for &l in &path(k as u32, 2) {
            let s = &h.segs[l as usize];
            let n = (s.len / 8.0).ceil() as usize;
            rivers.extend((0..=n).map(|t| s.point(s.len * t as f64 / n as f64)));
        }
    }
    (lakes, rivers)
}

/// Not a check: on real lakes and rivers at each LOD, how many wet columns
/// hold no water, and why. SHORE_LAKE_STEP / SHORE_LINK_STEP thin the sites,
/// SHORE_LODS="0,3,5" picks LODs.
#[test]
#[ignore]
fn shore_water_by_lod() {
    let g = world_gen();
    let step = |n: &str, d: usize| std::env::var(n).ok().and_then(|s| s.parse().ok()).unwrap_or(d);
    let (lakes, rivers) = water_sites(step("SHORE_LAKE_STEP", 3), step("SHORE_LINK_STEP", 60));
    println!("{} lake field cells, {} river course points", lakes.len(), rivers.len());
    println!("| LOD | voxel | kind | wet columns | lost | lost % | rounded onto bed | other | water on dry columns | walls | deepest wall (voxels) | perched |");
    println!("|---|---|---|---|---|---|---|---|---|---|---|---|");
    let lods: Vec<u8> = std::env::var("SHORE_LODS").map(|s| s.split(',').filter_map(|x| x.parse().ok()).collect()).unwrap_or((0..=g.planet.max_lod).collect());
    for lod in lods {
        for (name, dirs) in [("lakes", &lakes), ("rivers", &rivers)] {
            let fps = footprints_of(g, lod, dirs);
            let t = survey(g, lod, &fps);
            let k = if name == "lakes" { K_LAKE } else { K_RIVER };
            let pct = 100.0 * t.lost[k] as f64 / t.wet[k].max(1) as f64;
            println!("| {lod} | {} m | {} | {} | {} | {pct:.1} | {} | {} | {} | {} | {} | {} |", g.planet.voxel_m(lod), KINDS[k], t.wet[k], t.lost[k], t.rounded[k], t.other[k], t.dry_water, t.walls[k], t.wall_max, t.perched);
            for e in t.examples.iter().take(2) {
                println!("    {e}");
            }
        }
    }
}

/// Bug round #2: a wet column holds water at every LOD, whatever the level of
/// its lake or river. Before the fix a coarse column whose surface rounded
/// down lost it (the ground under a lake that shallow stands above the
/// rounded surface): half the lakes vanished from space, and a different
/// half at each LOD. And nothing else changes: no water in dry columns,
/// none over air.
#[test]
fn lakes_and_rivers_keep_their_water_at_every_lod() {
    let g = world_gen();
    let (lakes, rivers) = water_sites(9, 600);
    let mut seen = [0u64; 3];
    for lod in 1..=g.planet.max_lod {
        for dirs in [&lakes, &rivers] {
            let t = survey(g, lod, &footprints_of(g, lod, dirs));
            for k in 0..3 {
                seen[k] += t.wet[k];
                assert_eq!(t.lost[k], 0, "LOD {lod}: {} of {} wet {} columns hold no water, e.g. {:?}", t.lost[k], t.wet[k], KINDS[k], t.examples);
            }
            assert_eq!(t.dry_water, 0, "LOD {lod}: water in {} columns with no water at that voxel size", t.dry_water);
            assert_eq!(t.perched, 0, "LOD {lod}: {} water voxels over air", t.perched);
        }
    }
    assert!(seen[K_LAKE] > 2_000 && seen[K_RIVER] > 200, "the sweep found almost no water: {seen:?}");
}

// ---------------------------------------------------------------------------
// #4: what shows through the water, per LOD, at the same columns
// ---------------------------------------------------------------------------

/// The recorded sites: name, lat, lon.
const RECORDED: [(&str, f64, f64); 3] = [("041021", -32.02, 118.83), ("041200", 3.05, -15.16), ("041049_end", 7.90, 129.90)];

/// What a voxel column shows from above: the first block that isn't air and,
/// through water, the first opaque block under it.
fn seen(g: &Generator, region: &Region, f: u8, i: u32, j: u32) -> Option<(BlockId, BlockId)> {
    let p = &g.planet;
    let reg = registry();
    let (k0, k1) = region.k_range(p, f, i, j)?;
    let mut top = None;
    for k in (k0..=k1).rev() {
        let (id, _) = region.cell(p, f, i as i64, j as i64, k)?;
        if id == g.blocks.air {
            continue;
        }
        if top.is_none() {
            top = Some(id);
        }
        if !reg.is_liquid(id) {
            return Some((top?, id));
        }
    }
    top.map(|t| (t, t))
}

/// Directions on a square grid around a place (metres apart).
fn grid_dirs(g: &Generator, centre: DVec3, n: usize, spacing: f64) -> Vec<DVec3> {
    let face = g.bake.sample(centre).face;
    let (u0, v0) = bake::chart_uv(face, centre);
    let r = g.planet.radius;
    let half = (n as f64 - 1.0) * 0.5;
    (0..n * n).map(|t| chart_dir(face, u0 + ((t % n) as f64 - half) * spacing / r, v0 + ((t / n) as f64 - half) * spacing / r)).collect()
}

/// (what shows at each direction) at a LOD, on the real chunks.
fn seen_at(g: &Generator, lod: u8, dirs: &[DVec3]) -> Vec<Option<(BlockId, BlockId)>> {
    let p = &g.planet;
    let vb = (1u64 << lod) as f64;
    let at: Vec<Col> = dirs
        .iter()
        .map(|d| {
            let (f, i, j) = p.lattice_of_dir(*d);
            (f, (i / vb) as u32, (j / vb) as u32)
        })
        .collect();
    let region = Region::build(g, lod, &at);
    at.par_iter().map(|&(f, i, j)| seen(g, &region, f, i, j)).collect()
}

/// Not a check: the block visible through the water at the same columns, per
/// LOD, and how often it is the block LOD 0 shows there.
#[test]
#[ignore]
fn shore_floor_by_lod() {
    let g = world_gen();
    let reg = registry();
    let name = |id: BlockId| reg.def(id).name.clone();
    println!("| site | LOD | water columns | seen through the water | same as LOD 0 |");
    println!("|---|---|---|---|---|");
    for (site, lat, lon) in RECORDED {
        let dirs = grid_dirs(g, Planet::dir_of_lat_lon_deg(lat, lon), 96, 80.0);
        let base = seen_at(g, 0, &dirs);
        for lod in 0..=g.planet.max_lod {
            let s = seen_at(g, lod, &dirs);
            let mut hist: FastMap<BlockId, u32> = FastMap::default();
            let (mut water, mut both, mut same) = (0u32, 0u32, 0u32);
            for (a, b) in s.iter().zip(&base) {
                let Some((top, under)) = a else { continue };
                if !reg.is_liquid(*top) && *top != g.blocks.deep_ocean {
                    continue;
                }
                water += 1;
                *hist.entry(*under).or_default() += 1;
                if let Some((t0, u0)) = b
                    && reg.is_liquid(*t0)
                {
                    both += 1;
                    same += (u0 == under) as u32;
                }
            }
            let mut h: Vec<(BlockId, u32)> = hist.into_iter().collect();
            h.sort_by_key(|x| std::cmp::Reverse(x.1));
            let list: Vec<String> = h.iter().map(|(id, n)| format!("{} {:.1}%", name(*id), 100.0 * *n as f64 / water.max(1) as f64)).collect();
            println!("| {site} | {lod} | {water} | {} | {:.1}% of {both} |", list.join(", "), 100.0 * same as f64 / both.max(1) as f64);
        }
    }
}

/// Bug round #4: through the water every LOD shows the block LOD 0 shows
/// there. Before the fix a coarse water voxel took the place of the ground it
/// overlapped, so what lay under it was whatever the strata held a voxel or
/// two down, flipping wherever the ground crossed half a voxel: bands that
/// were different at every LOD. (The coarsest LOD's sea is opaque: nothing
/// shows through it, and its colour stands for the same floor.)
#[test]
fn water_shows_the_same_floor_at_every_lod() {
    let g = world_gen();
    let reg = registry();
    for (site, lat, lon) in RECORDED {
        let dirs = grid_dirs(g, Planet::dir_of_lat_lon_deg(lat, lon), 36, 210.0);
        let base = seen_at(g, 0, &dirs);
        for lod in 1..=g.planet.max_lod {
            let s = seen_at(g, lod, &dirs);
            let (mut both, mut same, mut other) = (0u32, 0u32, Vec::new());
            for (a, b) in s.iter().zip(&base) {
                let Some((top, under)) = a else { continue };
                if !reg.is_liquid(*top) {
                    continue;
                }
                if *under != g.blocks.sand && *under != g.blocks.gravel {
                    other.push(reg.def(*under).name.clone());
                }
                if let Some((t0, u0)) = b
                    && reg.is_liquid(*t0)
                {
                    both += 1;
                    same += (u0 == under) as u32;
                }
            }
            assert!(other.is_empty(), "{site} LOD {lod}: {} columns show something other than the sea floor through the water, e.g. {}", other.len(), other[0]);
            let share = same as f64 / both.max(1) as f64;
            println!("{site} LOD {lod}: {same} of {both} columns show what LOD 0 shows");
            assert!(both == 0 || share > 0.85, "{site} LOD {lod}: only {:.1}% of {both} columns show what LOD 0 shows", 100.0 * share);
        }
    }
}

/// Not a check: a digest of the LOD-0 chunks around lakes, rivers and the
/// recorded coasts. Two builds that print the same digest generate the same
/// LOD 0 there, so neither needs a generator version over the other.
#[test]
#[ignore]
fn shore_lod0_digest() {
    use crate::sha256::{hex, sha256};
    let g = world_gen();
    let p = &g.planet;
    let (mut dirs, rivers) = water_sites(9, 300);
    dirs.extend(rivers);
    for (_, lat, lon) in RECORDED {
        dirs.extend(grid_dirs(g, Planet::dir_of_lat_lon_deg(lat, lon), 12, 500.0));
    }
    let mut keys: Vec<ChunkKey> = footprints_of(g, 0, &dirs)
        .into_par_iter()
        .flat_map_iter(|(f, cx, cy)| {
            let key = ChunkKey::neisor(f, 0, cx, cy, 0);
            let cols = g.chunk_columns(key);
            let (mut lo, mut hi) = (f64::MAX, f64::MIN);
            for c in &cols {
                lo = lo.min(c.height_m as f64);
                hi = hi.max(c.height_m as f64);
                if c.water_m.is_finite() {
                    hi = hi.max(c.water_m);
                }
            }
            let z = |h: f64| ((p.sea_level as f64 + h / p.block_size) / CHUNK as f64).floor().max(0.0) as u32;
            (z(lo - 8.0)..=z(hi + 8.0)).map(move |cz| ChunkKey::neisor(f, 0, cx, cy, cz)).collect::<Vec<_>>()
        })
        .collect();
    keys.sort_by_key(|k| (k.face, k.cx, k.cy, k.cz));
    let parts: Vec<[u8; 32]> = keys.par_iter().map(|k| sha256(&g.generate(*k).chunk.to_bytes())).collect();
    let all: Vec<u8> = parts.iter().flatten().copied().collect();
    println!("LOD-0 digest: {} chunks, {}", keys.len(), hex(&sha256(&all)));
}

// ---------------------------------------------------------------------------
// #5: the shape of the coast
// ---------------------------------------------------------------------------

/// Connected components of `mask` (4-neighbour) on a px x px grid: for each,
/// (cells, touches the window's border).
fn components(mask: &[bool], px: usize) -> Vec<(u32, bool)> {
    let mut seen = vec![false; mask.len()];
    let mut out = Vec::new();
    let mut stack = Vec::new();
    for s in 0..mask.len() {
        if !mask[s] || seen[s] {
            continue;
        }
        seen[s] = true;
        stack.push(s);
        let (mut n, mut border) = (0u32, false);
        while let Some(c) = stack.pop() {
            n += 1;
            let (x, y) = (c % px, c / px);
            border |= x == 0 || y == 0 || x == px - 1 || y == px - 1;
            let mut go = |c2: usize| {
                if mask[c2] && !seen[c2] {
                    seen[c2] = true;
                    stack.push(c2);
                }
            };
            if x > 0 {
                go(c - 1);
            }
            if x + 1 < px {
                go(c + 1);
            }
            if y > 0 {
                go(c - px);
            }
            if y + 1 < px {
                go(c + px);
            }
        }
        out.push((n, border));
    }
    out
}

/// Chamfer distance (cells) from each cell to the nearest cell where `from` is true.
fn chamfer(from: &[bool], px: usize) -> Vec<f32> {
    let mut d: Vec<f32> = from.iter().map(|&b| if b { 0.0 } else { 1e9 }).collect();
    const D: f32 = std::f32::consts::SQRT_2;
    for y in 0..px {
        for x in 0..px {
            let i = y * px + x;
            let mut v = d[i];
            if x > 0 {
                v = v.min(d[i - 1] + 1.0);
            }
            if y > 0 {
                v = v.min(d[i - px] + 1.0);
                if x > 0 {
                    v = v.min(d[i - px - 1] + D);
                }
                if x + 1 < px {
                    v = v.min(d[i - px + 1] + D);
                }
            }
            d[i] = v;
        }
    }
    for y in (0..px).rev() {
        for x in (0..px).rev() {
            let i = y * px + x;
            let mut v = d[i];
            if x + 1 < px {
                v = v.min(d[i + 1] + 1.0);
            }
            if y + 1 < px {
                v = v.min(d[i + px] + 1.0);
                if x + 1 < px {
                    v = v.min(d[i + px + 1] + D);
                }
                if x > 0 {
                    v = v.min(d[i + px - 1] + D);
                }
            }
            d[i] = v;
        }
    }
    d
}

/// Pieces of `mask` thinner than 2 r: what an opening with a disc of radius r
/// (cells) takes away, counted as connected pieces of at least `min_cells`.
fn slivers(mask: &[bool], px: usize, r: f32, min_cells: u32) -> u32 {
    let outside: Vec<bool> = mask.iter().map(|b| !b).collect();
    let d = chamfer(&outside, px);
    let core: Vec<bool> = d.iter().map(|v| *v > r).collect();
    let d2 = chamfer(&core, px);
    let thin: Vec<bool> = (0..mask.len()).map(|i| mask[i] && d2[i] > r).collect();
    components(&thin, px).iter().filter(|c| c.0 >= min_cells).count() as u32
}

#[derive(Default, Clone, Copy, Debug)]
struct CoastStats {
    coast_km: f64,
    /// Land and sea pieces smaller than one bake texel: too small for the
    /// planet to know about, so noise made them.
    land_blobs: u32,
    sea_blobs: u32,
    land_slivers: u32,
    sea_slivers: u32,
    /// Share of the shoreline's land columns that are sand, and how many
    /// separate runs of sand the shoreline breaks into.
    sand: f64,
    sand_runs: u32,
    /// All pieces that do not reach the window's border (land + sea), and
    /// how many the bake's own mask has in the window: coves and islands kept.
    pieces: u32,
    bake_pieces: u32,
}

impl CoastStats {
    fn per_km(&self) -> f64 {
        (self.land_blobs + self.sea_blobs + self.land_slivers + self.sea_slivers) as f64 / self.coast_km.max(1e-3)
    }
}

/// The sea mask of a window (columns of the sea's water, sampled for voxels
/// of `voxel_m`) and its shoreline's top blocks.
fn window(g: &Generator, centre: DVec3, px: usize, spacing: f64, voxel_m: f64) -> (Vec<bool>, CoastStats) {
    let face = g.bake.sample(centre).face;
    let (u0, v0) = bake::chart_uv(face, centre);
    let r = g.planet.radius;
    let half = (px as f64 - 1.0) * 0.5;
    let at = |x: f64, y: f64| chart_dir(face, u0 + (x - half) * spacing / r, v0 + (half - y) * spacing / r);
    let sea: Vec<bool> = (0..px * px)
        .into_par_iter()
        .map(|i| {
            let c = g.column(at((i % px) as f64, (i / px) as f64), voxel_m);
            c.wet() && c.water_kind == WaterKind::Ocean
        })
        .collect();
    let land: Vec<bool> = sea.iter().map(|b| !b).collect();
    let bake_sea: Vec<bool> = (0..px * px).into_par_iter().map(|i| g.bake.sample(at((i % px) as f64, (i / px) as f64)).ocean >= 0.5).collect();
    let bake_land: Vec<bool> = bake_sea.iter().map(|b| !b).collect();
    let mut edges = 0u64;
    for y in 0..px {
        for x in 0..px {
            let i = y * px + x;
            if x + 1 < px && sea[i] != sea[i + 1] {
                edges += 1;
            }
            if y + 1 < px && sea[i] != sea[i + px] {
                edges += 1;
            }
        }
    }
    // A staircase of edges over-counts a slanted line by about 4/pi.
    let coast_km = edges as f64 * spacing * std::f64::consts::FRAC_PI_4 / 1000.0;
    let cell = spacing * spacing;
    let tex = 140.0 * 140.0;
    let blobs = |m: &[bool]| components(m, px).iter().filter(|c| !c.1 && (c.0 as f64) * cell < tex).count() as u32;
    let closed = |m: &[bool]| components(m, px).iter().filter(|c| !c.1).count() as u32;
    let rad = (6.0 * voxel_m.max(1.0) / spacing).max(3.0) as f32;
    // The shoreline: land cells next to the sea.
    let shore: Vec<usize> = (px + 1..px * px - px - 1).filter(|&i| i % px != 0 && i % px != px - 1 && !sea[i] && (sea[i - 1] || sea[i + 1] || sea[i - px] || sea[i + px])).collect();
    let is_sand: Vec<bool> = shore
        .par_iter()
        .map(|&i| {
            let (x, y) = ((i % px) as f64, (i / px) as f64);
            let c = g.column(at(x, y), voxel_m);
            let dh = |a: f64, b: f64| (g.column(at(x + a, y + b), voxel_m).height_m - c.height_m) / spacing as f32;
            let slope = (dh(1.0, 0.0).powi(2) + dh(0.0, 1.0).powi(2)).sqrt();
            g.surface(&c, slope).top == g.blocks.sand
        })
        .collect();
    let mut sand_mask = vec![false; px * px];
    for (k, &i) in shore.iter().enumerate() {
        sand_mask[i] = is_sand[k];
    }
    // Runs along the shoreline: sand cells joined across corners too.
    let thick = chamfer(&sand_mask, px).iter().map(|d| *d <= 2.0).collect::<Vec<bool>>();
    let sand_runs = components(&thick, px).len() as u32;
    let s = CoastStats {
        coast_km,
        land_blobs: blobs(&land),
        sea_blobs: blobs(&sea),
        land_slivers: slivers(&land, px, rad, 4),
        sea_slivers: slivers(&sea, px, rad, 4),
        sand: is_sand.iter().filter(|b| **b).count() as f64 / is_sand.len().max(1) as f64,
        sand_runs,
        pieces: closed(&land) + closed(&sea),
        bake_pieces: closed(&bake_land) + closed(&bake_sea),
    };
    (sea, s)
}

/// Five coasts besides the recorded ones, from gentle to steep: quantiles of
/// the bake's slope across points on its coastline.
fn other_coasts(g: &Generator) -> Vec<(String, DVec3)> {
    let mut pts: Vec<(f64, DVec3)> = super::tests::dirs_pub(60_000, 0xC0A57)
        .into_iter()
        .filter_map(|d| {
            let s = g.bake.sample(d);
            let (lat, _) = Planet::lat_lon_deg(d);
            if !(0.4..0.6).contains(&s.ocean) || s.edge_texels < 40.0 || lat.abs() > 66.0 || s.temp_c < 0.0 {
                return None;
            }
            Some(((s.elev_du_km.powi(2) + s.elev_dv_km.powi(2)).sqrt(), d))
        })
        .collect();
    pts.sort_by(|a, b| a.0.total_cmp(&b.0));
    [0.1, 0.3, 0.5, 0.7, 0.9].iter().map(|q| (format!("q{:02}", (q * 100.0) as u32), pts[((pts.len() - 1) as f64 * q) as usize].1)).collect()
}

fn coast_places(g: &Generator) -> Vec<(String, DVec3)> {
    let mut v: Vec<(String, DVec3)> = RECORDED[..2].iter().map(|(n, lat, lon)| (n.to_string(), Planet::dir_of_lat_lon_deg(*lat + if *n == "041021" { 0.07 } else { 0.0 }, *lon))).collect();
    v.extend(other_coasts(g));
    v
}

/// Bug round #5: a coast is one coherent line at every vertical scale and at
/// half-metre blocks, on foot and from altitude: no piece of land or sea too
/// small for the planet to know about, hardly a sliver, and no more closed
/// pieces than the bake's own coastline has. Before: 18-22 blobs and slivers
/// per km on most coasts, and ten or more times the bake's pieces.
#[test]
fn coasts_are_coherent() {
    let worlds = [("1/70", super::tests::test_generator()), ("1/8", super::tests::generator_pub(Some(0.125), 4096, 1536)), ("half-metre blocks", super::tests::half_metre_generator())];
    let mut measured = 0.0;
    for (world, g) in &worlds {
        let coarse = 32.0 * g.planet.block_size;
        for (name, d) in coast_places(g) {
            for (view, px, spacing, voxel) in [("LOD 0", 750usize, 2.0, g.planet.block_size), ("LOD 5", 250, coarse, coarse)] {
                let (_, s) = window(g, d, px, spacing, voxel);
                println!("{world} {name} {view}: {:.1} km of coast, {} blobs, {} slivers, {} closed pieces (bake {})", s.coast_km, s.land_blobs + s.sea_blobs, s.land_slivers + s.sea_slivers, s.pieces, s.bake_pieces);
                measured += s.coast_km;
                let at = format!("{world} {name} {view}");
                assert_eq!(s.land_blobs + s.sea_blobs, 0, "{at}: pieces smaller than a bake texel: {s:?}");
                assert!(s.per_km() < 1.0, "{at}: {:.2} slivers per km of coast: {s:?}", s.per_km());
                assert!(s.pieces <= s.bake_pieces + 1, "{at}: more pieces than the planet has: {s:?}");
            }
        }
    }
    assert!(measured > 100.0, "the windows found almost no coast ({measured:.1} km)");
}

/// The coast crosses cube edges without a step: the raster on each side
/// holds the same distance along the edge. Every side of every face.
#[test]
fn the_coast_meets_itself_across_cube_edges() {
    let g = world_gen();
    let p = &g.planet;
    let n = p.n as f64;
    let (mut crossed, mut worst) = (0u32, 0.0f64);
    for face in 0..6u8 {
        for side in 0..4u8 {
            for k in 0..2048 {
                let t = (k as f64 + 0.5) * n / 2048.0;
                let (a, b) = match side {
                    0 => ((0.5, t), (-0.5, t)),
                    1 => ((n - 0.5, t), (n + 0.5, t)),
                    2 => ((t, 0.5), (t, -0.5)),
                    _ => ((t, n - 0.5), (t, n + 0.5)),
                };
                let (sa, sb) = (g.bake.sample(p.dir(face, a.0, a.1)), g.bake.sample(p.dir(face, b.0, b.1)));
                let (ca, cb) = (sa.coast * p.radius, sb.coast * p.radius);
                if ca.abs().min(cb.abs()) < coast::BAND_M {
                    crossed += 1;
                    worst = worst.max((ca - cb).abs());
                }
            }
        }
    }
    println!("{crossed} samples of cube edge near a coast: largest step in the distance {worst:.2} m");
    assert!(crossed > 20, "only {crossed} samples near a coast");
    assert!(worst < 3.0, "the distance from the coast steps {worst:.2} m across a cube edge");
}

/// Not a check: slivers and blobs per km of coast at seven coasts, at LOD 0
/// (3 km windows at 2 m) and from altitude (LOD 5: 16 km at 32 m).
#[test]
#[ignore]
fn shore_coast_stats() {
    let g = world();
    let out = out_dir();
    let tag = std::env::var("SHORE_TAG").unwrap_or("run".into());
    println!("| coast | lat, lon | view | coast km | land blobs | sea blobs | land slivers | sea slivers | per km | closed pieces (bake's) | sand on shore | sand runs |");
    println!("|---|---|---|---|---|---|---|---|---|---|---|---|");
    for (name, d) in coast_places(&g) {
        let (lat, lon) = Planet::lat_lon_deg(d);
        for (view, px, spacing, voxel) in [("LOD 0", 1500usize, 2.0, g.planet.block_size), ("LOD 5", 500, 32.0, 32.0 * g.planet.block_size)] {
            let (sea, s) = window(&g, d, px, spacing, voxel);
            println!("| {name} | {lat:.3}, {lon:.3} | {view} | {:.1} | {} | {} | {} | {} | {:.2} | {} ({}) | {:.0}% | {} |", s.coast_km, s.land_blobs, s.sea_blobs, s.land_slivers, s.sea_slivers, s.per_km(), s.pieces, s.bake_pieces, 100.0 * s.sand, s.sand_runs);
            put(&format!("{out}/{tag}_{name}_{}.png", view.replace(' ', "")), px, |i| if sea[i] { [40, 90, 200] } else { [214, 196, 140] });
        }
    }
}
