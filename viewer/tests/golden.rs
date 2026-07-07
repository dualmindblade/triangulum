//! Proves the Rust noise port is bit-compatible with planetgen/noise.py.
//! Goldens come from scripts/export_noise_tables.py.

use glam::DVec3;
use triangulum_viewer::noise;

#[test]
fn noise_matches_python() {
    let raw = include_str!("noise_golden.json");
    let g: serde_json::Value = serde_json::from_str(raw).unwrap();
    let points: Vec<DVec3> = g["points"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| {
            DVec3::new(
                p[0].as_f64().unwrap(),
                p[1].as_f64().unwrap(),
                p[2].as_f64().unwrap(),
            )
        })
        .collect();
    let mut checked = 0;
    for case in g["cases"].as_array().unwrap() {
        let kind = case["kind"].as_str().unwrap();
        let freq = case["freq"].as_f64().unwrap();
        let seed = case["seed"].as_i64().unwrap();
        let values = case["values"].as_array().unwrap();
        for (p, want) in points.iter().zip(values) {
            let want = want.as_f64().unwrap();
            let got = match kind {
                "gradient" => noise::gradient_noise(*p * freq, seed),
                "fbm" => noise::fbm(*p, case["octaves"].as_u64().unwrap() as u32, freq, seed),
                "ridged" => noise::ridged(*p, case["octaves"].as_u64().unwrap() as u32, freq, seed),
                other => panic!("unknown kind {other}"),
            };
            assert!(
                (got - want).abs() < 1e-9,
                "{kind} seed {seed}: got {got}, python says {want}"
            );
            checked += 1;
        }
    }
    assert_eq!(checked, 240);
}
