//! Everything that touches Telegram: proving who a request came from, and talking back.
//!
//! Kept apart from the rest of the system so that the reservation rules never depend on Telegram,
//! and so the one piece of security-critical arithmetic in this codebase — the `initData` HMAC —
//! sits in a module small enough to read in full.

pub mod bot;
pub mod init_data;
pub mod messages;
pub mod updates;

pub use bot::{Bot, CallbackButton, SendError};
pub use updates::Update;
pub use init_data::{BotToken, InitData, TelegramUser, VerifyError, verify};
