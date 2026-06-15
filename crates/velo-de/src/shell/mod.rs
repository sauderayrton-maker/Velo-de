//! Shell protocol glue beyond `xdg_shell` (which lives in [`crate::state`]).
//! Currently just `wlr_layer_shell` arrangement math ([`layer`]); the
//! [`smithay::wayland::shell::wlr_layer`] handler impls live on
//! [`crate::state::State`] itself since they need direct access to its
//! fields.

pub mod layer;
