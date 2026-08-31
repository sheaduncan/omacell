//! Rate-limit and cell-budget guards.

use omacell_ai::budget::{RateLimit, check_cell_budget};
use omacell_conf::schema::package_defaults;

#[test]
fn rate_limit_trips() {
    let mut config = package_defaults().unwrap();
    config.ai.functions.max_requests_per_minute = 2;
    let mut limit = RateLimit::from_config(&config);
    limit.allow().unwrap();
    limit.allow().unwrap();
    let err = limit.allow().unwrap_err();
    assert_eq!(err.code, "ai.budget");
}

#[test]
fn cell_budget_trips() {
    let mut config = package_defaults().unwrap();
    config.ai.functions.max_cells_per_recalc = 3;
    check_cell_budget(&config, 3).unwrap();
    let err = check_cell_budget(&config, 4).unwrap_err();
    assert_eq!(err.code, "ai.budget");
}
