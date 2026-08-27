//! Core workbook model, formula engine, styles, and product identity for Omacell.
//!
//! Public types freeze after Gate G0 (WP-01). This crate has no I/O, no toolkit, and no async.
//!
//! Filled in by WP-01, WP-02, WP-03, WP-04, and WP-06.
#![forbid(unsafe_code)]
#![deny(missing_docs)]

pub mod product;

pub use product::{PRODUCT_DISPLAY_NAME, PRODUCT_NAME};
