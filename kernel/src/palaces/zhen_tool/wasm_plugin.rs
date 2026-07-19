use crate::error::ToolError;
use std::sync::Arc;
// ── WASM Plugin: dynamic .wasm tool loading via wasmtime ──

use std::path::{Path, PathBuf};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::palaces::zhen_tool::base::BaseTool;
use crate::stems::action::ExecContext;
use crate::stems::CeremoniesIntent;
use wasmtime_wasi::preview1::WasiP1Ctx;

// ── Plugin Manifest ────────────────────────────────────────

/// Deserialized from `plugin.toml` beside the .wasm file.
#[derive(Debug, Clone, Deserialize)]
pub struct PluginManifest {
    pub plugin: PluginMeta,
    #[serde(default)]
    pub tools: Vec<PluginToolDef>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PluginMeta {
    pub name: String,
    #[serde(default)]
    pub version: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PluginToolDef {
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub category: String,
    #[serde(default)]
    pub concurrency_safe: bool,
    #[serde(default)]
    pub parameters_schema: Option<Value>,
}

// ── WASI I/O Protocol ──────────────────────────────────────

/// Sent to the plugin via stdin (JSON line).
#[derive(Serialize)]
struct PluginRequest<'a> {
    tool: &'a str,
    input: &'a Value,
}

/// Received from the plugin via stdout (JSON line).
#[derive(Deserialize)]
struct PluginResponse {
    #[serde(default)]
    stdout: String,
    #[serde(default)]
    stderr: String,
    #[serde(default)]
    #[allow(dead_code)]
    exit_code: i32,
    #[serde(default)]
    error: Option<String>,
}

// ── WasmPlugin ─────────────────────────────────────────────

/// A `BaseTool` backed by a `.wasm` module run inside wasmtime.
pub struct WasmPlugin {
    meta: PluginToolDef,
    /// Pre-compiled WASM module, shared across invocations.
    module: Arc<wasmtime::Module>,
    /// wasmtime engine, shared across plugins.
    engine: Arc<wasmtime::Engine>,
    plugin_name: String,
    plugin_version: String,
}

impl WasmPlugin {
    pub fn new(
        def: PluginToolDef,
        module: Arc<wasmtime::Module>,
        engine: Arc<wasmtime::Engine>,
        plugin_name: String,
        plugin_version: String,
    ) -> Self {
        Self {
            meta: def,
            module,
            engine,
            plugin_name,
            plugin_version,
        }
    }
}

#[async_trait]
impl BaseTool for WasmPlugin {
    fn name(&self) -> &str {
        &self.meta.name
    }

    fn description(&self) -> String {
        format!(
            "{} (plugin: {} v{})",
            self.meta.description, self.plugin_name, self.plugin_version
        )
    }

    fn category(&self) -> &str {
        if self.meta.category.is_empty() {
            "wasm"
        } else {
            &self.meta.category
        }
    }

    fn ceremony(&self) -> CeremoniesIntent {
        CeremoniesIntent::Geng
    }

    fn parameters_schema(&self) -> Value {
        self.meta
            .parameters_schema
            .clone()
            .unwrap_or(serde_json::json!({
                "type": "object",
                "properties": {},
                "additionalProperties": true
            }))
    }

    fn is_concurrency_safe(&self) -> bool {
        self.meta.concurrency_safe
    }

    async fn execute(&self, input: Value, _ctx: &ExecContext) -> Result<String, ToolError> {
        let request = PluginRequest {
            tool: &self.meta.name,
            input: &input,
        };
        let request_json = serde_json::to_string(&request)
            .map_err(|e| format!("Plugin request serialization error: {e}"))?;

        // Set up WASI preview1 context with in-memory stdin/stdout
        let stdin_data = request_json.into_bytes();
        let stdin_pipe = wasmtime_wasi::pipe::MemoryInputPipe::new(stdin_data);
        let stdout_pipe = wasmtime_wasi::pipe::MemoryOutputPipe::new(64 * 1024);

        let wasi = wasmtime_wasi::WasiCtxBuilder::new()
            .stdin(stdin_pipe)
            .stdout(stdout_pipe.clone())
            .build_p1();

        let mut store: wasmtime::Store<WasiP1Ctx> = wasmtime::Store::new(&self.engine, wasi);
        let mut linker: wasmtime::Linker<WasiP1Ctx> = wasmtime::Linker::new(&self.engine);

        wasmtime_wasi::preview1::add_to_linker_async(&mut linker, |wasi| wasi)
            .map_err(|e| format!("Linker error: {e}"))?;

        // Set resource limits
        store
            .set_fuel(10_000_000)
            .map_err(|e| format!("Fuel limit error: {e}"))?;
        store.set_epoch_deadline(1);

        // Instantiate and call _start
        let instance: wasmtime::Instance = linker
            .instantiate_async(&mut store, &self.module)
            .await
            .map_err(|e| format!("Plugin instantiation error: {e}"))?;

        let start = instance
            .get_typed_func::<(), ()>(&mut store, "_start")
            .map_err(|e| format!("Plugin entry point error: {e}"))?;

        start
            .call_async(&mut store, ())
            .await
            .map_err(|e| format!("Plugin execution error: {e}"))?;

        drop(store);

        let output_bytes = stdout_pipe
            .try_into_inner()
            .ok_or_else(|| "Failed to read plugin stdout".to_string())?;
        let output_str = String::from_utf8_lossy(&output_bytes).to_string();
        let response: PluginResponse = serde_json::from_str(&output_str)
            .map_err(|e| format!("Plugin response parse error: {e}. Output: {output_str}"))?;

        if let Some(err) = response.error {
            return Err(format!("Plugin error: {err}").into());
        }

        Ok(if response.stderr.is_empty() {
            response.stdout
        } else {
            format!("stdout:\n{}\nstderr:\n{}", response.stdout, response.stderr)
        })
    }
}

// ── Manifest loading ───────────────────────────────────────

/// Load a `plugin.toml` manifest from a directory.
pub fn load_manifest(dir: &Path) -> Result<PluginManifest, String> {
    let path = dir.join("plugin.toml");
    let content = std::fs::read_to_string(&path)
        .map_err(|e| format!("Cannot read {}: {e}", path.display()))?;
    toml::from_str(&content).map_err(|e| format!("Invalid plugin.toml in {}: {e}", dir.display()))
}

pub fn wasm_path(dir: &Path, manifest: &PluginManifest) -> PathBuf {
    dir.join(format!("{}.wasm", manifest.plugin.name))
}
