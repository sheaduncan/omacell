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
use serde::{Deserialize, Serialize};
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
use crate::prompts::{PromptSet, PromptTemplate};
use crate::provider::{
    Cancel, ChatMessage, ChatRequest, ChatResponse, Role, Slot, ToolSpec, provider_from_config,
    provider_timeout, route_slot, validate_chat_request, validate_tool_call,
};
use crate::redact::redact_json;

const MAX_MODEL_TEXT_BYTES: usize = 1_048_576;
const MAX_CUSTOM_TASKS: usize = 128;
const MAX_CUSTOM_TASK_NAME_BYTES: usize = 128;
const MAX_CUSTOM_PROMPT_BYTES: usize = 256 * 1_024;

/// User-profile AI task registered by a trusted extension host.
#[derive(Clone, Debug, PartialEq)]
pub struct AiTaskSpec {
    /// Case-insensitive task name.
    pub name: String,
    /// Task-specific system prompt appended to the built-in system prompt.
    pub prompt: String,
    /// Default structured-output schema, or per-result schema for an AI function.
    pub schema: Option<Value>,
    /// Default tools when the caller does not supply any.
    pub tools: Vec<ToolSpec>,
}

impl AiTaskSpec {
    /// Validate the bounded task name, prompt, schema, and tools.
    pub fn validate(&self) -> Result<(), AiError> {
        validate_task(self)
    }
}

/// Bounded request surface exposed to trusted user-profile hooks.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct AiHookRequest {
    /// Stable task name; hooks cannot change it.
    pub task: String,
    /// Configured provider name. Hooks may route to another configured provider,
    /// but cannot downgrade a local-provider payload to a cloud provider.
    pub provider: String,
    /// Model name sent to the selected provider.
    pub model: String,
    /// Prompt messages after privacy filtering and task-template expansion.
    pub messages: Vec<ChatMessage>,
    /// Optional structured-output schema.
    pub schema: Option<Value>,
    /// Available tools.
    pub tools: Vec<ToolSpec>,
    /// Maximum generated tokens.
    pub max_output_tokens: u32,
}

/// Bounded response surface exposed to trusted user-profile hooks.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct AiHookResponse {
    /// Stable task name; hooks cannot change it.
    pub task: String,
    /// Actual provider used; hooks cannot change response metadata.
    pub provider: String,
    /// Actual model used; hooks cannot change response metadata.
    pub model: String,
    /// Provider response. Text and validated tool calls may be post-processed.
    pub response: ChatResponse,
}

/// Trusted user-profile request/response extension hooks.
pub trait AiHooks: Send + Sync {
    /// Stable version incorporated into AI-cell cache keys.
    ///
    /// Implementations must change this whenever deterministic hook behavior or
    /// routing changes.
    fn cache_version(&self) -> String {
        "1".into()
    }

    /// Transform or route one already policy-filtered request.
    fn on_request(&self, request: AiHookRequest) -> Result<AiHookRequest, AiError> {
        Ok(request)
    }

    /// Post-process one provider response before the caller receives it.
    fn on_response(&self, response: AiHookResponse) -> Result<AiHookResponse, AiError> {
        Ok(response)
    }
}

/// Queued async cell.
#[derive(Clone, Debug)]
struct Job {
    hash: ContentHash,
    name: String,
    args: Value,
}

struct RoutedResponse {
    response: ChatResponse,
    provider: String,
    model: String,
    template_version: String,
    extension_generation: u64,
}

struct FunctionExtension {
    task: String,
    template_version: String,
    value_schema: Option<Value>,
    generation: u64,
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
    tasks: BTreeMap<String, AiTaskSpec>,
    hooks: Option<Arc<dyn AiHooks>>,
    extension_generation: u64,
    pending_generation: u64,
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
                tasks: BTreeMap::new(),
                hooks: None,
                extension_generation: 0,
                pending_generation: 0,
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

    /// Atomically replace trusted user-profile task definitions and hooks.
    ///
    /// Retained script reload uses replacement rather than incremental
    /// registration so removed scripts cannot leave stale AI behavior behind.
    pub fn replace_extensions(
        &self,
        tasks: Vec<AiTaskSpec>,
        hooks: Option<Arc<dyn AiHooks>>,
    ) -> Result<(), AiError> {
        if tasks.len() > MAX_CUSTOM_TASKS {
            return Err(AiError::new(
                codes::PAYLOAD,
                format!("at most {MAX_CUSTOM_TASKS} custom AI tasks may be registered"),
            ));
        }
        let mut validated = BTreeMap::new();
        for mut task in tasks {
            validate_task(&task)?;
            let key = canonical_task_name(&task.name);
            task.name.clone_from(&key);
            if validated.insert(key.clone(), task).is_some() {
                return Err(AiError::new(
                    codes::PAYLOAD,
                    format!("duplicate custom AI task {key}"),
                ));
            }
        }
        if let Some(hooks) = &hooks {
            let version = hooks.cache_version();
            if version.is_empty() || version.len() > 256 {
                return Err(AiError::new(
                    codes::PAYLOAD,
                    "AI hook cache version must be 1..=256 bytes",
                ));
            }
        }
        let mut inner = self.lock();
        inner.tasks = validated;
        inner.hooks = hooks;
        inner.extension_generation = inner.extension_generation.wrapping_add(1);
        if !inner.pending.is_empty() {
            inner.pending_generation = inner.pending_generation.wrapping_add(1);
        }
        Ok(())
    }

    /// Generation of the current pending async-cell work, or `None` when idle.
    ///
    /// The generation advances when evaluation queues a new content hash or
    /// extensions replace pending work. Reinserting failed work alone does not
    /// cause an automatic retry loop.
    #[must_use]
    pub fn pending_generation(&self) -> Option<u64> {
        let inner = self.lock();
        (!inner.pending.is_empty()).then_some(inner.pending_generation)
    }

    fn function_extension(&self, function: &str) -> FunctionExtension {
        let key = canonical_task_name(function);
        let (custom, hooks, generation) = {
            let inner = self.lock();
            (
                inner.tasks.get(&key).cloned(),
                inner.hooks.clone(),
                inner.extension_generation,
            )
        };
        let (task, mut template, value_schema) = match custom {
            Some(custom) => {
                let template = custom_task_template(&custom);
                let value_schema = custom.schema.clone();
                (key, template, value_schema)
            }
            None => {
                let task = task_prompt(function).to_string();
                let template = self.prompts.get(&task);
                (task, template, None)
            }
        };
        if let Some(hooks) = hooks {
            template.version = format!("{}:hook:{}", template.version, hooks.cache_version());
        }
        FunctionExtension {
            task,
            template_version: template.version,
            value_schema,
            generation,
        }
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
        let policy = self.policy(wb);
        let stats = self.session_stats();
        StatusSegment::new(name, policy.local, policy.send.as_str(), &stats)
    }

    /// Privacy snapshot for this workbook.
    #[must_use]
    pub fn policy(&self, wb: Option<&Workbook>) -> PolicySnapshot {
        self.policy_for(Slot::Default, wb)
    }

    /// Privacy snapshot for the provider selected by `slot`.
    #[must_use]
    pub fn policy_for(&self, slot: Slot, wb: Option<&Workbook>) -> PolicySnapshot {
        self.policy_for_config(&self.config, slot, wb)
    }

    /// Capture a live policy configuration for the runtime's routed provider.
    #[must_use]
    pub fn policy_for_config(
        &self,
        config: &Config,
        slot: Slot,
        wb: Option<&Workbook>,
    ) -> PolicySnapshot {
        let (name, _) = route_slot(&self.config, slot);
        let local = provider_is_local(&self.config, &name);
        PolicySnapshot::capture(config, wb, local)
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
        tools: Vec<ToolSpec>,
    ) -> Result<ChatResponse, AiError> {
        self.chat_task_routed(slot, task, user, schema, tools, None)
            .map(|reply| reply.response)
    }

    fn chat_task_routed(
        &self,
        slot: Slot,
        task: &str,
        user: String,
        mut schema: Option<Value>,
        mut tools: Vec<ToolSpec>,
        expected_extension_generation: Option<u64>,
    ) -> Result<RoutedResponse, AiError> {
        if !self.config.ai.enabled {
            return Err(
                AiError::new(codes::DISABLED, "AI is disabled").with_hint("run omacell ai setup")
            );
        }
        let task_name = canonical_task_name(task);
        let (custom, hooks, extension_generation) = {
            let inner = self.lock();
            (
                inner.tasks.get(&task_name).cloned(),
                inner.hooks.clone(),
                inner.extension_generation,
            )
        };
        if expected_extension_generation.is_some_and(|expected| expected != extension_generation) {
            return Err(AiError::new(
                codes::EXTENSIONS,
                "AI runtime extensions changed before the request was sent",
            ));
        }
        if let Some(custom) = &custom {
            if schema.is_none() {
                schema.clone_from(&custom.schema);
            }
            if tools.is_empty() {
                tools.clone_from(&custom.tools);
            }
        }
        let mut task_t = custom
            .as_ref()
            .map(custom_task_template)
            .unwrap_or_else(|| self.prompts.get(task));
        if let Some(hooks) = &hooks {
            task_t.version = format!("{}:hook:{}", task_t.version, hooks.cache_version());
        }
        let system = self.prompts.get("system");
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
        let (provider_name, model) = route_slot(&self.config, slot);
        let configured_provider = provider_name.clone();
        let request = AiHookRequest {
            task: task_name.clone(),
            provider: provider_name,
            model,
            messages,
            schema,
            tools,
            max_output_tokens: 1024,
        };
        let request = match &hooks {
            Some(hooks) => hooks.on_request(request)?,
            None => request,
        };
        validate_hook_request(&task_name, &request)?;
        let spec = self
            .config
            .ai
            .providers
            .get(&request.provider)
            .ok_or_else(|| {
                AiError::new(codes::KIND, format!("no provider {}", request.provider))
            })?;
        if provider_is_local(&self.config, &configured_provider)
            && !provider_is_local(&self.config, &request.provider)
        {
            return Err(AiError::new(
                codes::PAYLOAD,
                "AI request hook cannot route a local-provider payload to a cloud provider",
            ));
        }
        let provider = provider_from_config(&request.provider, spec, Arc::clone(&self.transport))?;
        let req = ChatRequest {
            model: request.model.clone(),
            messages: request.messages,
            json_schema: request.schema,
            tools: request.tools,
            stream: false,
            max_output_tokens: request.max_output_tokens,
            cancel: Cancel::new(),
            timeout: provider_timeout(spec),
        };
        validate_chat_request(&req)?;
        let request_record = json!({
            "task": task_name,
            "provider": request.provider,
            "model": req.model,
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
        let raw = self.handle.block_on(provider.chat(req))?;
        if self.lock().extension_generation != extension_generation {
            return Err(AiError::new(
                codes::EXTENSIONS,
                "AI runtime extensions changed while the request was in flight",
            ));
        }
        let raw_usage = raw.usage;
        let raw_streamed = raw.streamed;
        let response = AiHookResponse {
            task: task_name.clone(),
            provider: request.provider.clone(),
            model: request.model.clone(),
            response: raw,
        };
        let mut response = match &hooks {
            Some(hooks) => hooks.on_response(response)?,
            None => response,
        };
        validate_hook_response(&task_name, &request.provider, &request.model, &response)?;
        response.response.usage = raw_usage;
        response.response.streamed = raw_streamed;
        let out = response.response;
        if out.text.len() > MAX_MODEL_TEXT_BYTES {
            return Err(AiError::new(
                codes::PAYLOAD,
                "model text response exceeds the 1 MiB limit",
            ));
        }
        let bytes = out.text.len() as u64;
        self.lock().session.record(request_bytes);
        let log = AuditLog::open(&self.state_dir)?;
        let routed_provider = request.provider.clone();
        let routed_model = request.model.clone();
        log.append(&LogRecord {
            ts: now_ms(),
            task: task_name,
            provider: request.provider,
            model: request.model,
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
                .then(|| json!({"request": request_record, "response": out.text.clone()})),
        })?;
        Ok(RoutedResponse {
            response: out,
            provider: routed_provider,
            model: routed_model,
            template_version: task_t.version,
            extension_generation,
        })
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
                let extension = self.function_extension(&name);
                let value_schema = extension.value_schema.unwrap_or_else(|| json!({}));
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
                                    "value": value_schema
                                }
                            }
                        }
                    }
                });
                let routed = match self.chat_task_routed(
                    Slot::Default,
                    &extension.task,
                    user,
                    Some(schema),
                    vec![],
                    Some(extension.generation),
                ) {
                    Ok(reply) => reply,
                    Err(err) if err.code == codes::EXTENSIONS => continue,
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
                let results = match parse_batch_results(&routed.response.text, chunk.len()) {
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
                let provider = routed.provider;
                let model = routed.model;
                let version = routed.template_version.clone();
                let mut g = self.lock();
                if g.extension_generation != routed.extension_generation {
                    continue;
                }
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
                        prompt_tokens: routed.response.usage.prompt_tokens,
                        completion_tokens: routed.response.usage.completion_tokens,
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

fn canonical_task_name(name: &str) -> String {
    name.to_ascii_lowercase()
}

fn validate_task(task: &AiTaskSpec) -> Result<(), AiError> {
    let name = task.name.trim();
    if name.is_empty()
        || name.len() > MAX_CUSTOM_TASK_NAME_BYTES
        || !name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
    {
        return Err(AiError::new(
            codes::PAYLOAD,
            "custom AI task names use at most 128 ASCII letters, digits, dots, underscores, or hyphens",
        ));
    }
    if task.prompt.trim().is_empty() || task.prompt.len() > MAX_CUSTOM_PROMPT_BYTES {
        return Err(AiError::new(
            codes::PAYLOAD,
            format!("custom AI task prompts must be 1..={MAX_CUSTOM_PROMPT_BYTES} bytes"),
        ));
    }
    let request = ChatRequest {
        model: "validation".into(),
        messages: vec![ChatMessage {
            role: Role::User,
            content: task.prompt.clone(),
            tool_call_id: None,
            tool_calls: Vec::new(),
        }],
        json_schema: task.schema.clone(),
        tools: task.tools.clone(),
        stream: false,
        max_output_tokens: 1,
        cancel: Cancel::new(),
        timeout: Duration::from_secs(1),
    };
    validate_chat_request(&request)
}

fn custom_task_template(task: &AiTaskSpec) -> PromptTemplate {
    let version = hash_json(&json!({
        "name": task.name,
        "prompt": task.prompt,
        "schema": task.schema,
        "tools": task.tools,
    }));
    PromptTemplate {
        version: format!("lua:{version}"),
        body: task.prompt.clone(),
    }
}

fn validate_hook_request(expected_task: &str, request: &AiHookRequest) -> Result<(), AiError> {
    if request.task != expected_task {
        return Err(AiError::new(
            codes::PAYLOAD,
            "AI request hook cannot change the task name",
        ));
    }
    if request.provider.is_empty()
        || request.provider.len() > 256
        || request.model.is_empty()
        || request.model.len() > 4_096
        || request.max_output_tokens > 1_048_576
    {
        return Err(AiError::new(
            codes::PAYLOAD,
            "AI request hook returned invalid routing or token metadata",
        ));
    }
    Ok(())
}

fn validate_hook_response(
    task: &str,
    provider: &str,
    model: &str,
    response: &AiHookResponse,
) -> Result<(), AiError> {
    if response.task != task || response.provider != provider || response.model != model {
        return Err(AiError::new(
            codes::PAYLOAD,
            "AI response hook cannot change task or routing metadata",
        ));
    }
    if response.response.text.len() > MAX_MODEL_TEXT_BYTES
        || response.response.tool_calls.len() > 128
    {
        return Err(AiError::new(
            codes::PAYLOAD,
            "AI response hook output exceeds its size limit",
        ));
    }
    for call in &response.response.tool_calls {
        validate_tool_call(&call.id, &call.name, &call.arguments, codes::PAYLOAD)?;
    }
    Ok(())
}

impl AsyncNodeProvider for AiRuntime {
    fn evaluate(&self, key: ContentHash, req: &AsyncRequest) -> AsyncState {
        if !self.config.ai.enabled {
            return AsyncState::Failed {
                hint: "ai.disabled".into(),
            };
        }
        let args = args_json(&req.args);
        let extension = self.function_extension(&req.name);
        let version = extension.template_version;
        let (provider, model) = route_slot(&self.config, Slot::Default);
        let input_hash = hash_json(&args);
        let mut g = self.lock();
        let hooks_route = g.hooks.is_some();
        g.cells.insert(req.cell, key);
        if let Some(entry) = g.cache.get(key).cloned()
            && cache_fresh(
                &entry,
                &version,
                if hooks_route {
                    &entry.provider
                } else {
                    &provider
                },
                if hooks_route { &entry.model } else { &model },
                &input_hash,
            )
            && let Ok(rt) = json_to_runtime(&entry.value)
        {
            g.results.insert(key, rt);
            return AsyncState::Ready(omacell_core::value::Value::Empty);
        }
        if let Some(disk) = cache::read_disk(&self.cache_dir, key)
            && cache_fresh(
                &disk,
                &version,
                if hooks_route {
                    &disk.provider
                } else {
                    &provider
                },
                if hooks_route { &disk.model } else { &model },
                &input_hash,
            )
            && let Ok(rt) = json_to_runtime(&disk.value)
        {
            g.cache.insert(key, disk.clone());
            g.results.insert(key, rt);
            return AsyncState::Ready(omacell_core::value::Value::Empty);
        }
        let inserted = g
            .pending
            .insert(
                key,
                Job {
                    hash: key,
                    name: req.name.clone(),
                    args,
                },
            )
            .is_none();
        if inserted {
            g.pending_generation = g.pending_generation.wrapping_add(1);
        }
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
