use rust_decimal::Decimal;
use sui_sentinel::{
    analysis::stress::{Accumulator, Basis, Policy, Status},
    decimal::DecimalString,
};

fn d(value: &str) -> Decimal {
    Decimal::from_str_exact(value).unwrap()
}
fn asset(symbol: &str, amount: &str) -> Vec<(String, Decimal)> {
    vec![(format!("{}::test::{symbol}", "a".repeat(64)), d(amount))]
}
fn policy() -> Policy {
    Policy::parse(
        r#"schema_version = 1
loss_tolerance = "0.05"
[[scenarios]]
name = "discount"
default_price_factor = "1"
recovery_discount = "0.10"
"#,
    )
    .unwrap()
}

#[test]
fn computes_per_position_shortfalls_and_does_not_net_unrelated_surplus() {
    let mut model = Accumulator::new(&policy()).unwrap();
    model.add(&asset("A", "110"), &asset("B", "100")).unwrap();
    let result = model.finish(true, Basis::Verified).unwrap();
    assert_eq!(result.scenarios[0].shortfall_usd, d("1"));
    assert_eq!(result.score, Some(d("20")));
    let mut model = Accumulator::new(&policy()).unwrap();
    model.add(&asset("A", "0"), &asset("B", "100")).unwrap();
    model.add(&asset("A", "1000"), &asset("B", "100")).unwrap();
    let result = model.finish(true, Basis::Verified).unwrap();
    assert_eq!(result.scenarios[0].shortfall_usd, d("100"));
    assert_eq!(result.scenarios[0].shortfall_positions, 1);
    assert_eq!(result.score, Some(d("100")));
}

#[test]
fn reports_matched_and_unmatched_price_override_assets() {
    let mut policy = policy();
    let matched_asset = asset("A", "1")[0].0.clone();
    let unmatched_asset = asset("Z", "1")[0].0.clone();
    policy.scenarios[0]
        .price_factors
        .insert(matched_asset.clone(), DecimalString(d("0.5")));
    policy.scenarios[0]
        .price_factors
        .insert(unmatched_asset.clone(), DecimalString(d("0.5")));
    let mut model = Accumulator::new(&policy).unwrap();
    // Only "A" ever appears in an evaluated position; "Z" is configured but never
    // observed in this capture's exposures - a likely typo or an asset absent
    // from the current book.
    model.add(&asset("A", "100"), &asset("B", "50")).unwrap();
    let result = model.finish(true, Basis::Verified).unwrap();
    assert!(
        result.scenarios[0]
            .matched_price_overrides
            .contains(&matched_asset)
    );
    assert!(
        !result.scenarios[0]
            .matched_price_overrides
            .contains(&unmatched_asset)
    );
    assert!(
        result.scenarios[0]
            .unmatched_price_overrides
            .contains(&unmatched_asset)
    );
    assert!(
        !result.scenarios[0]
            .unmatched_price_overrides
            .contains(&matched_asset)
    );
}

#[test]
fn asset_price_shocks_apply_identically_to_both_sides() {
    let mut policy = policy();
    policy.scenarios[0]
        .price_factors
        .insert(asset("A", "1")[0].0.clone(), DecimalString(d("0.5")));
    let mut model = Accumulator::new(&policy).unwrap();
    model.add(&asset("A", "110"), &asset("A", "100")).unwrap();
    let result = model.finish(true, Basis::Verified).unwrap();
    assert_eq!(result.scenarios[0].debt_usd, d("50"));
    assert_eq!(result.scenarios[0].shortfall_usd, d("0.5"));
    assert_eq!(result.score, Some(d("20")));
}

#[test]
fn selects_worst_scenario_rate_not_largest_dollar_loss() {
    use sui_sentinel::analysis::stress::{Outcome, score};
    let rows = [("large", "1000", "10"), ("small", "100", "2")].map(|(name, debt, loss)| Outcome {
        name: name.into(),
        debt_usd: d(debt),
        shortfall_usd: d(loss),
        shortfall_positions: 1,
        matched_price_overrides: Default::default(),
        unmatched_price_overrides: Default::default(),
    });
    assert_eq!(score(&rows, d("0.05")).unwrap(), Some(d("40")));
}

#[test]
fn does_not_publish_scores_for_cached_incomplete_excluded_or_debt_free_inputs() {
    for (complete, cached, exclude, expected) in [
        (true, true, false, Status::UnverifiedCachedInputs),
        (false, false, false, Status::IncompleteCoverage),
        (true, false, true, Status::IncompleteCoverage),
    ] {
        let mut model = Accumulator::new(&policy()).unwrap();
        model.add(&asset("A", "110"), &asset("B", "100")).unwrap();
        if exclude {
            model.exclude().unwrap();
        }
        let result = model
            .finish(
                complete,
                if cached {
                    Basis::Cached
                } else {
                    Basis::Verified
                },
            )
            .unwrap();
        assert_eq!(result.status, expected);
        assert!(result.score.is_none());
    }
    let result = Accumulator::new(&policy())
        .unwrap()
        .finish(true, Basis::Verified)
        .unwrap();
    assert_eq!(result.status, Status::NoDebt);
    assert!(result.score.is_none());
}

#[test]
fn rejects_bad_policies_negative_inputs_and_overflow_without_partial_mutation() {
    for modify in [
        (|p: &mut Policy| p.loss_tolerance = Decimal::ZERO) as fn(&mut Policy),
        |p| p.scenarios[0].recovery_discount = d("1.01"),
        |p| p.scenarios[0].default_price_factor = d("-1"),
        |p| p.scenarios.push(p.scenarios[0].clone()),
        |p| p.schema_version = 2,
    ] {
        let mut policy = policy();
        modify(&mut policy);
        assert!(Accumulator::new(&policy).is_err());
    }
    assert!(Policy::parse("unknown = true").is_err());
    let mut model = Accumulator::new(&policy()).unwrap();
    assert!(model.add(&asset("A", "-1"), &[]).is_err());
    assert!(
        model
            .add(
                &[],
                &[("A".into(), Decimal::MAX), ("B".into(), Decimal::MAX)]
            )
            .is_err()
    );
    let result = model.finish(true, Basis::Verified).unwrap();
    assert_eq!(result.evaluated_positions, 0);
    assert_eq!(result.scenarios[0].debt_usd, Decimal::ZERO);
}

#[test]
fn failed_later_scenario_cannot_commit_earlier_results() {
    let mut policy = policy();
    let mut second = policy.scenarios[0].clone();
    second.name = "second".into();
    second.default_price_factor = d("2");
    policy.scenarios.push(second);
    let mut model = Accumulator::new(&policy).unwrap();
    assert!(model.add(&[], &[("A".into(), Decimal::MAX)]).is_err());
    model.add(&[], &asset("A", "1")).unwrap();
    let result = model.finish(true, Basis::Verified).unwrap();
    assert_eq!(result.evaluated_positions, 1);
    assert_eq!(result.scenarios[0].debt_usd, d("1"));
    assert_eq!(result.scenarios[1].debt_usd, d("2"));
}

#[test]
fn failed_position_does_not_commit_override_coverage_in_any_scenario() {
    let mut policy = policy();
    let key = asset("A", "1")[0].0.clone();
    policy.scenarios[0]
        .price_factors
        .insert(key.clone(), DecimalString(d("1")));
    let mut second = policy.scenarios[0].clone();
    second.name = "overflow".into();
    second
        .price_factors
        .insert(key.clone(), DecimalString(d("2")));
    policy.scenarios.push(second);
    let mut model = Accumulator::new(&policy).unwrap();
    assert!(model.add(&[], &[(key.clone(), Decimal::MAX)]).is_err());
    model.exclude().unwrap();
    model.add(&[], &asset("B", "1")).unwrap();
    let result = model.finish(true, Basis::Cached).unwrap();
    assert_eq!(result.evaluated_positions, 1);
    for scenario in result.scenarios {
        assert!(scenario.matched_price_overrides.is_empty());
        assert!(scenario.unmatched_price_overrides.contains(&key));
        assert_eq!(scenario.debt_usd, d("1"));
    }
}
