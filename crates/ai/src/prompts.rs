//! Versioned Markdown prompt templates.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::error::{AiError, codes};

/// One loaded template.
#[derive(Clone, Debug)]
pub struct PromptTemplate {
    /// Stable version string (invalidates AI-cell cache).
    pub version: String,
    /// Template body.
    pub body: String,
}

/// File-overridable prompt set.
#[derive(Clone, Debug, Default)]
pub struct PromptSet {
    templates: BTreeMap<String, PromptTemplate>,
}

impl PromptSet {
    /// Load package defaults then sparse user overrides.
    pub fn load(default_dir: &Path, user_dir: Option<&Path>) -> Result<Self, AiError> {
        let mut set = Self::default();
        set.load_dir(&default_dir.join("ai/prompts"))?;
        if let Some(user) = user_dir {
            set.load_dir(&user.join("ai/prompts"))?;
        }
        Ok(set)
    }

    /// Package-default prompts compiled into the binary as fallbacks.
    #[must_use]
    pub fn builtin() -> Self {
        let mut set = Self::default();
        for (name, body) in [
            (
                "system",
                include_str!("../../../default/ai/prompts/system.md"),
            ),
            ("cell", include_str!("../../../default/ai/prompts/cell.md")),
            ("plan", include_str!("../../../default/ai/prompts/plan.md")),
            (
                "formula",
                include_str!("../../../default/ai/prompts/formula.md"),
            ),
            (
                "formula_explain",
                include_str!("../../../default/ai/prompts/formula_explain.md"),
            ),
            (
                "formula_fix",
                include_str!("../../../default/ai/prompts/formula_fix.md"),
            ),
            (
                "formula_refactor",
                include_str!("../../../default/ai/prompts/formula_refactor.md"),
            ),
            (
                "complete",
                include_str!("../../../default/ai/prompts/complete.md"),
            ),
            (
                "import",
                include_str!("../../../default/ai/prompts/import.md"),
            ),
            (
                "audit",
                include_str!("../../../default/ai/prompts/audit.md"),
            ),
            (
                "describe",
                include_str!("../../../default/ai/prompts/describe.md"),
            ),
            (
                "agent",
                include_str!("../../../default/ai/prompts/agent.md"),
            ),
            (
                "extract",
                include_str!("../../../default/ai/prompts/extract.md"),
            ),
            (
                "classify",
                include_str!("../../../default/ai/prompts/classify.md"),
            ),
            ("fill", include_str!("../../../default/ai/prompts/fill.md")),
            (
                "table",
                include_str!("../../../default/ai/prompts/table.md"),
            ),
            (
                "translate",
                include_str!("../../../default/ai/prompts/translate.md"),
            ),
        ] {
            set.templates.insert(name.into(), parse_template(body));
        }
        set
    }

    fn load_dir(&mut self, dir: &Path) -> Result<(), AiError> {
        if !dir.is_dir() {
            return Ok(());
        }
        let mut names: Vec<PathBuf> = std::fs::read_dir(dir)
            .map_err(|err| AiError::new(codes::PAYLOAD, err.to_string()))?
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.extension().is_some_and(|ext| ext == "md"))
            .collect();
        names.sort();
        for path in names {
            let stem = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("unknown")
                .to_string();
            let text = std::fs::read_to_string(&path)
                .map_err(|err| AiError::new(codes::PAYLOAD, err.to_string()))?;
            self.templates.insert(stem, parse_template(&text));
        }
        Ok(())
    }

    /// Template by task name.
    #[must_use]
    pub fn get(&self, name: &str) -> PromptTemplate {
        self.templates
            .get(name)
            .cloned()
            .unwrap_or_else(|| parse_template("<!-- version: 0 -->\n"))
    }
}

fn parse_template(text: &str) -> PromptTemplate {
    let version = text
        .lines()
        .find_map(|line| {
            line.split("version:")
                .nth(1)
                .map(|rest| rest.trim().trim_end_matches("-->").trim().to_string())
        })
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| "1".into());
    PromptTemplate {
        version,
        body: text.to_string(),
    }
}
