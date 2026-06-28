use std::fs;
use std::path::Path;

// rust-embed (`#[folder = "../../frontend/dist"]` in spa.rs) needs that directory
// to exist at compile time. The CI cargo jobs (fmt/clippy/test/build) don't run
// `npm run build`, and `vite build` deletes the tracked `.gitkeep`, so the folder
// can be absent — which makes the embed macro skip generating `Assets::get`.
// Ensure the folder (with a placeholder) exists so the crate always compiles; the
// real SPA assets are produced by `npm run build` before the Docker image build.
fn main() {
    let dist = Path::new("../../frontend/dist");
    let _ = fs::create_dir_all(dist);
    let keep = dist.join(".gitkeep");
    if !keep.exists() {
        let _ = fs::write(&keep, b"");
    }

    // Build metadata for the About page + the auto-update build id. Prefer a
    // GIT_SHA build-arg (Docker images have no .git), else read git locally,
    // else "unknown".
    let git_sha = std::env::var("GIT_SHA")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .or_else(|| {
            std::process::Command::new("git")
                .args(["rev-parse", "--short", "HEAD"])
                .output()
                .ok()
                .filter(|o| o.status.success())
                .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        })
        .unwrap_or_else(|| "unknown".into());
    println!("cargo:rustc-env=GIT_SHA={git_sha}");
    println!("cargo:rerun-if-env-changed=GIT_SHA");

    // Release channel: "auto" (the :auto-update rolling tag) self-updates; anything
    // else never does. Set via Docker build-arg; defaults to "dev" for local builds.
    let channel = std::env::var("VANTAGE_CHANNEL").unwrap_or_else(|_| "dev".into());
    println!("cargo:rustc-env=VANTAGE_CHANNEL={channel}");
    println!("cargo:rerun-if-env-changed=VANTAGE_CHANNEL");
    let build_date = std::process::Command::new("date")
        .args(["-u", "+%Y-%m-%d %H:%M UTC"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default();
    println!("cargo:rustc-env=BUILD_DATE={build_date}");

    println!("cargo:rerun-if-changed=build.rs");
}
