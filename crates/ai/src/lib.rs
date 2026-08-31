//! AI provider protocols, privacy policy, workbook card, and audit log.
//!
//! Async is confined to this crate and MCP. WP-22: providers, card, redaction.
//! WP-23: in-app features and `AI()` functions.
#![forbid(unsafe_code)]
#![deny(missing_docs)]

pub mod anthropic;
pub mod audit;
pub mod budget;
pub mod card;
pub mod error;
pub mod http;
pub mod openai;
pub mod policy;
pub mod provider;
pub mod redact;
pub mod secrets;
pub mod setup;

pub use audit::{AuditLog, LogRecord, SessionStats, StatusSegment};
pub use budget::{RateLimit, UsageTotals, check_cell_budget};
pub use card::{CardLevel, CardRequest, estimate_tokens};
pub use error::AiError;
pub use http::{RecordingTransport, ReplayTransport, ReqwestTransport, SharedTransport};
pub use policy::{
    AI_PART, PolicySnapshot, SendLevel, WorkbookAi, build_card, fence_data, workbook_config_overlay,
};
pub use provider::{
    Cancel, ChatMessage, ChatRequest, ChatResponse, Provider, Role, Slot, ToolCall, ToolSpec,
    Usage, endpoint_is_loopback, provider_from_config, provider_timeout, route_slot,
};
pub use redact::{Kind, Suggestion, redact_json, redact_text};
pub use setup::{DetectedProvider, SetupPatch, apply_setup_patch, detect_local};
