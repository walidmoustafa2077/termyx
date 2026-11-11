//! Window chrome: the tab strip and its menu.
//!
//! Everything here is coordinates and state, never pixels, so the layout can
//! be tested without a window.

pub mod context_menu;
pub mod find_bar;
pub mod menu;
pub mod rename;
pub mod spellcheck_menu;
pub mod tab_bar;
pub mod tab_menu;
pub mod tooltip;
