//! Convertible bonds against the QuantLib R package Version 1.42 (`ConvertibleFixedCouponBond` with
//! `BinomialCRRConvertibleEngine`; tools/golden/p16_convertible.R). Inputs are synthetic.

mod common;

use common::golden_rows;
use rmql::convertible::{
    compute_convertible_value, compute_convertible_value_like_quantlib, Callability,
    CallabilityKind, ConversionExercise, ConversionTerms, ConvertibleInputs, ConvertibleTerms,
    Coupon,
};

fn number(s: &str) -> f64 {
    s.parse().unwrap()
}

fn terms_of(r: &[String]) -> ConvertibleTerms {
    let days: Vec<f64> = r[1].split(';').map(number).collect();
    let exercise = match r[0].as_str() {
        "european" => ConversionExercise::European(days[0] / 365.0),
        "american" => ConversionExercise::American {
            start: days[0] / 365.0,
            end: days[1] / 365.0,
        },
        "bermudan" => ConversionExercise::Bermudan(days.iter().map(|d| d / 365.0).collect()),
        other => panic!("{other}"),
    };
    let coupons = r[10]
        .split('|')
        .map(|c| {
            let (d, a) = c.split_once(':').unwrap();
            Coupon {
                time: number(d) / 365.0,
                amount: number(a),
            }
        })
        .collect();
    let callabilities = if r[11] == "-" {
        vec![]
    } else {
        r[11]
            .split('|')
            .map(|c| {
                let f: Vec<&str> = c.split(':').collect();
                Callability {
                    time: number(f[0]) / 365.0,
                    kind: if f[1] == "call" {
                        CallabilityKind::Call
                    } else {
                        CallabilityKind::Put
                    },
                    price: number(f[2]),
                    trigger: if f[3] == "NA" {
                        None
                    } else {
                        Some(number(f[3]))
                    },
                }
            })
            .collect()
    };
    ConvertibleTerms {
        redemption: number(&r[3]),
        conversion: ConversionTerms::Fixed {
            ratio: number(&r[2]),
        },
        exercise,
        coupons,
        callabilities,
    }
}

fn inputs_of(r: &[String]) -> ConvertibleInputs {
    ConvertibleInputs {
        spot: number(&r[5]),
        rate: number(&r[6]),
        dividend_yield: number(&r[7]),
        volatility: number(&r[8]),
        credit_spread: number(&r[4]),
    }
}

/// QuantLib 1.42's time grid ends at (end / steps) * steps, which can be below the exercise end by one unit in the last
/// place; an American conversion at the last time is then not allowed (compute_convertible_value_like_quantlib).
fn quantlib_skips_conversion_at_maturity(r: &[String]) -> bool {
    let end: f64 = r[1].split(';').next_back().map(number).unwrap() / 365.0;
    let steps: f64 = number(&r[9]);
    r[0] == "american" && end > (end / steps) * steps
}

#[test]
fn values_agree_with_quantlib() {
    let rows = golden_rows("convertible_bonds.csv");
    assert_eq!(rows.len(), 300);
    let (mut worst_like, mut worst_other, mut worst_artifact, mut artifact_rows, mut differing) =
        (0.0f64, 0.0f64, 0.0f64, 0, 0);
    for r in &rows {
        let (terms, inputs, steps) = (terms_of(r), inputs_of(r), r[9].parse().unwrap());
        let expected = number(&r[12]);
        // the same method with QuantLib's rounding: all 300 rows
        let like = compute_convertible_value_like_quantlib(&terms, &inputs, steps).unwrap();
        let relative = (like - expected).abs() / expected.abs();
        worst_like = worst_like.max(relative);
        assert!(
            relative < 1e-9,
            "{}: {like} against {expected} (relative {relative:e})",
            r[..10].join(",")
        );
        // the value without that artifact: the same where QuantLib's grid does not end below the exercise end; elsewhere it can
        // differ in either direction (a conversion at maturity changes the conversion probabilities, which blend the discount rates
        // of the earlier nodes; the value is not monotone in the right to convert), by a fraction of a percent
        let value = compute_convertible_value(&terms, &inputs, steps).unwrap();
        let relative = (value - expected).abs() / expected.abs();
        if quantlib_skips_conversion_at_maturity(r) {
            artifact_rows += 1;
            worst_artifact = worst_artifact.max(relative);
            assert!(
                relative < 0.01,
                "{}: {value} against {expected} (relative {relative:e})",
                r[..10].join(",")
            );
            if relative > 1e-9 {
                differing += 1;
            }
        } else {
            worst_other = worst_other.max(relative);
            assert!(
                relative < 1e-9,
                "{}: {value} against {expected} (relative {relative:e})",
                r[..10].join(",")
            );
        }
    }
    println!("largest relative difference: {worst_like:e} (QuantLib's rounding), {worst_other:e} (other rows); {artifact_rows} rows where QuantLib's grid ends below the exercise end, {differing} of them differ, by at most {worst_artifact:e}");
    assert!(differing > 0, "the artifact never matters in the data");
}
