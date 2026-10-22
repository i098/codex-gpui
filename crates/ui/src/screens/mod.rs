//! One module per parity screen; each `build` draws the screen in its refs/<screen> fixture state.

pub mod automations;
pub mod cloud;
pub mod cloud_task;
pub mod composer;
pub mod computer_use_native;
pub use settings::computer_use;
pub mod dictation;
pub mod diff;
pub mod home;
pub mod hotkey;
pub mod mini;
pub mod mobile;
pub mod settings;
pub mod sidebar;
pub mod terminal;
pub mod thread;
