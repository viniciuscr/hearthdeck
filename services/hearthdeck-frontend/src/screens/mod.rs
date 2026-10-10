//! One module per screen.
//!
//! Each screen owns its state, its `Message`, its `update` and its `view`;
//! [`crate::app`] routes to it and stays thin. See `docs/frontend-rewrite.md`.

pub mod library;
