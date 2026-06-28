// ── Plugin Manager: WASM plugin discovery & lifecycle ──

use std::path::Path;
use std::sync::Arc;

use crate::palaces::zhen_tool::registry::ToolRegistry;
use crate::palaces::zhen_tool::wasm_plugin::{self, WasmPlugin};

/// Discovers and loads WASM plugins from a directory.
///
/// Each plugin lives in its own subdirectory containing:
/// - `plugin.toml` — manifest with plugin metadata and tool definitions
/// - `{plugin_name}.wasm` — the WASM module
pub struct PluginManager {
    engine: Arc<wasmtime::Engine>,
}

impl PluginManager {
    pub fn new() -> Result<Self, String> {
        let mut config = wasmtime::Config::new();
        config.async_support(true);
        config.epoch_interruption(true);
        config.consume_fuel(true);
        let engine = wasmtime::Engine::new(&config)
            .map_err(|e| format!("Failed to create wasmtime engine: {e}"))?;
        Ok(Self {
            engine: Arc::new(engine),
        })
    }

    /// Scan `plugins_dir` and register discovered tools into `registry`.
    ///
    /// Returns the number of tools registered.
    pub fn load_from_dir(
        &self,
        plugins_dir: &Path,
        registry: &mut ToolRegistry,
    ) -> Result<usize, String> {
        if !plugins_dir.is_dir() {
            return Ok(0);
        }

        let mut count = 0;
        let entries = std::fs::read_dir(plugins_dir)
            .map_err(|e| format!("Cannot read plugins dir {}: {e}", plugins_dir.display()))?;

        for entry in entries {
            let entry = match entry {
                Ok(e) => e,
                Err(_) => continue,
            };

            let dir = entry.path();
            if !dir.is_dir() {
                continue;
            }

            match self.load_plugin(&dir, registry) {
                Ok(n) => {
                    if n > 0 {
                        tracing::info!(
                            "Loaded plugin '{}' with {n} tool(s)",
                            dir.file_name()
                                .map(|n| n.to_string_lossy())
                                .unwrap_or_default()
                        );
                    }
                    count += n;
                }
                Err(e) => {
                    tracing::warn!("Failed to load plugin from {}: {e}", dir.display());
                }
            }
        }

        Ok(count)
    }

    fn load_plugin(&self, dir: &Path, registry: &mut ToolRegistry) -> Result<usize, String> {
        let manifest = wasm_plugin::load_manifest(dir)?;
        let wasm_path = wasm_plugin::wasm_path(dir, &manifest);

        if !wasm_path.exists() {
            return Err(format!("WASM file not found: {}", wasm_path.display()));
        }

        let wasm_bytes = std::fs::read(&wasm_path)
            .map_err(|e| format!("Cannot read {}: {e}", wasm_path.display()))?;

        let module = wasmtime::Module::new(&self.engine, &wasm_bytes)
            .map_err(|e| format!("WASM compile error for {}: {e}", wasm_path.display()))?;

        let module = Arc::new(module);
        let plugin_name = manifest.plugin.name.clone();
        let plugin_version = manifest.plugin.version.clone();

        for def in &manifest.tools {
            let tool = WasmPlugin::new(
                def.clone(),
                module.clone(),
                self.engine.clone(),
                plugin_name.clone(),
                plugin_version.clone(),
            );
            registry.register_external(Arc::new(tool));
        }

        Ok(manifest.tools.len())
    }
}
