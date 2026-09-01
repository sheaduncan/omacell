//! In-app agent panel retained model.

use omacell_ui::{AgentPanel, AgentRole};

#[test]
fn agent_panel_retains_turns_draft_and_session_policy_status() {
    let mut panel = AgentPanel::default();
    panel.push_turn(AgentRole::User, "clean this sheet");
    panel.push_turn(AgentRole::Assistant, "proposed 2 commands");
    panel.draft = "next turn".into();
    panel.set_autopilot(true, "range Sheet1!A1:B10", 2, 12);

    let body = panel.body();
    assert!(body.contains("autopilot ON"));
    assert!(body.contains("2/12 ops"));
    assert!(body.contains("You: clean this sheet"));
    assert!(body.contains("Agent: proposed 2 commands"));
    assert!(body.contains("> next turn"));
}
