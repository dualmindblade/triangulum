//! Triangulum engine (M1a).
//!
//!   cargo run --release                         # fly (WASD, mouse, scroll speed, P presets)
//!   cargo run --release -- --at 12.5 -40.2 300  # start at lat, lon, altitude (m above sea)
//!   cargo run --release -- --at 12.5 -40.2 300 --look 45 -20   # plus heading and pitch (deg)
//!   cargo run --release -- --bench --out out/run1
//!   cargo run --release -- --shots --out out/shots --vscale 0.125 --sea-level 1024
//!   cargo run --release -- --find-places
//!
//! World settings come from `assets/world.json`; every one can be overridden:
//!   --seed N --block-size S --height H --sea-level K --n N --bake DIR --vscale F --roughness F
//! Other options: --lod-factor F (5) --threads N --no-ao --no-split --skirt N
//!   --no-vsync --size WxH --hide-water --markers --backend vulkan|dx12
//!   --frames N --skip-flight --only pose,pose --label TEXT --which a,b
//!   --screenshot PATH --exit-after SECS   (app: save a frame with the HUD, then quit)

pub mod app;
pub mod bench;
pub mod camera;
pub mod cubesphere_stub;
pub mod engine;
pub mod grid;
pub mod mesh;
pub mod render;
pub mod stream;
pub mod ui;
pub mod world;
pub mod worldgen;

use engine::Engine;
use glam::DVec3;
use grid::WorldConfig;
use mesh::MeshOptions;
use std::sync::Arc;
use stream::LodParams;
use world::Registry;

struct Args {
    world: String,
    blocks: String,
    seed: Option<u64>,
    block_size: Option<f64>,
    height: Option<u32>,
    sea_level: Option<u32>,
    n: Option<u32>,
    bake: Option<String>,
    vscale: Option<f64>,
    roughness: Option<f64>,
    at: Option<(f64, f64, f64)>,
    look: Option<(f64, f64)>,
    bench: bool,
    shots: bool,
    find_places: bool,
    probe: bool,
    out: String,
    frames: usize,
    skip_flight: bool,
    only: Option<String>,
    which: Option<String>,
    label: String,
    lod_factor: f64,
    threads: usize,
    ao: bool,
    split: bool,
    skirt: u32,
    vsync: bool,
    width: u32,
    win_height: u32,
    hide_water: bool,
    markers: bool,
    backend: Option<String>,
    screenshot: Option<String>,
    exit_after: Option<f64>,
}

fn parse_args() -> Args {
    let root = env!("CARGO_MANIFEST_DIR");
    let mut a = Args {
        world: format!("{root}/assets/world.json"),
        blocks: format!("{root}/assets/blocks.json"),
        seed: None,
        block_size: None,
        height: None,
        sea_level: None,
        n: None,
        bake: None,
        vscale: None,
        roughness: None,
        at: None,
        look: None,
        bench: false,
        shots: false,
        find_places: false,
        probe: false,
        out: format!("{root}/out/run"),
        frames: 120,
        skip_flight: false,
        only: None,
        which: None,
        label: String::new(),
        lod_factor: 5.0,
        threads: 0,
        ao: true,
        split: true,
        skirt: 2,
        vsync: true,
        width: 1920,
        win_height: 1080,
        hide_water: false,
        markers: false,
        backend: None,
        screenshot: None,
        exit_after: None,
    };
    let mut it = std::env::args().skip(1);
    while let Some(k) = it.next() {
        let mut val = || it.next().unwrap_or_else(|| panic!("missing value after {k}"));
        match k.as_str() {
            "--world" => a.world = val(),
            "--blocks" => a.blocks = val(),
            "--seed" => a.seed = Some(val().parse().unwrap()),
            "--block-size" => a.block_size = Some(val().parse().unwrap()),
            "--height" => a.height = Some(val().parse().unwrap()),
            "--sea-level" => a.sea_level = Some(val().parse().unwrap()),
            "--n" => a.n = Some(val().parse().unwrap()),
            "--bake" => a.bake = Some(val()),
            "--vscale" => a.vscale = Some(val().parse().unwrap()),
            "--roughness" => a.roughness = Some(val().parse().unwrap()),
            "--at" => {
                let lat: f64 = val().parse().unwrap();
                let lon: f64 = val().parse().unwrap();
                let alt: f64 = val().parse().unwrap();
                a.at = Some((lat, lon, alt));
            }
            "--look" => {
                let heading: f64 = val().parse().unwrap();
                let pitch: f64 = val().parse().unwrap();
                a.look = Some((heading, pitch));
            }
            "--bench" => a.bench = true,
            "--shots" => a.shots = true,
            "--find-places" => a.find_places = true,
            "--probe" => a.probe = true,
            "--out" => a.out = val(),
            "--frames" => a.frames = val().parse().unwrap(),
            "--skip-flight" => a.skip_flight = true,
            "--only" => a.only = Some(val()),
            "--which" => a.which = Some(val()),
            "--label" => a.label = val(),
            "--lod-factor" => a.lod_factor = val().parse().unwrap(),
            "--threads" => a.threads = val().parse().unwrap(),
            "--no-ao" => a.ao = false,
            "--no-split" => a.split = false,
            "--skirt" => a.skirt = val().parse().unwrap(),
            "--no-vsync" => a.vsync = false,
            "--size" => {
                let s = val();
                let (w, h) = s.split_once('x').expect("--size WxH");
                a.width = w.parse().unwrap();
                a.win_height = h.parse().unwrap();
            }
            "--hide-water" => a.hide_water = true,
            "--markers" => a.markers = true,
            "--backend" => a.backend = Some(val()),
            "--screenshot" => a.screenshot = Some(val()),
            "--exit-after" => a.exit_after = Some(val().parse().unwrap()),
            "--help" | "-h" => {
                println!("{}", include_str!("main.rs").lines().take_while(|l| l.starts_with("//!")).map(|l| l.trim_start_matches("//!").trim_start_matches(' ')).collect::<Vec<_>>().join("\n"));
                std::process::exit(0);
            }
            other => panic!("unknown argument {other} (try --help)"),
        }
    }
    a
}

fn main() {
    let args = parse_args();
    if args.threads > 0 {
        rayon::ThreadPoolBuilder::new().num_threads(args.threads).build_global().unwrap();
    }
    let mut config = WorldConfig::load(&args.world).unwrap_or_else(|e| panic!("{e}"));
    if let Some(v) = args.seed { config.seed = v; }
    if let Some(v) = args.block_size { config.block_size_m = v; }
    if let Some(v) = args.height { config.world_height = v; }
    if let Some(v) = args.sea_level { config.sea_level = v; }
    if let Some(v) = args.n { config.columns_per_face = v; }
    if let Some(v) = &args.bake { config.bake = v.clone(); }
    if let Some(v) = args.vscale { config.vertical_scale = Some(v); }
    if let Some(v) = args.roughness { config.roughness = v; }

    let t0 = std::time::Instant::now();
    let registry = Arc::new(Registry::load(&args.blocks).unwrap_or_else(|e| panic!("{e}")));
    let bake: Arc<dyn worldgen::bake::Bake> = Arc::new(worldgen::bake::open(&config.bake).unwrap_or_else(|e| panic!("load bake {}: {e}", config.bake)));
    println!("bake loaded in {:.2}s: {}", t0.elapsed().as_secs_f64(), bake.describe());
    println!("blocks: {} definitions from {}", registry.len(), args.blocks);
    let lod = LodParams { factor: args.lod_factor, max_inflight: (rayon::current_num_threads() * 2).max(4) };
    let engine = Engine::build(config, registry, bake, lod, MeshOptions { split: args.split, ao: args.ao }, args.skirt, args.hide_water, Vec::new(), args.markers).unwrap_or_else(|e| panic!("{e}"));
    let p = engine.planet;
    println!(
        "planet {}: s = {} m, H = {} blocks (sea level {}), N = {}, R = {:.3} km, vertical scale {:.5} (1/{:.1}), roughness {}, max LOD {} ({} m voxels), lod factor {}, {} rayon threads",
        engine.config.name,
        p.block_size,
        p.height,
        p.sea_level,
        p.n,
        p.radius / 1000.0,
        engine.vscale(),
        1.0 / engine.vscale(),
        engine.config.roughness,
        p.max_lod,
        p.voxel_m(p.max_lod),
        lod.factor,
        rayon::current_num_threads()
    );
    let engine = Arc::new(engine);

    if args.probe {
        // Height profile along 96 columns around --at, in the lattice i direction.
        let (lat, lon, _) = args.at.expect("--probe needs --at LAT LON ALT");
        let dir = grid::Planet::dir_of_lat_lon_deg(lat, lon);
        let (f, i, j) = p.lattice_of_dir(dir);
        let mut prev = f64::NAN;
        let mut max_step: f64 = 0.0;
        for di in -48..48 {
            let d = p.dir(f, i.floor() + di as f64 + 0.5, j.floor() + 0.5);
            let c = engine.generator.column(d, p.block_size);
            let h = c.height_m as f64;
            if !prev.is_nan() {
                max_step = max_step.max((h - prev).abs());
            }
            if di % 8 == 0 {
                println!("i{:+4}: height {:8.2} m  ocean {}  temp {:.1}", di, h, c.ocean, c.temp_c);
            }
            prev = h;
        }
        println!("max |step| between adjacent columns over 96 columns: {max_step:.2} m");
        return;
    }
    if args.find_places {
        let pl = engine.places;
        for (name, dir, v) in [("highest land", pl.peak.0, pl.peak.1), ("deepest ocean", pl.trench.0, pl.trench.1), ("steepest coast", pl.coast.0, pl.coast.1)] {
            let (lat, lon) = grid::Planet::lat_lon_deg(dir);
            let (f, i, j) = p.lattice_of_dir(dir);
            let c = engine.generator.column(dir, p.block_size);
            println!("{name}: lat {lat:.4} lon {lon:.4} (face {f} i {i:.0} j {j:.0}); bake value {v:.3}; height at this scale {:.1} m, ocean {}", c.height_m, c.ocean);
        }
        for pr in engine.presets() {
            let (lat, lon) = grid::Planet::lat_lon_deg(pr.dir);
            println!("preset {}: lat {lat:.4} lon {lon:.4} alt {} m AGL heading {} pitch {}", pr.name, pr.alt_agl, pr.heading_deg, pr.pitch_deg);
        }
        return;
    }
    if args.bench {
        bench::run_bench(engine, bench::BenchOptions { out: args.out, frames: args.frames, skip_flight: args.skip_flight, only: args.only, backend: args.backend, label: args.label });
        return;
    }
    if args.shots {
        bench::run_shots(engine, bench::ShotOptions { out: args.out, backend: args.backend, which: args.which });
        return;
    }
    let start = match args.at {
        Some((lat, lon, alt)) => {
            let dir = grid::Planet::dir_of_lat_lon_deg(lat, lon);
            let (heading, pitch) = args.look.unwrap_or((0.0, if alt > 2000.0 { -45.0 } else { 0.0 }));
            engine.camera_asl(dir, alt, heading, pitch)
        }
        None => {
            let pr = &engine.presets()[0];
            engine.camera_at(pr.dir, pr.alt_agl, pr.heading_deg, pr.pitch_deg)
        }
    };
    let _ = DVec3::ZERO;
    app::run(engine, app::AppOptions { width: args.width, height: args.win_height, vsync: args.vsync, start, backend: args.backend, screenshot: args.screenshot, exit_after: args.exit_after });
}
