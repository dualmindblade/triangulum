//! triangulum-viewer library: cube-sphere LOD rendering over the planetgen
//! dataset. The binary in main.rs is a thin shell over these modules; tests
//! (notably the Python-parity noise goldens) link against the library.

pub mod camera;
pub mod moon;
pub mod noise;
pub mod noise_grad;
pub mod orbits;
pub mod planet;
pub mod player;
pub mod renderer;
pub mod rivers;
pub mod terrain;
pub mod ui;
pub mod voxel;
pub mod weather;
