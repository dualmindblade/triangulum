//! Export the teleport map's exact lunar tab without opening a window.
//!
//!   cargo run --release --example moonmap -- OUT.png [SEED] [WIDTH]

use std::io::BufWriter;
use std::path::Path;

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let out = args.get(1).map(String::as_str).unwrap_or("moon-map.png");
    let seed = args.get(2).and_then(|v| v.parse().ok()).unwrap_or(42i64);
    let width = args
        .get(3)
        .and_then(|v| v.parse().ok())
        .unwrap_or(1024usize)
        .max(64);
    let height = width / 2;
    let moon = triangulum_viewer::moon::MoonGenerator::new(seed);
    let (large_maria, mid_maria) = moon.mare_counts();
    let image = triangulum_viewer::ui::full_moon_map(seed, width, height);

    let path = Path::new(out);
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)?;
    }
    let file = BufWriter::new(std::fs::File::create(path)?);
    let mut encoder = png::Encoder::new(file, width as u32, height as u32);
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header()?;
    let mut rgb = Vec::with_capacity(width * height * 3);
    for pixel in image.pixels {
        rgb.extend_from_slice(&[pixel.r(), pixel.g(), pixel.b()]);
    }
    writer.write_image_data(&rgb)?;
    println!(
        "{} (seed {seed}: {large_maria} large maria, {mid_maria} mid maria)",
        path.display()
    );
    Ok(())
}
