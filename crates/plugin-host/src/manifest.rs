use crate::PluginError;
use rusteditor_plugin_api::{
    ABI_MAJOR, ABI_MINOR, CAP_ALL, CAP_COMMANDS, CAP_READ_SELECTION, CAP_REPLACE_TEXT, TARGET,
};
use serde::Deserialize;
use std::{
    fs,
    path::{Component, Path, PathBuf},
};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Manifest {
    pub id: String,
    pub name: String,
    pub version: String,
    pub abi_major: u16,
    pub abi_minor: u16,
    pub target: String,
    pub library: String,
    pub capabilities: Vec<String>,
}

pub(crate) struct Candidate {
    pub manifest: Manifest,
    pub path: PathBuf,
    pub capabilities: u64,
}

pub(crate) fn identifier(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && !id.starts_with('.')
        && !id.ends_with('.')
        && !id.contains("..")
        && id
            .bytes()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'.' || c == b'-')
}

pub(crate) fn read(root: &Path, directory: &Path) -> Result<Candidate, PluginError> {
    let fail = |message: &str| PluginError(format!("{}: {message}", directory.display()));
    let directory = directory.canonicalize()?;
    if !directory.starts_with(root) || !directory.is_dir() {
        return Err(fail("plugin directory escapes root"));
    }
    let manifest_path = directory.join("plugin.toml").canonicalize()?;
    if !manifest_path.starts_with(&directory) {
        return Err(fail("manifest escapes plugin directory"));
    }
    if fs::metadata(&manifest_path)?.len() > 64 * 1024 {
        return Err(fail("manifest is too large"));
    }
    let manifest: Manifest =
        toml::from_str(&fs::read_to_string(manifest_path)?).map_err(|e| fail(&e.to_string()))?;
    if !identifier(&manifest.id)
        || manifest.name.trim().is_empty()
        || manifest.name.len() > 256
        || manifest.version.is_empty()
        || manifest.version.len() > 64
    {
        return Err(fail("invalid plugin identity"));
    }
    if manifest.abi_major != ABI_MAJOR || manifest.abi_minor > ABI_MINOR {
        return Err(fail("unsupported ABI version"));
    }
    if manifest.target != TARGET {
        return Err(fail("target does not match host"));
    }
    // v1 accepts a single library filename, with no directory components.
    let path = Path::new(&manifest.library);
    let components: Vec<_> = path.components().collect();
    if components.len() != 1
        || !matches!(components[0], Component::Normal(_))
        || manifest.library.contains(['/', '\\', ':'])
    {
        return Err(fail("library must be a confined filename"));
    }
    if path.extension().and_then(|x| x.to_str()) != Some(std::env::consts::DLL_EXTENSION) {
        return Err(fail("incorrect native library extension"));
    }
    let library = directory.join(path).canonicalize()?;
    if !library.starts_with(&directory) || !library.is_file() {
        return Err(fail("library escapes plugin directory"));
    }
    let mut capabilities = 0;
    for capability in &manifest.capabilities {
        let bit = match capability.as_str() {
            "read-selection" => CAP_READ_SELECTION,
            "replace-text" => CAP_REPLACE_TEXT,
            "commands" => CAP_COMMANDS,
            _ => return Err(fail("unknown capability")),
        };
        if capabilities & bit != 0 {
            return Err(fail("duplicate capability"));
        }
        capabilities |= bit;
    }
    if capabilities & !CAP_ALL != 0 || capabilities & CAP_COMMANDS == 0 {
        return Err(fail("text plugins must declare commands"));
    }
    Ok(Candidate {
        manifest,
        path: library,
        capabilities,
    })
}
