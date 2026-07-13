#[path = "../build_identity.rs"]
mod build_identity;

fn main() {
    let server = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let viewer = server
        .parent()
        .expect("server directory has a viewer parent");
    let repo = viewer
        .parent()
        .expect("viewer directory has a repository parent");
    build_identity::emit(viewer, repo);
}
