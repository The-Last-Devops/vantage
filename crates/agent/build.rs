fn main() {
    // Build id for the auto-update channel. Prefer a GIT_SHA build-arg (Docker
    // images have no .git), else read git locally, else "unknown".
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

    // Release channel: only "auto" (the :auto-update rolling tag) self-updates;
    // anything else ("stable"/"dev") never does. Set via Docker build-arg.
    let channel = std::env::var("VANTAGE_CHANNEL").unwrap_or_else(|_| "dev".into());
    println!("cargo:rustc-env=VANTAGE_CHANNEL={channel}");
    println!("cargo:rerun-if-env-changed=VANTAGE_CHANNEL");

    println!("cargo:rerun-if-changed=build.rs");
}
