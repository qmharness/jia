//! Channel adapters (Telegram / WeChat) for jia.
//!
//! Panic policy: the workspace is built with `panic = "abort"` in release
//! mode. Any task that panics will abort the whole process; there is no
//! in-process catch_unwind / restart. Process-level recovery is the
//! responsibility of the external supervisor (launchd, systemd, etc.).

pub mod telegram;
pub mod wechat;
