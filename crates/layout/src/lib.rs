//! Layout: box-tree construction and sizing algorithms.
//!
//! [`box_tree`] turns a `style::StyledDom` into a tree of [`LayoutBox`]es;
//! [`block`] (M6) lays normal-flow block boxes out into a [`Fragment`]
//! tree — real width/margin resolution (CSS2.1 §10.3.3) and margin
//! collapsing (CSS2 §8.3.1); [`inline`] (M7) lays its inline-level
//! content out for real too — line boxes, greedy line breaking against
//! `text`-measured words, baseline alignment across mixed font sizes,
//! `text-align` — see that module's docs for its own v1 scope (no
//! `overflow-wrap`, no bidi, block-in-inline doesn't restructure the box
//! tree). `paint` consumes the fragment tree this crate produces.

pub mod block;
pub mod box_tree;
pub mod inline;

pub use block::{layout, Fragment, FragmentKind};
pub use box_tree::{build_box_tree, BoxKind, LayoutBox};
