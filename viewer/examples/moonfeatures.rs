//! Print deterministic lunar landmarks for headless evidence scripts.

fn print_group(name: &str, probes: &[triangulum_viewer::moon::MoonFeatureProbe]) {
    println!("{name} ({})", probes.len());
    for p in probes {
        println!(
            "  lat {:+9.4} lon {:+10.4} radius {:7.4} deg",
            p.lat_deg, p.lon_deg, p.radius_deg
        );
    }
}

fn direction(lat_deg: f64, lon_deg: f64) -> glam::DVec3 {
    let (lat, lon) = (lat_deg.to_radians(), lon_deg.to_radians());
    glam::DVec3::new(lat.cos() * lon.cos(), lat.cos() * lon.sin(), lat.sin())
}

fn lat_lon(p: glam::DVec3) -> (f64, f64) {
    (p.z.asin().to_degrees(), p.y.atan2(p.x).to_degrees())
}

fn print_surviving_rays(
    moon: &triangulum_viewer::moon::MoonGenerator,
    carriers: &[triangulum_viewer::moon::MoonFeatureProbe],
) {
    println!("surviving ray maxima");
    for carrier in carriers {
        let center = direction(carrier.lat_deg, carrier.lon_deg);
        let reference = if center.z.abs() < 0.88 {
            glam::DVec3::Z
        } else {
            glam::DVec3::X
        };
        let major = (reference - center * reference.dot(center)).normalize();
        let minor = center.cross(major).normalize();
        let mut best = (0.0f64, center, 0.0f64);
        for radial_step in 0..40 {
            let radial = 1.35 + radial_step as f64 * 0.30;
            for bearing_step in 0..144 {
                let bearing = bearing_step as f64 * std::f64::consts::TAU / 144.0;
                let tangent = major * bearing.cos() + minor * bearing.sin();
                let theta = carrier.radius_deg.to_radians() * radial;
                let p = center * theta.cos() + tangent * theta.sin();
                let sample = moon.sample(p);
                if sample.ray > best.0 {
                    best = (sample.ray, p, sample.albedo);
                }
            }
        }
        let (lat, lon) = lat_lon(best.1);
        println!(
            "  carrier {:+8.3} {:+9.3} -> ray lat {lat:+9.4} lon {lon:+10.4} strength {:.3} albedo {:.3}",
            carrier.lat_deg, carrier.lon_deg, best.0, best.2
        );
    }
}

fn print_radius_histogram(moon: &triangulum_viewer::moon::MoonGenerator) {
    let radii = moon.crater_radius_samples_on_face(0);
    let lo = radii.iter().copied().fold(f64::INFINITY, f64::min);
    let hi = radii.iter().copied().fold(0.0f64, f64::max);
    let (log_lo, log_hi) = (lo.log10(), hi.log10());
    const BINS: usize = 36;
    let mut counts = [0usize; BINS];
    for radius in &radii {
        let t = ((radius.log10() - log_lo) / (log_hi - log_lo)).clamp(0.0, 1.0);
        let bin = ((t * BINS as f64) as usize).min(BINS - 1);
        counts[bin] += 1;
    }
    println!("crater radius histogram face=0 samples={}", radii.len());
    println!("radius_histogram_csv lower_deg,upper_deg,count");
    for (bin, count) in counts.into_iter().enumerate() {
        let lower = 10f64.powf(log_lo + (log_hi - log_lo) * bin as f64 / BINS as f64);
        let upper = 10f64.powf(log_lo + (log_hi - log_lo) * (bin + 1) as f64 / BINS as f64);
        println!("radius_histogram_csv {lower:.9},{upper:.9},{count}");
    }
}

fn main() {
    let seed = std::env::args()
        .nth(1)
        .and_then(|v| v.parse().ok())
        .unwrap_or(42);
    let moon = triangulum_viewer::moon::MoonGenerator::new(seed);
    let (large, mid) = moon.mare_counts();
    println!("seed {seed}: maria large={large} mid={mid}");
    let counts = moon.crater_probe_counts();
    println!("face-0 crater octave counts: {counts:?}");
    print_group("largest craters", &moon.largest_crater_probes());
    let carriers = moon.ray_carrier_probes();
    print_group("ray carriers", &carriers);
    print_surviving_rays(&moon, &carriers);
    print_group("mare-edge craters", &moon.mare_edge_crater_probes());
    print_group(
        "rim-straddling craters",
        &moon.rim_straddling_crater_probes(),
    );
    print_radius_histogram(&moon);
}
