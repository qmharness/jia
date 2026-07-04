//! WeChat iLink bot.
mod types;
mod bot;

pub use types::{qr_login, load_credentials};
pub use bot::spawn_wechat_bot;
