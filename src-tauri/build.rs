fn main() {
    // the name of the release (`shown` under [package.metadata.maestro]; the Cargo version when there is none) is built in
    println!("cargo:rerun-if-changed=Cargo.toml");
    let manifest = std::fs::read_to_string("Cargo.toml").unwrap_or_default();
    let shown = manifest.lines().find_map(|l| l.trim().strip_prefix("shown").map(|v| v.trim_start_matches([' ', '=']).trim().trim_matches('"').to_string())).filter(|v| !v.is_empty()).unwrap_or_else(|| std::env::var("CARGO_PKG_VERSION").unwrap_or_default());
    println!("cargo:rustc-env=MAESTRO_VERSION={shown}");
    tauri_build::build()
}
