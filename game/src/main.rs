//! Triangulum engine (M1b).
//!
//!   cargo run --release                         # play: walk on the hills (WASD, mouse, G to fly)
//!   cargo run --release -- --at 12.5 -40.2 300  # start at lat, lon, altitude (m above sea; flies when high)
//!   cargo run --release -- --at 12.5 -40.2 300 --look 45 -20   # plus heading and pitch (deg)
//!   cargo run --release -- --play scripts/m1-done.play         # scripted harness (see src/play.rs)
//!   cargo run --release -- --bench --out out/run1
//!   cargo run --release -- --shots --out out/shots --vscale 0.125 --sea-level 1024
//!   cargo run --release -- --find-places
//!   cargo run --release -- --landform-shots --out out/lf --label after   # landform evidence renders
//!   cargo run --release -- --terrain-map --out out/maps    # plan-view hillshades of the landform places
//!   cargo run --release -- --terrain-stats --out out/stats # slope, fidelity, anisotropy, LOD checks
//!
//! World settings come from `assets/world.json`; every one can be overridden:
//!   --seed N --block-size S --height H --sea-level K --n N --bake DIR --vscale F --roughness F --landform F
//! (a relative "bake" in world.json is resolved against the game/ directory)
//! Saves: --save DIR (default saves/<world name>)  --no-save  --keep-save (harness: keep its save)
//! Player: --walk | --fly (default: walk when starting within 50 m of the ground)
//!   --hotbar stone,dirt,...  (blocks on keys 1-9; default: the first nine solid blocks)
//! Other options: --lod-factor F (5) --threads N --no-ao --no-split --skirt N
//!   --no-vsync --size WxH --hide-water --markers --backend vulkan|dx12
//!   --frames N --skip-flight --only pose,pose --label TEXT --which a,b --map-px N
//!   --screenshot PATH --exit-after SECS   (app: save a frame with the HUD, then quit)

pub mod app;
pub mod bench;
pub mod camera;
pub mod engine;
pub mod grid;
pub mod landform_shots;
pub mod mesh;
pub mod play;
pub mod player;
pub mod render;
pub mod session;
pub mod sha256;
pub mod stream;
pub mod ui;
pub mod world;
pub mod worldgen;

use engine::Engine;
use grid::WorldConfig;
use mesh::MeshOptions;
use player::{Mode, Player};
use session::Session;
use std::path::Path;
use std::sync::Arc;
use stream::LodParams;
use world::save::SaveDir;
use world::{EditStore, Registry};

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
    landform: Option<f64>,
    at: Option<(f64, f64, f64)>,
    look: Option<(f64, f64)>,
    bench: bool,
    shots: bool,
    find_places: bool,
    probe: bool,
    landform_shots: bool,
    terrain_map: bool,
    terrain_stats: bool,
    map_px: usize,
    play: Option<String>,
    out: Option<String>,
    save: Option<String>,
    no_save: bool,
    keep_save: bool,
    walk: Option<bool>,
    hotbar: Option<String>,
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
        landform: None,
        at: None,
        look: None,
        bench: false,
        shots: false,
        find_places: false,
        probe: false,
        landform_shots: false,
        terrain_map: false,
        terrain_stats: false,
        map_px: 768,
        play: None,
        out: None,
        save: None,
        no_save: false,
        keep_save: false,
        walk: None,
        hotbar: None,
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
            "--landform" => a.landform = Some(val().parse().unwrap()),
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
            "--landform-shots" => a.landform_shots = true,
            "--terrain-map" => a.terrain_map = true,
            "--terrain-stats" => a.terrain_stats = true,
            "--map-px" => a.map_px = val().parse().unwrap(),
            "--play" => a.play = Some(val()),
            "--out" => a.out = Some(val()),
            "--save" => a.save = Some(val()),
            "--no-save" => a.no_save = true,
            "--keep-save" => a.keep_save = true,
            "--walk" => a.walk = Some(true),
            "--fly" => a.walk = Some(false),
            "--hotbar" => a.hotbar = Some(val()),
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

fn fail(msg: &str) -> ! {
    eprintln!("error: {msg}");
    std::process::exit(2)
}

fn main() {
    let args = parse_args();
    let root = env!("CARGO_MANIFEST_DIR");
    if args.threads > 0 {
        rayon::ThreadPoolBuilder::new().num_threads(args.threads).build_global().unwrap();
    }
    let mut config = WorldConfig::load(&args.world).unwrap_or_else(|e| fail(&e));
    // A relative bake path in world.json is relative to the game/ directory,
    // so the engine runs from any current directory.
    if Path::new(&config.bake).is_relative() {
        config.bake = format!("{root}/{}", config.bake);
    }
    if let Some(v) = args.seed { config.seed = v; }
    if let Some(v) = args.block_size { config.block_size_m = v; }
    if let Some(v) = args.height { config.world_height = v; }
    if let Some(v) = args.sea_level { config.sea_level = v; }
    if let Some(v) = args.n { config.columns_per_face = v; }
    if let Some(v) = &args.bake { config.bake = v.clone(); }
    if let Some(v) = args.vscale { config.vertical_scale = Some(v); }
    if let Some(v) = args.roughness { config.roughness = v; }
    if let Some(v) = args.landform { config.landform = v; }
    // The generator reads the landform dial from here (Engine::build only
    // passes the roughness dial through).
    worldgen::set_default_landform(config.landform);

    let t0 = std::time::Instant::now();
    let registry = Arc::new(Registry::load(&args.blocks).unwrap_or_else(|e| fail(&e)));
    let bake: Arc<dyn worldgen::bake::Bake> = Arc::new(worldgen::bake::open(&config.bake).unwrap_or_else(|e| fail(&format!("load bake {}: {e}", config.bake))));
    println!("bake loaded in {:.2}s: {}", t0.elapsed().as_secs_f64(), bake.describe());
    println!("blocks: {} definitions from {}", registry.len(), args.blocks);
    let bake_sha = world::save::bake_identity(&config.bake).unwrap_or_else(|e| fail(&format!("bake identity: {e}")));

    // Saves: load before the engine exists so a mismatch refuses early.
    let tools = args.bench || args.shots || args.find_places || args.probe;
    let edits = Arc::new(EditStore::new());
    let save_dir = if args.no_save || tools {
        None
    } else if let Some(script) = &args.play {
        let stem = Path::new(script).file_stem().and_then(|s| s.to_str()).unwrap_or("play").to_string();
        let dir = args.save.clone().unwrap_or_else(|| format!("{root}/saves/_play/{stem}"));
        if !args.keep_save {
            let _ = std::fs::remove_dir_all(&dir);
        }
        Some(SaveDir::new(dir))
    } else {
        Some(SaveDir::new(args.save.clone().unwrap_or_else(|| format!("{root}/saves/{}", config.name))))
    };
    if let Some(dir) = &save_dir {
        match dir.load_into(&config, &registry, &bake_sha, &edits) {
            Ok(report) => {
                for w in &report.warnings {
                    println!("save warning: {w}");
                }
                println!("save {}: {} edited chunk(s) loaded", dir.path.display(), report.chunks);
            }
            Err(e) => fail(&e),
        }
    }

    let lod = LodParams { factor: args.lod_factor, max_inflight: (rayon::current_num_threads() * 2).max(4) };
    let mut engine = Engine::build(config, registry.clone(), bake, edits, lod, MeshOptions { split: args.split, ao: args.ao }, args.skirt, args.hide_water, Vec::new(), args.markers).unwrap_or_else(|e| fail(&e));
    if let Some(h) = &args.hotbar {
        engine.hotbar = registry.hotbar_from(h).unwrap_or_else(|e| fail(&e));
    }
    let p = engine.planet;
    println!(
        "planet {}: s = {} m, H = {} blocks (sea level {}), N = {}, R = {:.3} km, vertical scale {:.5} (1/{:.1}), {}, max LOD {} ({} m voxels), lod factor {}, {} rayon threads",
        engine.config.name,
        p.block_size,
        p.height,
        p.sea_level,
        p.n,
        p.radius / 1000.0,
        engine.vscale(),
        1.0 / engine.vscale(),
        engine.generator.describe_relief(),
        p.max_lod,
        p.voxel_m(p.max_lod),
        lod.factor,
        rayon::current_num_threads()
    );
    let engine = Arc::new(engine);
    let out = |default: &str| args.out.clone().unwrap_or_else(|| format!("{root}/{default}"));

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
            let (f, i, j) = p.lattice_of_dir(pr.dir);
            let k = p.sea_level as f64 + engine.ground(pr.dir) / p.block_size;
            println!("preset {}: lat {lat:.4} lon {lon:.4} alt {} m AGL heading {} pitch {}; lattice face {f} i {i:.0} j {j:.0} ground k {k:.0}", pr.name, pr.alt_agl, pr.heading_deg, pr.pitch_deg);
        }
        return;
    }
    if args.landform_shots {
        landform_shots::run_landform_shots(engine, landform_shots::LandformShotOptions { out: out("out/run"), backend: args.backend, which: args.which, label: args.label });
        return;
    }
    if args.terrain_map || args.terrain_stats {
        let g = &engine.generator;
        let places = worldgen::inspect::find_land_places(engine.bake.as_ref());
        let named = landform_shots::places_with_names(&places);
        let out_dir = out("out/run");
        let _ = std::fs::create_dir_all(&out_dir);
        let mut report = format!("# terrain inspection: {}\n\nvertical scale 1/{:.1}, s = {} m, {}\n\n", args.label, 1.0 / engine.vscale(), p.block_size, g.describe_relief());
        for (name, dir) in &named {
            let (lat, lon) = grid::Planet::lat_lon_deg(*dir);
            let b = engine.bake.sample(*dir);
            report.push_str(&format!(
                "- {name}: lat {lat:.4} lon {lon:.4} (bake: elev {:.3} km, rough {:.3} km, uplift {:.2}, erodibility {:.2}, flow {:.2}, temp {:.1} C, precip {:.0} mm, ocean {:.2})\n",
                b.elev_km, b.rough_km, b.uplift, b.erodibility, b.flow_log10, b.temp_c, b.precip_mm, b.ocean
            ));
        }
        if args.terrain_map {
            report.push_str("\n## maps\n\n");
            report.push_str(&worldgen::inspect::terrain_maps(g, &named, &out_dir, args.map_px));
        }
        if args.terrain_stats {
            let t = std::time::Instant::now();
            report.push_str("\n## slope (rise over run)\n\n");
            report.push_str(&worldgen::inspect::slope_report(g, &named));
            report.push_str("\n## large-scale fidelity\n\n");
            report.push_str(&worldgen::inspect::fidelity_report(g));
            report.push_str("\n## lattice artifacts (aspect anisotropy)\n\n");
            report.push_str(&worldgen::inspect::anisotropy_report(g));
            report.push_str("\n## LOD consistency\n\n");
            report.push_str(&worldgen::inspect::lod_report(g));
            report.push_str(&format!("\n(stats took {:.1} s)\n", t.elapsed().as_secs_f64()));
        }
        print!("{report}");
        let path = format!("{out_dir}/terrain.md");
        std::fs::write(&path, report).unwrap();
        println!("wrote {path}");
        return;
    }
    if args.bench {
        bench::run_bench(engine, bench::BenchOptions { out: out("out/run"), frames: args.frames, skip_flight: args.skip_flight, only: args.only, backend: args.backend, label: args.label });
        return;
    }
    if args.shots {
        bench::run_shots(engine, bench::ShotOptions { out: out("out/shots"), backend: args.backend, which: args.which });
        return;
    }

    // The player: on the hills preset, or at --at.
    let (dir, alt_asl, heading, pitch) = match args.at {
        Some((lat, lon, alt)) => {
            let dir = grid::Planet::dir_of_lat_lon_deg(lat, lon);
            let (heading, pitch) = args.look.unwrap_or((0.0, if alt - engine.ground(dir) > 2000.0 { -45.0 } else { 0.0 }));
            (dir, Some(alt), heading, pitch)
        }
        None => {
            let pr = &engine.presets()[0];
            (pr.dir, None, pr.heading_deg, pr.pitch_deg)
        }
    };
    let ground = engine.ground(dir);
    let walk = args.walk.unwrap_or_else(|| alt_asl.is_none_or(|a| a - ground <= 50.0));
    let mut session = Session::new(engine.clone(), Player::new(dir * (p.radius + alt_asl.unwrap_or(ground + 2.0)), heading, pitch, if walk { Mode::Walk } else { Mode::Fly }, p.block_size), save_dir, bake_sha);
    if walk {
        session.teleport(dir, None);
    }

    if let Some(script) = args.play {
        let stem = Path::new(&script).file_stem().and_then(|s| s.to_str()).unwrap_or("play").to_string();
        let code = play::run_play(session, play::PlayOptions { script, out: out(&format!("out/play/{stem}")), width: args.width, height: args.win_height, backend: args.backend });
        std::process::exit(code);
    }
    app::run(session, app::AppOptions { width: args.width, height: args.win_height, vsync: args.vsync, backend: args.backend, screenshot: args.screenshot, exit_after: args.exit_after });
}
