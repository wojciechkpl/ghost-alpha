//! Plugin system for GhostAlpha — allows users to bring their own agents.
//!
//! The registry holds trait objects for each pluggable component. Users can
//! register custom implementations of `DataProvider`, `Screener`,
//! `SentimentScorer`, and `AlertChannel` before running the pipeline.
//!
//! # Example
//!
//! ```rust,ignore
//! use ghost_alpha::plugins::Registry;
//! use ghost_alpha::data::fetcher::FinnhubClient;
//! use ghost_alpha::alerts::telegram::TelegramChannel;
//!
//! let mut registry = Registry::new();
//! registry.register_provider("finnhub", FinnhubClient::new("key".into()));
//! registry.register_channel("telegram", TelegramChannel::new("token", "chat"));
//! ```

pub mod registry;

pub use registry::Registry;
