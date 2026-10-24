-- Migration V5: extended screen_results with Phase 2.5 signal quality fields
ALTER TABLE screen_results ADD COLUMN atr REAL DEFAULT 0.0;
ALTER TABLE screen_results ADD COLUMN risk_reward REAL DEFAULT 0.0;
ALTER TABLE screen_results ADD COLUMN liquidity TEXT DEFAULT 'Illiquid';
ALTER TABLE screen_results ADD COLUMN weighted_rs REAL DEFAULT 0.0;
ALTER TABLE screen_results ADD COLUMN regime TEXT DEFAULT 'Sideways';
