//! Pure-Rust core of Velo-de's "Spaces" layout model.
//!
//! This crate has no GUI/compositor dependencies: it's the grid of
//! [`model::Space`]s, the scrollable [`model::Strip`] of [`model::Column`]s
//! within each one, the [`anim::Animated`] spring-physics engine that drives
//! every transition, and the [`layout`] math that turns all of that into
//! window rectangles. `velo-de` (the compositor crate) wraps a single
//! [`model::Grid`] per output and drives it from input/keybindings.

pub mod anim;
pub mod geometry;
pub mod layout;
pub mod model;

pub use anim::{Animatable, Animated, SpringParams};
pub use geometry::{Rect, Size, Vec2};
pub use layout::{place_window, WindowLayout};
pub use model::{Column, ColumnLayout, Command, Direction, Event, Grid, IdGen, NotNan, Overview, Space, SpaceFrame, Strip, WindowId};
