use criterion::{Criterion, criterion_group, criterion_main};
use omacell_ai::card::{CardLevel, CardRequest};
use omacell_ai::policy::{PolicySnapshot, SendLevel, build_card};
use omacell_core::limits::MAX_ROWS;
use omacell_core::workbook::Workbook;

fn million_cell_columns(c: &mut Criterion) {
    let mut workbook = Workbook::new();
    let sheet = workbook.active_sheet();
    workbook.set_cell_contents(sheet, 0, 0, "value").unwrap();
    for row in 1..MAX_ROWS {
        workbook.set_cell_contents(sheet, row, 0, "1").unwrap();
    }
    let policy = PolicySnapshot {
        enabled: true,
        send: SendLevel::Full,
        suggest_redaction: false,
        log_content: false,
        marks: Vec::new(),
        local: true,
    };
    let request = CardRequest {
        level: CardLevel::Columns,
        token_budget: 16_384,
        ..CardRequest::default()
    };
    c.bench_function("card_columns_1m_cells", |b| {
        b.iter(|| build_card(&workbook, None, request.clone(), &policy).unwrap())
    });
}

criterion_group!(benches, million_cell_columns);
criterion_main!(benches);
