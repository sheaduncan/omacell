//! Request accounting and rate limits from `[ai.functions]`.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use omacell_conf::schema::Config;

use crate::error::{AiError, codes};

/// In-process rate limiter.
#[derive(Debug)]
pub struct RateLimit {
    max_per_minute: u32,
    stamps: VecDeque<Instant>,
}

impl RateLimit {
    /// From config.
    #[must_use]
    pub fn from_config(config: &Config) -> Self {
        Self {
            max_per_minute: config.ai.functions.max_requests_per_minute,
            stamps: VecDeque::new(),
        }
    }

    /// Record a request or return `ai.budget`.
    pub fn allow(&mut self) -> Result<(), AiError> {
        let now = Instant::now();
        let window = Duration::from_secs(60);
        while self
            .stamps
            .front()
            .is_some_and(|t| now.duration_since(*t) > window)
        {
            self.stamps.pop_front();
        }
        if self.stamps.len() as u32 >= self.max_per_minute {
            return Err(AiError::new(
                codes::BUDGET,
                format!(
                    "rate limit {} requests/minute exceeded",
                    self.max_per_minute
                ),
            ));
        }
        self.stamps.push_back(now);
        Ok(())
    }

    /// Apply a live request-per-minute limit without discarding the current
    /// window's accounting.
    pub fn set_max_per_minute(&mut self, max_per_minute: u32) {
        self.max_per_minute = max_per_minute.max(1);
    }
}

/// Guardrail for AI-cell batches (WP-23 uses this).
pub fn check_cell_budget(config: &Config, cells: u32) -> Result<(), AiError> {
    check_cell_budget_limit(config.ai.functions.max_cells_per_recalc, cells)
}

/// Guardrail for an already-snapshotted live AI-function limit.
pub(crate) fn check_cell_budget_limit(max_cells: u32, cells: u32) -> Result<(), AiError> {
    if cells > max_cells {
        return Err(AiError::new(
            codes::BUDGET,
            format!(
                "{cells} AI cells exceeds max_cells_per_recalc {}",
                max_cells
            ),
        )
        .with_hint("raise [ai.functions] max_cells_per_recalc or shrink the range"));
    }
    Ok(())
}

/// Per-provider usage totals for `omacell ai usage --json`.
#[derive(Clone, Debug, Default, serde::Serialize)]
pub struct UsageTotals {
    /// Prompt tokens.
    pub prompt_tokens: u64,
    /// Completion tokens.
    pub completion_tokens: u64,
    /// Requests.
    pub requests: u64,
}
