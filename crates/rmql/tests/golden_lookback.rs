//! Continuously monitored lookback options against the QuantLib R package Version 1.42
//! (tools/golden/p16_lookback.R). Inputs are synthetic. Rows have a drift r - q of 1e-5 or -1e-5 in place of 0,
//! where QuantLib returns NaN or an infinite value.

mod common;

use common::golden_rows;
use rmql::lookback::{
    compute_lookback_greeks, compute_lookback_value, LookbackInputs, LookbackStrike,
};
use rmql::options::OptionType;

fn number(s: &str) -> f64 {
    s.parse().unwrap()
}

#[test]
fn continuous_values_agree_with_quantlib() {
    let rows = golden_rows("option_lookback.csv");
    assert_eq!(rows.len(), 400);
    let mut worst: f64 = 0.0;
    for r in &rows {
        let strike = match r[0].as_str() {
            "floating" => LookbackStrike::Floating,
            "fixed" => LookbackStrike::Fixed,
            other => panic!("{other}"),
        };
        let kind = if r[1] == "call" {
            OptionType::Call
        } else {
            OptionType::Put
        };
        let inputs = LookbackInputs {
            spot: number(&r[2]),
            running_extremum: number(&r[3]),
            strike: number(&r[4]),
            rate: number(&r[5]),
            dividend_yield: number(&r[6]),
            volatility: number(&r[7]),
            time: number(&r[8]) / 365.0,
        };
        let expected = number(&r[9]);
        let got = compute_lookback_value(strike, kind, &inputs).unwrap();
        let relative = (got - expected).abs() / expected.abs().max(1e-300);
        worst = worst.max(relative);
        assert!(
            relative < 1e-11 || (got - expected).abs() < 1e-11,
            "{r:?}: {got} against {expected}"
        );
    }
    println!("largest relative difference: {worst:e}");
}

#[test]
fn greeks_by_automatic_differentiation_agree_with_differences() {
    let rows = golden_rows("option_lookback.csv");
    for r in rows.iter().take(60) {
        let strike = if r[0] == "floating" {
            LookbackStrike::Floating
        } else {
            LookbackStrike::Fixed
        };
        let kind = if r[1] == "call" {
            OptionType::Call
        } else {
            OptionType::Put
        };
        let i = LookbackInputs {
            spot: number(&r[2]),
            running_extremum: number(&r[3]),
            strike: number(&r[4]),
            rate: number(&r[5]),
            dividend_yield: number(&r[6]),
            volatility: number(&r[7]),
            time: number(&r[8]) / 365.0,
        };
        let g = compute_lookback_greeks(strike, kind, &i).unwrap();
        let v = |j: LookbackInputs| compute_lookback_value(strike, kind, &j).unwrap();
        // the extremum is held when the spot moves, which keeps the spot on its side of the extremum only away from it
        let h = 1e-5 * i.spot;
        let spot_moved = |d: f64| LookbackInputs {
            spot: i.spot + d,
            ..i
        };
        if (i.running_extremum - i.spot).abs() > 10.0 * h {
            let delta = (v(spot_moved(h)) - v(spot_moved(-h))) / (2.0 * h);
            assert!(
                (g.delta - delta).abs() < 1e-6 * (1.0 + delta.abs()),
                "delta {r:?}: {} {delta}",
                g.delta
            );
            let gamma = (v(spot_moved(h)) - 2.0 * g.value + v(spot_moved(-h))) / (h * h);
            assert!(
                (g.gamma - gamma).abs() < 1e-3 * (1.0 + gamma.abs()),
                "gamma {r:?}: {} {gamma}",
                g.gamma
            );
        }
        let hv = 1e-6;
        let vega = (v(LookbackInputs {
            volatility: i.volatility + hv,
            ..i
        }) - v(LookbackInputs {
            volatility: i.volatility - hv,
            ..i
        })) / (2.0 * hv);
        assert!(
            (g.vega - vega).abs() < 1e-5 * (1.0 + vega.abs()),
            "vega {r:?}: {} {vega}",
            g.vega
        );
        let rho = (v(LookbackInputs {
            rate: i.rate + hv,
            ..i
        }) - v(LookbackInputs {
            rate: i.rate - hv,
            ..i
        })) / (2.0 * hv);
        assert!(
            (g.rho - rho).abs() < 1e-5 * (1.0 + rho.abs()),
            "rho {r:?}: {} {rho}",
            g.rho
        );
    }
}
