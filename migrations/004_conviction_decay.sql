-- Migration V4: conviction decay tracking for flagged stocks
ALTER TABLE flagged_stocks ADD COLUMN latest_score REAL;
ALTER TABLE flagged_stocks ADD COLUMN last_rescored_at TEXT;
