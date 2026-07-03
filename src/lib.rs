use std::sync::Arc;
// ── Module declarations ───────────────────────────────────────

// Core types
pub mod error;
pub mod types;
pub mod utils;

// Architecture modules
pub mod geju;
pub mod palaces;
pub mod plates;
pub mod principles;
pub mod stems;
pub mod telemetry;
pub mod vijnana;
pub mod zuowang;

// Terminal UI
#[cfg(feature = "tui")]
pub mod tui;

// Re-export for backward-compatible paths
pub use palaces::dui_gateway as gateway;
pub use palaces::kun_config as config;
pub use palaces::zhong_core as provider;

// ── 起局 (qi ju) — Constellation entry point ──────────────────

use palaces::kun_config::AppConfig;
use plates::di_earth::EarthPlate;

/// 起局 (qi ju) — Assemble the agent runtime.
///
/// This is the primary entry point for creating a Jia agent runtime.
/// It assembles the Earth Plate with all infrastructure components,
/// registers built-in tools, and returns an `Arc<EarthPlate>` for shared use.
pub fn init(config: AppConfig) -> Arc<EarthPlate> {
    tracing::info!("jia::init — assembling Earth Plate");
    let earth = EarthPlate::assemble(config);
    tracing::info!(
        "jia::init — Earth Plate assembled ({} tools registered)",
        earth.tools.list_names().len()
    );
    earth
}

/// Create a test JiaCore backed by a mock provider.
///
/// Each string in `responses` is streamed character-by-character as one
/// `infer()` response. Available only behind the `test-utils` feature or
/// in `#[cfg(test)]` builds.
#[cfg(any(test, feature = "test-utils"))]
pub fn make_test_core(responses: Vec<String>) -> palaces::zhong_core::JiaCore {
    palaces::zhong_core::JiaCore::with_mock(responses)
}
