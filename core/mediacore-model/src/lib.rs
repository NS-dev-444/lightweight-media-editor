//! mediacore-model — the platform-independent core of the editor.
//!
//! No FFmpeg, no platform APIs, no UI. That is deliberate: this crate
//! cross-compiles for Windows from any machine, which keeps RISK_REGISTER.md
//! R-21 (the deferred Windows port decaying into a rewrite) partly enforceable
//! without a Windows runner.

pub mod time;
pub mod asset;
pub mod timeline;
pub mod command;
pub mod ops;
pub mod editor;
pub mod relink;
pub mod cache;
pub mod captions;
pub mod render;
pub mod project;
