//! Shared AI runtime: async cells, chat, settle, and task dispatch.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use omacell_conf::schema::Config;
use omacell_core::eval::RuntimeValue;
use omacell_core::graph::CellCoord;
use omacell_core::recalc::{
    AsyncNodeProvider, AsyncRequest, AsyncState, ContentHash, RecalcEngine,
};
use omacell_core::workbook::Workbook;
use serde_json::{Value, json};
use tokio::runtime::Handle;

use crate::audit::{AuditLog, LogRecord, SessionStats, StatusSegment, hash_json, now_ms};
use crate::budget::{RateLimit, check_cell_budget};
use crate::cache::{self, AiCache, CacheEntry};
use crate::card::{CardLevel, CardRequest};
use crate::error::{AiError, codes};
use crate::functions::{args_json, json_to_runtime, task_prompt};
use crate::http::SharedTransport;
use crate::policy::{PolicySnapshot, build_card, fence_data, provider_is_local};
use crate::prompts::PromptSet;
use crate::provider::{
    ChatMessage, ChatRequest, ChatResponse, Role, Slot, provider_from_config, provider_timeout,
    route_slot,
};
use crate::redact::redact_json;

const MAX_MODEL_TEXT_BYTES: usize = 1_048_576;

/// Queued async cell.
#[derive(Clone, Debug)]
struct Job {
    hash: ContentHash,
    name: String,
    args: Value,
}

/// Process-wide AI runtime (sync `evaluate`, async `settle`).
pub struct AiRuntime {
    handle: Handle,
    config: Config,
    transport: SharedTransport,
    prompts: PromptSet,
    cache_dir: PathBuf,
    state_dir: PathBuf,
    inner: Mutex<Inner>,
}

struct Inner {
    cache: AiCache,
    pending: HashMap<ContentHash, Job>,
    results: HashMap<ContentHash, RuntimeValue>,
    cells: HashMap<CellCoord, ContentHash>,
    rate: RateLimit,
    session: SessionStats,
    confirm: Option<String>,
    catalog: BTreeSet<String>,
    catalog_payload: Vec<Value>,
}

impl AiRuntime {
    /// Construct around a live tokio handle.
    #[must_use]
    pub fn new(
        handle: Handle,
        config: Config,
        transport: SharedTransport,
        prompts: PromptSet,
        cache_dir: PathBuf,
        state_dir: PathBuf,
        cache: AiCache,
    ) -> Arc<Self> {
        Arc::new(Self {
            handle,
            config: config.clone(),
            transport,
            prompts,
            cache_dir,
            state_dir,
            inner: Mutex::new(Inner {
                cache,
                pending: HashMap::new(),
                results: HashMap::new(),
                cells: HashMap::new(),
                rate: RateLimit::from_config(&config),
                session: SessionStats::default(),
                confirm: None,
                catalog: BTreeSet::new(),
                catalog_payload: Vec::new(),
            }),
        })
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// Budget confirmation message, if the last settle tripped a cap.
    #[must_use]
    pub fn confirmation(&self) -> Option<String> {
        self.lock().confirm.clone()
    }

    /// Effective config.
    #[must_use]
    pub fn config(&self) -> &Config {
        &self.config
    }

    /// State directory (conversation, log).
    #[must_use]
    pub fn state_dir(&self) -> &std::path::Path {
        &self.state_dir
    }

    /// Public command catalog the planner may emit.
    pub fn set_catalog(&self, entries: Vec<(String, Value)>) {
        let mut g = self.lock();
        g.catalog = entries.iter().map(|(id, _)| id.clone()).collect();
        g.catalog_payload = entries.into_iter().map(|(_, entry)| entry).collect();
    }

    /// Snapshot of the planner catalog.
    #[must_use]
    pub fn catalog(&self) -> BTreeSet<String> {
        self.lock().catalog.clone()
    }

    /// Command documentation and argument schemas sent to planners.
    #[must_use]
    pub fn catalog_payload(&self) -> Vec<Value> {
        self.lock().catalog_payload.clone()
    }

    /// Session counters for the status line.
    #[must_use]
    pub fn session_stats(&self) -> SessionStats {
        self.lock().session.clone()
    }

    /// Status-line segment.
    #[must_use]
    pub fn status_segment(&self, wb: Option<&Workbook>) -> StatusSegment {
        let (name, _) = route_slot(&self.config, Slot::Default);
        let local = provider_is_local(&self.config, &name);
        let policy = PolicySnapshot::capture(&self.config, wb, local);
        let stats = self.session_stats();
        StatusSegment::new(name, local, policy.send.as_str(), &stats)
    }

    /// Privacy snapshot for this workbook.
    #[must_use]
    pub fn policy(&self, wb: Option<&Workbook>) -> PolicySnapshot {
        self.policy_for(Slot::Default, wb)
    }

    /// Privacy snapshot for the provider selected by `slot`.
    #[must_use]
    pub fn policy_for(&self, slot: Slot, wb: Option<&Workbook>) -> PolicySnapshot {
        let (name, _) = route_slot(&self.config, slot);
        let local = provider_is_local(&self.config, &name);
        PolicySnapshot::capture(&self.config, wb, local)
    }

    /// Fenced workbook card (privacy choke point).
    pub fn workbook_card(
        &self,
        wb: &Workbook,
        engine: Option<&RecalcEngine>,
        selection: Option<String>,
    ) -> Result<Value, AiError> {
        self.workbook_card_for(Slot::Default, wb, engine, selection)
    }

    /// Fenced workbook card using the privacy policy of `slot`'s provider.
    pub fn workbook_card_for(
        &self,
        slot: Slot,
        wb: &Workbook,
        engine: Option<&RecalcEngine>,
        selection: Option<String>,
    ) -> Result<Value, AiError> {
        let policy = self.policy_for(slot, Some(wb));
        let req = CardRequest {
            selection,
            ..CardRequest::default()
        };
        let level = match policy.send {
            crate::policy::SendLevel::Schema => CardLevel::Summary,
            crate::policy::SendLevel::Sample => CardLevel::Sample,
            crate::policy::SendLevel::Full => CardLevel::Columns,
        };
        let req = CardRequest { level, ..req };
        let (card, _) = build_card(wb, engine, req, &policy)?;
        Ok(card)
    }

    /// Install a newly opened workbook's cache and discard per-workbook state.
    ///
    /// Returns the previous cache so a staged open can roll back on failure.
    pub fn replace_workbook_cache(&self, cache: AiCache) -> AiCache {
        let mut g = self.lock();
        g.pending.clear();
        g.results.clear();
        g.cells.clear();
        g.confirm = None;
        std::mem::replace(&mut g.cache, cache)
    }

    /// Force the next evaluate of `hash` to miss the cache.
    pub fn refresh_key(&self, hash: ContentHash) {
        let mut g = self.lock();
        if g.cache.get(hash).is_some_and(|e| e.pinned) {
            return;
        }
        g.cache.remove(hash);
        g.results.remove(&hash);
    }

    /// Refresh every unpinned AI cell, or only `cells` when non-empty.
    pub fn refresh_cells(&self, cells: &[CellCoord]) {
        let hashes: Vec<ContentHash> = {
            let g = self.lock();
            if cells.is_empty() {
                g.cells.values().copied().collect()
            } else {
                cells
                    .iter()
                    .filter_map(|c| g.cells.get(c).copied())
                    .collect()
            }
        };
        for hash in hashes {
            self.refresh_key(hash);
        }
    }

    /// Pin a cache entry.
    pub fn pin_key(&self, hash: ContentHash) {
        if let Some(entry) = self.lock().cache.entries.get_mut(&AiCache::key(hash)) {
            entry.pinned = true;
        }
    }

    /// Pin cells (empty = all known AI cells).
    pub fn pin_cells(&self, cells: &[CellCoord]) {
        let hashes: Vec<ContentHash> = {
            let g = self.lock();
            if cells.is_empty() {
                g.cells.values().copied().collect()
            } else {
                cells
                    .iter()
                    .filter_map(|c| g.cells.get(c).copied())
                    .collect()
            }
        };
        for hash in hashes {
            self.pin_key(hash);
        }
    }

    /// Provenance for a cell, if cached.
    #[must_use]
    pub fn provenance(&self, cell: CellCoord) -> Option<CacheEntry> {
        let g = self.lock();
        let hash = *g.cells.get(&cell)?;
        g.cache.get(hash).cloned()
    }

    /// Persist cache into `wb.custom_parts`.
    pub fn write_workbook_cache(&self, wb: &mut Workbook) -> Result<(), AiError> {
        let bytes = self.lock().cache.to_bytes()?;
        wb.custom_parts
            .insert(cache::AICACHE_PART.to_string(), bytes);
        Ok(())
    }

    /// Chat a named task with structured output. Workbook JSON must already be fenced.
    pub fn chat_task(
        &self,
        slot: Slot,
        task: &str,
        user: String,
        schema: Option<Value>,
        tools: Vec<crate::provider::ToolSpec>,
    ) -> Result<ChatResponse, AiError> {
        if !self.config.ai.enabled {
            return Err(
                AiError::new(codes::DISABLED, "AI is disabled").with_hint("run omacell ai setup")
            );
        }
        let (provider_name, model) = route_slot(&self.config, slot);
        let spec = self
            .config
            .ai
            .providers
            .get(&provider_name)
            .ok_or_else(|| AiError::new(codes::KIND, format!("no provider {provider_name}")))?;
        let provider = provider_from_config(&provider_name, spec, Arc::clone(&self.transport))?;
        let system = self.prompts.get("system");
        let task_t = self.prompts.get(task);
        let messages = vec![
            ChatMessage {
                role: Role::System,
                content: format!("{}\n{}", system.body, task_t.body),
                tool_call_id: None,
                tool_calls: Vec::new(),
            },
            ChatMessage {
                role: Role::User,
                content: user.clone(),
                tool_call_id: None,
                tool_calls: Vec::new(),
            },
        ];
        let req = ChatRequest {
            model: model.clone(),
            messages,
            json_schema: schema,
            tools,
            stream: false,
            max_output_tokens: 1024,
            cancel: crate::provider::Cancel::new(),
            timeout: provider_timeout(spec),
        };
        let request_record = json!({
            "task": task,
            "model": model,
            "messages": &req.messages,
            "schema": &req.json_schema,
            "tools": &req.tools,
            "max_output_tokens": req.max_output_tokens,
        });
        let request_bytes = serde_json::to_vec(&request_record)
            .map_err(|err| AiError::new(codes::PAYLOAD, err.to_string()))?
            .len() as u64;
        let request_hash = hash_json(&request_record);
        self.lock().rate.allow()?;
        let started = std::time::Instant::now();
        let out = self.handle.block_on(provider.chat(req))?;
        if out.text.len() > MAX_MODEL_TEXT_BYTES {
            return Err(AiError::new(
                codes::PAYLOAD,
                "model text response exceeds the 1 MiB limit",
            ));
        }
        let bytes = out.text.len() as u64;
        self.lock().session.record(request_bytes);
        let log = AuditLog::open(&self.state_dir)?;
        log.append(&LogRecord {
            ts: now_ms(),
            task: task.into(),
            provider: provider_name,
            model,
            request_bytes,
            response_bytes: bytes,
            request_hash,
            latency_ms: started.elapsed().as_millis() as u64,
            usage: out.usage,
            content: self
                .config
                .ai
                .privacy
                .log_content
                .then(|| json!({"request": user, "response": out.text.clone()})),
        })?;
        Ok(out)
    }

    /// Drain pending AI cells in batches (default 50).
    pub fn settle(&self, policy: &PolicySnapshot) -> Result<usize, AiError> {
        let batch = self.config.ai.functions.batch_size.max(1) as usize;
        let mut jobs: Vec<Job> = {
            let mut g = self.lock();
            g.confirm = None;
            let n = g.pending.len() as u32;
            if let Err(err) = check_cell_budget(&self.config, n) {
                g.confirm = Some(err.message.clone());
                return Err(err);
            }
            g.pending.drain().map(|(_, j)| j).collect()
        };
        if jobs.is_empty() {
            return Ok(0);
        }
        jobs.sort_by_key(|job| job.hash.0);
        let retry_jobs = jobs.clone();
        let mut completed = HashSet::new();
        let mut groups: BTreeMap<String, Vec<Job>> = BTreeMap::new();
        for job in jobs {
            groups.entry(job.name.clone()).or_default().push(job);
        }
        let mut done = 0usize;
        for (name, group) in groups {
            for chunk in group.chunks(batch) {
                let payload = json!({
                    "task": name,
                    "rows": chunk.iter().map(|j| json!({"hash": cache::AiCache::key(j.hash), "args": j.args})).collect::<Vec<_>>(),
                });
                let mut data = payload;
                if policy.suggest_redaction {
                    let _ = redact_json(&mut data);
                }
                let user = fence_data("AI cell batch", &data);
                let schema = json!({
                    "type": "object",
                    "required": ["results"],
                    "additionalProperties": false,
                    "properties": {
                        "results": {
                            "type": "array",
                            "minItems": chunk.len(),
                            "maxItems": chunk.len(),
                            "items": {
                                "type": "object",
                                "required": ["i", "value"],
                                "additionalProperties": false,
                                "properties": {
                                    "i": {"type": "integer", "minimum": 0, "maximum": chunk.len().saturating_sub(1)},
                                    "value": {}
                                }
                            }
                        }
                    }
                });
                let task = task_prompt(&name);
                let reply = match self.chat_task(Slot::Default, task, user, Some(schema), vec![]) {
                    Ok(reply) => reply,
                    Err(err) => {
                        let mut g = self.lock();
                        for job in &retry_jobs {
                            if !completed.contains(&job.hash) {
                                g.pending.insert(job.hash, job.clone());
                            }
                        }
                        return Err(err);
                    }
                };
                let results = match parse_batch_results(&reply.text, chunk.len()) {
                    Ok(results) => results,
                    Err(err) => {
                        let mut g = self.lock();
                        for job in &retry_jobs {
                            if !completed.contains(&job.hash) {
                                g.pending.insert(job.hash, job.clone());
                            }
                        }
                        return Err(err);
                    }
                };
                let (provider, model) = route_slot(&self.config, Slot::Default);
                let version = self.prompts.get(task).version;
                let mut g = self.lock();
                for (i, job) in chunk.iter().enumerate() {
                    let (value, runtime) = &results[i];
                    let input_hash = hash_json(&job.args);
                    let entry = CacheEntry {
                        task: job.name.clone(),
                        template_version: version.clone(),
                        provider: provider.clone(),
                        model: model.clone(),
                        value: value.clone(),
                        prompt_hash: input_hash.clone(),
                        input_hash,
                        ts: now_ms(),
                        prompt_tokens: reply.usage.prompt_tokens,
                        completion_tokens: reply.usage.completion_tokens,
                        pinned: false,
                    };
                    g.cache.insert(job.hash, entry.clone());
                    g.results.insert(job.hash, runtime.clone());
                    if let Err(error) = cache::write_disk(&self.cache_dir, job.hash, &entry) {
                        tracing::warn!(code = %error.code, message = %error.message, "AI disk cache write failed");
                    }
                    completed.insert(job.hash);
                    done += 1;
                }
            }
        }
        Ok(done)
    }
}

impl AsyncNodeProvider for AiRuntime {
    fn evaluate(&self, key: ContentHash, req: &AsyncRequest) -> AsyncState {
        if !self.config.ai.enabled {
            return AsyncState::Failed {
                hint: "ai.disabled".into(),
            };
        }
        let args = args_json(&req.args);
        let task = task_prompt(&req.name);
        let version = self.prompts.get(task).version;
        let (provider, model) = route_slot(&self.config, Slot::Default);
        let input_hash = hash_json(&args);
        let mut g = self.lock();
        g.cells.insert(req.cell, key);
        if let Some(entry) = g.cache.get(key).cloned()
            && cache_fresh(&entry, &version, &provider, &model, &input_hash)
            && let Ok(rt) = json_to_runtime(&entry.value)
        {
            g.results.insert(key, rt);
            return AsyncState::Ready(omacell_core::value::Value::Empty);
        }
        if let Some(disk) = cache::read_disk(&self.cache_dir, key)
            && cache_fresh(&disk, &version, &provider, &model, &input_hash)
            && let Ok(rt) = json_to_runtime(&disk.value)
        {
            g.cache.insert(key, disk.clone());
            g.results.insert(key, rt);
            return AsyncState::Ready(omacell_core::value::Value::Empty);
        }
        g.pending.insert(
            key,
            Job {
                hash: key,
                name: req.name.clone(),
                args,
            },
        );
        let keep = self.config.ai.functions.keep_stale;
        AsyncState::Pending {
            cached: keep.then_some(omacell_core::value::Value::Empty),
        }
    }

    fn runtime_result(&self, key: ContentHash) -> Option<RuntimeValue> {
        self.lock().results.get(&key).cloned()
    }
}

fn cache_fresh(
    entry: &CacheEntry,
    version: &str,
    provider: &str,
    model: &str,
    input_hash: &str,
) -> bool {
    entry.input_hash == input_hash
        && (entry.pinned
            || (entry.template_version == version
                && entry.provider == provider
                && entry.model == model))
}

fn parse_batch_results(text: &str, expected: usize) -> Result<Vec<(Value, RuntimeValue)>, AiError> {
    let parsed: Value = serde_json::from_str(text)
        .map_err(|err| AiError::new(codes::PAYLOAD, format!("invalid AI cell JSON: {err}")))?;
    let rows = parsed
        .get("results")
        .and_then(Value::as_array)
        .ok_or_else(|| AiError::new(codes::PAYLOAD, "AI cell response is missing results"))?;
    if rows.len() != expected {
        return Err(AiError::new(
            codes::PAYLOAD,
            format!(
                "AI cell response returned {} of {expected} results",
                rows.len()
            ),
        ));
    }
    let mut ordered: Vec<Option<(Value, RuntimeValue)>> = vec![None; expected];
    for row in rows {
        let index = row
            .get("i")
            .and_then(Value::as_u64)
            .and_then(|i| usize::try_from(i).ok())
            .filter(|i| *i < expected)
            .ok_or_else(|| AiError::new(codes::PAYLOAD, "AI cell result has invalid index"))?;
        if ordered[index].is_some() {
            return Err(AiError::new(
                codes::PAYLOAD,
                format!("AI cell response repeats result {index}"),
            ));
        }
        let value = row
            .get("value")
            .cloned()
            .ok_or_else(|| AiError::new(codes::PAYLOAD, "AI cell result is missing value"))?;
        let runtime = json_to_runtime(&value)?;
        ordered[index] = Some((value, runtime));
    }
    ordered
        .into_iter()
        .enumerate()
        .map(|(i, value)| {
            value.ok_or_else(|| {
                AiError::new(
                    codes::PAYLOAD,
                    format!("AI cell response is missing result {i}"),
                )
            })
        })
        .collect()
}

/// Debounce helper for completion (`[ai.completion] debounce`).
#[must_use]
pub fn debounce_ms(config: &Config) -> Duration {
    Duration::from_millis(u64::from(config.ai.completion.debounce.max(1)))
}

/// Whether ghost completion should run.
#[must_use]
pub fn completion_enabled(config: &Config, fast_is_local: bool) -> bool {
    match config.ai.completion.mode.as_str() {
        "on" => true,
        "off" => false,
        _ => fast_is_local,
    }
}

/// Locality of the `fast` slot.
#[must_use]
pub fn fast_is_local(config: &Config) -> bool {
    let (name, _) = route_slot(config, Slot::Fast);
    provider_is_local(config, &name)
}
