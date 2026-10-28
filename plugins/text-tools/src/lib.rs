//! Example native extension; the desktop never links this crate.
use rusteditor_plugin_sdk::{
    Host, Plugin,
    api::{Status, UNSUPPORTED},
    export_plugin,
};

struct TextTools {
    host: Host,
}

impl Plugin for TextTools {
    const ID: &'static str = "org.rusteditor.text-tools";
    const VERSION: &'static str = env!("CARGO_PKG_VERSION");

    fn create(host: Host) -> Result<Self, Status> {
        Ok(Self { host })
    }

    fn initialize(&mut self) -> Result<(), Status> {
        self.host
            .register_command("org.rusteditor.text-tools.uppercase", "Uppercase selection")
    }

    fn invoke(&mut self, command: &str) -> Result<(), Status> {
        if command != "org.rusteditor.text-tools.uppercase" {
            return Err(UNSUPPORTED);
        }
        let snapshot = self.host.selection()?;
        self.host
            .replace_selection(&snapshot, &snapshot.text.to_uppercase())
    }

    fn shutdown(&mut self) -> Result<(), Status> {
        self.host
            .unregister_command("org.rusteditor.text-tools.uppercase")
    }
}

export_plugin!(TextTools);
