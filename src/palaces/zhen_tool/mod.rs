//! zhen_tool — Tool Registry (震三)

pub mod base;
pub mod browser_cdp;
pub mod builtin;
pub mod computer_driver;
#[cfg(feature = "mcp")]
pub mod mcp;
#[cfg(feature = "wasm-plugin")]
pub mod plugin_manager;
pub mod registry;
#[cfg(feature = "wasm-plugin")]
pub mod wasm_plugin;

pub use base::BaseTool;
pub use registry::ToolRegistry;
