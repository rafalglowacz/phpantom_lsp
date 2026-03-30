//! Type cleaning and classification utilities for PHPDoc types.
//!
//! This module was split into focused submodules for navigability:
//!
//! - [`super::type_strings`]: Foundational type string manipulation (constants,
//!   splitting, cleaning, stripping, scalar checks, self/static replacement)
//! - [`super::shapes`]: Array shape and object shape parsing
//!
//! All public and crate-visible items are re-exported here so that existing
//! `use crate::docblock::types::*` and `use super::types::*` call sites
//! continue to work without modification.

// ─── Re-exports: type_strings ───────────────────────────────────────────────

pub(crate) use super::type_strings::PHPDOC_TYPE_KEYWORDS;
pub use super::type_strings::clean_type;
pub(crate) use super::type_strings::{split_generic_args, split_type_token};

// ─── Re-exports: shapes ─────────────────────────────────────────────────────

pub use super::shapes::{
    extract_array_shape_value_type, extract_object_shape_property_type, is_object_shape,
    parse_array_shape, parse_object_shape,
};

// ─── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
#[path = "types_tests.rs"]
mod tests;
