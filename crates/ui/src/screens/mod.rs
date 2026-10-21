//! One module per parity screen; each `build` draws the screen in its refs/<screen> fixture state.

pub mod automations;
pub mod composer;
pub mod computer_use_native;
pub use settings::computer_use;
pub mod diff;
pub mod home;
pub mod mobile;
pub mod settings;
pub mod sidebar;
pub mod signin;
pub mod terminal;
pub mod thread;
