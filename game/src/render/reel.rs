//! Repeatable M1c evidence: four poses x four solar hours, plus a July script.

use crate::{bench::Harness, engine::Engine, grid::Planet};
use std::{sync::Arc, time::Duration};

pub fn run(engine: Arc<Engine>, out: &str, backend: Option<&str>) {
    let mut h = Harness::new(engine.clone(), 1920, 1080, out, backend);
    let presets = engine.presets();
    let hills = presets[0].dir;
    let peak = engine.places.peak.0;
    // Same low-ground approach used by the vertical-scale comparison tool.
    let approach = (0..16)
        .map(|i| {
            let dir = engine.offset_dir(peak, i as f64 * 22.5, 4000.0);
            (dir, engine.ground(dir))
        })
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .unwrap()
        .0;
    let (heading, distance) = engine.heading_to(approach, peak);
    let pitch = ((engine.ground(peak) - engine.ground(approach) - 1.7) / distance)
        .atan()
        .to_degrees()
        * 0.5;
    let poses = [
        ("hills", engine.camera_at(hills, 1.7, -60.0, 0.0), 0.05),
        (
            "snow",
            engine.camera_at(approach, 1.7, heading, pitch),
            0.05,
        ),
        (
            "10km",
            engine.camera_asl(hills, 10_000.0, -60.0, -35.0),
            10.0,
        ),
        (
            "orbit",
            engine.camera_asl(hills, 500_000.0, -60.0, -85.0),
            500.0,
        ),
    ];
    let mut manifest = Vec::new();
    let mut comparison = String::from(
        "# M1c: same coordinates and solar directions; no weather.\nweather off\nmode fly\n",
    );
    for (name, cam, old_alt_km) in poses {
        let (lat, lon, alt) = cam.lat_lon_alt(&engine.planet);
        // At high altitude, match altitude/radius so July's much larger
        // planet has the same angular diameter. July's ground comparison is
        // at 50 m to clear foliage; engine ground eyes remain 1.7 m.
        let old_alt_km = if old_alt_km > 1.0 {
            old_alt_km * 8_660_254.037844386 / engine.planet.radius
        } else {
            old_alt_km
        };
        let (settle_s, _, settled) = h.settle(&cam, Duration::from_secs(240));
        assert!(
            settled,
            "{name}: streaming did not settle; evidence would be incomplete"
        );
        // Sunset is much later at the snowy range's latitude. Use actual
        // solar elevations for golden hour/dusk, not a misleading 18:00 label.
        let evening_hour = |altitude_deg: f64| {
            let (mut lo, mut hi) = (12.0, 23.999);
            for _ in 0..36 {
                let hour = (lo + hi) * 0.5;
                let t = h.gpu.sky.settings.starting_seconds(hour, lon.to_radians());
                let altitude = h
                    .gpu
                    .sky
                    .settings
                    .at(t)
                    .sun
                    .normalize()
                    .dot(cam.pos.normalize())
                    .asin()
                    .to_degrees();
                if altitude > altitude_deg {
                    lo = hour;
                } else {
                    hi = hour;
                }
            }
            (lo + hi) * 0.5
        };
        for (phase, hour) in [
            ("noon", 12.0),
            ("golden", evening_hour(5.0)),
            ("dusk", evening_hour(-3.0)),
            ("night", 0.0),
        ] {
            h.gpu.sky.clock.seconds = h.gpu.sky.settings.starting_seconds(hour, lon.to_radians());
            h.gpu.sky.clock.speed = 0.0;
            h.gpu.sky.set_observer(cam.pos, false);
            let frame = h.frame(&cam);
            let stem = format!("{name}_{phase}");
            h.save(&format!("{out}/{stem}.png"), None);
            let ephemeris = h.gpu.sky.settings.at(h.gpu.sky.clock.seconds);
            let (sun_lat, sun_lon) =
                Planet::lat_lon_deg(ephemeris.sun - cam.pos / engine.planet.radius);
            let sun_alt = ephemeris
                .sun
                .normalize()
                .dot(cam.pos.normalize())
                .asin()
                .to_degrees();
            manifest.push(serde_json::json!({
                "name": stem, "latitude": lat, "longitude": lon, "altitude_asl_m": alt,
                "heading": cam.heading.to_degrees(), "pitch": cam.pitch.to_degrees(),
                "local_solar_hour": hour, "absolute_seconds": h.gpu.sky.clock.seconds,
                "sun_lat": sun_lat, "sun_lon": sun_lon, "sun_altitude_deg": sun_alt,
                "gpu_ms_provisional": frame.gpu_ms, "settled_seconds": settle_s,
                "viewer_altitude_agl_km": old_alt_km,
            }));
            comparison.push_str(&format!("teleport {lat:.9} {lon:.9} {old_alt_km:.9}\nlook {:.9} {:.9}\nsun {sun_lat:.9} {sun_lon:.9}\nshot {stem}\n", cam.heading.to_degrees(), cam.pitch.to_degrees()));
            println!(
                "{stem}: at {lat:.6} {lon:.6} {alt:.3}, sun altitude {sun_alt:.2}, GPU {:.3} ms (provisional)",
                frame.gpu_ms
            );
        }
    }
    std::fs::write(
        format!("{out}/manifest.json"),
        serde_json::to_string_pretty(&manifest).unwrap(),
    )
    .unwrap();
    std::fs::write(format!("{out}/july-comparison.play"), comparison).unwrap();

    // Immersion uses the eye's actual liquid voxel, including sea-level
    // altitudes below zero. The control isolates the July tint numerically.
    let ocean = engine.places.trench.0;
    let cam = engine.camera_asl(ocean, -3.0, 0.0, 15.0);
    let (face, i, j, k) = engine.planet.lattice_of_pos(cam.pos);
    let id = engine
        .world
        .block_lattice(face, i.floor() as i64, j.floor() as i64, k.floor() as i64);
    assert!(
        engine.registry.is_liquid(id),
        "underwater evidence must start in a liquid voxel"
    );
    h.gpu.sky.clock.seconds = h
        .gpu
        .sky
        .settings
        .starting_seconds(12.0, ocean.y.atan2(ocean.x));
    assert!(h.settle(&cam, Duration::from_secs(240)).2);
    for (name, submerged) in [("underwater_control", false), ("underwater", true)] {
        h.gpu.sky.set_observer(cam.pos, submerged);
        h.frame(&cam);
        h.save(&format!("{out}/{name}.png"), None);
    }
    let first = h.gpu.read_rgba();
    h.gpu.sky.clock.advance(360.0); // paused clock: settle/draw count must not matter
    h.gpu.sky.set_observer(cam.pos, true);
    h.frame(&cam);
    assert_eq!(
        first,
        h.gpu.read_rgba(),
        "paused capture must be byte-repeatable"
    );
    println!("underwater liquid/negative-altitude and paused byte-repeatability checks passed");
}
