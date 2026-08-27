//! Tables (structured ranges) registry (spec F-1.4).
//!
//! Structured-reference parsing is WP-03. Auto-expand on adjacent entry is
//! WP-17/WP-18; this package stores the flag.

use rustc_hash::FxHashMap;
use serde::{Deserialize, Serialize};

use crate::addr::SheetId;
use crate::error::CoreError;
use crate::names::validate_defined_name;

/// Handle for a table in a workbook.
///
/// ```
/// use omacell_core::tables::TableId;
/// assert_eq!(TableId::new(0).index(), 0);
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct TableId(u32);

impl TableId {
    /// Wrap an index.
    #[must_use]
    pub const fn new(index: u32) -> Self {
        Self(index)
    }

    /// Numeric id.
    #[must_use]
    pub const fn index(self) -> u32 {
        self.0
    }
}

/// One table column.
///
/// ```
/// use omacell_core::tables::TableColumn;
/// let c = TableColumn { name: "Amount".into() };
/// assert_eq!(c.name, "Amount");
/// ```
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TableColumn {
    /// Header caption.
    pub name: String,
}

/// A structured table.
///
/// ```
/// use omacell_core::addr::SheetId;
/// use omacell_core::tables::{Table, TableId};
/// let t = Table::new(TableId::new(0), "Sales", SheetId::new(0), 0, 0, 10, 2);
/// assert!(t.has_header);
/// ```
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Table {
    /// Stable id.
    pub id: TableId,
    /// Table name (unique in the workbook, case-insensitive).
    pub name: String,
    /// Sheet the range lives on.
    pub sheet: SheetId,
    /// Inclusive start row.
    pub start_row: u32,
    /// Inclusive start column.
    pub start_col: u16,
    /// Inclusive end row.
    pub end_row: u32,
    /// Inclusive end column.
    pub end_col: u16,
    /// Header row present (Excel default).
    pub has_header: bool,
    /// Totals row present.
    pub has_totals: bool,
    /// Banded row style.
    pub banded_rows: bool,
    /// Banded column style.
    pub banded_cols: bool,
    /// Auto-expand on adjacent entry (WP-17).
    pub auto_expand: bool,
    /// Column headers.
    pub columns: Vec<TableColumn>,
}

impl Table {
    /// Construct a table with Excel-like defaults (header on, auto-expand on).
    #[must_use]
    pub fn new(
        id: TableId,
        name: impl Into<String>,
        sheet: SheetId,
        start_row: u32,
        start_col: u16,
        end_row: u32,
        end_col: u16,
    ) -> Self {
        let n_cols = u32::from(end_col.saturating_sub(start_col)) + 1;
        let columns = (0..n_cols)
            .map(|i| TableColumn {
                name: format!("Column{}", i + 1),
            })
            .collect();
        Self {
            id,
            name: name.into(),
            sheet,
            start_row,
            start_col,
            end_row,
            end_col,
            has_header: true,
            has_totals: false,
            banded_rows: true,
            banded_cols: false,
            auto_expand: true,
            columns,
        }
    }
}

/// Workbook table registry.
///
/// ```
/// use omacell_core::addr::SheetId;
/// use omacell_core::tables::{Table, TableRegistry};
/// let mut r = TableRegistry::new();
/// let id = r.insert(Table::new(
///     omacell_core::tables::TableId::new(0),
///     "Sales",
///     SheetId::new(0),
///     0, 0, 4, 1,
/// )).unwrap();
/// assert_eq!(r.get(id).unwrap().name, "Sales");
/// ```
#[derive(Clone, Debug, Default)]
pub struct TableRegistry {
    tables: FxHashMap<TableId, Table>,
    by_name: FxHashMap<String, TableId>,
    next: u32,
}

impl TableRegistry {
    /// Empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert a table. `table.id` is overwritten with a fresh id.
    pub fn insert(&mut self, mut table: Table) -> Result<TableId, CoreError> {
        validate_table_name(&table.name)?;
        let nk = table.name.to_lowercase();
        if self.by_name.contains_key(&nk) {
            return Err(CoreError::table_name(format!(
                "table name {:?} already exists",
                table.name
            )));
        }
        let id = TableId::new(self.next);
        self.next += 1;
        table.id = id;
        self.by_name.insert(nk, id);
        self.tables.insert(id, table);
        Ok(id)
    }

    /// Re-insert a table keeping its id (undo restore).
    pub fn restore(&mut self, table: Table) -> Result<(), CoreError> {
        validate_table_name(&table.name)?;
        let nk = table.name.to_lowercase();
        if let Some(&id) = self.by_name.get(&nk)
            && id != table.id
        {
            return Err(CoreError::table_name(format!(
                "table name {:?} already exists",
                table.name
            )));
        }
        self.by_name.insert(nk, table.id);
        if table.id.index() >= self.next {
            self.next = table.id.index() + 1;
        }
        self.tables.insert(table.id, table);
        Ok(())
    }

    /// Remove by id.
    pub fn remove(&mut self, id: TableId) -> Option<Table> {
        let t = self.tables.remove(&id)?;
        self.by_name.remove(&t.name.to_lowercase());
        Some(t)
    }

    /// Borrow a table.
    #[must_use]
    pub fn get(&self, id: TableId) -> Option<&Table> {
        self.tables.get(&id)
    }

    /// Lookup by name (case-insensitive).
    #[must_use]
    pub fn get_by_name(&self, name: &str) -> Option<&Table> {
        self.by_name
            .get(&name.to_lowercase())
            .and_then(|id| self.tables.get(id))
    }

    /// Sorted by id.
    pub fn iter(&self) -> impl Iterator<Item = &Table> {
        let mut ids: Vec<TableId> = self.tables.keys().copied().collect();
        ids.sort_unstable();
        ids.into_iter().filter_map(|id| self.tables.get(&id))
    }

    /// Count.
    #[must_use]
    pub fn len(&self) -> usize {
        self.tables.len()
    }

    /// Empty?
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.tables.is_empty()
    }
}

fn validate_table_name(name: &str) -> Result<(), CoreError> {
    validate_defined_name(name).map_err(|e| CoreError::table_name(e.message))
}
