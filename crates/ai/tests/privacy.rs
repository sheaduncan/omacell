//! Privacy choke-point tests.

use omacell_ai::card::{CardLevel, CardRequest};
use omacell_ai::import_assist::import_request_payload;
use omacell_ai::policy::{AI_PART, PolicySnapshot, SendLevel, WorkbookAi, build_card};
use omacell_ai::redact::redact_text;
use omacell_conf::schema::package_defaults;
use omacell_core::addr::{CellRef, RangeRef};
use omacell_core::ops::{
    Shift, delete_rows, insert_cells, insert_rows, move_range_cells, move_range_cells_between,
};
use omacell_core::sort::{SortKey, SortSpec, sort_range};
use omacell_core::workbook::Workbook;
use omacell_io::csv::{ImportPlan, PreviewCell, PreviewRows};

fn book_with_secret() -> Workbook {
    let mut wb = Workbook::new();
    let sheet = wb.active_sheet();
    wb.set_cell_contents(sheet, 0, 0, "email").unwrap();
    wb.set_cell_contents(sheet, 1, 0, "alice@example.com")
        .unwrap();
    wb.set_cell_contents(sheet, 0, 1, "n").unwrap();
    wb.set_cell_contents(sheet, 1, 1, "999888").unwrap();
    wb
}

#[test]
fn schema_level_payloads_contain_no_cell_values() {
    let wb = book_with_secret();
    let mut config = package_defaults().unwrap();
    config.ai.privacy.send = "schema".into();
    config.ai.privacy.local_full = false;
    let policy = PolicySnapshot::capture(&config, Some(&wb), false);
    assert_eq!(policy.send, SendLevel::Schema);
    let (card, _) = build_card(
        &wb,
        None,
        CardRequest {
            level: CardLevel::Sample,
            sample_rows: 5,
            ..CardRequest::default()
        },
        &policy,
    )
    .unwrap();
    let dumped = card.to_string();
    assert!(!dumped.contains("alice@example.com"), "{dumped}");
    assert!(!dumped.contains("999888"), "{dumped}");
    assert!(dumped.contains("formula_count") || dumped.contains("sheets"));
}

#[test]
fn import_preview_uses_the_same_privacy_boundary() {
    let mut config = package_defaults().unwrap();
    config.ai.privacy.send = "schema".into();
    config.ai.privacy.local_full = false;
    let schema = PolicySnapshot::capture(&config, None, false);
    let preview = PreviewRows {
        header: Some(vec!["email".into()]),
        rows: vec![vec![PreviewCell {
            raw: "alice@example.com".into(),
            would_become: "alice@example.com".into(),
            kind: "text".into(),
            changed: false,
        }]],
    };
    let payload = import_request_payload(ImportPlan::default(), preview.clone(), &schema).unwrap();
    let dumped = payload.to_string();
    assert!(!dumped.contains("alice@example.com"), "{dumped}");
    assert!(dumped.contains("\"kind\":\"text\""), "{dumped}");

    config.ai.privacy.send = "full".into();
    config.ai.privacy.suggest_redaction = true;
    let full = PolicySnapshot::capture(&config, None, false);
    let payload = import_request_payload(ImportPlan::default(), preview, &full).unwrap();
    let dumped = payload.to_string();
    assert!(dumped.contains("[REDACTED:email]"), "{dumped}");
    assert!(!dumped.contains("alice@example.com"), "{dumped}");
}

#[test]
fn redaction_applied_on_card_path() {
    let wb = book_with_secret();
    let mut config = package_defaults().unwrap();
    config.ai.privacy.send = "full".into();
    config.ai.privacy.suggest_redaction = true;
    let policy = PolicySnapshot::capture(&config, Some(&wb), true);
    let (card, suggestions) = build_card(
        &wb,
        None,
        CardRequest {
            level: CardLevel::Sample,
            sample_rows: 5,
            ..CardRequest::default()
        },
        &policy,
    )
    .unwrap();
    let dumped = card.to_string();
    assert!(
        dumped.contains("[REDACTED:email]")
            || suggestions.iter().any(|s| s.kind.as_str() == "email")
    );
    assert!(!dumped.contains("alice@example.com"), "{dumped}");
}

#[test]
fn detected_numeric_columns_do_not_leak_min_or_max() {
    let mut wb = Workbook::new();
    let sheet = wb.active_sheet();
    wb.set_cell_contents(sheet, 0, 0, "payment card").unwrap();
    wb.set_cell_contents(sheet, 1, 0, "4111111111111111")
        .unwrap();
    let mut config = package_defaults().unwrap();
    config.ai.privacy.send = "full".into();
    config.ai.privacy.suggest_redaction = true;
    let policy = PolicySnapshot::capture(&config, Some(&wb), false);
    let (card, suggestions) = build_card(
        &wb,
        None,
        CardRequest {
            level: CardLevel::Columns,
            ..CardRequest::default()
        },
        &policy,
    )
    .unwrap();
    let dumped = card.to_string();
    assert!(!dumped.contains("4111111111111111"), "{dumped}");
    assert!(suggestions.iter().any(|item| item.kind.as_str() == "card"));
    assert!(card["columns"][0].get("min").is_none(), "{dumped}");
    assert!(card["columns"][0].get("max").is_none(), "{dumped}");
}

#[test]
fn workbook_override_and_loopback_defaults() {
    let mut wb = Workbook::new();
    let part = WorkbookAi {
        privacy_send: Some("schema".into()),
        redact: vec![],
    };
    wb.custom_parts
        .insert(AI_PART.into(), serde_json::to_vec(&part).unwrap());
    let mut config = package_defaults().unwrap();
    config.ai.privacy.send = "schema".into();
    config.ai.privacy.local_full = true;
    let local = PolicySnapshot::capture(&config, None, true);
    assert_eq!(local.send, SendLevel::Full);
    let cloud = PolicySnapshot::capture(&config, None, false);
    assert_eq!(cloud.send, SendLevel::Schema);
    let over = PolicySnapshot::capture(&config, Some(&wb), true);
    assert_eq!(over.send, SendLevel::Schema);
}

#[test]
fn accepted_redact_marks_replace_cells() {
    let mut wb = Workbook::new();
    let sheet = wb.active_sheet();
    wb.set_cell_contents(sheet, 0, 0, "Name").unwrap();
    wb.set_cell_contents(sheet, 1, 0, "Ada").unwrap();
    wb.custom_parts.insert(
        AI_PART.into(),
        serde_json::to_vec(&WorkbookAi {
            privacy_send: Some("full".into()),
            redact: vec!["Sheet1!A2".into()],
        })
        .unwrap(),
    );
    let mut config = package_defaults().unwrap();
    config.ai.privacy.send = "full".into();
    config.ai.privacy.suggest_redaction = false;
    let policy = PolicySnapshot::capture(&config, Some(&wb), true);
    let (card, _) = build_card(
        &wb,
        None,
        CardRequest {
            level: CardLevel::Sample,
            sample_rows: 5,
            ..CardRequest::default()
        },
        &policy,
    )
    .unwrap();
    let dumped = card.to_string();
    assert!(dumped.contains("[REDACTED:mark]"), "{dumped}");
    assert!(!dumped.contains("Ada"), "{dumped}");
}

#[test]
fn accepted_redact_marks_replace_column_headers() {
    let mut wb = Workbook::new();
    let sheet = wb.active_sheet();
    wb.set_cell_contents(sheet, 0, 0, "PRIVATE_REVIEW_HEADER")
        .unwrap();
    wb.set_number(sheet, 1, 0, 1.0).unwrap();
    wb.custom_parts.insert(
        AI_PART.into(),
        serde_json::to_vec(&WorkbookAi {
            privacy_send: Some("full".into()),
            redact: vec!["Sheet1!A1".into()],
        })
        .unwrap(),
    );
    let mut config = package_defaults().unwrap();
    config.ai.privacy.send = "full".into();
    config.ai.privacy.suggest_redaction = false;
    let policy = PolicySnapshot::capture(&config, Some(&wb), true);
    let (card, _) = build_card(
        &wb,
        None,
        CardRequest {
            level: CardLevel::Columns,
            ..CardRequest::default()
        },
        &policy,
    )
    .unwrap();
    let dumped = card.to_string();
    assert!(dumped.contains("[REDACTED:mark]"), "{dumped}");
    assert!(!dumped.contains("PRIVATE_REVIEW_HEADER"), "{dumped}");
}

#[test]
fn accepted_redact_marks_follow_structural_row_edits() {
    let mut wb = Workbook::new();
    let sheet = wb.active_sheet();
    wb.set_cell_contents(sheet, 0, 0, "Header").unwrap();
    wb.set_cell_contents(sheet, 1, 0, "PRIVATE_REVIEW_PAYROLL")
        .unwrap();
    wb.custom_parts.insert(
        AI_PART.into(),
        serde_json::to_vec(&WorkbookAi {
            privacy_send: Some("full".into()),
            redact: vec!["Sheet1!A2".into()],
        })
        .unwrap(),
    );

    insert_rows(&mut wb, sheet, 1, 1).unwrap();

    let mut config = package_defaults().unwrap();
    config.ai.privacy.send = "full".into();
    config.ai.privacy.suggest_redaction = false;
    let policy = PolicySnapshot::capture(&config, Some(&wb), true);
    assert!(policy.marks.iter().any(|mark| mark == "Sheet1!A3"));
    let (card, _) = build_card(
        &wb,
        None,
        CardRequest {
            level: CardLevel::Full,
            range: Some("Sheet1!A1:A3".into()),
            ..CardRequest::default()
        },
        &policy,
    )
    .unwrap();
    let dumped = card.to_string();
    assert!(dumped.contains("[REDACTED:mark]"), "{dumped}");
    assert!(!dumped.contains("PRIVATE_REVIEW_PAYROLL"), "{dumped}");

    wb.undo().unwrap();
    assert_marked_secret_absent(&wb, "Sheet1!A1:A2", "PRIVATE_REVIEW_PAYROLL");
    wb.redo().unwrap();
    assert_marked_secret_absent(&wb, "Sheet1!A1:A3", "PRIVATE_REVIEW_PAYROLL");
}

#[test]
fn accepted_redact_marks_follow_move_and_sort_permutations() {
    let mut moved = Workbook::new();
    let sheet = moved.active_sheet();
    moved.set_text(sheet, 1, 0, "PRIVATE_MOVED_VALUE").unwrap();
    install_mark(&mut moved, "Sheet1!A2");
    moved
        .transact_try(|workbook| {
            move_range_cells(
                workbook,
                sheet,
                RangeRef::from_corners(CellRef::new(1, 0).unwrap(), CellRef::new(1, 0).unwrap()),
                CellRef::new(2, 1).unwrap(),
            )
        })
        .unwrap();
    assert_marked_secret_absent(&moved, "Sheet1!A1:B3", "PRIVATE_MOVED_VALUE");
    moved.undo().unwrap();
    assert_marked_secret_absent(&moved, "Sheet1!A1:B3", "PRIVATE_MOVED_VALUE");

    let mut sorted = Workbook::new();
    let sheet = sorted.active_sheet();
    sorted.set_number(sheet, 1, 0, 2.0).unwrap();
    sorted
        .set_text(sheet, 1, 1, "PRIVATE_SORTED_VALUE")
        .unwrap();
    sorted.set_number(sheet, 2, 0, 1.0).unwrap();
    install_mark(&mut sorted, "Sheet1!B2");
    sort_range(
        &mut sorted,
        sheet,
        RangeRef::from_corners(CellRef::new(1, 0).unwrap(), CellRef::new(2, 1).unwrap()),
        &SortSpec {
            keys: vec![SortKey {
                offset: 0,
                descending: false,
                by: Default::default(),
                custom_list: Vec::new(),
            }],
            ..SortSpec::default()
        },
    )
    .unwrap();
    assert_marked_secret_absent(&sorted, "Sheet1!A1:B3", "PRIVATE_SORTED_VALUE");
    sorted.undo().unwrap();
    assert_marked_secret_absent(&sorted, "Sheet1!A1:B3", "PRIVATE_SORTED_VALUE");
}

#[test]
fn accepted_redact_marks_follow_delete_band_cross_sheet_and_rename_edits() {
    let mut deleted = Workbook::new();
    let sheet = deleted.active_sheet();
    deleted
        .set_text(sheet, 2, 0, "PRIVATE_DELETED_SHIFT")
        .unwrap();
    install_mark(&mut deleted, "Sheet1!A3");
    deleted
        .transact_try(|workbook| delete_rows(workbook, sheet, 1, 1))
        .unwrap();
    assert_marked_secret_absent(&deleted, "Sheet1!A1:A2", "PRIVATE_DELETED_SHIFT");
    deleted.undo().unwrap();
    assert_marked_secret_absent(&deleted, "Sheet1!A1:A3", "PRIVATE_DELETED_SHIFT");

    let mut band = Workbook::new();
    let sheet = band.active_sheet();
    band.set_text(sheet, 1, 1, "PRIVATE_BAND_SHIFT").unwrap();
    install_mark(&mut band, "Sheet1!B2");
    band.transact_try(|workbook| {
        insert_cells(
            workbook,
            sheet,
            RangeRef::from_corners(CellRef::new(1, 1).unwrap(), CellRef::new(1, 1).unwrap()),
            Shift::Down,
        )
    })
    .unwrap();
    assert_marked_secret_absent(&band, "Sheet1!A1:B3", "PRIVATE_BAND_SHIFT");

    let mut crossed = Workbook::new();
    let source = crossed.active_sheet();
    let destination = crossed.add_sheet("Destination").unwrap();
    crossed
        .set_text(source, 1, 0, "PRIVATE_CROSS_SHEET")
        .unwrap();
    install_mark(&mut crossed, "Sheet1!A2");
    crossed
        .transact_try(|workbook| {
            move_range_cells_between(
                workbook,
                source,
                RangeRef::from_corners(CellRef::new(1, 0).unwrap(), CellRef::new(1, 0).unwrap()),
                destination,
                CellRef::new(2, 1).unwrap(),
            )
        })
        .unwrap();
    assert_marked_secret_absent(&crossed, "Destination!A1:B3", "PRIVATE_CROSS_SHEET");

    crossed.rename_sheet(destination, "Renamed").unwrap();
    assert_marked_secret_absent(&crossed, "Renamed!A1:B3", "PRIVATE_CROSS_SHEET");
    crossed.undo().unwrap();
    assert_marked_secret_absent(&crossed, "Destination!A1:B3", "PRIVATE_CROSS_SHEET");
}

fn install_mark(workbook: &mut Workbook, mark: &str) {
    workbook.custom_parts.insert(
        AI_PART.into(),
        serde_json::to_vec(&WorkbookAi {
            privacy_send: Some("full".into()),
            redact: vec![mark.into()],
        })
        .unwrap(),
    );
}

fn assert_marked_secret_absent(workbook: &Workbook, range: &str, secret: &str) {
    let mut config = package_defaults().unwrap();
    config.ai.privacy.send = "full".into();
    config.ai.privacy.suggest_redaction = false;
    let policy = PolicySnapshot::capture(&config, Some(workbook), true);
    let (card, _) = build_card(
        workbook,
        None,
        CardRequest {
            level: CardLevel::Full,
            range: Some(range.into()),
            ..CardRequest::default()
        },
        &policy,
    )
    .unwrap();
    let dumped = card.to_string();
    assert!(!dumped.contains(secret), "{dumped}");
}

#[test]
fn redaction_applies_to_cell_inputs_and_image_alts() {
    let (text, suggestions) = redact_text("contact alice@example.com or +1 415 555 0100");
    assert!(text.contains("[REDACTED:email]"), "{text}");
    assert!(text.contains("[REDACTED:phone]"), "{text}");
    assert!(suggestions.iter().any(|s| s.kind.as_str() == "email"));
    assert!(suggestions.iter().all(|s| !s.sample.contains('@')));
    let (card, _) = redact_text("4111111111111111 GB82WEST12345698765432 123-45-6789");
    assert!(card.contains("[REDACTED:card]"), "{card}");
    assert!(card.contains("[REDACTED:iban]"), "{card}");
    assert!(card.contains("[REDACTED:national-id]"), "{card}");
}

#[test]
fn malformed_workbook_privacy_fails_closed() {
    let mut wb = Workbook::new();
    wb.custom_parts.insert(AI_PART.into(), b"not json".to_vec());
    let mut config = package_defaults().unwrap();
    config.ai.privacy.send = "full".into();
    config.ai.privacy.local_full = true;
    let policy = PolicySnapshot::capture(&config, Some(&wb), true);
    assert_eq!(policy.send, SendLevel::Schema);

    wb.custom_parts.insert(
        AI_PART.into(),
        serde_json::to_vec(&WorkbookAi {
            privacy_send: Some("unrestricted".into()),
            redact: vec![],
        })
        .unwrap(),
    );
    let policy = PolicySnapshot::capture(&config, Some(&wb), true);
    assert_eq!(policy.send, SendLevel::Schema);
}

#[test]
fn schema_keeps_formulas_but_strips_values() {
    let mut wb = Workbook::new();
    let sheet = wb.active_sheet();
    wb.set_cell_contents(sheet, 0, 0, "secret@example.com")
        .unwrap();
    wb.set_cell_contents(sheet, 0, 1, "=A1&\"secret@example.com\"")
        .unwrap();
    let mut config = package_defaults().unwrap();
    config.ai.privacy.send = "schema".into();
    config.ai.privacy.suggest_redaction = true;
    let policy = PolicySnapshot::capture(&config, Some(&wb), false);
    let (card, _) = build_card(
        &wb,
        None,
        CardRequest {
            level: CardLevel::Full,
            range: Some("Sheet1!A1:B1".into()),
            token_budget: 4_096,
            ..CardRequest::default()
        },
        &policy,
    )
    .unwrap();
    let dumped = card.to_string();
    assert!(dumped.contains("formula"), "{dumped}");
    assert!(!dumped.contains("\"value\""), "{dumped}");
    assert!(!dumped.contains("secret@example.com"), "{dumped}");
}

#[test]
fn full_cards_are_paginated_and_budgeted() {
    let mut wb = Workbook::new();
    let sheet = wb.active_sheet();
    wb.set_cell_contents(sheet, 0, 0, "first").unwrap();
    wb.set_cell_contents(sheet, 1_048_575, 0, "last").unwrap();
    let policy = PolicySnapshot {
        enabled: true,
        send: SendLevel::Full,
        suggest_redaction: false,
        log_content: false,
        marks: vec![],
        local: true,
    };
    let (card, _) = build_card(
        &wb,
        None,
        CardRequest {
            level: CardLevel::Full,
            range: Some("Sheet1!A1:A1048576".into()),
            limit: 64,
            token_budget: 2_048,
            ..CardRequest::default()
        },
        &policy,
    )
    .unwrap();
    assert!(card["page"]["truncated"].as_bool().unwrap());
    assert!(card["page"]["returned_rows"].as_u64().unwrap() <= 64);
    assert!(card["tokens"].as_u64().unwrap() <= 2_048);
    assert_eq!(
        card["tokens"].as_u64().unwrap() as usize,
        omacell_ai::estimate_tokens(&card)
    );
    assert!(card["truncated"].as_bool().unwrap());
}
