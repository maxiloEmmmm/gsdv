//! Discovers user-owned Claude settings profiles and resolves them at launch.

use std::fs;
use std::path::PathBuf;

/// Returns the local profile directory. Example: `/home/me` -> `/home/me/.gsdv/claudes`.
fn directory() -> Option<PathBuf> {
    crate::home::home_dir().map(|home| home.join(".gsdv").join("claudes"))
}

/// Accepts one filename stem. Example: `work.eu` -> valid, `../work` -> invalid.
fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name == name.trim()
        && !matches!(name, "." | "..")
        && !name.contains(['/', '\\', ':'])
        && !name.chars().any(char::is_control)
}

/// Lists profile titles without loading credentials. Example: `x.json` -> `x`.
pub(super) fn names() -> Vec<String> {
    let Some(directory) = directory() else {
        return Vec::new();
    };
    let Ok(entries) = fs::read_dir(directory) else {
        return Vec::new();
    };
    let mut names = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().is_some_and(|ext| ext == "json")
            && let Some(name) = path.file_stem().and_then(|stem| stem.to_str())
            && valid_name(name)
            && path.is_file()
        {
            names.push(name.to_owned());
        }
    }
    names.sort_unstable();
    names
}

/// Resolves a selected profile to the argument Claude reads. Example: `x` -> `…/claudes/x.json`.
pub(super) fn settings_arg(name: &str) -> anyhow::Result<String> {
    anyhow::ensure!(valid_name(name), "Invalid Claude provider name");
    let directory =
        directory().ok_or_else(|| anyhow::anyhow!("Cannot locate Claude provider directory"))?;
    let path = directory.join(format!("{name}.json"));
    // A deleted profile must surface a launch error before Claude can fall back to another host.
    anyhow::ensure!(
        path.is_file(),
        "Claude provider file not found: {}",
        path.display()
    );
    // Claude validates the settings schema. Check the JSON object here so malformed files cannot
    // silently fall back to the default provider, and keep file content out of error messages.
    let file = fs::File::open(&path).map_err(|error| {
        anyhow::anyhow!("Cannot open Claude provider {}: {error}", path.display())
    })?;
    anyhow::ensure!(
        file.metadata()?.len() <= 2 * 1024 * 1024,
        "Claude provider file exceeds 2 MiB: {}",
        path.display()
    );
    let settings = serde_json::from_reader::<_, serde_json::Map<String, serde_json::Value>>(
        std::io::BufReader::new(file),
    );
    anyhow::ensure!(
        settings.is_ok(),
        "Claude provider must be a JSON object: {}",
        path.display()
    );
    path.into_os_string()
        .into_string()
        .map_err(|_| anyhow::anyhow!("Claude provider path is not UTF-8"))
}
