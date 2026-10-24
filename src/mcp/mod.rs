//! MCP (Model Context Protocol) server for GhostAlpha.
//!
//! Exposes the screening engine, market regime detection, and portfolio analysis
//! as MCP tools that any AI agent (Claude, GPT, etc.) can call.
//!
//! # Architecture
//!
//! ```text
//! AI Agent (Claude Desktop, VS Code, etc.)
//!    │  stdin/stdout (JSON-RPC)
//!    ▼
//! ghost-alpha-mcp (this binary)
//!    │
//!    ├─ screen_stock     → MinerviniScreener + Scorer
//!    ├─ get_regime       → RegimeState from DB
//!    ├─ list_providers   → Plugin Registry introspection
//!    └─ get_config       → Masked config display
//! ```

pub mod server;
pub mod tools;
