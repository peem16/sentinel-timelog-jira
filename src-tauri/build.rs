use std::collections::HashMap;
use std::io::Write;
use std::path::Path;

const CRED_VARS: [&str; 6] = [
    "TIMELOG_ATLASSIAN_CLIENT_ID",
    "TIMELOG_ATLASSIAN_CLIENT_SECRET",
    "TIMELOG_GOOGLE_CLIENT_ID",
    "TIMELOG_GOOGLE_CLIENT_SECRET",
    "TIMELOG_GITHUB_CLIENT_ID",
    "TIMELOG_GITHUB_CLIENT_SECRET",
];

/// Parse `KEY=VALUE` lines from .env files (project root wins over src-tauri).
fn read_env_files() -> HashMap<String, String> {
    let mut map = HashMap::new();
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR");
    let manifest = Path::new(&manifest_dir);
    // absolute paths — cargo's rerun-if-changed is unreliable with `..` relatives
    let candidates = [
        manifest.parent().unwrap_or(manifest).join(".env"), // project root
        manifest.join(".env"),                              // src-tauri/
    ];
    for path in &candidates {
        // Always watch the path — even if absent now, cargo must rebuild when
        // it is later created (or deleted). Emitting this only when the file
        // exists would miss the create case entirely.
        println!("cargo:rerun-if-changed={}", path.display());
        if !path.exists() {
            continue;
        }
        let Ok(contents) = std::fs::read_to_string(path) else {
            continue;
        };
        for line in contents.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if let Some((k, v)) = line.split_once('=') {
                let k = k.trim().to_string();
                let v = v.trim().trim_matches('"').trim_matches('\'').to_string();
                map.entry(k).or_insert(v); // first file (project root) wins
            }
        }
    }
    map
}

fn main() {
    let file_vals = read_env_files();

    // Generate a constants file consumed via include! in oauth.rs. Writing the
    // file (rather than cargo:rustc-env) makes cargo recompile the crate through
    // the depfile whenever the baked-in values change.
    let out_dir = std::env::var("OUT_DIR").expect("OUT_DIR not set");
    let dest = Path::new(&out_dir).join("credentials.rs");
    let mut f = std::fs::File::create(&dest).expect("cannot write credentials.rs");

    for var in CRED_VARS {
        println!("cargo:rerun-if-env-changed={var}");
        // a real shell env var wins over the .env file
        let val = std::env::var(var)
            .ok()
            .filter(|s| !s.trim().is_empty())
            .or_else(|| file_vals.get(var).cloned().filter(|s| !s.is_empty()));
        let konst = format!("EMBED_{}", var.trim_start_matches("TIMELOG_"));
        match val {
            Some(v) => writeln!(f, "pub const {konst}: Option<&str> = Some({v:?});").unwrap(),
            None => writeln!(f, "pub const {konst}: Option<&str> = None;").unwrap(),
        }
    }

    tauri_build::build()
}
