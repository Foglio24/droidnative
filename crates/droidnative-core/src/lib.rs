//! DroidNative core: host analysis, GPU backends, Android runtime management.

pub mod apk;
pub mod exec;
pub mod extras;
pub mod fs;
pub mod i18n;
pub mod gpu;
pub mod host;
pub mod ini;
pub mod nvidia_stack;
pub mod privileged;
pub mod report;
pub mod runtime;
pub mod setup;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
