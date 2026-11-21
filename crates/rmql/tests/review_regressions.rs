//! Regression tests for the boundary cases found in the 2026-11-21 code review.

use rmql::market_risk::{compute_traffic_light, TrafficLight};
use rmql::math::interpolation::{Linear, MonotonicCubicSpline};
use rmql::math::solver1d::{solve_brent_with_step, solve_newton_safe};
use rmql::math::sobol::SobolSequence;

#[test]
fn nan_input_does_not_panic_in_interpolation() {
    let linear = Linear::new(vec![0.0, 1.0, 2.0], vec![1.0, 2.0, 3.0]).unwrap();
    assert!(linear.value(f64::NAN).is_nan());
    let cubic = MonotonicCubicSpline::new(vec![0.0, 1.0, 2.0], vec![1.0, 2.0, 4.0]).unwrap();
    assert!(cubic.value(f64::NAN).is_nan());
    assert!(Linear::new(vec![0.0, f64::NAN, 2.0], vec![1.0, 2.0, 3.0]).is_err());
    assert!(MonotonicCubicSpline::new(vec![0.0, f64::NAN, 2.0], vec![1.0, 2.0, 3.0]).is_err());
}

#[test]
fn sobol_limit_is_one_less_than_two_to_the_32() {
    assert_eq!(SobolSequence::MAX_POINTS, (1u64 << 32) - 1);
    // the last allowed counter is u32::MAX - 1, which needs the 32nd direction number only
    let mut sobol = SobolSequence::new(2).unwrap();
    sobol.next_integers();
    assert_eq!(sobol.next_integers().len(), 2);
}

#[test]
fn traffic_light_does_not_underflow_with_many_observations() {
    // (0.95)^20000 is about 1e-446, below the smallest f64
    let (zone, p) = compute_traffic_light(20_000, 20_000, 0.95).unwrap();
    assert_eq!(zone, TrafficLight::Red);
    assert!((p - 1.0).abs() < 1e-12);
    let (zone, p) = compute_traffic_light(10, 20_000, 0.95).unwrap();
    assert_eq!(zone, TrafficLight::Green);
    assert!(p < 1e-100 || p == 0.0);
    // a small case is unchanged: P(X <= 4) for Binomial(250, 0.01) is 0.8922...
    let (zone, p) = compute_traffic_light(4, 250, 0.99).unwrap();
    assert_eq!(zone, TrafficLight::Green);
    assert!((p - 0.8922).abs() < 1e-3);
}

#[test]
fn solvers_reject_a_zero_step() {
    assert!(solve_brent_with_step(|x| Ok(x - 1.0), 1e-12, 0.0, 0.0, 100).is_err());
    assert!(solve_newton_safe(|x| Ok(x - 1.0), |_| Ok(1.0), 1e-12, 0.0, 0.0, 100).is_err());
    assert!(solve_newton_safe(|x| Ok(x - 1.0), |_| Ok(1.0), 1e-12, 0.0, f64::NAN, 100).is_err());
}

// ---- cash flows, rates, OIS, portfolio, LMM, PRDC ----

use rmql::autodiff::Recorder;
use rmql::cashflow_analysis::compute_flows_npv_bps;
use rmql::curves::YieldCurve;
use rmql::interest_rate::{Compounding, InterestRate};
use rmql::ois_swap::{compute_ois_swap_daily_rates, Fixings, OvernightCoupon};
use rmql::time::{Actual365Kind, Date, DayCount};
use rmql::yen_bond::{compute_price_from_compound_yield, CompoundFrequencies};

fn date(y: i32, m: u32, d: u32) -> Date {
    Date::from_ymd(y, m, d).unwrap()
}

fn flat_curve(on: Date, rate: f64) -> YieldCurve {
    let dc = DayCount::Actual365Fixed(Actual365Kind::Standard);
    YieldCurve::flat(on, InterestRate::new(rate, dc, Compounding::Continuous, 0).unwrap())
}

#[test]
fn an_implied_rate_at_a_compound_of_one_keeps_its_derivative() {
    let recorder = Recorder::new();
    let compound = recorder.input(1.0);
    let dc = DayCount::Actual365Fixed(Actual365Kind::Standard);
    let rate = InterestRate::implied_rate(compound, dc.clone(), Compounding::Continuous, 0, 2.0).unwrap();
    // r = ln(compound) / t, so dr/dcompound = 1 / t = 0.5 at compound = 1
    let gradient = recorder.gradient(rate.rate, &[compound]).unwrap();
    assert!((gradient[0].mean() - 0.5).abs() < 1e-12);
    assert!(InterestRate::implied_rate(1.0_f64, dc.clone(), Compounding::Continuous, 0, 0.0).unwrap().rate == 0.0);
    assert!(InterestRate::implied_rate(1.0_f64, dc.clone(), Compounding::Continuous, 0, -1.0).is_err());
}

#[test]
fn a_compound_yield_near_zero_gives_a_finite_price() {
    let f = CompoundFrequencies { coupon: 2, compound: 1 };
    for y in [0.0, 1e-14, -1e-14, 1e-13] {
        let p = compute_price_from_compound_yield(5.0, 1.0, y, 100.0, f, None).unwrap();
        assert!(p.is_finite() && (p - 105.0).abs() < 1e-6, "{y}: {p}");
    }
}

#[test]
fn flows_check_their_lengths_and_skip_the_curve_for_occurred_flows() {
    let today = date(2026, 11, 2);
    let curve = flat_curve(today, 0.02);
    let dates = [date(2026, 1, 1), date(2027, 11, 2)];
    assert!(compute_flows_npv_bps(&dates, &[1.0], &[1.0, 1.0], &[1.0, 1.0], &curve, today, false, today).is_err());
    // the first flow is before the curve's date: no error, a 0 npv
    let v = compute_flows_npv_bps(&dates, &[1.0, 100.0], &[1.0, 1.0], &[1.0, 1.0], &curve, today, false, today).unwrap();
    assert_eq!(v.npvs[0], 0.0);
    assert!(v.discounts[0].is_nan());
    assert!(v.npv > 90.0 && v.npv < 100.0);
}

#[test]
fn daily_rates_of_a_coupon_that_started_before_the_curve_use_the_fixings() {
    let today = date(2026, 11, 2);
    let forecast = flat_curve(today, 0.02);
    let (d0, d1, d2) = (date(2026, 10, 30), date(2026, 11, 2), date(2026, 11, 3));
    let coupon = OvernightCoupon {
        payment_date: d2,
        accrual_start: d0,
        accrual_end: d2,
        accrual: 4.0 / 360.0,
        value_dates: vec![d0, d1, d2],
        day_fractions: vec![3.0 / 360.0, 1.0 / 360.0],
    };
    let mut fixings = Fixings::new();
    fixings.insert(d0, 0.01);
    let days = compute_ois_swap_daily_rates(&coupon, &forecast, today, &fixings).unwrap();
    assert!(days[0].forward_rate.is_nan());
    assert_eq!(days[0].applied_rate, 0.01);
    assert!(days[1].forward_rate.is_finite());
    // no fixing for the past day: still an error
    assert!(compute_ois_swap_daily_rates(&coupon, &forecast, today, &Fixings::new()).is_err());
}

use rmql::lmm_market::{compute_lmm_caplet_value, Discretization, LmmPaths, LmmSpec, Measure};
use rmql::options::ForwardModel;
use rmql::portfolio::{compute_optimal_portfolio, PortfolioConstraints, PortfolioMethod, ReturnMatrix};
use rmql::prdc::{generate_prdc_three_factor_paths, HullWhiteFactor, PrdcThreeFactor};

#[test]
fn target_volatility_works_with_long_only_bounds_and_infinite_uppers() {
    // 3 assets, 8 periods: asset 3 has the highest mean and the highest volatility
    let values = [
        0.01, 0.02, 0.06, 0.00, 0.01, -0.04, 0.02, 0.03, 0.09, 0.01, 0.02, 0.00, //
        0.00, 0.01, 0.03, 0.02, 0.02, 0.07, 0.01, 0.00, -0.05, 0.02, 0.03, 0.08,
    ];
    let returns = ReturnMatrix { values: &values, assets: 3 };
    let long_only = PortfolioConstraints { weight_sum: 1.0, lower: vec![0.0; 3], upper: vec![f64::INFINITY; 3] };
    let low = compute_optimal_portfolio(&returns, &long_only, &PortfolioMethod::MinimumVariance).unwrap();
    let top = compute_optimal_portfolio(&returns, &long_only, &PortfolioMethod::MaximumReturn).unwrap();
    let target = 0.5 * (low.volatility + top.volatility);
    let fit = compute_optimal_portfolio(&returns, &long_only, &PortfolioMethod::TargetVolatility { target }).unwrap();
    assert!(fit.volatility <= target * (1.0 + 1e-6));
    assert!(fit.expected_return >= low.expected_return - 1e-12);
    assert!((fit.weights.iter().sum::<f64>() - 1.0).abs() < 1e-9);
    // short sales allowed with finite upper bounds only: the maximum return is the lowest mean asset cut back
    let capped = PortfolioConstraints { weight_sum: 1.0, lower: vec![f64::NEG_INFINITY; 3], upper: vec![0.8; 3] };
    let best = compute_optimal_portfolio(&returns, &capped, &PortfolioMethod::MaximumReturn).unwrap();
    assert!((best.weights.iter().sum::<f64>() - 1.0).abs() < 1e-9);
    assert!(best.weights.iter().all(|&w| w <= 0.8 + 1e-12));
    // neither all lower nor all upper bounds finite: the return is unbounded
    let open = PortfolioConstraints { weight_sum: 1.0, lower: vec![0.0, f64::NEG_INFINITY, 0.0], upper: vec![f64::INFINITY, 1.0, 1.0] };
    assert!(compute_optimal_portfolio(&returns, &open, &PortfolioMethod::MaximumReturn).is_err());
}

fn lmm_spec() -> LmmSpec {
    LmmSpec {
        times: vec![0.5, 1.0, 1.5],
        forwards: vec![0.02, 0.02],
        volatilities: vec![0.2, 0.2],
        correlation: vec![1.0, 0.5, 0.5, 1.0],
        discount_start: 0.99,
        backward_looking: false,
        model: ForwardModel::Black,
        volatility_grid: Some(vec![0.2, 0.21, 0.22, 0.30, 0.31, 0.32]),
        factors: None,
        measure: Measure::Spot,
        discretization: Discretization::Euler,
    }
}

#[test]
fn lmm_volatility_after_the_last_date_uses_the_last_column() {
    let s = lmm_spec();
    assert_eq!(s.volatility_at(1, 10.0), 0.32);
    assert_eq!(s.volatility_at(0, 10.0), 0.22);
    assert_eq!(s.volatility_at(1, 0.1), 0.30);
}

#[test]
fn lmm_values_reject_paths_of_the_wrong_shape() {
    let s = lmm_spec();
    let empty = LmmPaths { state: vec![], numeraire: vec![], observation: None, observed: vec![] };
    assert!(compute_lmm_caplet_value(&s, &empty, 0, 0.02).is_err());
    let short = LmmPaths { state: vec![vec![vec![0.02, 0.02]]], numeraire: vec![vec![1.0]], observation: None, observed: vec![] };
    assert!(compute_lmm_caplet_value(&s, &short, 0, 0.02).is_err());
}

#[test]
fn a_zero_rate_volatility_keeps_the_random_numbers_of_the_other_factors() {
    let on = date(2026, 11, 2);
    let (dom, fgn) = (flat_curve(on, 0.01), flat_curve(on, 0.03));
    let model = |sigma_d: f64| PrdcThreeFactor {
        spot: 150.0,
        fx_volatility: 0.12,
        fx_volatility_term: None,
        domestic: &dom,
        foreign: &fgn,
        domestic_rate: HullWhiteFactor { mean_reversion: 0.02, volatility: sigma_d },
        foreign_rate: HullWhiteFactor { mean_reversion: 0.02, volatility: 0.01 },
        correlation: [0.0, 0.0, 0.0],
    };
    let times = [1.0, 2.0, 3.0];
    let zero = generate_prdc_three_factor_paths(&times, &model(0.0), 50, 4, 7).unwrap();
    let tiny = generate_prdc_three_factor_paths(&times, &model(1e-5), 50, 4, 7).unwrap();
    for (a, b) in zero.fx.iter().zip(&tiny.fx) {
        for (x, y) in a.iter().zip(b) {
            assert!((x / y - 1.0).abs() < 1e-3, "{x} {y}");
        }
    }
    assert!(generate_prdc_three_factor_paths(&[1.0, f64::NAN], &model(0.0), 50, 4, 7).is_err());
}
