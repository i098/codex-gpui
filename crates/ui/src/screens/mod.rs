//! One module per native screen, with live data and recorded parity fixtures.

pub mod automations;
pub mod cloud_task;
pub mod composer;
pub mod computer_use_native;
pub use settings::computer_use;
pub mod diff;
pub mod home;
pub mod mobile;
pub mod pull_requests;
pub mod settings;
pub mod sidebar;
pub mod signin;
pub mod terminal;
pub mod thread;
