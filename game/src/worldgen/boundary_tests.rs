//! Boundaries, measured on the real generator (needs the bake).
//! The `boundary_*` tests marked ignored are measuring tools: they print
//! numbers and write maps to BOUNDARY_OUT (default out/boundaries).

use super::*;
use crate::grid::Planet;
use crate::worldgen::inspect::{chart_dir, write_png};
use rayon::prelude::*;

fn out_dir() -> String {
    let d = std::env::var("BOUNDARY_OUT").unwrap_or_else(|_| concat!(env!("CARGO_MANIFEST_DIR"), "/out/boundaries").to_string());
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn env_f64(name: &str, default: f64) -> f64 {
    std::env::var(name).ok().and_then(|s| s.parse().ok()).unwrap_or(default)
}

#[derive(Clone, Copy, Debug)]
struct Sample {
    t_bake: f32,
    p_bake: f32,
    c: Column,
}

fn samples(g: &Generator, centre: DVec3, px: usize, spacing: f64, voxel_m: f64) -> Vec<Sample> {
    let face = g.bake.sample(centre).face;
    let (u0, v0) = bake::chart_uv(face, centre);
    let r = g.planet.radius;
    let half = (px as f64 - 1.0) * 0.5;
    (0..px * px)
        .into_par_iter()
        .map(|i| {
            let (x, y) = ((i % px) as f64, (i / px) as f64);
            let dir = chart_dir(face, u0 + (x - half) * spacing / r, v0 + (half - y) * spacing / r);
            let s = g.bake.sample(dir);
            Sample { t_bake: s.temp_c as f32, p_bake: s.precip_mm as f32, c: g.column(dir, voxel_m) }
        })
        .collect()
}

/// Connected pieces of `mask` (4-neighbours): (cells, touches the window's edge).
fn pieces(mask: &[bool], px: usize) -> Vec<(u32, bool)> {
    let mut seen = vec![false; mask.len()];
    let mut out = Vec::new();
    let mut stack = Vec::new();
    for s in 0..mask.len() {
        if !mask[s] || seen[s] {
            continue;
        }
        seen[s] = true;
        stack.push(s);
        let (mut n, mut edge) = (0u32, false);
        while let Some(c) = stack.pop() {
            n += 1;
            let (x, y) = (c % px, c / px);
            edge |= x == 0 || y == 0 || x == px - 1 || y == px - 1;
            for (ok, c2) in [(x > 0, c.wrapping_sub(1)), (x + 1 < px, c + 1), (y > 0, c.wrapping_sub(px)), (y + 1 < px, c + px)] {
                if ok && mask[c2] && !seen[c2] {
                    seen[c2] = true;
                    stack.push(c2);
                }
            }
        }
        out.push((n, edge));
    }
    out
}

/// How much a two-way class map is scattered. `a` is the class, `valid` the
/// cells it is decided on (the other cells are neither class).
#[derive(Clone, Copy, Debug, Default)]
struct Scatter {
    cells: [u64; 2],
    /// Pieces of each class, the window's edge not counted as a piece of line.
    pieces: [u32; 2],
    /// Cells in pieces under `SMALL_M2`.
    small_cells: [u64; 2],
    small_pieces: [u32; 2],
    line_km: f64,
}

/// A piece under a hectare (100 m x 100 m) is scatter: no boundary on this
/// planet is that fine at the scale the bake draws it.
const SMALL_M2: f64 = 10_000.0;

impl Scatter {
    fn pieces_per_km(&self) -> f64 {
        (self.small_pieces[0] + self.small_pieces[1]) as f64 / self.line_km.max(1e-9)
    }
    fn small_share(&self) -> f64 {
        let (s, c) = (self.small_cells[0] + self.small_cells[1], self.cells[0] + self.cells[1]);
        s as f64 / c.max(1) as f64
    }
}

fn scatter(a: &[bool], valid: &[bool], px: usize, spacing: f64) -> Scatter {
    let mut s = Scatter::default();
    let cell = spacing * spacing;
    for (k, want) in [true, false].into_iter().enumerate() {
        let m: Vec<bool> = a.iter().zip(valid).map(|(v, ok)| *ok && *v == want).collect();
        s.cells[k] = m.iter().filter(|b| **b).count() as u64;
        for (n, edge) in pieces(&m, px) {
            s.pieces[k] += 1;
            if !edge && n as f64 * cell < SMALL_M2 {
                s.small_pieces[k] += 1;
                s.small_cells[k] += n as u64;
            }
        }
    }
    let mut edges = 0u64;
    for y in 0..px {
        for x in 0..px {
            let i = y * px + x;
            if !valid[i] {
                continue;
            }
            if x + 1 < px && valid[i + 1] && a[i] != a[i + 1] {
                edges += 1;
            }
            if y + 1 < px && valid[i + px] && a[i] != a[i + px] {
                edges += 1;
            }
        }
    }
    // A staircase of edges over-counts a slanted line by about 4/pi.
    s.line_km = edges as f64 * spacing * std::f64::consts::FRAC_PI_4 / 1000.0;
    s
}

fn row(name: &str, s: &Scatter) -> String {
    format!(
        "| {name} | {:.1} | {} / {} | {} / {} | {:.2} | {:.2}% |",
        s.line_km,
        s.pieces[0],
        s.pieces[1],
        s.small_pieces[0],
        s.small_pieces[1],
        s.pieces_per_km(),
        100.0 * s.small_share()
    )
}

fn class_png(path: &str, px: usize, f: impl Fn(usize) -> [u8; 3] + Sync) {
    let mut rgba = vec![255u8; px * px * 4];
    for i in 0..px * px {
        rgba[i * 4..i * 4 + 3].copy_from_slice(&f(i));
    }
    write_png(path, px as u32, px as u32, &rgba);
}

/// Not a check: which term of the old rule scatters each boundary at Andrew's
/// four poses: the bake's field alone, with the lapse of the full landform
/// (the sea floor's included), with the dither, and both.
#[test]
#[ignore]
fn boundary_causes() {
    let g = super::tests::test_generator();
    println!("| where | window | |grad T| C/km | |grad P| mm/km | dither moves T line | dither moves P line |");
    println!("|---|---|---|---|---|---|");
    for (name, lat, lon, size, spacing) in [("lake", 6.960, 38.669, 1500.0, 1.0), ("biome", 2.110, 24.218, 3000.0, 1.5), ("snow", 9.899, 21.908, 6000.0, 3.0), ("polar", -83.716, 85.137, 10000.0, 5.0)] {
        let dir = Planet::dir_of_lat_lon_deg(lat, lon);
        let px = (size / spacing) as usize;
        let v = samples(&g, dir, px, spacing, spacing);
        let dirs = window_dirs(&g, dir, px, spacing);
        let lapse: Vec<f32> = dirs
            .par_iter()
            .map(|d| {
                let dh = g.lf.sample(*d, &g.bake.sample(*d), spacing, g.strata.column_offset_m(*d)).dh;
                g.lapse_c(dh) as f32
            })
            .collect();
        let (mut gt, mut gp, mut n) = (0.0f64, 0.0f64, 0.0f64);
        for y in (1..px - 1).step_by(16) {
            for x in (1..px - 1).step_by(16) {
                let i = y * px + x;
                gt += ((v[i + 1].t_bake - v[i - 1].t_bake) as f64).hypot((v[i + px].t_bake - v[i - px].t_bake) as f64) / (2.0 * spacing);
                gp += ((v[i + 1].p_bake - v[i - 1].p_bake) as f64).hypot((v[i + px].p_bake - v[i - px].p_bake) as f64) / (2.0 * spacing);
                n += 1.0;
            }
        }
        let (gt, gp) = (gt / n * 1000.0, gp / n * 1000.0);
        println!("| {name} | {:.1} km at {spacing} m | {gt:.2} | {gp:.1} | {:.0} m | {:.0} m |", size / 1000.0, 1500.0 / gt, 40_000.0 / gp);
        let land: Vec<bool> = v.iter().map(|s| !s.c.wet()).collect();
        let water: Vec<bool> = v.iter().map(|s| s.c.wet()).collect();
        println!("{name}: line | km | pieces | small pieces | small per km | cells in small pieces");
        for (label, with_lapse, with_dither) in [("bake only", false, false), ("bake + lapse", true, false), ("bake + dither", false, true), ("the old rule", true, true)] {
            let t = |i: usize| v[i].t_bake - if with_lapse { lapse[i] } else { 0.0 } + if with_dither { 1.5 * v[i].c.dither } else { 0.0 };
            let snow: Vec<bool> = (0..px * px).map(|i| t(i) < SNOW_C).collect();
            let ice: Vec<bool> = (0..px * px).map(|i| t(i) < ICE_C).collect();
            println!("{}", row(&format!("snow, {label}"), &scatter(&snow, &land, px, spacing)));
            println!("{}", row(&format!("ice, {label}"), &scatter(&ice, &water, px, spacing)));
        }
        let dry: Vec<bool> = v.iter().map(|s| s.p_bake + 40.0 * s.c.dither < DRY_MM).collect();
        println!("{}", row("dry, the old rule", &scatter(&dry, &land, px, spacing)));
    }
}

/// The block a player sees from above at each sample: the top of `surface`,
/// the water's own colour tinted by what shows through it.
fn top_rgb(g: &Generator, v: &[Sample], i: usize, px: usize, spacing: f64) -> [u8; 3] {
    let b = &g.blocks;
    let c = &v[i].c;
    let at = |k: usize| v[k.min(px * px - 1)].c.height_m;
    let dhx = at(i + 1) - at(i.saturating_sub(1));
    let dhy = at(i + px) - at(i.saturating_sub(px));
    let slope = (dhx * dhx + dhy * dhy).sqrt() / (2.0 * spacing as f32);
    let sf = g.surface(c, slope);
    let colour = |id: BlockId| -> [u8; 3] {
        if id == b.grass {
            [84, 138, 80]
        } else if id == b.dry_grass {
            [134, 134, 76]
        } else if id == b.sand {
            [180, 165, 125]
        } else if id == b.snow {
            [222, 225, 230]
        } else if id == b.stone {
            [120, 116, 112]
        } else if id == b.gravel {
            [98, 96, 104]
        } else {
            [150, 120, 100]
        }
    };
    if c.wet() {
        if c.temp_c < -4.0 {
            return [166, 208, 232];
        }
        let f = colour(sf.top);
        return [(f[0] as f32 * 0.45 + 12.0) as u8, (f[1] as f32 * 0.45 + 40.0) as u8, (f[2] as f32 * 0.45 + 90.0) as u8];
    }
    colour(sf.top)
}

/// Not a check: a plan map of what shows from above around BOUNDARY_AT="lat lon"
/// (BOUNDARY_SIZE km, BOUNDARY_PX pixels, BOUNDARY_VOXEL m, BOUNDARY_NAME).
#[test]
#[ignore]
fn boundary_map() {
    let g = super::tests::test_generator();
    let out = out_dir();
    let at: Vec<f64> = std::env::var("BOUNDARY_AT").unwrap_or("9.899 21.908".into()).split_whitespace().filter_map(|s| s.parse().ok()).collect();
    let size = env_f64("BOUNDARY_SIZE", 6.0) * 1000.0;
    let px = env_f64("BOUNDARY_PX", 1500.0) as usize;
    let spacing = size / px as f64;
    let voxel = env_f64("BOUNDARY_VOXEL", spacing.max(1.0));
    let name = std::env::var("BOUNDARY_NAME").unwrap_or("site".into());
    let t = std::time::Instant::now();
    let v = samples(&g, Planet::dir_of_lat_lon_deg(at[0], at[1]), px, spacing, voxel);
    println!("{px} x {px} columns in {:.2} s", t.elapsed().as_secs_f64());
    class_png(&format!("{out}/{name}.png"), px, |i| top_rgb(&g, &v, i, px, spacing));
    let land: Vec<bool> = v.iter().map(|s| !s.c.wet()).collect();
    let water: Vec<bool> = v.iter().map(|s| s.c.wet()).collect();
    let snow: Vec<bool> = v.iter().map(|s| s.c.temp_c < -3.0).collect();
    let ice: Vec<bool> = v.iter().map(|s| s.c.temp_c < -4.0).collect();
    let dry: Vec<bool> = v.iter().map(|s| s.c.precip < 320.0).collect();
    println!("| line | km | pieces | small pieces | small per km | cells in small pieces |");
    println!("|---|---|---|---|---|---|");
    println!("{}", row("snow", &scatter(&snow, &land, px, spacing)));
    println!("{}", row("ice", &scatter(&ice, &water, px, spacing)));
    println!("{}", row("dry", &scatter(&dry, &land, px, spacing)));
    let lake: Vec<bool> = v.iter().map(|s| s.c.wet() && s.c.water_kind == WaterKind::Lake).collect();
    let all = vec![true; px * px];
    println!("{}", row("lake", &scatter(&lake, &all, px, spacing)));
}

/// The kinds of line this mission is about: what they divide, and which
/// columns they are decided on.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Line {
    Snow,
    Ice,
    Dry,
    Desert,
}

impl Line {
    fn name(self) -> &'static str {
        match self {
            Line::Snow => "snow",
            Line::Ice => "ice",
            Line::Dry => "dry grass",
            Line::Desert => "desert",
        }
    }
    fn valid(self, c: &Column) -> bool {
        match self {
            Line::Ice => c.wet(),
            _ => !c.wet(),
        }
    }
    /// The class by the rule in the game: the two fields, no dither.
    fn class(self, t: f32, p: f32) -> bool {
        match self {
            Line::Snow => t < SNOW_C,
            Line::Ice => t < ICE_C,
            Line::Dry => p < DRY_MM,
            Line::Desert => p < DESERT_MM && t > DESERT_C,
        }
    }
    /// A place on the bake's own contour of this line?
    fn on(self, s: &bake::BakeSample) -> bool {
        match self {
            Line::Snow => s.ocean < 0.05 && s.lake < 0.05 && (s.temp_c + 3.0).abs() < 0.3,
            Line::Ice => s.ocean > 0.95 && (s.temp_c + 4.0).abs() < 0.3,
            Line::Dry => s.ocean < 0.05 && s.lake < 0.05 && (s.precip_mm - 320.0).abs() < 12.0 && s.temp_c > 0.0,
            Line::Desert => s.ocean < 0.05 && s.lake < 0.05 && (s.precip_mm - 130.0).abs() < 8.0 && s.temp_c > 10.0,
        }
    }
}

/// The rule before this mission, rebuilt from the same fields: the bake's
/// values at the column itself, the lapse of the column's full landform, and
/// the dither on the value. It fails the gates below; the numbers beside the
/// new rule's are the proof that they can fail.
fn old_class(g: &Generator, line: Line, dir: DVec3, s: &Sample, voxel_m: f64) -> bool {
    let b = g.bake.sample(dir);
    let dh = g.lf.sample(dir, &b, voxel_m, g.strata.column_offset_m(dir)).dh;
    line.class((s.t_bake as f64 - g.lapse_c(dh)) as f32 + 1.5 * s.c.dither, s.p_bake + 40.0 * s.c.dither)
}

/// `n` places on a line, spread over the planet (at least 4 degrees apart),
/// away from the cube edges only so the windows stay simple.
fn line_sites(g: &Generator, line: Line, n: usize) -> Vec<DVec3> {
    let mut out: Vec<DVec3> = Vec::new();
    for d in super::tests::dirs_pub(400_000, 0xB0DA) {
        let s = g.bake.sample(d);
        if s.edge_texels > 60.0 && line.on(&s) && out.iter().all(|o| o.dot(d) < 4f64.to_radians().cos()) {
            out.push(d);
            if out.len() == n {
                break;
            }
        }
    }
    out
}

fn window_dirs(g: &Generator, centre: DVec3, px: usize, spacing: f64) -> Vec<DVec3> {
    let face = g.bake.sample(centre).face;
    let (u0, v0) = bake::chart_uv(face, centre);
    let r = g.planet.radius;
    let half = (px as f64 - 1.0) * 0.5;
    (0..px * px).map(|i| chart_dir(face, u0 + ((i % px) as f64 - half) * spacing / r, v0 + (half - (i / px) as f64) * spacing / r)).collect()
}

/// One line in one window: the new rule, the old one, and the bake's own
/// contour (near a field's minimum the bake itself has small pieces, and
/// those are real).
fn measure(g: &Generator, line: Line, centre: DVec3, px: usize, spacing: f64) -> (Scatter, Scatter, Scatter) {
    let v = samples(g, centre, px, spacing, spacing);
    let dirs = window_dirs(g, centre, px, spacing);
    let valid: Vec<bool> = v.iter().map(|s| line.valid(&s.c)).collect();
    let new: Vec<bool> = v.iter().map(|s| line.class(s.c.temp_c, s.c.precip)).collect();
    let old: Vec<bool> = v.par_iter().zip(&dirs).map(|(s, d)| old_class(g, line, *d, s, spacing)).collect();
    let own: Vec<bool> = v.iter().map(|s| line.class(s.t_bake, s.p_bake)).collect();
    (scatter(&new, &valid, px, spacing), scatter(&old, &valid, px, spacing), scatter(&own, &valid, px, spacing))
}

/// Round 2 #4: a boundary is a line and it stays one. Over places spread along
/// every kind of line, on foot and from altitude, on Neisor and on a planet
/// twice its size: hardly a piece under a hectare per km of line, and hardly
/// any of a class in such pieces. The old rule, measured in the same windows,
/// is what this is against.
#[test]
fn boundaries_are_lines_not_scatter() {
    let big = {
        let planet = Planet::new(1.0, 2048, 512, 262_144);
        let vs = planet.radius / 6_371_000.0;
        Generator::new(planet, super::tests::shared_bake_pub(), 42, vs, 1.0, 1.0, &super::hydro_tests::registry()).unwrap()
    };
    let worlds = [("Neisor", super::tests::test_generator()), ("N = 262144", big)];
    let (mut km_new, mut km_old, mut small_new, mut small_old) = (0.0, 0.0, 0u32, 0u32);
    for (world, g) in &worlds {
        for line in [Line::Snow, Line::Ice, Line::Dry, Line::Desert] {
            let sites = line_sites(g, line, 4);
            assert!(!sites.is_empty(), "{world}: no {} line", line.name());
            for (k, d) in sites.iter().enumerate() {
                for (view, px, spacing) in [("on foot", 1024usize, 3.0), ("from altitude", 512, 48.0)] {
                    let (n, o, own) = measure(g, line, *d, px, spacing);
                    println!("{world}, {} {k}, {view}:\n{}\n{}\n{}", line.name(), row("  new", &n), row("  old", &o), row("  bake", &own));
                    if n.line_km > 0.5 {
                        km_new += n.line_km;
                        let extra = (n.small_pieces[0] + n.small_pieces[1]).saturating_sub(own.small_pieces[0] + own.small_pieces[1]);
                        small_new += extra;
                        assert!(extra as f64 / n.line_km < 1.0 || extra <= 6, "{world}, {} {k}, {view}: scattered: {n:?} (bake {own:?})", line.name());
                        assert!(n.small_share() < 0.002 + own.small_share(), "{world}, {} {k}, {view}: scattered: {n:?} (bake {own:?})", line.name());
                    }
                    if o.line_km > 0.5 {
                        km_old += o.line_km;
                        small_old += o.small_pieces[0] + o.small_pieces[1];
                    }
                }
            }
        }
    }
    let (rate_new, rate_old) = (small_new as f64 / km_new, small_old as f64 / km_old);
    println!("over {km_new:.0} km of line: {rate_new:.3} pieces under a hectare per km; the old rule over {km_old:.0} km of line: {rate_old:.2}");
    assert!(km_new > 100.0, "the windows found almost no line");
    assert!(rate_new < 0.3, "new rule: {rate_new:.3} per km");
    assert!(rate_old > 2.0, "the old rule should fail this gate: {rate_old:.3} per km");
}

/// The optimisation is invisible: a chunk that took the cheap path holds every
/// column on the side of every threshold that the exact path puts it on, at
/// every LOD, in the places most likely to break it (right beside the lines).
#[test]
fn far_chunks_hold_no_boundary() {
    let g = super::tests::test_generator();
    let p = g.planet;
    let places: Vec<DVec3> = [Line::Snow, Line::Ice, Line::Dry, Line::Desert].iter().flat_map(|l| line_sites(&g, *l, 12)).collect();
    let classes = |t: f32, pr: f32| (t < SNOW_C, t < ICE_C, t > DESERT_C, pr < DESERT_MM, pr < DRY_MM);
    for lod in 0..=p.max_lod.min(8) {
        let vb = (1u64 << lod) as f64;
        let e = CHUNK as f64 * vb;
        // Chunks around each place, spread out over a few km.
        let reach = 1i64 << 8u8.saturating_sub(lod).min(5);
        let keys: Vec<ChunkKey> = places
            .iter()
            .flat_map(|d| {
                let (f, i, j) = p.lattice_of_dir(*d);
                let (ci, cj) = ((i / e) as i64, (j / e) as i64);
                (-3i64..=3).flat_map(move |a| (-3i64..=3).map(move |b| (f, ci + a * reach, cj + b * reach)))
            })
            .filter(|(_, a, b)| *a >= 0 && *b >= 0 && ((*a as f64 + 1.0) * e) < p.n as f64 && ((*b as f64 + 1.0) * e) < p.n as f64)
            .map(|(f, a, b)| ChunkKey::neisor(f, lod, a as u32, b as u32, (p.sea_level as f64 / e) as u32))
            .collect();
        let vm = p.voxel_m(lod);
        let (near, wrong): (usize, usize) = keys
            .par_iter()
            .map(|key| {
                let cols = g.chunk_columns(*key);
                let (i0, j0, _) = key.origin_blocks();
                let mut wrong = 0usize;
                for x in 0..PAD {
                    for y in 0..PAD {
                        let c = &cols[x * PAD + y];
                        let dir = p.dir(key.face, i0 as f64 + (x as f64 - 0.5) * vb, j0 as f64 + (y as f64 - 0.5) * vb);
                        let e = g.column(dir, vm);
                        // On the open sea only the ice line means anything.
                        wrong += if c.ocean { (c.temp_c < ICE_C) != (e.temp_c < ICE_C) } else { classes(c.temp_c, c.precip) != classes(e.temp_c, e.precip) } as usize;
                    }
                }
                (g.chunk_holds_a_threshold(*key) as usize, wrong)
            })
            .reduce(|| (0, 0), |a, b| (a.0 + b.0, a.1 + b.1));
        println!("LOD {lod}: {} chunks, {near} take the exact path, {wrong} columns wrong", keys.len());
        assert_eq!(wrong, 0, "LOD {lod}");
    }
}

/// Not a check: one line at several window sizes, the same place, so the
/// character can be compared across scales (BOUNDARY_AT, BOUNDARY_NAME).
/// Each window is 512 x 512 at its own voxel size, as the LOD there would be.
#[test]
#[ignore]
fn boundary_zoom() {
    let g = super::tests::test_generator();
    let out = out_dir();
    let at: Vec<f64> = std::env::var("BOUNDARY_AT").unwrap_or("9.899 21.908".into()).split_whitespace().filter_map(|s| s.parse().ok()).collect();
    let name = std::env::var("BOUNDARY_NAME").unwrap_or("zoom".into());
    let dir = Planet::dir_of_lat_lon_deg(at[0], at[1]);
    let px = 512usize;
    for spacing in [1.0f64, 4.0, 16.0, 64.0] {
        let v = samples(&g, dir, px, spacing, spacing);
        class_png(&format!("{out}/{name}_{}m.png", spacing as u32), px, |i| top_rgb(&g, &v, i, px, spacing));
    }
}

/// Valid and class of one sample: which line a measurement is about.
type Classify<'a> = &'a (dyn Fn(&Sample, DVec3, f64) -> (bool, bool) + Sync);

/// How rough a line is at a scale: over boxes `b` cells wide that hold a piece
/// of it, the mean of (the line's spread across its own main axis in the box)
/// / (the box). A smooth curve scores near 0 and falls with the box; a line as
/// rough at every scale as at this one scores the same at every box; scatter
/// scores near the 0.29 of cells strewn evenly over the box.
fn roughness(a: &[bool], valid: &[bool], px: usize, b: usize) -> Option<f64> {
    let nb = px / b;
    // Per box: count, sum x, sum y, sum xx, sum yy, sum xy.
    let mut bins = vec![[0f64; 6]; nb * nb];
    for y in 0..px - 1 {
        for x in 0..px - 1 {
            let i = y * px + x;
            if valid[i] && ((valid[i + 1] && a[i] != a[i + 1]) || (valid[i + px] && a[i] != a[i + px])) && x / b < nb && y / b < nb {
                let (fx, fy) = ((x % b) as f64, (y % b) as f64);
                let c = &mut bins[(y / b) * nb + x / b];
                c[0] += 1.0;
                c[1] += fx;
                c[2] += fy;
                c[3] += fx * fx;
                c[4] += fy * fy;
                c[5] += fx * fy;
            }
        }
    }
    let (mut sum, mut n) = (0.0, 0.0);
    for c in &bins {
        if c[0] < 0.75 * b as f64 {
            continue; // a corner clipped: too little line to have a direction
        }
        let (mx, my) = (c[1] / c[0], c[2] / c[0]);
        let (xx, yy, xy) = (c[3] / c[0] - mx * mx, c[4] / c[0] - my * my, c[5] / c[0] - mx * my);
        let small = 0.5 * (xx + yy) - (0.25 * (xx - yy) * (xx - yy) + xy * xy).sqrt();
        sum += small.max(0.0).sqrt() / b as f64;
        n += 1.0;
    }
    (n >= 3.0).then(|| sum / n)
}

/// A line's roughness at each box size, (box m, roughness) from three nested
/// windows that follow the line in: 64, 8 and 1 m columns, as the LOD there
/// would be. Below 8 cells a box is the staircase of the raster, not the line.
fn roughness_profile(g: &Generator, centre: DVec3, px: usize, f: Classify) -> Vec<(f64, f64)> {
    let mut c = centre;
    let mut out = Vec::new();
    for spacing in [64.0, 8.0, 1.0] {
        let v = samples(g, c, px, spacing, spacing);
        let dirs = window_dirs(g, c, px, spacing);
        let classes: Vec<(bool, bool)> = v.par_iter().zip(&dirs).map(|(s, d)| f(s, *d, spacing)).collect();
        let valid: Vec<bool> = classes.iter().map(|x| x.0).collect();
        let a: Vec<bool> = classes.iter().map(|x| x.1).collect();
        for b in [8usize, 16, 32] {
            if let Some(r) = roughness(&a, &valid, px, b) {
                out.push((b as f64 * spacing, r));
            }
        }
        // Follow the line in: the nearest piece of it to the window's centre.
        let mid = (px / 2) as i64;
        let near = (1..px * px - px - 1)
            .filter(|&i| i % px + 1 < px && valid[i] && valid[i + 1] && valid[i + px] && (a[i] != a[i + 1] || a[i] != a[i + px]))
            .min_by_key(|&i| {
                let (x, y) = ((i % px) as i64 - mid, (i / px) as i64 - mid);
                x * x + y * y
            });
        match near {
            Some(i) => c = dirs[i],
            None => break,
        }
    }
    out
}

/// Mean roughness per box size over several profiles.
fn mean_profile(all: &[Vec<(f64, f64)>]) -> Vec<(f64, f64)> {
    let mut sum: std::collections::BTreeMap<i64, (f64, f64)> = Default::default();
    for p in all {
        for (m, d) in p {
            let e = sum.entry(m.log2().round() as i64).or_default();
            e.0 += d;
            e.1 += 1.0;
        }
    }
    sum.into_iter().filter(|(_, v)| v.1 >= 2.0).map(|(k, v)| (2f64.powi(k as i32), v.0 / v.1)).collect()
}

fn range(p: &[(f64, f64)]) -> f64 {
    let (lo, hi) = p.iter().fold((f64::MAX, f64::MIN), |r, x| (r.0.min(x.1), r.1.max(x.1)));
    hi / lo
}

fn print_profile(name: &str, p: &[(f64, f64)]) {
    println!("  {name} {} (max / min {:.1})", p.iter().map(|(m, d)| format!("{m:.0}:{d:.3}")).collect::<Vec<_>>().join(" "), range(p));
}

/// Round 2 #4, "a discontinuity in the distribution of noise octaves": a line's
/// roughness changes smoothly from 8 m boxes to 2 km ones, with no step. The
/// old rule is smooth noise contours in the short boxes and scatter in the
/// long, with a jump between them where the dither's 41 m is.
#[test]
fn lines_are_rough_at_every_scale() {
    let g = super::tests::test_generator();
    for line in [Line::Snow, Line::Ice, Line::Dry] {
        let sites = line_sites(&g, line, 4);
        let new_f = |s: &Sample, _: DVec3, _: f64| (line.valid(&s.c), line.class(s.c.temp_c, s.c.precip));
        let old_f = |s: &Sample, d: DVec3, v: f64| (line.valid(&s.c), old_class(&g, line, d, s, v));
        let new = mean_profile(&sites.iter().map(|d| roughness_profile(&g, *d, 512, &new_f)).collect::<Vec<_>>());
        let old = mean_profile(&sites.iter().map(|d| roughness_profile(&g, *d, 512, &old_f)).collect::<Vec<_>>());
        println!("{}: box m : roughness", line.name());
        print_profile("new", &new);
        print_profile("old", &old);
        assert!(new.len() >= 7, "{}: too few scales: {new:?}", line.name());
        assert!(new.iter().all(|x| x.1 > 0.03), "{}: smooth at some scale: {new:?}", line.name());
        // From one box to the next the old rule jumps; a line can't.
        let jump = |p: &[(f64, f64)]| p.windows(2).map(|w| w[1].1 / w[0].1).fold(0.0, f64::max);
        println!("  largest step between neighbouring boxes: new {:.2}, old {:.2}", jump(&new), jump(&old));
        assert!(jump(&new) < 1.6, "{}: a step in the line's roughness: {new:?}", line.name());
        assert!(jump(&old) > 1.7, "{}: the old rule should fail this: {old:?}", line.name());
    }
}

/// Not a check: the same profile for lake outlines and coasts, so they can be
/// laid beside the others (they have no old rule to rebuild here).
#[test]
#[ignore]
fn boundary_water_lines() {
    let g = super::tests::test_generator();
    let lakes: Vec<DVec3> = {
        let h = g.hydro.as_ref().unwrap();
        let mut v: Vec<DVec3> = Vec::new();
        for c in h.field.iter().step_by(7).filter(|c| c.lake >= 0) {
            if v.iter().all(|o| o.dot(c.pos) < 6f64.to_radians().cos()) {
                v.push(c.pos);
            }
            if v.len() == 8 {
                break;
            }
        }
        v
    };
    let lake_f = |s: &Sample, _: DVec3, _: f64| (true, s.c.wet() && s.c.water_kind == WaterKind::Lake);
    print_profile("lakes ", &mean_profile(&lakes.iter().map(|d| roughness_profile(&g, *d, 512, &lake_f)).collect::<Vec<_>>()));
    let coasts: Vec<DVec3> = super::tests::dirs_pub(200_000, 0xC0)
        .into_iter()
        .filter(|d| {
            let s = g.bake.sample(*d);
            (0.4..0.6).contains(&s.ocean) && s.edge_texels > 60.0
        })
        .fold(Vec::new(), |mut v: Vec<DVec3>, d| {
            if v.len() < 8 && v.iter().all(|o| o.dot(d) < 6f64.to_radians().cos()) {
                v.push(d);
            }
            v
        });
    let sea_f = |s: &Sample, _: DVec3, _: f64| (true, s.c.wet() && s.c.water_kind == WaterKind::Ocean);
    print_profile("coasts", &mean_profile(&coasts.iter().map(|d| roughness_profile(&g, *d, 512, &sea_f)).collect::<Vec<_>>()));
}

/// Not a check: the point of a line nearest each of Andrew's four poses, for
/// the evidence script's ladders.
#[test]
#[ignore]
fn boundary_find() {
    let g = super::tests::test_generator();
    for (name, lat, lon, kind) in [("lake", 6.960, 38.669, 0), ("biome", 2.110, 24.218, 1), ("snow", 9.899, 21.908, 2), ("polar", -83.716, 85.137, 3)] {
        let mut c = Planet::dir_of_lat_lon_deg(lat, lon);
        for (px, spacing) in [(512usize, 32.0), (512, 4.0), (256, 1.0)] {
            let v = samples(&g, c, px, spacing, spacing);
            let dirs = window_dirs(&g, c, px, spacing);
            let class = |s: &Sample| match kind {
                0 => s.c.wet() && s.c.water_kind == WaterKind::Lake,
                1 => Line::Dry.class(s.c.temp_c, s.c.precip),
                2 => Line::Snow.class(s.c.temp_c, s.c.precip),
                _ => Line::Ice.class(s.c.temp_c, s.c.precip),
            };
            let mid = (px / 2) as i64;
            let near = (1..px * px - px - 1)
                .filter(|&i| i % px + 1 < px && (class(&v[i]) != class(&v[i + 1]) || class(&v[i]) != class(&v[i + px])))
                .min_by_key(|&i| {
                    let (x, y) = ((i % px) as i64 - mid, (i / px) as i64 - mid);
                    x * x + y * y
                });
            if let Some(i) = near {
                c = dirs[i];
            }
        }
        let (la, lo) = Planet::lat_lon_deg(c);
        println!("{name}: {la:.5} {lo:.5}");
    }
}

/// Chamfer distance (cells) from every cell to the nearest set cell.
fn distance_to(set: &[bool], px: usize) -> Vec<f32> {
    const D: f32 = std::f32::consts::SQRT_2;
    let mut d: Vec<f32> = set.iter().map(|b| if *b { 0.0 } else { 1e9 }).collect();
    let pass = |d: &mut Vec<f32>, i: usize, j: usize, k: f32| {
        d[i] = d[i].min(d[j] + k);
    };
    for y in 0..px {
        for x in 0..px {
            let i = y * px + x;
            if x > 0 {
                pass(&mut d, i, i - 1, 1.0);
            }
            if y > 0 {
                pass(&mut d, i, i - px, 1.0);
                if x > 0 {
                    pass(&mut d, i, i - px - 1, D);
                }
                if x + 1 < px {
                    pass(&mut d, i, i - px + 1, D);
                }
            }
        }
    }
    for y in (0..px).rev() {
        for x in (0..px).rev() {
            let i = y * px + x;
            if x + 1 < px {
                pass(&mut d, i, i + 1, 1.0);
            }
            if y + 1 < px {
                pass(&mut d, i, i + px, 1.0);
                if x + 1 < px {
                    pass(&mut d, i, i + px + 1, D);
                }
                if x > 0 {
                    pass(&mut d, i, i + px - 1, D);
                }
            }
        }
    }
    d
}

/// The line seen from orbit is the line you walk along: at each voxel size,
/// wherever a coarse column's class differs from the finest one at the same
/// place, the finest line is within a couple of voxels. The old dither, read
/// at a coarse voxel, was the same value at the same place and speckle in the
/// picture, so the comparison for it is the scatter gate, not this one.
#[test]
fn a_line_is_the_same_line_at_every_lod() {
    let g = super::tests::test_generator();
    for line in [Line::Snow, Line::Ice, Line::Dry] {
        for d in line_sites(&g, line, 3) {
            for voxel in [4.0f64, 16.0, 64.0, 256.0] {
                let px = 384usize;
                let coarse = samples(&g, d, px, voxel, voxel);
                let fine = samples(&g, d, px, voxel, 1.0);
                let a: Vec<bool> = fine.iter().map(|s| line.class(s.c.temp_c, s.c.precip)).collect();
                let b: Vec<bool> = coarse.iter().map(|s| line.class(s.c.temp_c, s.c.precip)).collect();
                // The fine line's cells, on this lattice.
                let mut on = vec![false; px * px];
                for y in 0..px - 1 {
                    for x in 0..px - 1 {
                        let i = y * px + x;
                        if a[i] != a[i + 1] || a[i] != a[i + px] {
                            on[i] = true;
                        }
                    }
                }
                let dist = distance_to(&on, px);
                let mut off: Vec<f32> = (0..px * px).filter(|&i| a[i] != b[i]).map(|i| dist[i]).collect();
                off.sort_by(|x, y| x.total_cmp(y));
                let at = |q: f64| off.get(((off.len() as f64 - 1.0) * q) as usize).copied().unwrap_or(0.0);
                println!("{}: voxels of {voxel} m: {} of {} columns differ from the finest, 99% within {:.1} voxels of its line, worst {:.1}", line.name(), off.len(), px * px, at(0.99), at(1.0));
                // Near a line only, by a voxel or two. (A patch that barely
                // exists, on a field flat around its threshold, can come and
                // go whole: that is the worst, and it's why this is a percentile.)
                assert!(at(0.99) <= 4.0, "{} at {voxel} m: {} voxels", line.name(), at(0.99));
                assert!((off.len() as f64) < 0.02 * (px * px) as f64, "{} at {voxel} m: {} columns", line.name(), off.len());
            }
        }
    }
}

/// No seam at a cube edge: walking across an edge a column at a time, on every
/// side of every face, the steps between the columns that straddle it are of a
/// size with the steps beside them on the same walks. (One step says little: the
/// warp can stretch a metre into dozens of metres of the field anywhere.)
#[test]
fn boundaries_cross_cube_edges_without_a_seam() {
    let g = super::tests::test_generator();
    let p = g.planet;
    let n = p.n as f64;
    const K: i32 = 6;
    let walks: Vec<(f64, f64, Vec<(f64, f64)>)> = (0..6u8)
        .into_par_iter()
        .flat_map_iter(|face| (0..4u8).flat_map(move |side| (0..256).map(move |k| (face, side, k))))
        .map(|(face, side, k)| {
            let t = (k as f64 + 0.5) * n / 256.0;
            // Column m of the walk: negative inside the face, positive past the side.
            let at = |m: i32| {
                let d = m as f64 + 0.5;
                match side {
                    0 => p.dir(face, -d, t),
                    1 => p.dir(face, n + d, t),
                    2 => p.dir(face, t, -d),
                    _ => p.dir(face, t, n + d),
                }
            };
            let walk: Vec<Column> = (-K..K).map(|m| g.column(at(m), 1.0)).collect();
            let (mut cross_t, mut cross_p, mut beside) = (0.0, 0.0, Vec::new());
            for (m, w) in walk.windows(2).enumerate() {
                if w[0].lapse_share != w[1].lapse_share {
                    continue; // a shore, not an edge
                }
                let (dt, dp) = ((w[0].temp_c - w[1].temp_c).abs() as f64, (w[0].precip - w[1].precip).abs() as f64);
                if m as i32 == K - 1 {
                    (cross_t, cross_p) = (dt, dp);
                } else {
                    beside.push((dt, dp));
                }
            }
            (cross_t, cross_p, beside)
        })
        .collect();
    let high = |mut v: Vec<f64>| {
        v.sort_by(|a, b| a.total_cmp(b));
        v[v.len() * 99 / 100]
    };
    let (ct, cp) = (high(walks.iter().map(|w| w.0).collect()), high(walks.iter().map(|w| w.1).collect()));
    let (bt, bp) = (high(walks.iter().flat_map(|w| w.2.iter().map(|x| x.0)).collect()), high(walks.iter().flat_map(|w| w.2.iter().map(|x| x.1)).collect()));
    println!("99% of steps across the cube's edges under {ct:.4} C, {cp:.2} mm; beside them {bt:.4} C, {bp:.2} mm");
    assert!(ct < 2.0 * bt + 0.01 && cp < 2.0 * bp + 4.0, "a seam: {ct} C, {cp} mm against {bt} C, {bp} mm");
}

/// With real height in the temperature the snow line follows the contours,
/// roughly: at 1/8, where the landform stands hundreds of metres high, the
/// ground along the line spans little of what the window spans, and the line
/// is still a line.
#[test]
fn on_a_slope_the_snow_line_follows_the_contours() {
    let g = super::tests::generator_pub(Some(0.125), 4096, 1536);
    let (mut measured, mut share) = (0, 0.0f32);
    for d in line_sites(&g, Line::Snow, 24) {
        let (px, spacing) = (768usize, 8.0);
        let v = samples(&g, d, px, spacing, spacing);
        let (lo, hi) = v.iter().fold((f32::MAX, f32::MIN), |r, s| (r.0.min(s.c.height_m), r.1.max(s.c.height_m)));
        let land: Vec<bool> = v.iter().map(|s| !s.c.wet()).collect();
        let snow: Vec<bool> = v.iter().map(|s| Line::Snow.class(s.c.temp_c, s.c.precip)).collect();
        let mut heights = Vec::new();
        for y in 0..px - 1 {
            for x in 0..px - 1 {
                let i = y * px + x;
                if land[i] && land[i + 1] && land[i + px] && (snow[i] != snow[i + 1] || snow[i] != snow[i + px]) {
                    heights.push(v[i].c.height_m);
                }
            }
        }
        if hi - lo < 150.0 || heights.len() < 300 {
            continue; // not a mountain
        }
        heights.sort_by(|a, b| a.total_cmp(b));
        let (p10, p90) = (heights[heights.len() / 10], heights[heights.len() * 9 / 10]);
        let s = scatter(&snow, &land, px, spacing);
        println!("window {:.0}..{:.0} m: the snow line runs at {p10:.0}..{p90:.0} m; {}", lo, hi, row("line", &s));
        measured += 1;
        share += (p90 - p10) / (hi - lo);
        assert!(s.small_share() < 0.01, "scattered: {s:?}");
    }
    assert!(measured >= 2, "found only {measured} mountains the snow line crosses");
    println!("over {measured} mountains the snow line spans {:.2} of the height in view", share / measured as f32);
    assert!(share / measured as f32 <= 0.5, "the line wanders over the slopes");
}

/// Not a check: what stands along a row of columns (BOUNDARY_ROW="face i0 i1 j"),
/// to place shore scripts: each run of the same kind and top block.
#[test]
#[ignore]
fn boundary_row() {
    let g = super::tests::test_generator();
    let p = g.planet;
    let r: Vec<u32> = std::env::var("BOUNDARY_ROW").unwrap_or("1 1900 2300 59004".into()).split_whitespace().filter_map(|s| s.parse().ok()).collect();
    let mut last = (WaterKind::None, 0u16 as BlockId, false);
    for i in r[1]..=r[2] {
        let dir = p.dir(r[0] as u8, i as f64 + 0.5, r[3] as f64 + 0.5);
        let c = g.column(dir, 1.0);
        let top = g.surface(&c, 0.0).top;
        let now = (c.water_kind, top, c.wet());
        if now != last || i % 32 == 0 {
            println!("i {i}: h {:.2} water {:.2} {:?} top {} wet {}", c.height_m, c.water_m, c.water_kind, top, c.wet());
            last = now;
        }
    }
}

/// Not a check: what a chunk's columns cost at each LOD, over random chunks on
/// the sea-level layer of the whole planet, one thread. Timing needs the machine
/// to itself.
#[test]
#[ignore]
fn boundary_cost() {
    use std::hint::black_box;
    use std::time::Instant;
    let g = super::tests::test_generator();
    let p = g.planet;
    let dirs = super::tests::dirs_pub(600, 0xC057);
    for lod in 0..=8u8 {
        let e = CHUNK as f64 * (1u64 << lod) as f64;
        let keys: Vec<ChunkKey> = dirs
            .iter()
            .map(|d| {
                let (f, i, j) = p.lattice_of_dir(*d);
                ChunkKey::neisor(f, lod, (i / e) as u32, (j / e) as u32, (p.sea_level as f64 / e) as u32)
            })
            .collect();
        let t = Instant::now();
        for k in &keys {
            black_box(g.chunk_columns(*k));
        }
        let us = t.elapsed().as_micros() as f64 / keys.len() as f64;
        let near = keys.iter().filter(|k| g.chunk_holds_a_threshold(**k)).count();
        println!("LOD {lod}: {us:.0} us a chunk, {near} of {} take the exact path", keys.len());
    }
}
