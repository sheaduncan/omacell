//! Palette fuzzy ranking snapshot.

use insta::assert_debug_snapshot;
use omacell_bus::CommandJson;
use omacell_ui::Palette;

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
    assert!(p.prompt.as_ref().unwrap().contains("WP-23"));
    assert_debug_snapshot!((empty, cell));
}
