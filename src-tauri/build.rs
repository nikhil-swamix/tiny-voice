fn main() {
    if std::env::var("PROFILE").as_deref() == Ok("release") && tauri_build::is_dev() {
        panic!("Release builds require --features custom-protocol. Use npm run desktop.");
    }
    tauri_build::build()
}
