#![allow(unsafe_code)]
use rusteditor_plugin_host::PluginHost;
use std::path::Path;

pub fn load_trusted_plugins(host: &mut PluginHost, path: &Path) -> Result<Vec<String>, String> {
    // SAFETY: Only the explicit --plugin-dir or executable-adjacent installation
    // directory is used. Installing/enabling a native plugin grants it process
    // privileges and requires trust in its ABI, memory and lifecycle behavior.
    let report = unsafe { host.load_directory(path) }.map_err(|e| e.to_string())?;
    Ok(report.errors.into_iter().map(|e| e.to_string()).collect())
}
