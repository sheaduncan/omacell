//! Retained in-app agent conversation panel.

/// Speaker in the in-app agent transcript.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AgentRole {
    /// Interactive user.
    User,
    /// In-app model.
    Assistant,
    /// Local policy/status message.
    System,
}

/// One bounded transcript row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentTurn {
    /// Speaker.
    pub role: AgentRole,
    /// Display text.
    pub text: String,
}

/// Toolkit-neutral in-app agent panel state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentPanel {
    /// Recent transcript rows.
    pub turns: Vec<AgentTurn>,
    /// Current prompt entry.
    pub draft: String,
    /// A turn is running.
    pub busy: bool,
    /// Explicit per-session autopilot consent.
    pub autopilot: bool,
    /// Human-readable selected scope.
    pub scope: String,
    /// Charged operations.
    pub used_ops: usize,
    /// Session operation cap.
    pub max_ops: usize,
}

impl Default for AgentPanel {
    fn default() -> Self {
        Self {
            turns: Vec::new(),
            draft: String::new(),
            busy: false,
            autopilot: false,
            scope: "review required".into(),
            used_ops: 0,
            max_ops: 0,
        }
    }
}

impl AgentPanel {
    /// Append one bounded display row.
    pub fn push_turn(&mut self, role: AgentRole, text: impl Into<String>) {
        const MAX_TURNS: usize = 200;
        const MAX_TURN_CHARS: usize = 8_192;
        let mut text = text.into();
        if text.chars().count() > MAX_TURN_CHARS {
            text = text.chars().take(MAX_TURN_CHARS).collect();
            text.push('…');
        }
        self.turns.push(AgentTurn { role, text });
        if self.turns.len() > MAX_TURNS {
            let excess = self.turns.len() - MAX_TURNS;
            self.turns.drain(..excess);
        }
    }

    /// Update explicit consent and operation accounting.
    pub fn set_autopilot(
        &mut self,
        enabled: bool,
        scope: impl Into<String>,
        used_ops: usize,
        max_ops: usize,
    ) {
        self.autopilot = enabled;
        self.scope = scope.into();
        self.used_ops = used_ops;
        self.max_ops = max_ops;
    }

    /// Human-readable transcript and keyboard help.
    #[must_use]
    pub fn body(&self) -> String {
        let state = if self.autopilot { "ON" } else { "off" };
        let mut lines = vec![format!(
            "autopilot {state} · {} · {}/{} ops",
            self.scope, self.used_ops, self.max_ops
        )];
        lines.push("F8 toggles explicit session autopilot · Esc closes".into());
        lines.push(String::new());
        for turn in &self.turns {
            let label = match turn.role {
                AgentRole::User => "You",
                AgentRole::Assistant => "Agent",
                AgentRole::System => "Policy",
            };
            lines.push(format!("{label}: {}", turn.text));
        }
        lines.push(String::new());
        lines.push(format!("> {}", self.draft));
        if self.busy {
            lines.push("working… (Esc cancels the active request)".into());
        } else {
            lines.push("Enter sends · Backspace edits".into());
        }
        lines.join("\n")
    }
}
