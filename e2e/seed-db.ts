/**
 * Seed an e2e test database with sample data.
 * Runs the ghost-alpha migrate command, then inserts test rows via SQLite.
 *
 * Usage: npx tsx e2e/seed-db.ts
 */
import { execSync } from "child_process";
import { existsSync, unlinkSync } from "fs";

const DB_PATH = "e2e_test.db";

// Remove stale DB
if (existsSync(DB_PATH)) unlinkSync(DB_PATH);

// Run migrations
execSync(
  `GHOST_ALPHA_FINNHUB_API_KEY=test GHOST_ALPHA_DB_PATH=${DB_PATH} ./target/release/ghost-alpha migrate`,
  { stdio: "inherit" }
);

// Seed data via sqlite3
const today = new Date().toISOString().slice(0, 10);

// Generate past dates for multi-day data
function daysAgo(n: number): string {
  const d = new Date();
  d.setDate(d.getDate() - n);
  return d.toISOString().slice(0, 10);
}

const day1 = daysAgo(2);
const day2 = daysAgo(1);
const day3 = today;

const sql = `
-- Screen results for 3 days (provides heatmap + grade distribution data)
INSERT INTO screen_results (run_date, ticker, minervini_pass, conditions_met, adx, rs_percentile, volume_zscore, technical_score, volume_score, sentiment_score, composite_score, grade, close_price, pct_from_52w_high, weighted_rs)
VALUES
  ('${day1}', 'NVDA', 1, 8, 34.0, 95.0, 2.5, 46.0, 24.0, 9.0, 91.0, 'A+', 870.00, -2.5, 0.35),
  ('${day1}', 'AAPL', 1, 7, 20.0, 83.0, 1.3, 38.0, 19.0, 7.0, 79.0, 'A',  190.00, -6.0, 0.22),
  ('${day1}', 'MSFT', 1, 7, 18.0, 80.0, 1.0, 36.0, 17.0, 6.0, 75.0, 'B',  412.00, -5.0, 0.20),
  ('${day1}', 'META', 0, 5, 14.0, 58.0, 0.3, 23.0, 11.0, 4.0, 62.0, 'C',  500.00, -13.0, 0.10),
  ('${day1}', 'TSLA', 0, 4, 11.0, 42.0, 0.1, 16.0, 9.0, 2.0, 48.0, 'D',  175.00, -26.0, 0.05),

  ('${day2}', 'NVDA', 1, 8, 35.0, 96.0, 2.6, 47.0, 24.5, 9.5, 92.5, 'A+', 872.00, -2.3, 0.36),
  ('${day2}', 'AAPL', 1, 7, 21.0, 84.0, 1.4, 39.0, 19.5, 7.5, 80.5, 'A',  191.00, -5.7, 0.23),
  ('${day2}', 'MSFT', 1, 7, 19.0, 81.0, 1.1, 37.0, 17.5, 6.5, 76.5, 'A',  413.50, -4.5, 0.21),
  ('${day2}', 'META', 0, 5, 15.0, 59.0, 0.4, 24.0, 11.5, 4.5, 63.5, 'B',  502.00, -12.5, 0.11),
  ('${day2}', 'TSLA', 0, 4, 12.0, 44.0, 0.15, 17.0, 9.5, 2.5, 50.0, 'C',  177.00, -25.5, 0.06),

  ('${day3}', 'NVDA', 1, 8, 35.0, 97.0, 2.8, 48.0, 25.0, 10.0, 94.2, 'A+', 875.50, -2.1, 0.38),
  ('${day3}', 'AAPL', 1, 7, 21.0, 85.0, 1.5, 40.0, 20.0,  8.0, 82.1, 'A',  192.30, -5.3, 0.24),
  ('${day3}', 'MSFT', 1, 7, 19.0, 82.0, 1.2, 38.0, 18.0,  7.0, 78.5, 'A',  415.60, -4.1, 0.22),
  ('${day3}', 'META', 0, 5, 15.0, 60.0, 0.5, 25.0, 12.0,  5.0, 65.0, 'B',  505.20, -12.0, 0.12),
  ('${day3}', 'TSLA', 0, 4, 12.0, 45.0, 0.2, 18.0, 10.0,  3.0, 52.3, 'C',  178.90, -25.0, 0.07);

-- Alerts across multiple days
INSERT INTO alert_log (ticker, alert_type, severity, message, sent_at)
VALUES
  ('NVDA', 'NewSignal', 'CRITICAL', 'Strong Minervini setup with volume breakout', '${day3} 09:30:00'),
  ('AAPL', 'ScoreChange', 'WARNING', 'Score increased from 70 to 82', '${day3} 09:35:00'),
  ('META', 'RSDrop', 'INFO', 'RS percentile dropped below 60', '${day3} 10:00:00'),
  ('MSFT', 'NewSignal', 'WARNING', 'New Minervini qualification', '${day2} 09:30:00'),
  ('NVDA', 'VolumeSpike', 'CRITICAL', 'Volume Z-score exceeded 2.5', '${day1} 09:45:00');

-- Market regime history (3 days)
INSERT INTO market_regime (date, spy_close, spy_sma200, breadth_pct, regime)
VALUES
  ('${day1}', 518.20, 484.50, 66.0, 'Bull'),
  ('${day2}', 519.30, 484.80, 67.2, 'Bull'),
  ('${day3}', 520.40, 485.20, 68.5, 'Bull');

-- Flagged stocks
INSERT INTO flagged_stocks (ticker, flagged_date, entry_score, entry_price, conditions_at_entry, status)
VALUES
  ('NVDA', '${today}', 94.2, 875.50, 8, 'ACTIVE'),
  ('PLTR', '2026-02-15', 88.0, 72.30, 7, 'ACTIVE');

-- Watchlist
INSERT INTO watchlist (ticker, added_at, notes) VALUES ('AMD', '${today}', 'Watching for breakout');
INSERT INTO watchlist (ticker, added_at, notes) VALUES ('GOOGL', '${today}', NULL);
`;

execSync(`sqlite3 ${DB_PATH} "${sql.replace(/"/g, '\\"')}"`, {
  stdio: "inherit",
});

console.log(`Seeded ${DB_PATH} with test data for ${today}`);
