//! Core workbook model, formula engine, styles, and product identity for Omacell.
//!
//! Public types freeze after Gate G0 (WP-01). This crate has no I/O, no toolkit, and no async.
//!
//! Filled in by WP-01, WP-02, WP-03, WP-04, and WP-06.
//!
//! WP-02 adds the in-memory workbook (`workbook`, `sheet`, `storage`, `intern`,
//! `geometry`, `names`, `tables`, `undo`). Formula parsing is WP-03.
#![forbid(unsafe_code)]
#![deny(missing_docs)]
#![warn(rustdoc::broken_intra_doc_links)]

pub mod addr;
pub mod changeset;
pub mod command;
pub mod date_system;
pub mod error;
pub mod event;
pub mod formula;
pub mod geometry;
pub mod intern;
pub mod limits;
pub mod locale;
pub mod names;
pub mod product;
pub mod sheet;
pub mod storage;
pub mod style;
pub mod tables;
pub mod undo;
pub mod value;
pub mod workbook;

pub use product::{PRODUCT_DISPLAY_NAME, PRODUCT_NAME};

#[cfg(test)]
mod corpus;
