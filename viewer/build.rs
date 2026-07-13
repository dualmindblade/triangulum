#[path = "build_identity.rs"]
mod build_identity;

fn main() {
    let viewer = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let repo = viewer
        .parent()
        .expect("viewer directory has a repository parent");
    build_identity::emit(viewer, repo);
}
