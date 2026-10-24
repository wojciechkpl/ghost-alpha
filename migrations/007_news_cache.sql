CREATE TABLE IF NOT EXISTS news_cache (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    ticker      TEXT NOT NULL,
    headline    TEXT NOT NULL,
    summary     TEXT NOT NULL,
    published_at TEXT NOT NULL,
    source      TEXT NOT NULL,
    cached_at   TEXT NOT NULL DEFAULT (datetime('now')),
    UNIQUE(ticker, headline)
);
CREATE INDEX IF NOT EXISTS idx_news_cache_ticker_pub
    ON news_cache(ticker, published_at DESC);
