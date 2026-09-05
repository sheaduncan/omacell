//! Fixed-host live-provider measurements for the §12.1 release gate.

use std::collections::BTreeMap;
use std::error::Error;
use std::sync::Arc;
use std::time::Instant;

use omacell_ai::complete::complete_schema;
use omacell_ai::http::{ReqwestTransport, SharedTransport};
use omacell_ai::plan::plan_schema;
use omacell_ai::prompts::PromptSet;
use omacell_ai::{AiRuntime, Slot};
use omacell_conf::schema::{AiProvider, package_defaults};
use serde_json::json;

fn emit(id: &str, value: f64) {
    eprintln!("OMACELL_PERF_RESULT {}", json!({"id": id, "value": value}));
}

fn required(name: &str) -> Result<String, Box<dyn Error>> {
    std::env::var(name)
        .map_err(|_| format!("fixed-host performance requires {name}").into())
        .and_then(|value| {
            if value.trim().is_empty() {
                Err(format!("fixed-host performance requires non-empty {name}").into())
            } else {
                Ok(value)
            }
        })
}

struct LiveRuntime {
    runtime: Arc<AiRuntime>,
    _handle: tokio::runtime::Runtime,
    _state: tempfile::TempDir,
}

fn live_runtime(prefix: &str, local: bool) -> Result<LiveRuntime, Box<dyn Error>> {
    let endpoint = required(&format!("OMACELL_PERF_{prefix}_ENDPOINT"))?;
    let model = required(&format!("OMACELL_PERF_{prefix}_MODEL"))?;
    let mut config = package_defaults()?;
    config.ai.enabled = true;
    config.ai.providers.clear();
    let secret_env = (!local).then_some("OMACELL_PERF_CLOUD_TOKEN".to_string());
    if !local {
        let _ = required("OMACELL_PERF_CLOUD_TOKEN")?;
    }
    config.ai.providers.insert(
        "performance".into(),
        AiProvider {
            kind: "openai_compatible".into(),
            endpoint,
            local,
            secret_env,
            secret_cmd: None,
            timeout: 30_000,
            headers: BTreeMap::new(),
        },
    );
    let routed = format!("performance:{model}");
    config.ai.models.fast.clone_from(&routed);
    config.ai.models.default = routed;
    config.ai.functions.max_requests_per_minute = 1_000;
    let handle = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    let state = tempfile::tempdir()?;
    let transport: SharedTransport = Arc::new(ReqwestTransport::new()?);
    let runtime = AiRuntime::new(
        handle.handle().clone(),
        config,
        transport,
        PromptSet::builtin(),
        state.path().join("cache"),
        state.path().join("state"),
        Default::default(),
    );
    Ok(LiveRuntime {
        runtime,
        _handle: handle,
        _state: state,
    })
}

fn measure_task(
    runtime: &AiRuntime,
    slot: Slot,
    task: &str,
    prompt: &str,
    schema: serde_json::Value,
) -> Result<f64, Box<dyn Error>> {
    let started = Instant::now();
    let _ = runtime.chat_task(slot, task, prompt.to_string(), Some(schema), Vec::new())?;
    Ok(started.elapsed().as_secs_f64() * 1_000.0)
}

fn main() -> Result<(), Box<dyn Error>> {
    if std::env::var_os("OMACELL_FIXED_PERF").is_none() {
        eprintln!("live model performance skipped outside the fixed-host gate");
        return Ok(());
    }

    let local = live_runtime("LOCAL", true)?;
    // The task API returns a completed response. Measuring the whole small
    // completion is conservative for the first-token target.
    emit(
        "inline_completion_first_token_ms",
        measure_task(
            &local.runtime,
            Slot::Fast,
            "complete",
            "Complete this spreadsheet formula: =SUM(A1:",
            complete_schema(),
        )?,
    );
    emit(
        "local_plan_ms",
        measure_task(
            &local.runtime,
            Slot::Default,
            "plan",
            "Return an empty command plan for a no-op request.",
            plan_schema(),
        )?,
    );
    emit(
        "ai_batch_cells",
        f64::from(local.runtime.config().ai.functions.batch_size),
    );

    let cloud = live_runtime("CLOUD", false)?;
    emit(
        "cloud_plan_ms",
        measure_task(
            &cloud.runtime,
            Slot::Default,
            "plan",
            "Return an empty command plan for a no-op request.",
            plan_schema(),
        )?,
    );
    Ok(())
}
