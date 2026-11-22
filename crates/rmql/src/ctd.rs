//! The intrinsic value of the choice of collateral currency (Cheapest to Deposit, CTD,
//! LagrangeFinance II-3.7.5, second step): discount curves of the same cash flows under each
//! eligible collateral currency, joined day by day at the highest forward.
//!
//! Each curve discounts the cash-flow currency under one collateral currency: the overnight curve
//! of that currency when it is also the collateral, a cross-currency curve from
//! [`crate::xccy::bootstrap_xccy_curve`] otherwise. The party that posts collateral picks, each day,
//! the currency whose collateral rate, converted by the basis, is the highest, so the cash flows are
//! discounted at `exp(-Σ max_c f_c)` over the daily forwards `f_c = ln(P_c(d) / P_c(d + 1))`.
//!
//! [`create_ctd_curve`] uses today's forwards only: the intrinsic value. [`compute_ctd_time_value`] adds
//! the time value of switching later, with Ornstein-Uhlenbeck spreads over the first curve (a design of
//! this crate, `docs/spec_rates_fx_extension.md` 24 章). Haircuts, substitution rules and costs are not
//! included.

// `!(x >= 0.0)` is intended: it also rejects NaN
#![allow(clippy::neg_cmp_op_on_partial_ord)]

use crate::curves::{LogDiscountInterpolation, YieldCurve};
use crate::time::{Date, DayCount};
use crate::Error;
use rayon::prelude::*;
use crate::math::normal::{compute_normal_cdf, compute_normal_pdf};
use crate::math::random::generate_standard_normal;

/// A discount curve under one collateral currency.
#[derive(Debug, Clone, PartialEq)]
pub struct CollateralOption {
    pub name: String,
    pub curve: YieldCurve,
}

/// A run of days with the same cheapest collateral.
#[derive(Debug, Clone, PartialEq)]
pub struct CtdChoice {
    pub start: Date,
    pub end: Date,
    /// Position in the options given.
    pub option: usize,
    /// The chosen forward minus the next highest, summed over the run (continuous, not annualized).
    pub advantage: f64,
}

/// The discount curve with the daily choice, and the choices.
#[derive(Debug, Clone, PartialEq)]
pub struct CtdCurve {
    pub curve: YieldCurve,
    pub choices: Vec<CtdChoice>,
}

/// Joins the curves at the highest daily forward from `reference` to `end`. Every curve must have
/// `reference` as its reference date and reach `end`. The result is log-linear between the days, so
/// on each day it equals the product of the chosen daily discount ratios.
pub fn create_ctd_curve(reference: Date, end: Date, day_count: &DayCount, options: &[CollateralOption]) -> Result<CtdCurve, Error> {
    if options.is_empty() {
        return Err(Error::InvalidInput { reason: "no collateral curves given".into() });
    }
    if end <= reference {
        return Err(Error::InvalidInput { reason: format!("the end {end} must follow the reference date {reference}") });
    }
    if let Some(o) = options.iter().find(|o| o.curve.reference_date() != reference) {
        return Err(Error::InvalidInput {
            reason: format!("the curve {} starts on {}, not on {reference}", o.name, o.curve.reference_date()),
        });
    }
    let days = (end.serial() - reference.serial()) as usize;
    let mut dates = Vec::with_capacity(days + 1);
    let mut discounts = Vec::with_capacity(days + 1);
    let mut previous: Vec<f64> = options.iter().map(|o| o.curve.discount(reference)).collect::<Result<_, Error>>()?;
    let mut log_discount = 0.0;
    dates.push(reference);
    discounts.push(1.0);
    let mut choices: Vec<CtdChoice> = Vec::new();
    for k in 1..=days {
        let d = Date::from_serial(reference.serial() as i64 + k as i64)?;
        let current: Vec<f64> = options.iter().map(|o| o.curve.discount(d)).collect::<Result<_, Error>>()?;
        let forwards: Vec<f64> = previous.iter().zip(&current).map(|(p, c)| (p / c).ln()).collect();
        let (best, &top) = forwards
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.total_cmp(b.1))
            .expect("at least one curve");
        let second = forwards.iter().enumerate().filter(|(i, _)| *i != best).map(|(_, f)| *f).fold(f64::NEG_INFINITY, f64::max);
        let advantage = if second.is_finite() { top - second } else { 0.0 };
        log_discount -= top;
        dates.push(d);
        discounts.push(log_discount.exp());
        let start = Date::from_serial(d.serial() as i64 - 1)?;
        match choices.last_mut() {
            Some(run) if run.option == best => {
                run.end = d;
                run.advantage += advantage;
            }
            _ => choices.push(CtdChoice { start, end: d, option: best, advantage }),
        }
        previous = current;
    }
    let curve = YieldCurve::interpolated_discounts(dates, discounts, day_count.clone(), LogDiscountInterpolation::LogLinear)?;
    Ok(CtdCurve { curve, choices })
}

/// The dynamics of one collateral currency's spread over the reference (the first option): the
/// deviation `X` of its rate spread from today's forward spread is an Ornstein-Uhlenbeck process,
/// `dX = -kappa X dt + sigma dW`, `X(0) = 0`, with `sigma` in rate units a year.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SpreadDynamics {
    pub volatility: f64,
    pub mean_reversion: f64,
}

/// The discount factors of the collateral choice at given dates, with and without its time value.
#[derive(Debug, Clone, PartialEq)]
pub struct CtdTimeValue {
    pub dates: Vec<Date>,
    /// The choice by today's forwards ([`create_ctd_curve`]).
    pub intrinsic: Vec<f64>,
    /// `E[exp(-L)]` by Monte Carlo, `L` the log discount of the daily choice on a path.
    pub value: Vec<f64>,
    pub standard_error: Vec<f64>,
    /// `exp(-E[L])` from the same paths: the value without the convexity of the exponential.
    pub first_order: Vec<f64>,
    /// The standard error of `E[L]`, for comparing `first_order` with `first_order_formula` in logs.
    pub log_standard_error: Vec<f64>,
    /// With two options, `exp(-E[L])` in closed form: each day's choice is a floor on the spread, a
    /// Bachelier formula. `None` with another number of options.
    pub first_order_formula: Option<Vec<f64>>,
}

fn ou_covariance(rho: f64, a: &SpreadDynamics, b: &SpreadDynamics, h: f64) -> f64 {
    let k = a.mean_reversion + b.mean_reversion;
    let time = if k.abs() < 1e-12 { h } else { (1.0 - (-k * h).exp()) / k };
    rho * a.volatility * b.volatility * time
}

/// The time value of the choice of collateral (a design of this crate, `docs/spec_rates_fx_extension.md`
/// 24 章). On each day `d` the poster of collateral picks the option whose daily forward is the highest.
/// With `f_c(d)` today's daily forwards (as in [`create_ctd_curve`]), `t_d` the year fraction to the
/// start of the day and `Delta_d` the day's, the chosen forward is
/// `f_0(d) + max(0, max_c (f_c(d) - f_0(d) + X_c(t_d) Delta_d))`, with `X_c` the deviations of
/// [`SpreadDynamics`] (one per option after the first, correlated by `correlation`, by rows). The
/// reference's rate is taken as independent of the spreads, and the spreads' expectations as today's
/// forward spreads. Exact Ornstein-Uhlenbeck steps of one day; `paths` paths from `seed`.
///
/// The paths run on `threads` worker threads (0: rayon's default, usually one per logical processor; every count,
/// also 1, runs on worker threads, see [`crate::batch::map_parallel`]). Each path depends on its own index only, and
/// the sums are taken in the order of the paths, so the result does not depend on the number of threads.
#[allow(clippy::too_many_arguments)]
pub fn compute_ctd_time_value(
    reference: Date,
    dates: &[Date],
    day_count: &DayCount,
    options: &[CollateralOption],
    spreads: &[SpreadDynamics],
    correlation: &[f64],
    paths: usize,
    seed: u64,
    threads: usize,
) -> Result<CtdTimeValue, Error> {
    let invalid = |reason: String| Err(Error::InvalidInput { reason });
    if options.is_empty() || spreads.len() + 1 != options.len() {
        return invalid(format!("give one spread dynamics per option after the first ({} options, {} spreads)", options.len(), spreads.len()));
    }
    if dates.is_empty() || dates.windows(2).any(|w| w[0] >= w[1]) || dates[0] <= reference {
        return invalid("the dates must increase and follow the reference date".into());
    }
    if spreads.iter().any(|s| !(s.volatility >= 0.0) || !s.volatility.is_finite() || !(s.mean_reversion >= 0.0) || !s.mean_reversion.is_finite()) {
        return invalid("the volatilities and mean reversions must be finite and non-negative".into());
    }
    if !(2..=1_000_000).contains(&paths) {
        return invalid(format!("give from 2 to 1000000 paths, not {paths}"));
    }
    let m = spreads.len();
    if m > 0 {
        crate::lmm::check_correlation(correlation, m).map_err(|e| Error::InvalidInput { reason: format!("the spread correlation: {e:?}") })?;
    }
    let end = dates[dates.len() - 1];
    let intrinsic_curve = create_ctd_curve(reference, end, day_count, options)?;
    let days = (end.serial() - reference.serial()) as usize;
    // today's daily forwards, the spreads over the reference (by day, then option), and the days' year fractions
    let mut reference_forward = Vec::with_capacity(days);
    let mut spread = Vec::with_capacity(days * m);
    let mut start_time = Vec::with_capacity(days);
    let mut length = Vec::with_capacity(days);
    let mut previous: Vec<f64> = options.iter().map(|o| o.curve.discount(reference)).collect::<Result<_, Error>>()?;
    for k in 1..=days {
        let from = Date::from_serial(reference.serial() as i64 + k as i64 - 1)?;
        let to = Date::from_serial(reference.serial() as i64 + k as i64)?;
        let current: Vec<f64> = options.iter().map(|o| o.curve.discount(to)).collect::<Result<_, Error>>()?;
        let forwards: Vec<f64> = previous.iter().zip(&current).map(|(p, c)| (p / c).ln()).collect();
        reference_forward.push(forwards[0]);
        spread.extend((0..m).map(|c| forwards[c + 1] - forwards[0]));
        start_time.push(day_count.year_fraction(reference, from, None, None)?);
        length.push(day_count.year_fraction(from, to, None, None)?);
        previous = current;
    }
    // The one-day steps on the spreads that move: the Cholesky factor of the step's covariance (one per step length) and
    // the decay of each spread, laid out by day, so that the loop over the paths computes and allocates nothing.
    // X moves from the start of day k - 1 to the start of day k; day 0 starts at X = 0.
    let active: Vec<usize> = (0..m).filter(|&c| spreads[c].volatility > 0.0).collect();
    let na = active.len();
    let mut moves = vec![false; days];
    let mut decay = vec![0.0; days * na];
    let mut factor = vec![0.0; days * na * na];
    let mut by_length: Vec<(f64, Vec<Vec<f64>>)> = Vec::new();
    for k in 1..days {
        let h = start_time[k] - start_time[k - 1];
        if !(h > 0.0) || active.is_empty() {
            continue;
        }
        let slot = match by_length.iter().position(|(x, _)| *x == h) {
            Some(slot) => slot,
            None => {
                let cov: Vec<Vec<f64>> = active
                    .iter()
                    .map(|&a| active.iter().map(|&b| ou_covariance(correlation[a * m + b], &spreads[a], &spreads[b], h)).collect())
                    .collect();
                by_length.push((h, crate::math::linear::compute_cholesky(&cov)?));
                by_length.len() - 1
            }
        };
        moves[k] = true;
        for (row, &a) in active.iter().enumerate() {
            let base = k * na + row;
            decay[base] = (-spreads[a].mean_reversion * h).exp();
            factor[base * na..(base + 1) * na].copy_from_slice(&by_length[slot].1[row]);
        }
    }
    let index: Vec<usize> = dates.iter().map(|d| (d.serial() - reference.serial()) as usize).collect();
    // One path: the log discount at each of the dates. Each path depends on its own index only (counter-based draws).
    let simulate = |p: usize| -> Vec<f64> {
        let mut x = vec![0.0; m];
        let mut z = vec![0.0; na];
        let mut out = vec![0.0; dates.len()];
        let mut log_discount = 0.0;
        let mut next = 0;
        for k in 0..days {
            if moves[k] {
                for (i, w) in z.iter_mut().enumerate() {
                    *w = generate_standard_normal(seed, p as u64, (k * m + i) as u64);
                }
                for (row, &a) in active.iter().enumerate() {
                    let base = k * na + row;
                    let shock: f64 = factor[base * na..(base + 1) * na].iter().zip(z.iter()).map(|(l, w)| l * w).sum();
                    x[a] = x[a] * decay[base] + shock;
                }
            }
            let best = (0..m).map(|c| spread[k * m + c] + x[c] * length[k]).fold(0.0, f64::max);
            log_discount += reference_forward[k] + best;
            while next < index.len() && index[next] == k + 1 {
                out[next] = log_discount;
                next += 1;
            }
        }
        out
    };
    let (mut sum, mut sum_squares, mut sum_log) = (vec![0.0; dates.len()], vec![0.0; dates.len()], vec![0.0; dates.len()]);
    let mut sum_log_squares = vec![0.0; dates.len()];
    // blocks of paths: the paths of a block in parallel, then their sums in the order of the paths
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(threads)
        .build()
        .map_err(|e| Error::InvalidInput { reason: format!("cannot start {threads} threads: {e}") })?;
    let block = (1_000_000 / dates.len()).clamp(64, 65_536);
    pool.install(|| {
        let mut first = 0;
        while first < paths {
            let last = (first + block).min(paths);
            let rows: Vec<Vec<f64>> = (first..last).into_par_iter().map(simulate).collect();
            for log_discount in &rows {
                for (next, &l) in log_discount.iter().enumerate() {
                    let v = (-l).exp();
                    sum[next] += v;
                    sum_squares[next] += v * v;
                    sum_log[next] += l;
                    sum_log_squares[next] += l * l;
                }
            }
            first = last;
        }
    });
    let n = paths as f64;
    let value: Vec<f64> = sum.iter().map(|s| s / n).collect();
    let standard_error = sum.iter().zip(&sum_squares).map(|(s, q)| ((q - s * s / n) / (n - 1.0)).max(0.0).sqrt() / n.sqrt()).collect();
    let first_order = sum_log.iter().map(|s| (-s / n).exp()).collect();
    let log_standard_error = sum_log.iter().zip(&sum_log_squares).map(|(s, q)| ((q - s * s / n) / (n - 1.0)).max(0.0).sqrt() / n.sqrt()).collect();
    let first_order_formula = if m == 1 {
        let s = &spreads[0];
        let mut log_discount = 0.0;
        let mut out = Vec::with_capacity(dates.len());
        let mut next = 0;
        for k in 0..days {
            let sd = ou_covariance(1.0, s, s, start_time[k]).sqrt() * length[k];
            let mean = spread[k * m];
            let floor = if sd > 0.0 {
                let z = mean / sd;
                mean * compute_normal_cdf(z) + sd * compute_normal_pdf(z)
            } else {
                mean.max(0.0)
            };
            log_discount += reference_forward[k] + floor;
            while next < index.len() && index[next] == k + 1 {
                out.push((-log_discount).exp());
                next += 1;
            }
        }
        Some(out)
    } else {
        None
    };
    let intrinsic = dates.iter().map(|d| intrinsic_curve.curve.discount(*d)).collect::<Result<_, Error>>()?;
    Ok(CtdTimeValue { dates: dates.to_vec(), intrinsic, value, standard_error, first_order, log_standard_error, first_order_formula })
}

#[cfg(test)]
// the tests compare several vectors by the same index
#[allow(clippy::needless_range_loop)]
mod tests {
    use super::*;
    use crate::interest_rate::{Compounding, InterestRate};
    use crate::time::Actual365Kind;

    fn d(y: i32, m: u32, day: u32) -> Date {
        Date::from_ymd(y, m, day).unwrap()
    }

    fn act365() -> DayCount {
        DayCount::Actual365Fixed(Actual365Kind::Standard)
    }

    fn zero(rates: &[f64]) -> YieldCurve {
        let today = d(2026, 11, 4);
        let dates = vec![today, d(2027, 11, 4), d(2029, 11, 4), d(2036, 11, 4)];
        YieldCurve::zero(dates, rates.to_vec(), act365(), Compounding::Continuous, 0).unwrap()
    }

    #[test]
    fn one_curve_is_reproduced_and_the_choice_never_discounts_less() {
        let today = d(2026, 11, 4);
        let end = d(2036, 11, 4);
        let jpy = CollateralOption { name: "JPY".into(), curve: zero(&[0.005, 0.005, 0.008, 0.012]) };
        // crosses the yen curve: higher at the short end, lower at the long end
        let usd = CollateralOption { name: "USD".into(), curve: zero(&[0.009, 0.009, 0.007, 0.006]) };
        let alone = create_ctd_curve(today, end, &act365(), std::slice::from_ref(&jpy)).unwrap();
        let both = create_ctd_curve(today, end, &act365(), &[jpy.clone(), usd.clone()]).unwrap();
        for day in [d(2027, 1, 1), d(2029, 11, 4), d(2033, 6, 30), end] {
            let single = jpy.curve.discount(day).unwrap();
            assert!((alone.curve.discount(day).unwrap() - single).abs() < 1e-13);
            let ctd = both.curve.discount(day).unwrap();
            assert!(ctd <= single + 1e-15 && ctd <= usd.curve.discount(day).unwrap() + 1e-15);
        }
        assert_eq!(alone.choices.len(), 1);
        // the dollar is chosen first and the yen later
        assert_eq!(both.choices.first().unwrap().option, 1);
        assert_eq!(both.choices.last().unwrap().option, 0);
        assert!(both.choices.iter().all(|c| c.advantage >= 0.0));
    }

    fn crossing() -> (CollateralOption, CollateralOption) {
        let jpy = CollateralOption { name: "JPY".into(), curve: zero(&[0.005, 0.005, 0.008, 0.012]) };
        let usd = CollateralOption { name: "USD".into(), curve: zero(&[0.009, 0.009, 0.007, 0.006]) };
        (jpy, usd)
    }

    #[test]
    fn without_volatility_the_time_value_is_zero() {
        let today = d(2026, 11, 4);
        let dates = [d(2027, 11, 4), d(2029, 11, 4), d(2036, 11, 4)];
        let (jpy, usd) = crossing();
        let still = [SpreadDynamics { volatility: 0.0, mean_reversion: 0.1 }];
        let r = compute_ctd_time_value(today, &dates, &act365(), &[jpy.clone(), usd], &still, &[1.0], 10, 1, 1).unwrap();
        let formula = r.first_order_formula.as_ref().unwrap();
        for i in 0..dates.len() {
            assert!((r.value[i] / r.intrinsic[i] - 1.0).abs() < 1e-12, "{i}");
            assert!((formula[i] / r.intrinsic[i] - 1.0).abs() < 1e-12, "{i}");
            assert!(r.standard_error[i] < 1e-15);
        }
        // one currency: no choice, the curve itself
        let alone = compute_ctd_time_value(today, &dates, &act365(), std::slice::from_ref(&jpy), &[], &[], 10, 1, 1).unwrap();
        for (i, day) in dates.iter().enumerate() {
            assert!((alone.value[i] / jpy.curve.discount(*day).unwrap() - 1.0).abs() < 1e-12);
        }
        assert!(alone.first_order_formula.is_none());
    }

    #[test]
    fn the_time_value_follows_the_bachelier_floors() {
        let today = d(2026, 11, 4);
        let dates = [d(2027, 11, 4), d(2029, 11, 4), d(2036, 11, 4)];
        let (jpy, usd) = crossing();
        let moving = [SpreadDynamics { volatility: 0.005, mean_reversion: 0.1 }];
        let r = compute_ctd_time_value(today, &dates, &act365(), &[jpy.clone(), usd.clone()], &moving, &[1.0], 2000, 7, 1).unwrap();
        let formula = r.first_order_formula.as_ref().unwrap();
        for i in 0..dates.len() {
            // the expected log discount is a sum of Bachelier floors: unbiased against the paths
            let (simulated, closed) = (-r.first_order[i].ln(), -formula[i].ln());
            assert!((simulated - closed).abs() < 4.0 * r.log_standard_error[i], "{i}: {simulated} against {closed} ± {}", r.log_standard_error[i]);
            // Jensen on the same paths, and the floors are worth at least their intrinsic value
            assert!(r.value[i] >= r.first_order[i]);
            assert!(formula[i] <= r.intrinsic[i] * (1.0 + 1e-15));
        }
        // over ten years the option to switch later is worth more than the noise
        let last = dates.len() - 1;
        assert!(r.intrinsic[last] - r.value[last] > 4.0 * r.standard_error[last], "{} against {} ± {}", r.intrinsic[last], r.value[last], r.standard_error[last]);
        // more volatility, more time value
        let wild = [SpreadDynamics { volatility: 0.01, mean_reversion: 0.1 }];
        let w = compute_ctd_time_value(today, &dates, &act365(), &[jpy.clone(), usd.clone()], &wild, &[1.0], 10, 7, 1).unwrap();
        assert!(w.first_order_formula.unwrap()[last] < formula[last]);
        // three currencies: a third option can only add value to the poster, and the paths keep Jensen
        let eur = CollateralOption { name: "EUR".into(), curve: zero(&[0.007, 0.007, 0.0075, 0.009]) };
        let both = [moving[0], SpreadDynamics { volatility: 0.004, mean_reversion: 0.0 }];
        let three = compute_ctd_time_value(today, &dates, &act365(), &[jpy.clone(), usd.clone(), eur], &both, &[1.0, 0.5, 0.5, 1.0], 2000, 7, 1).unwrap();
        assert!(three.first_order_formula.is_none());
        for i in 0..dates.len() {
            assert!(three.value[i] >= three.first_order[i]);
            assert!(three.intrinsic[i] <= r.intrinsic[i] * (1.0 + 1e-15));
            assert!(three.value[i] < r.value[i] + 4.0 * (three.standard_error[i] + r.standard_error[i]));
        }
        // invalid inputs
        assert!(compute_ctd_time_value(today, &dates, &act365(), &[jpy.clone(), usd.clone()], &[], &[], 10, 1, 1).is_err());
        assert!(compute_ctd_time_value(today, &[dates[1], dates[0]], &act365(), &[jpy.clone(), usd.clone()], &moving, &[1.0], 10, 1, 1).is_err());
        assert!(compute_ctd_time_value(today, &dates, &act365(), &[jpy.clone(), usd.clone()], &[SpreadDynamics { volatility: -0.1, mean_reversion: 0.0 }], &[1.0], 10, 1, 1).is_err());
        assert!(compute_ctd_time_value(today, &dates, &act365(), &[jpy, usd], &moving, &[1.0, 0.0], 10, 1, 1).is_err());
    }

    #[test]
    fn the_result_does_not_depend_on_the_number_of_threads() {
        let today = d(2026, 11, 4);
        let dates = [d(2027, 5, 4), d(2028, 11, 4), d(2031, 11, 4)];
        let jpy = CollateralOption { name: "JPY".into(), curve: zero(&[0.005, 0.005, 0.005, 0.012]) };
        let usd = CollateralOption { name: "USD".into(), curve: zero(&[0.009, 0.009, 0.009, 0.006]) };
        let eur = CollateralOption { name: "EUR".into(), curve: zero(&[0.007, 0.007, 0.011, 0.009]) };
        let both = [SpreadDynamics { volatility: 0.004, mean_reversion: 0.1 }, SpreadDynamics { volatility: 0.002, mean_reversion: 0.0 }];
        let options = [jpy, usd, eur];
        let run = |threads| compute_ctd_time_value(today, &dates, &act365(), &options, &both, &[1.0, 0.5, 0.5, 1.0], 700, 11, threads).unwrap();
        let first = run(1);
        for threads in [2, 3, 8, 0] {
            assert_eq!(run(threads), first, "{threads} threads");
        }
    }

    #[test]
    fn invalid_inputs_are_errors() {
        let today = d(2026, 11, 4);
        let jpy = CollateralOption { name: "JPY".into(), curve: zero(&[0.005, 0.005, 0.008, 0.012]) };
        assert!(create_ctd_curve(today, today, &act365(), std::slice::from_ref(&jpy)).is_err());
        assert!(create_ctd_curve(today, d(2030, 1, 1), &act365(), &[]).is_err());
        let other = CollateralOption {
            name: "late".into(),
            curve: YieldCurve::flat(d(2026, 11, 5), InterestRate::new(0.01, act365(), Compounding::Continuous, 0).unwrap()),
        };
        assert!(create_ctd_curve(today, d(2030, 1, 1), &act365(), &[jpy, other]).is_err());
    }
}
