use std::env;
use std::fs;
use std::path::{Path, PathBuf};

/// Cargo build script entrypoint for embedding remote web assets.
fn main() {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("missing manifest dir"));
    let web_dist = manifest_dir.join("web").join("dist");
    let index_html = web_dist.join("index.html");

    println!("cargo:rerun-if-changed=web/dist");

    if !index_html.is_file() {
        panic!("web/dist/index.html not found; run `make build` or `cd web && pnpm build` first");
    }

    let mut assets = Vec::new();
    collect_web_assets(&web_dist, &web_dist, &mut assets);
    assets.sort_by(|left, right| left.route.cmp(&right.route));

    let output = generate_remote_web_assets(&assets);
    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("missing out dir"));
    fs::write(out_dir.join("remote_web_assets.rs"), output)
        .expect("failed to write remote web asset manifest");
}

/// Recursively collects files from web/dist for Rust include generation.
fn collect_web_assets(root: &Path, dir: &Path, assets: &mut Vec<WebAsset>) {
    let entries = fs::read_dir(dir).expect("failed to read web dist directory");
    for entry in entries {
        let entry = entry.expect("failed to read web dist entry");
        let path = entry.path();
        if path.is_dir() {
            collect_web_assets(root, &path, assets);
            continue;
        }
        if !path.is_file() {
            continue;
        }
        if let Some(asset) = web_asset_from_path(root, &path) {
            assets.push(asset);
        }
    }
}

/// Converts one dist file path into an embedded route asset.
fn web_asset_from_path(root: &Path, path: &Path) -> Option<WebAsset> {
    let relative = path
        .strip_prefix(root)
        .expect("web asset must be under dist root");
    let relative_route = relative
        .iter()
        .map(|part| part.to_string_lossy())
        .collect::<Vec<_>>()
        .join("/");
    let route = if relative_route == "index.html" {
        "/".to_string()
    } else {
        format!("/{relative_route}")
    };
    if route != "/" && !route.starts_with("/assets/") {
        return None;
    }
    Some(WebAsset {
        route,
        mime: mime_for_path(path),
        path: path
            .canonicalize()
            .expect("failed to canonicalize web asset path"),
    })
}

/// Generates Rust source included by the remote server module.
fn generate_remote_web_assets(assets: &[WebAsset]) -> String {
    let mut output = String::from("static REMOTE_WEB_ASSETS: &[RemoteWebAsset] = &[\n");
    for asset in assets {
        output.push_str("    RemoteWebAsset {\n");
        output.push_str(&format!(
            "        path: \"{}\",\n",
            rust_string(&asset.route)
        ));
        output.push_str(&format!("        mime: \"{}\",\n", asset.mime));
        output.push_str(&format!(
            "        bytes: include_bytes!(\"{}\"),\n",
            rust_string(&asset.path.to_string_lossy())
        ));
        output.push_str("    },\n");
    }
    output.push_str("];\n");
    output
}

/// Returns the HTTP content type for a static asset path.
fn mime_for_path(path: &Path) -> &'static str {
    match path.extension().and_then(|extension| extension.to_str()) {
        Some("html") => "text/html; charset=utf-8",
        Some("js") => "application/javascript; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("json") => "application/json; charset=utf-8",
        Some("png") => "image/png",
        Some("jpg") | Some("jpeg") => "image/jpeg",
        Some("svg") => "image/svg+xml",
        Some("ico") => "image/x-icon",
        Some("txt") => "text/plain; charset=utf-8",
        Some("woff") => "font/woff",
        Some("woff2") => "font/woff2",
        _ => "application/octet-stream",
    }
}

/// Escapes a string for generated Rust string literals.
fn rust_string(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

/// One web asset discovered from the Vite dist directory.
struct WebAsset {
    /// HTTP route served by the remote server.
    route: String,
    /// Content type returned with the asset.
    mime: &'static str,
    /// Absolute file path used by include_bytes.
    path: PathBuf,
}
