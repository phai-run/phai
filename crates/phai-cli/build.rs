//! Ensure `web/dist` exists before the crate compiles.
//!
//! `serve_assets.rs` embeds `web/dist` via `include_dir!`, which requires the
//! directory to exist at compile time. The built SPA is **not** committed (it is
//! a generated artifact; committing the minified bundle pollutes the
//! source-quality gate and bloats the repo). Instead:
//!
//! - CI / release builds run `pnpm build` in `web/` first, producing the real
//!   bundle, which this script leaves untouched.
//! - A plain `cargo build` with no web build (e.g. a contributor touching only
//!   Rust) gets a placeholder `index.html` so the build stays pure-Rust — no
//!   Node toolchain required on the user's machine (ADR-0001).

use std::fs;
use std::path::Path;

const PLACEHOLDER: &str = r#"<!doctype html>
<html lang="pt-BR"><head><meta charset="utf-8"><title>phai</title></head>
<body style="font-family:system-ui;background:#08060B;color:#F1F5F9;padding:40px">
<h1>&#966; phai</h1>
<p>O app web não foi compilado neste binário.</p>
<p>Rode <code>pnpm -C crates/phai-cli/web install &amp;&amp; pnpm -C crates/phai-cli/web build</code>
e recompile.</p>
</body></html>
"#;

fn main() {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR");
    let dist = Path::new(&manifest_dir).join("web/dist");
    let index = dist.join("index.html");

    // Recompile when the built bundle changes so include_dir! picks it up.
    println!("cargo:rerun-if-changed=web/dist");

    if !index.exists() {
        fs::create_dir_all(&dist).expect("create web/dist");
        fs::write(&index, PLACEHOLDER).expect("write placeholder index.html");
        println!(
            "cargo:warning=web/dist not built — embedding a placeholder. \
             Run `pnpm -C crates/phai-cli/web build` for the real UI."
        );
    }
}
