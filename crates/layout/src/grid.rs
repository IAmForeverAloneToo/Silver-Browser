//! Grid layout (M18): a grid container's items placed in its rows and
//! columns by css-grid-2. [`placement`] makes the explicit grid and
//! places the items in it (§7, §8); the tracks are sized and the items
//! laid out in their areas by §12 (docs/design/grid.md).

// Until the grid is laid out (M18 step 3), only the tests place items.
#[allow(dead_code)]
pub(crate) mod placement;
