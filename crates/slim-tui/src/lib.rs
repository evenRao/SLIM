//! SLIM Terminal User Interface (TUI) crate.

pub mod app;
pub mod catalog;
pub mod event;
pub mod item_actions;
pub mod management;
pub mod people;
pub mod terminal;
pub mod ui;

pub use app::App;
pub use terminal::run;
pub use terminal::run_with_login;
