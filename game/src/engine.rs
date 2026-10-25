//! The engine context shared by the app, the bench, the harness and the
//! render tools: planet, registry, bake, generator, edits, LOD policy,
//! lighting, and the notable places on the planet.

use crate::camera::Camera;
use crate::grid::{Planet, WorldConfig};
use crate::mesh::MeshOptions;
use crate::stream::{LodParams, Streamer};
use crate::world::{BlockId, EditStore, Registry, World};
use crate::worldgen::bake::Bake;
use crate::worldgen::{Generator, Marker};
use glam::DVec3;
use std::sync::Arc;

/// Notable places found by scanning the bake.
#[derive(Clone, Copy, Debug)]
pub struct Places {
    /// Highest land texel: direction and elevation (km, Earth scale).
    pub peak: (DVec3, f64),
    /// Deepest ocean texel.
    pub trench: (DVec3, f64),
    /// Steepest coastal texel (|gradient| km per texel at |elev| < 0.3 km).
    pub coast: (DVec3, f64),
}

pub fn find_places(bake: &dyn Bake) -> Places {
    let mut peak = (DVec3::Z, f64::MIN);
    let mut trench = (DVec3::Z, f64::MAX);
    let mut coast = (DVec3::Z, f64::MIN);
    bake.for_each_texel(&mut |dir, elev, ocean, grad| {
        let e = elev as f64;
        if !ocean && e > peak.1 {
            peak = (dir, e);
        }
        if ocean && e < trench.1 {
            trench = (dir, e);
        }
        if e.abs() < 0.3 && (grad as f64) > coast.1 {
            coast = (dir, grad as f64);
        }
    });
    Places { peak, trench, coast }
}

#[derive(Clone, Debug)]
pub struct Preset {
    pub name: String,
    pub dir: DVec3,
    pub alt_agl: f64,
    pub heading_deg: f64,
    pub pitch_deg: f64,
}

pub struct Engine {
    pub config: WorldConfig,
    pub planet: Planet,
    pub registry: Arc<Registry>,
    pub bake: Arc<dyn Bake>,
    pub generator: Arc<Generator>,
    /// The edit overlay, shared by every World built from this engine.
    pub edits: Arc<EditStore>,
    /// Generator + edits: what streaming, physics and picking read.
    pub world: Arc<World>,
    pub lod: LodParams,
    pub mesh_opts: MeshOptions,
    pub sun: DVec3,
    pub colours: Vec<[f32; 4]>,
    pub quad_mb: u64,
    pub slot_capacity: u32,
    pub water_alpha: f32,
    pub places: Places,
    pub skirt: u32,
    pub hide_water: bool,
    pub markers_on: bool,
    pub markers: Vec<Marker>,
    /// Blocks on keys 1-9.
    pub hotbar: Vec<BlockId>,
}

/// The prototype's two benchmark anchors (face, gnomonic a, b).
pub const HILLS: (u8, f64, f64) = (2, 0.52, -0.09);
pub const MOUNTAINS: (u8, f64, f64) = (2, 0.4467, -0.2290);

pub fn gnomonic_dir(face: u8, a: f64, b: f64) -> DVec3 {
    let (axis, right, up) = cubesphere::FACES[face as usize];
    (axis + a * right + b * up).normalize()
}

impl Engine {
    #[allow(clippy::too_many_arguments)]
    pub fn build(
        config: WorldConfig,
        registry: Arc<Registry>,
        bake: Arc<dyn Bake>,
        edits: Arc<EditStore>,
        lod: LodParams,
        mesh_opts: MeshOptions,
        skirt: u32,
        hide_water: bool,
        markers: Vec<Marker>,
        markers_on: bool,
    ) -> Result<Self, String> {
        let planet = Planet::try_new(config.block_size_m, config.world_height, config.sea_level, config.columns_per_face)?;
        let vscale = config.vertical_scale.unwrap_or(planet.radius / 6_371_000.0);
        // TODO(landforms): when Opus's landforms v2 lands it adds a `landform`
        // dial to WorldConfig; pass it here (and in `rebuild`) explicitly,
        // next to roughness, and drop the process-wide default main.rs sets.
        let mut generator = Generator::new(planet, bake.clone(), config.seed, vscale, config.roughness, &registry)?;
        generator.skirt = skirt;
        generator.hide_water = hide_water;
        if markers_on {
            generator.markers = markers.clone();
        }
        let generator = Arc::new(generator);
        let places = find_places(bake.as_ref());
        // Mid-morning sun from the south-east, relative to the hills anchor (fixed in world space).
        let hills = gnomonic_dir(HILLS.0, HILLS.1, HILLS.2);
        let (north, east, up) = Camera::local_frame(hills);
        let sun = (up * 0.75 + north * -0.35 + east * 0.55).normalize();
        let hotbar = registry.default_hotbar();
        Ok(Self {
            colours: registry.colour_table(),
            world: Arc::new(World::new(generator.clone(), registry.clone(), edits.clone())),
            config,
            planet,
            registry,
            bake,
            generator,
            edits,
            lod,
            mesh_opts,
            sun,
            quad_mb: 512,
            slot_capacity: 1 << 17,
            water_alpha: 0.62,
            places,
            skirt,
            hide_water,
            markers_on,
            markers,
            hotbar,
        })
    }

    pub fn vscale(&self) -> f64 {
        self.generator.vscale
    }

    /// Sun direction for a camera position: mid-morning from the south-east
    /// in the camera's own tangent frame, so lighting reads the same at
    /// every latitude. The real sun and sky arrive with M1c.
    pub fn sun_at(&self, pos: DVec3) -> DVec3 {
        let (north, east, up) = Camera::local_frame(pos);
        (up * 0.75 + north * -0.35 + east * 0.55).normalize()
    }

    /// A copy with different debug flags (the generator is rebuilt; the bake
    /// and the edits are shared).
    pub fn with_flags(&self, hide_water: bool, markers_on: bool) -> Engine {
        self.with_markers(self.markers.clone(), hide_water, markers_on)
    }

    pub fn with_markers(&self, markers: Vec<Marker>, hide_water: bool, markers_on: bool) -> Engine {
        self.rebuild(markers, hide_water, markers_on, self.edits.clone())
    }

    /// The same engine over another edit store (reloading a save).
    pub fn with_edits(&self, edits: Arc<EditStore>) -> Engine {
        self.rebuild(self.markers.clone(), self.hide_water, self.markers_on, edits)
    }

    fn rebuild(&self, markers: Vec<Marker>, hide_water: bool, markers_on: bool, edits: Arc<EditStore>) -> Engine {
        let mut generator = Generator::new(self.planet, self.bake.clone(), self.config.seed, self.generator.vscale, self.config.roughness, &self.registry).expect("generator");
        generator.skirt = self.skirt;
        generator.hide_water = hide_water;
        if markers_on {
            generator.markers = markers.clone();
        }
        let generator = Arc::new(generator);
        Engine {
            config: self.config.clone(),
            planet: self.planet,
            registry: self.registry.clone(),
            bake: self.bake.clone(),
            world: Arc::new(World::new(generator.clone(), self.registry.clone(), edits.clone())),
            generator,
            edits,
            lod: self.lod,
            mesh_opts: self.mesh_opts,
            sun: self.sun,
            colours: self.colours.clone(),
            quad_mb: self.quad_mb,
            slot_capacity: self.slot_capacity,
            water_alpha: self.water_alpha,
            places: self.places,
            skirt: self.skirt,
            hide_water,
            markers_on,
            markers,
            hotbar: self.hotbar.clone(),
        }
    }

    pub fn streamer(&self) -> Streamer {
        Streamer::new(self.world.clone(), self.registry.clone(), self.lod, self.mesh_opts)
    }

    /// Ground height under a direction (m above sea level, water = 0).
    pub fn ground(&self, dir: DVec3) -> f64 {
        self.generator.ground_height_m(dir)
    }

    pub fn camera_at(&self, dir: DVec3, alt_agl: f64, heading_deg: f64, pitch_deg: f64) -> Camera {
        let h = self.ground(dir);
        Camera { pos: dir * (self.planet.radius + h + alt_agl), heading: heading_deg.to_radians(), pitch: pitch_deg.to_radians(), fov_y: 60f64.to_radians() }
    }

    /// Camera at an absolute altitude above sea level.
    pub fn camera_asl(&self, dir: DVec3, alt_asl: f64, heading_deg: f64, pitch_deg: f64) -> Camera {
        Camera { pos: dir * (self.planet.radius + alt_asl), heading: heading_deg.to_radians(), pitch: pitch_deg.to_radians(), fov_y: 60f64.to_radians() }
    }

    /// Direction `dist_m` along `heading_deg` from `dir` over the sphere.
    pub fn offset_dir(&self, dir: DVec3, heading_deg: f64, dist_m: f64) -> DVec3 {
        let (north, east, _) = Camera::local_frame(dir);
        let t = north * heading_deg.to_radians().cos() + east * heading_deg.to_radians().sin();
        let th = dist_m / self.planet.radius;
        (dir * th.cos() + t * th.sin()).normalize()
    }

    /// Heading (deg) from `from` toward `to`, and the great-circle distance (m).
    pub fn heading_to(&self, from: DVec3, to: DVec3) -> (f64, f64) {
        let (north, east, _) = Camera::local_frame(from);
        let d = to - from;
        let heading = d.dot(east).atan2(d.dot(north)).to_degrees();
        let dist = from.dot(to).clamp(-1.0, 1.0).acos() * self.planet.radius;
        (heading, dist)
    }

    pub fn presets(&self) -> Vec<Preset> {
        let hills = gnomonic_dir(HILLS.0, HILLS.1, HILLS.2);
        let mountains = gnomonic_dir(MOUNTAINS.0, MOUNTAINS.1, MOUNTAINS.2);
        let (peak_dir, _) = self.places.peak;
        let peak_view = self.offset_dir(peak_dir, 200.0, 3500.0);
        let (peak_heading, _) = self.heading_to(peak_view, peak_dir);
        let coast = self.places.coast.0;
        vec![
            Preset { name: "hills".into(), dir: hills, alt_agl: 1.7, heading_deg: -60.0, pitch_deg: 0.0 },
            Preset { name: "mountains".into(), dir: mountains, alt_agl: 1.7, heading_deg: -155.0, pitch_deg: 0.0 },
            Preset { name: "highest peak".into(), dir: peak_view, alt_agl: 1.7, heading_deg: peak_heading, pitch_deg: 5.0 },
            Preset { name: "steep coast".into(), dir: coast, alt_agl: 300.0, heading_deg: 45.0, pitch_deg: -20.0 },
            Preset { name: "orbit".into(), dir: hills, alt_agl: 500_000.0, heading_deg: -60.0, pitch_deg: -89.0 },
        ]
    }
}
