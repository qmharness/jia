use std::sync::LazyLock;

use prometheus::{
    Counter, CounterVec, Encoder, Gauge, Histogram, HistogramVec, TextEncoder, register_counter,
    register_counter_vec, register_gauge, register_histogram, register_histogram_vec,
};

use crate::plates::shen_spirit::RuntimeEvent;

// ── Counters ────────────────────────────────────────────────────

pub static JIA_TURNS_TOTAL: LazyLock<Counter> = LazyLock::new(|| {
    register_counter!("jia_turns_total", "Total agent turns executed")
        .expect("metric registration failed")
});

pub static JIA_TOOL_CALLS_TOTAL: LazyLock<CounterVec> = LazyLock::new(|| {
    register_counter_vec!(
        "jia_tool_calls_total",
        "Total tool calls executed",
        &["tool", "geju"]
    )
    .expect("metric registration failed")
});

pub static JIA_ERRORS_TOTAL: LazyLock<CounterVec> = LazyLock::new(|| {
    register_counter_vec!("jia_errors_total", "Total errors by source", &["source"])
        .expect("metric registration failed")
});

pub static JIA_GEJU_EVALS: LazyLock<CounterVec> = LazyLock::new(|| {
    register_counter_vec!(
        "jia_geju_evals_total",
        "GeJu evaluations by execution mode",
        &["mode"]
    )
    .expect("metric registration failed")
});

// ── Gauges ──────────────────────────────────────────────────────

pub static JIA_ACTIVE_SESSIONS: LazyLock<Gauge> = LazyLock::new(|| {
    register_gauge!("jia_active_sessions", "Number of active agent sessions")
        .expect("metric registration failed")
});

pub static JIA_SEEDS_TOTAL: LazyLock<Gauge> = LazyLock::new(|| {
    register_gauge!("jia_seeds_total", "Total seeds in the store")
        .expect("metric registration failed")
});

pub static JIA_ATMA_GRAHA: LazyLock<Gauge> = LazyLock::new(|| {
    register_gauge!("jia_atma_graha", "Current ātma-grāha value (0-1)")
        .expect("metric registration failed")
});

pub static JIA_REQUESTS_TOTAL: LazyLock<CounterVec> = LazyLock::new(|| {
    register_counter_vec!(
        "jia_requests_total",
        "Total agent requests by provider and model",
        &["provider", "model"]
    )
    .expect("metric registration failed")
});

// ── EventBus drops ──────────────────────────────────────────────

pub static JIA_EVENTBUS_DROPS_TOTAL: LazyLock<Counter> = LazyLock::new(|| {
    register_counter!(
        "jia_eventbus_drops_total",
        "Total events dropped due to full EventBus channel"
    )
    .expect("metric registration failed")
});

pub static JIA_LLM_INPUT_TOKENS_TOTAL: LazyLock<Counter> = LazyLock::new(|| {
    register_counter!(
        "jia_llm_input_tokens_total",
        "Total LLM input (prompt) tokens"
    )
    .expect("metric registration failed")
});

pub static JIA_LLM_OUTPUT_TOKENS_TOTAL: LazyLock<Counter> = LazyLock::new(|| {
    register_counter!(
        "jia_llm_output_tokens_total",
        "Total LLM output (completion) tokens"
    )
    .expect("metric registration failed")
});

pub static JIA_SESSIONS_COMPLETED_TOTAL: LazyLock<Counter> = LazyLock::new(|| {
    register_counter!(
        "jia_sessions_completed_total",
        "Total agent sessions completed"
    )
    .expect("metric registration failed")
});

// ── Compaction ───────────────────────────────────────────────────

pub static JIA_TOKENS_COMPACTED_TOTAL: LazyLock<Counter> = LazyLock::new(|| {
    register_counter!(
        "jia_tokens_compacted_total",
        "Total tokens removed by context compaction"
    )
    .expect("metric registration failed")
});

// ── Histograms ──────────────────────────────────────────────────

pub static JIA_TOOL_DURATION_SECONDS: LazyLock<HistogramVec> = LazyLock::new(|| {
    register_histogram_vec!(
        "jia_tool_duration_seconds",
        "Tool execution duration in seconds",
        &["tool"],
        vec![0.01, 0.05, 0.1, 0.5, 1.0, 2.0, 5.0, 10.0, 30.0, 60.0]
    )
    .expect("metric registration failed")
});

pub static JIA_LLM_DURATION_SECONDS: LazyLock<Histogram> = LazyLock::new(|| {
    register_histogram!(
        "jia_llm_duration_seconds",
        "LLM inference duration in seconds",
        vec![0.1, 0.5, 1.0, 2.0, 5.0, 10.0, 20.0, 40.0, 80.0, 160.0]
    )
    .expect("metric registration failed")
});

pub static JIA_REQUEST_DURATION_SECONDS: LazyLock<Histogram> = LazyLock::new(|| {
    register_histogram!(
        "jia_request_duration_seconds",
        "Total agent request duration in seconds",
        vec![0.5, 1.0, 2.0, 5.0, 10.0, 30.0, 60.0, 120.0, 300.0, 600.0]
    )
    .expect("metric registration failed")
});

/// Force all LazyLock metrics to register with the default Prometheus registry.
/// Call once at startup so `/metrics` returns all metrics even before agent activity.
pub fn ensure_registered() {
    let _ = &*JIA_TURNS_TOTAL;
    let _ = &*JIA_TOOL_CALLS_TOTAL;
    let _ = &*JIA_ERRORS_TOTAL;
    let _ = &*JIA_GEJU_EVALS;
    let _ = &*JIA_ACTIVE_SESSIONS;
    let _ = &*JIA_SEEDS_TOTAL;
    let _ = &*JIA_ATMA_GRAHA;
    let _ = &*JIA_REQUESTS_TOTAL;
    let _ = &*JIA_EVENTBUS_DROPS_TOTAL;
    let _ = &*JIA_LLM_INPUT_TOKENS_TOTAL;
    let _ = &*JIA_LLM_OUTPUT_TOKENS_TOTAL;
    let _ = &*JIA_SESSIONS_COMPLETED_TOTAL;
    let _ = &*JIA_TOKENS_COMPACTED_TOTAL;
    let _ = &*JIA_TOOL_DURATION_SECONDS;
    let _ = &*JIA_LLM_DURATION_SECONDS;
    let _ = &*JIA_REQUEST_DURATION_SECONDS;
}

// ── Metrics collector ───────────────────────────────────────────

/// Spawn a task that consumes runtime events and updates Prometheus metrics.
pub async fn run_collector(mut rx: tokio::sync::broadcast::Receiver<RuntimeEvent>) {
    while let Ok(event) = rx.recv().await {
        match event {
            RuntimeEvent::TurnStart { .. } => {
                JIA_TURNS_TOTAL.inc();
            }
            RuntimeEvent::ToolCall { .. } => {
                // Tool call counted on GeJuResult with both labels
            }
            RuntimeEvent::ToolResult { .. } => {}
            RuntimeEvent::GeJuResult {
                tool,
                pattern,
                mode,
            } => {
                JIA_TOOL_CALLS_TOTAL
                    .with_label_values(&[&tool, &pattern])
                    .inc();
                JIA_GEJU_EVALS.with_label_values(&[&mode]).inc();
            }
            RuntimeEvent::Error { source, .. } => {
                JIA_ERRORS_TOTAL.with_label_values(&[&source]).inc();
            }
            RuntimeEvent::LlmUsage {
                input_tokens,
                output_tokens,
            } => {
                JIA_LLM_INPUT_TOKENS_TOTAL.inc_by(input_tokens as f64);
                JIA_LLM_OUTPUT_TOKENS_TOTAL.inc_by(output_tokens as f64);
            }
            RuntimeEvent::SessionEnd { .. } => {
                JIA_SESSIONS_COMPLETED_TOTAL.inc();
            }
            _ => {}
        }
    }
}

// ── Metrics handlers ────────────────────────────────────────────

/// Axum handler that returns Prometheus text-format metrics.
pub fn metrics_handler() -> String {
    let encoder = TextEncoder::new();
    let metric_families = prometheus::gather();
    let mut buffer = Vec::new();
    if encoder.encode(&metric_families, &mut buffer).is_err() {
        return String::new();
    }
    String::from_utf8(buffer).unwrap_or_default()
}

/// Build a JSON object from all registered Prometheus metrics.
/// CounterVec metrics are grouped by their last label (protobuf sorts
/// labels alphabetically, so the last label is typically the primary
/// dimension — e.g. "tool" from ["geju", "tool"]).
pub fn metrics_json() -> serde_json::Value {
    let families = prometheus::gather();
    let mut obj = serde_json::Map::new();

    for fam in &families {
        let name = fam.get_name();
        let metrics = fam.get_metric();

        if metrics.is_empty() {
            continue;
        }

        // Counter/Gauge with labels → nested object keyed by last label value
        if metrics.len() > 1 || !metrics[0].get_label().is_empty() {
            let mut sub = serde_json::Map::new();
            for m in metrics {
                let key = m
                    .get_label()
                    .last()
                    .map(|l| l.get_value().to_string())
                    .unwrap_or_else(|| name.to_string());
                let val = m.get_counter().get_value() + m.get_gauge().get_value();
                let prev = sub.get(&key).and_then(|v| v.as_f64()).unwrap_or(0.0);
                sub.insert(
                    key,
                    serde_json::Value::Number(
                        serde_json::Number::from_f64(prev + val)
                            .unwrap_or(serde_json::Number::from(0)),
                    ),
                );
            }
            obj.insert(name.to_string(), serde_json::Value::Object(sub));
        } else {
            // Single unlabeled metric
            let val = metrics[0].get_counter().get_value() + metrics[0].get_gauge().get_value();
            obj.insert(
                name.to_string(),
                serde_json::Value::Number(
                    serde_json::Number::from_f64(val).unwrap_or(serde_json::Number::from(0)),
                ),
            );
        }
    }

    serde_json::Value::Object(obj)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gauges_register_and_export() {
        // Set values
        JIA_SEEDS_TOTAL.set(99.0);
        JIA_ATMA_GRAHA.set(0.77);

        // Verify gather includes both gauges
        let families = prometheus::gather();

        let seeds = families.iter().find(|f| f.get_name() == "jia_seeds_total");
        assert!(seeds.is_some(), "jia_seeds_total should be registered");
        let sv = seeds
            .expect("metric registration failed")
            .get_metric()
            .first()
            .expect("metric registration failed")
            .get_gauge()
            .get_value();
        assert!(
            (sv - 99.0).abs() < 0.01,
            "jia_seeds_total should be 99, got {sv}"
        );

        let ag = families.iter().find(|f| f.get_name() == "jia_atma_graha");
        assert!(ag.is_some(), "jia_atma_graha should be registered");
        let av = ag
            .expect("metric registration failed")
            .get_metric()
            .first()
            .expect("metric registration failed")
            .get_gauge()
            .get_value();
        assert!(
            (av - 0.77).abs() < 0.01,
            "jia_atma_graha should be 0.77, got {av}"
        );

        // Verify handler output
        let output = metrics_handler();
        assert!(
            output.contains("jia_seeds_total"),
            "output missing seeds_total"
        );
        assert!(
            output.contains("jia_atma_graha"),
            "output missing atma_graha"
        );
    }

    #[test]
    fn metrics_json_includes_gauges_and_counters() {
        JIA_TURNS_TOTAL.inc();
        JIA_TOOL_CALLS_TOTAL
            .with_label_values(&["shell", "direct"])
            .inc_by(2.0);
        JIA_TOOL_CALLS_TOTAL
            .with_label_values(&["shell", "guarded"])
            .inc_by(3.0);
        JIA_TOOL_CALLS_TOTAL
            .with_label_values(&["read", "direct"])
            .inc();
        JIA_ERRORS_TOTAL.with_label_values(&["tool"]).inc();

        let json = metrics_json();

        // Unlabeled counter → number
        assert!(
            json["jia_turns_total"].as_f64().is_some_and(|v| v >= 1.0),
            "jia_turns_total should be >= 1.0, got {:?}",
            json["jia_turns_total"]
        );

        // Two-label CounterVec → accumulated by last label (alphabetically: "tool" from ["geju", "tool"])
        let tools = &json["jia_tool_calls_total"];
        let shell = tools["shell"].as_f64().unwrap_or(0.0);
        let read = tools["read"].as_f64().unwrap_or(0.0);
        assert!(
            shell >= 5.0,
            "shell should be >= 5.0 (2 direct + 3 guarded), got {shell}. Full json: {json}"
        );
        assert!(read >= 1.0, "read should be >= 1.0, got {read}");

        // Single-label CounterVec
        let errors = &json["jia_errors_total"];
        assert!(
            errors["tool"].as_f64().is_some_and(|v| v >= 1.0),
            "errors[tool] should be >= 1.0, got {:?}",
            errors["tool"]
        );
    }
}
