//! Palette fuzzy ranking snapshot.

use insta::assert_debug_snapshot;
use omacell_bus::CommandJson;
use omacell_ui::{AiPlanProvider, Palette};

fn cmd(id: &str, doc: &str) -> CommandJson {
    CommandJson {
        id: id.into(),
        doc: doc.into(),
        mutating: false,
        changeset_eligible: false,
        default_keys: vec![],
        arg_schema: serde_json::json!({"type": "object"}),
    }
}

#[test]
fn ranking_is_stable() {
    let commands = vec![
        cmd("cell.set", "Set a cell"),
        cmd("cell.clear", "Clear a cell"),
        cmd("view.freeze", "Freeze panes"),
        cmd("nav.left", "Move left"),
        cmd("palette.open", "Open palette"),
    ];
    let mut p = Palette::default();
    p.remember("view.freeze");
    p.rank(&commands, "");
    let empty: Vec<_> = p.hits.iter().map(|h| h.id.clone()).collect();
    p.rank(&commands, "cell");
    let cell: Vec<_> = p.hits.iter().map(|h| h.id.clone()).collect();
    p.rank(&commands, "?hello");
    assert_eq!(
        p.prompt.as_deref(),
        Some("Press Enter to propose an AI plan")
    );
    assert_debug_snapshot!((empty, cell));
}

struct Planner;

impl AiPlanProvider for Planner {
    fn plan(&self, prompt: &str) -> Option<String> {
        Some(format!("plan: {prompt}"))
    }
}

#[test]
fn ai_provider_and_schema_prompt_are_reachable() {
    let mut command = cmd("view.select", "Select a range");
    command.arg_schema = serde_json::json!({
        "type": "object",
        "properties": {"range": {"type": "string"}},
        "required": ["range"]
    });
    let mut palette = Palette::default();
    palette.rank_with_ai(&[command.clone()], "?select A1", Some(&Planner));
    assert_eq!(palette.prompt.as_deref(), Some("plan: select A1"));
    palette.prompt_for(&command);
    assert_eq!(palette.prompt.as_deref(), Some("range: string"));
}
