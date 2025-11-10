use nu_plugin::{serve_plugin, MsgPackSerializer, Plugin, PluginCommand};

mod commands;
pub use commands::*;

pub struct ToonPlugin;

impl Plugin for ToonPlugin {
    fn version(&self) -> String {
        // This automatically uses the version of your package from Cargo.toml as the plugin version
        // sent to Nushell
        env!("CARGO_PKG_VERSION").into()
    }

    fn commands(&self) -> Vec<Box<dyn PluginCommand<Plugin = Self>>> {
        vec![
            // Commands should be added here
            Box::new(FromToon),
            Box::new(ToToon),
        ]
    }
}

fn main() {
    serve_plugin(&ToonPlugin, MsgPackSerializer);
}
