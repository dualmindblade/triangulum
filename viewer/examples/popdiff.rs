//! LOD-pop meter: fly a fixed path, render every frame headless, and report
//! frame-to-frame pixel differences. Smooth camera motion gives a low, even
//! diff baseline; a tile LOD swap that changes geometry shows as a spike.
//! Run twice to compare:  TRI_NO_MORPH=1  vs  normal (geomorphing on).
//! Usage: cargo run --release --example popdiff -- [LAT LON ALT_KM YAW N STEP_DEG DAY_LEN_S]
use triangulum_viewer::camera::Camera;
use triangulum_viewer::planet::Planet;
use triangulum_viewer::renderer::Renderer;

fn main() -> anyhow::Result<()> {
    let a: Vec<String> = std::env::args().collect();
    let get = |i: usize, d: f64| a.get(i).and_then(|s| s.parse().ok()).unwrap_or(d);
    let (lat, lon, alt, yaw) = (get(1, 4.99), get(2, -29.4), get(3, 6.0), get(4, 268.0));
    let n = get(5, 60.0) as usize;

    let assets = if std::path::Path::new("viewer/assets/meta.json").exists() {
        "viewer/assets"
    } else {
        "assets"
    };
    let planet = Planet::load(assets)?;
    let instance =
        wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        compatible_surface: None,
        force_fallback_adapter: false,
        apply_limit_buckets: false,
    }))?;
    let (device, queue) =
        pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default()))?;
    let mut renderer = Renderer::new(
        device,
        queue,
        wgpu::TextureFormat::Rgba8UnormSrgb,
        (800, 450),
        1.0,
    );
    // optional: run the day/night cycle during the flight (0 = static sun)
    renderer.day_len_s = get(7, 0.0);
    renderer.sun_ref_lon = lon.to_radians();

    let mut camera = Camera {
        lon: lon.to_radians(),
        lat: lat.to_radians(),
        altitude_km: alt,
        radius_km: planet.radius_km,
        ground_km: 0.0,
        yaw: yaw.to_radians(),
        pitch: (-14f64).to_radians(),
    };
    let edits = Default::default();

    let dir = std::env::temp_dir().join("triangulum_popdiff");
    std::fs::create_dir_all(&dir)?;
    let mode = if std::env::var_os("TRI_NO_MORPH").is_some() { "raw" } else { "morph" };

    // fly a straight line at constant altitude: distant terrain barely moves
    // on screen, so honest motion contributes little and LOD swaps stand out
    let step_lon_deg = get(6, 0.004);
    let mut prev: Option<Vec<u8>> = None;
    let mut prev_keys: Option<std::collections::HashSet<_>> = None;
    let mut diffs: Vec<f64> = Vec::new();
    for k in 0..n {
        camera.lon = (lon + step_lon_deg * k as f64).to_radians();
        camera.ground_km = triangulum_viewer::terrain::ground_height_km(
            &planet,
            camera.position().normalize(),
            1.0,
        );
        // instrument the LOD selection: report which levels swapped tiles
        // this frame, so pixel-diff spikes can be pinned on real swaps
        let keys: std::collections::HashSet<_> = triangulum_viewer::terrain::select_tiles(
            camera.position(),
            planet.radius_km,
            0.35,
            None,
        )
        .into_iter()
        .collect();
        if let Some(pk) = prev_keys.as_ref() {
            let mut lv: Vec<u8> = keys.symmetric_difference(pk).map(|t| t.level).collect();
            if !lv.is_empty() {
                lv.sort_unstable();
                println!("frame {k:3}: SWAP {} tiles, levels {:?}", lv.len(), lv);
            }
        }
        prev_keys = Some(keys);
        let path = dir.join(format!("{mode}_{k:03}.png"));
        let path = path.to_str().unwrap();
        renderer.capture(&planet, &camera, &edits, path)?;
        let raw = load_png(path)?;
        if let Some(p) = prev.as_ref() {
            let mut sum = 0u64;
            let mut hist = vec![0u32; 256];
            for (x, y) in raw.iter().zip(p.iter()) {
                let d = x.abs_diff(*y);
                sum += d as u64;
                hist[d as usize] += 1;
            }
            let total = raw.len() as u64;
            let mut acc = 0u64;
            let mut p999 = 0usize;
            for (v, c) in hist.iter().enumerate() {
                acc += *c as u64;
                if acc * 1000 >= total * 999 {
                    p999 = v;
                    break;
                }
            }
            diffs.push(sum as f64 / total as f64);
            println!("frame {k:3}: mean {:6.3}  p99.9 {p999:3}", sum as f64 / total as f64);
        }
        prev = Some(raw);
    }
    diffs.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let med = diffs[diffs.len() / 2];
    let max = *diffs.last().unwrap();
    println!(
        "[{mode}] {} steps: median mean-diff {med:.3}, worst {max:.3}, worst/median {:.2}",
        diffs.len(),
        max / med.max(1e-9)
    );
    Ok(())
}

fn load_png(path: &str) -> anyhow::Result<Vec<u8>> {
    let decoder = png::Decoder::new(std::io::BufReader::new(std::fs::File::open(path)?));
    let mut reader = decoder.read_info()?;
    let mut buf = vec![0; reader.output_buffer_size().unwrap_or(0)];
    let info = reader.next_frame(&mut buf)?;
    buf.truncate(info.buffer_size());
    Ok(buf)
}
