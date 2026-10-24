//! Integration test suite entry point.
//!
//! Run with: `cargo test --test integration`

#[path = "common/mod.rs"]
mod common;

#[path = "integration/db_test.rs"]
mod db_test;

#[path = "integration/screener_test.rs"]
mod screener_test;

#[path = "integration/alert_test.rs"]
mod alert_test;

#[path = "integration/sentiment_test.rs"]
mod sentiment_test;

#[path = "integration/backtest_test.rs"]
mod backtest_test;
