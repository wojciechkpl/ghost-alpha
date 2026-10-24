use ghost_alpha::projection::monte_carlo::{CorrelationRules, McConfig, ReturnDistribution};

#[test]
fn mc_config_default_fields() {
    let cfg = McConfig::default();
    assert_eq!(cfg.n_paths, 5_000);
    assert_eq!(cfg.seed, 42);
    assert_eq!(cfg.horizon_years, 5);
}

#[test]
fn mc_config_default_distribution_is_lognormal() {
    let cfg = McConfig::default();
    assert!(
        matches!(cfg.return_distribution, ReturnDistribution::Lognormal),
        "expected Lognormal variant"
    );
}

#[test]
fn correlation_rules_default_canonical_values() {
    let r = CorrelationRules::default();
    assert_eq!(r.us_core_within, 0.92);
    assert_eq!(r.treasury_within, 0.85);
    assert_eq!(r.equity_to_treasury, -0.15);
    assert_eq!(r.cash_to_anything, 0.0);
}

#[test]
fn mc_config_round_trips_json() {
    let cfg = McConfig::default();
    let json = serde_json::to_string(&cfg).expect("serialize McConfig");
    let back: McConfig = serde_json::from_str(&json).expect("deserialize McConfig");
    assert_eq!(back.seed, cfg.seed);
}

#[test]
fn correlation_rules_round_trips_json() {
    let r = CorrelationRules::default();
    let json = serde_json::to_string(&r).expect("serialize CorrelationRules");
    let back: CorrelationRules = serde_json::from_str(&json).expect("deserialize CorrelationRules");
    assert_eq!(back.us_core_within, r.us_core_within);
}

#[test]
fn return_distribution_lognormal_round_trips_json() {
    let d = ReturnDistribution::Lognormal;
    let json = serde_json::to_string(&d).expect("serialize Lognormal");
    let back: ReturnDistribution = serde_json::from_str(&json).expect("deserialize Lognormal");
    assert!(matches!(back, ReturnDistribution::Lognormal));
}

#[test]
fn return_distribution_normal_clipped_round_trips_json() {
    let d = ReturnDistribution::NormalClipped { floor: -0.99 };
    let json = serde_json::to_string(&d).expect("serialize NormalClipped");
    let back: ReturnDistribution = serde_json::from_str(&json).expect("deserialize NormalClipped");
    assert!(
        matches!(back, ReturnDistribution::NormalClipped { floor } if (floor - (-0.99)).abs() < 1e-12)
    );
}
