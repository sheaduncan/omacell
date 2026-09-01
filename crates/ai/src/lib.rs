//! AI provider protocols, privacy policy, workbook card, and audit log.
//!
//! Async is confined to this crate and MCP. WP-22: providers, card, redaction.
//! WP-23: in-app features and `AI()` functions.
#![forbid(unsafe_code)]
#![deny(missing_docs)]

pub mod agent;
pub mod anthropic;
pub mod audit;
pub mod audit_ai;
pub mod autopilot;
pub mod budget;
pub mod cache;
pub mod card;
pub mod complete;
pub mod error;
pub mod formula;
pub mod functions;
pub mod http;
pub mod import_assist;
pub mod openai;
pub mod plan;
pub mod policy;
pub mod prompts;
pub mod provider;
pub mod redact;
pub mod runtime;
pub mod secrets;
pub mod setup;

pub use agent::{Skill, load_skills, validate_tool};
pub use audit::{AuditLog, LogRecord, SessionStats, StatusSegment};
pub use autopilot::{AutopilotPolicy, AutopilotScope};
pub use budget::{RateLimit, UsageTotals, check_cell_budget};
pub use card::{CardLevel, CardRequest, estimate_tokens};
pub use error::AiError;
pub use functions::{is_ai_formula, register_ai_functions, strip_ai_formulas};
pub use http::{RecordingTransport, ReplayTransport, ReqwestTransport, SharedTransport};
pub use plan::{Plan, PlannedCommand, forbidden, parse_plan, to_calls};
pub use policy::{
    AI_PART, PolicySnapshot, SendLevel, WorkbookAi, build_card, fence_data, workbook_config_overlay,
};
pub use prompts::PromptSet;
pub use provider::{
    Cancel, ChatMessage, ChatRequest, ChatResponse, Provider, Role, Slot, ToolCall, ToolSpec,
    Usage, endpoint_is_loopback, provider_from_config, provider_timeout, route_slot,
};
pub use redact::{Kind, Suggestion, redact_json, redact_text};
pub use runtime::AiRuntime;
pub use setup::{DetectedProvider, SetupPatch, apply_setup_patch, detect_local};
