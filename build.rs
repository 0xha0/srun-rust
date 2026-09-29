fn main() {
    let target = std::env::var("TARGET").unwrap_or_else(|_| "unknown".to_string());
    println!("cargo:rustc-env=SRUN_TARGET={target}");
    // Optional compile-time default config path, e.g. for firmware builds:
    //   SRUN_DEFAULT_CONFIG=/jffs/srun/config.json cargo build --release
    println!("cargo:rerun-if-env-changed=SRUN_DEFAULT_CONFIG");
    if let Ok(p) = std::env::var("SRUN_DEFAULT_CONFIG") {
        if !p.is_empty() {
            println!("cargo:rustc-env=SRUN_DEFAULT_CONFIG={p}");
        }
    }
    println!("cargo:rerun-if-changed=build.rs");
}
