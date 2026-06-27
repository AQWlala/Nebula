//! Multi-channel messaging bridge — v1.2
//!
//! Bridges nine-snake with JiuwenSwarm's multi-channel delivery fabric,
//! enabling the desktop agent to send/receive messages through WeChat,
//! Feishu, Telegram, Web, and other channels without baking channel-specific
//! SDKs into the Tauri binary.
//!
//! ## Architecture
//!
//! ```text
//!  WeChat / Feishu / Telegram / Web
//!            │
//!     JiuwenSwarm Delivery
//!            │
//!    ┌───────▼────────┐
//!    │  MessageBridge │  ← this module
//!    │  (HTTP bridge) │
//!    └───────┬────────┘
//!            │
//!     nine-snake core
//!```
//!
//! ## Key invariants
//! * All channel I/O goes through JiuwenSwarm — this module never talks
//!   to WeChat/Feishu SDKs directly.
//! * The bridge is **opt-in**: when no JiuwenSwarm endpoint is configured
//!   (`NINE_SNAKE_BRIDGE_URL` is unset), the bridge is a no-op.
//! * Incoming messages are polled or received via a local HTTP callback
//!   endpoint; outgoing messages are POSTed to JiuwenSwarm's agent API.
//! * Message format follows the JiuwenSwarm agent turn protocol for
//!   maximum compatibility.

pub mod bridge;
pub mod discord;
pub mod router;
pub mod telegram;
pub mod types;
pub mod webchat;

pub use bridge::MessageBridge;
pub use discord::DiscordBotAdapter;
pub use router::{ChannelRouter, DiscordAdapter, TelegramAdapter, WebChatAdapter};
pub use telegram::TelegramBotAdapter;
pub use types::*;
pub use webchat::WebChatService;
