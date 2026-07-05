//! WeChat iLink bot.
mod bot;
mod types;

pub use bot::spawn_wechat_bot;
pub use types::{load_credentials, qr_login};
