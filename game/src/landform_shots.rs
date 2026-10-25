//! Landform evidence renders: the same five places at the current engine
//! settings, captioned, for before/after comparison of the terrain
//! generator. Cameras are placed from the bake alone (not from the
//! generated relief) so before and after frames line up, except the
//! eye-height shot, which has to stand on the generated ground.
//!
//!   --landform-shots --out DIR [--label TEXT] [--which a,b]

use crate::bench::Harness;
use crate::camera::Camera;
use crate::engine::Engine;
use crate::worldgen::Marker;
use crate::worldgen::inspect::{LandPlaces, bake_ground_m, find_land_places};
use glam::DVec3;
use std::sync::Arc;
use std::time::Duration;

pub struct LandformShotOptions {
    pub out: String,
    pub backend: Option<String>,
    pub which: Option<String>,
    pub label: String,
}

pub fn places_with_names(p: &LandPlaces) -> Vec<(&'static str, DVec3)> {
    vec![("peak", p.peak), ("coast", p.coast), ("river", p.river), ("plain", p.plain)]
}

pub fn run_landform_shots(engine: Arc<Engine>, opts: LandformShotOptions) {
    let mut h = Harness::new(engine.clone(), 1920, 1080, &opts.out, opts.backend.as_deref());
    let p = engine.planet;
    let g = engine.generator.clone();
    let places = find_land_places(engine.bake.as_ref());
    let b = g.blocks;
    let vs = engine.vscale();
    let scale_label = format!("vertical scale 1/{:.0}", 1.0 / vs);
    let world_label = format!("s = {} m, H {} blocks, sea {}", p.block_size, p.height, p.sea_level);
    let dials = g.describe_relief();
    let gen_label = if opts.label.is_empty() { String::new() } else { format!(" | {}", opts.label) };
    let marker_pair = |dir: DVec3| -> Vec<Marker> {
        let beside = engine.offset_dir(dir, 90.0, 8.0 * p.block_size);
        vec![
            Marker { dir, height_blocks: (100.0 / p.block_size) as u32, width_blocks: (4.0 / p.block_size).max(1.0) as u32, block: b.marker_red, below: false },
            Marker { dir: beside, height_blocks: (2.0 / p.block_size) as u32, width_blocks: 1, block: b.marker_white, below: false },
        ]
    };
    for (name, dir) in places_with_names(&places) {
        let (lat, lon) = crate::grid::Planet::lat_lon_deg(dir);
        println!("place {name}: lat {lat:.4} lon {lon:.4}, bake ground {:.1} m at this scale", bake_ground_m(&g, dir));
    }
    // Heading from `from` with the lowest bake ground `dist` out (open views).
    let lowest_heading = |centre: DVec3, dist: f64| -> f64 {
        let mut best = (0.0, f64::MAX);
        for k in 0..16 {
            let hd = k as f64 * 22.5;
            let gr = g.bake.sample(engine.offset_dir(centre, hd, dist)).elev_km;
            if gr < best.1 {
                best = (hd, gr);
            }
        }
        best.0
    };
    // Downhill heading of the bake surface at a direction.
    let downhill_heading = |d: DVec3| -> f64 {
        let e0 = g.bake.sample(d).elev_km;
        let en = g.bake.sample(engine.offset_dir(d, 0.0, 300.0)).elev_km - e0;
        let ee = g.bake.sample(engine.offset_dir(d, 90.0, 300.0)).elev_km - e0;
        (-ee).atan2(-en).to_degrees()
    };

    struct Shot {
        name: &'static str,
        cam: Camera,
        markers: Vec<Marker>,
        caption: String,
    }
    let mut shots = Vec::new();
    let peak_bake = bake_ground_m(&g, places.peak);
    let approach = lowest_heading(places.peak, 4000.0);
    // (a) the highest range from the ground, 3 km out, eye 1.7 m on the generated ground.
    {
        let cam_dir = engine.offset_dir(places.peak, approach, 3000.0);
        let (heading, dist) = engine.heading_to(cam_dir, places.peak);
        let ground = engine.ground(cam_dir);
        let pitch = ((peak_bake - ground) / dist).atan().to_degrees() * 0.6;
        shots.push(Shot {
            name: "a_range_ground",
            cam: engine.camera_at(cam_dir, 1.7, heading, pitch),
            markers: marker_pair(places.peak),
            caption: format!("(a) highest range from the ground, 3 km out, eye 1.7 m | {scale_label} | {world_label} | {dials}{gen_label} | summit (bake) {peak_bake:.0} m, red marker 100 m at the summit"),
        });
    }
    // (b) the same range from 1 km above the bake ground, 5 km out, looking at the summit.
    {
        let cam_dir = engine.offset_dir(places.peak, approach, 5000.0);
        let (heading, dist) = engine.heading_to(cam_dir, places.peak);
        let alt = bake_ground_m(&g, cam_dir) + 1000.0;
        let pitch = ((peak_bake - alt) / dist).atan().to_degrees() - 4.0;
        shots.push(Shot {
            name: "b_range_1km",
            cam: engine.camera_asl(cam_dir, alt, heading, pitch),
            markers: marker_pair(places.peak),
            caption: format!("(b) highest range from 1 km above the ground, 5 km out | {scale_label} | {world_label} | {dials}{gen_label} | camera {alt:.0} m ASL, red marker 100 m at the summit"),
        });
    }
    // (c) the coast from 2 km offshore at 250 m, water shown.
    {
        let hd = lowest_heading(places.coast, 2000.0);
        let cam_dir = engine.offset_dir(places.coast, hd, 2000.0);
        let (heading, _) = engine.heading_to(cam_dir, places.coast);
        shots.push(Shot {
            name: "c_coast",
            cam: engine.camera_asl(cam_dir, 250.0, heading, -10.0),
            markers: marker_pair(places.coast),
            caption: format!("(c) the steepest coast from 2 km offshore at 250 m | {scale_label} | {world_label} | {dials}{gen_label} | red marker 100 m on the shore"),
        });
    }
    // (d) the river valley, looking down-valley from 2 km upstream, 300 m above the floor.
    {
        let down = downhill_heading(places.river);
        let cam_dir = engine.offset_dir(places.river, down + 180.0, 2000.0);
        let (heading, _) = engine.heading_to(cam_dir, places.river);
        let alt = bake_ground_m(&g, places.river) + 300.0;
        shots.push(Shot {
            name: "d_river_valley",
            cam: engine.camera_asl(cam_dir, alt, heading, -12.0),
            markers: marker_pair(places.river),
            caption: format!("(d) river valley (most discharge on temperate relief), down-valley from 2 km upstream, 300 m above the floor | {scale_label} | {world_label} | {dials}{gen_label} | red marker 100 m in the valley"),
        });
    }
    // (e) plains from 40 m.
    {
        let alt = bake_ground_m(&g, places.plain) + 40.0;
        shots.push(Shot {
            name: "e_plains",
            cam: engine.camera_asl(places.plain, alt, 60.0, -7.0),
            markers: Vec::new(),
            caption: format!("(e) temperate plains from 40 m | {scale_label} | {world_label} | {dials}{gen_label}"),
        });
    }

    for s in shots.into_iter().filter(|s| opts.which.as_deref().is_none_or(|w| w.split(',').any(|n| s.name.starts_with(n)))) {
        let e = Arc::new(engine.with_markers(s.markers.clone(), false, true));
        h.set_engine(e);
        let (secs, _, ok) = h.settle(&s.cam, Duration::from_secs(300));
        let path = format!("{}/{}.png", opts.out, s.name);
        h.save(&path, Some(&s.caption));
        let (lat, lon, alt) = s.cam.lat_lon_alt(&p);
        println!("{} -> {path} (settled {secs:.1} s{}; cam lat {lat:.4} lon {lon:.4} alt {alt:.0} m heading {:.1} pitch {:.1}; {} draws)", s.name, if ok { "" } else { " TIMEOUT" }, s.cam.heading.to_degrees(), s.cam.pitch.to_degrees(), h.streamer.stats.drawn);
    }
}
