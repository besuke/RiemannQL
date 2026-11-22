//! Lookback options under Black-Scholes-Merton: the payoff depends on the minimum or the maximum of the
//! asset price over a set of monitoring dates. Every price is a function generic over [`Operand`], so
//! its sensitivities come from automatic differentiation (`docs/spec_lookback_convertible.md`).
//!
//! | | call | put |
//! |---|---|---|
//! | floating strike | `S_T - m` | `M - S_T` |
//! | fixed strike `K` | `max(M - K, 0)` | `max(K - m, 0)` |
//!
//! `m` and `M` are the minimum and the maximum of the price over the monitoring dates, including the
//! value observed so far (`running_extremum`: a minimum for a floating call and a fixed put, a maximum
//! for a floating put and a fixed call, as QuantLib's `minmax`).
//!
//! **Continuous monitoring** has a closed form (Goldman, Sosin and Gatto 1979; Conze and Viswanathan 1991;
//! the formulas are standard and are written here from them, not copied from QuantLib's source). With
//! `b = r - q` the formulas divide by `b`; around `b = 0` the bracket is replaced by its expansion in
//! `b` to the third order (QuantLib 1.42 returns NaN or an infinite value at `b = 0` exactly).
//!
//! **Discrete monitoring** is a binomial tree in `z = S / m` (floating call; `S / M` for a put), the
//! price being homogeneous of degree 1 in `(S, m)`: `V = m w(z)`. A tree in `ln z` is anchored at `z = 1`.
//! On a monitoring date the extremum is updated: for a call, a price below the minimum (`z < 1`) becomes
//! the new minimum, and the value in terms of the old `m` is `z` times the value at `z = 1`:
//! `w_before(z) = z w_after(1)` for `z < 1`, `w_before(z) = w_after(z)` otherwise (a put, mirrored). A
//! fixed-strike option reduces to a floating one, since `max(M, K) = S_T + (max(M, K) - S_T)`: a fixed
//! call is a floating put with the maximum `max(M, K)`, plus `S e^{-qT} - K e^{-rT}`, and a fixed put is
//! a floating call with the minimum `min(m, K)`, plus `K e^{-rT} - S e^{-qT}`.

// `!(x > 0.0)` is intended: it also rejects NaN
#![allow(clippy::neg_cmp_op_on_partial_ord)]

use crate::autodiff::{Operand, Recorder, Var};
use crate::options::OptionType;
use crate::Error;

/// A floating or a fixed strike.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LookbackStrike {
    Floating,
    Fixed,
}

/// The inputs of a lookback option: flat continuous rates and time in years. `strike` is used by a fixed
/// strike only.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LookbackInputs<T = f64> {
    pub spot: T,
    pub running_extremum: T,
    pub strike: T,
    pub rate: T,
    pub dividend_yield: T,
    pub volatility: T,
    pub time: T,
}

/// A value and its sensitivities. `gamma` of a tree comes from the second difference of the values on the
/// tree's nodes around the spot; `theta` is minus the derivative with respect to `time`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LookbackGreeks {
    pub value: f64,
    pub delta: f64,
    pub gamma: f64,
    pub vega: f64,
    pub rho: f64,
    pub dividend_rho: f64,
    pub theta: f64,
    pub strike_sensitivity: f64,
    pub extremum_sensitivity: f64,
}

fn invalid<T>(reason: String) -> Result<T, Error> {
    Err(Error::InvalidInput { reason })
}

/// A minimum is tracked by a floating call and a fixed put.
fn tracks_minimum(strike: LookbackStrike, kind: OptionType) -> bool {
    matches!(
        (strike, kind),
        (LookbackStrike::Floating, OptionType::Call) | (LookbackStrike::Fixed, OptionType::Put)
    )
}

impl<T: Operand> LookbackInputs<T> {
    fn check(&self, strike: LookbackStrike, kind: OptionType) -> Result<(), Error> {
        for (name, x) in [
            ("spot", self.spot),
            ("running_extremum", self.running_extremum),
            ("volatility", self.volatility),
            ("time", self.time),
        ] {
            if !(x.primal() > 0.0) {
                return invalid(format!("{name} must be positive, not {}", x.primal()));
            }
        }
        if strike == LookbackStrike::Fixed && !(self.strike.primal() > 0.0) {
            return invalid(format!(
                "strike must be positive, not {}",
                self.strike.primal()
            ));
        }
        let (spot, extremum) = (self.spot.primal(), self.running_extremum.primal());
        if tracks_minimum(strike, kind) {
            if extremum > spot {
                return invalid(format!(
                    "the minimum so far {extremum} must not exceed the spot {spot}"
                ));
            }
        } else if extremum < spot {
            return invalid(format!(
                "the maximum so far {extremum} must not be below the spot {spot}"
            ));
        }
        Ok(())
    }
}

impl LookbackInputs<f64> {
    fn lift<'t>(&self, recorder: &'t Recorder) -> LookbackInputs<Var<'t>> {
        LookbackInputs {
            spot: recorder.input(self.spot),
            running_extremum: recorder.input(self.running_extremum),
            strike: recorder.input(self.strike),
            rate: recorder.input(self.rate),
            dividend_yield: recorder.input(self.dividend_yield),
            volatility: recorder.input(self.volatility),
            time: recorder.input(self.time),
        }
    }
}

/// A fixed-strike option as a floating-strike one plus a constant: the floating kind, its extremum and the constant.
struct Reduced<T> {
    kind: OptionType,
    extremum: T,
    offset: T,
}

fn reduce<T: Operand>(
    strike: LookbackStrike,
    kind: OptionType,
    i: &LookbackInputs<T>,
) -> Reduced<T> {
    let (spot_value, strike_value) = (
        i.spot * (-i.dividend_yield * i.time).exp(),
        i.strike * (-i.rate * i.time).exp(),
    );
    match (strike, kind) {
        (LookbackStrike::Floating, _) => Reduced {
            kind,
            extremum: i.running_extremum,
            offset: i.spot.like(0.0),
        },
        (LookbackStrike::Fixed, OptionType::Call) => Reduced {
            kind: OptionType::Put,
            extremum: i.running_extremum.max(i.strike),
            offset: spot_value - strike_value,
        },
        (LookbackStrike::Fixed, OptionType::Put) => Reduced {
            kind: OptionType::Call,
            extremum: i.running_extremum.min(i.strike),
            offset: strike_value - spot_value,
        },
    }
}

/// Below this `|b|` the bracket of the formulas is replaced by its expansion in `b`: the rounding error of
/// the formula grows like `1e-16 / |b|` and the error of the expansion (to the third order) like `b^3`.
const SMALL_DRIFT: f64 = 1e-4;

/// A floating-strike option under continuous monitoring: `l` is the minimum so far for a call, the maximum
/// for a put. `expansion` forces the expansion (`Some(true)`) or the formula (`Some(false)`) for the bracket, for the tests.
#[allow(clippy::too_many_arguments)]
fn continuous_floating<T: Operand>(
    kind: OptionType,
    spot: T,
    l: T,
    rate: T,
    dividend: T,
    volatility: T,
    time: T,
    expansion: Option<bool>,
) -> T {
    let b = rate - dividend;
    let tau = time.sqrt();
    let std_dev = volatility * tau;
    let ln_x = (spot / l).ln();
    let half_variance = volatility * volatility * 0.5;
    let a1 = (ln_x + (b + half_variance) * time) / std_dev;
    let a2 = a1 - std_dev;
    let discount = (-rate * time).exp();
    let carry = (-dividend * time).exp();
    // `eps` is the sign of the arguments of the bracket's normal distributions, `sign` the sign of the bracket
    let (main, eps, sign) = match kind {
        OptionType::Call => (
            spot * carry * a1.normal_cdf() - l * discount * a2.normal_cdf(),
            -1.0,
            1.0,
        ),
        OptionType::Put => (
            l * discount * (-a2).normal_cdf() - spot * carry * (-a1).normal_cdf(),
            1.0,
            -1.0,
        ),
    };
    let use_expansion = expansion.unwrap_or(b.primal().abs() < SMALL_DRIFT);
    let bracket_over_b = if !use_expansion {
        // G(b) / b, G(b) = x^(-2b/s^2) N(eps (a1 - 2 b tau / s)) - e^(bT) N(eps a1)
        let power = (b * -2.0 / (volatility * volatility) * ln_x).exp();
        let g = power * ((a1 - b * 2.0 * tau / volatility) * eps).normal_cdf()
            - (b * time).exp() * (a1 * eps).normal_cdf();
        g / b
    } else {
        // G(b) / b = G'(0) + G''(0) b / 2 + G'''(0) b^2 / 6, with the values at b = 0 (a0 for a1)
        let a0 = (ln_x + half_variance * time) / std_dev;
        let cdf = (a0 * eps).normal_cdf();
        let pdf = a0.normal_pdf();
        let kappa = tau / volatility;
        let lambda_ln = ln_x * 2.0 / (volatility * volatility);
        let first = -((lambda_ln + time) * cdf) - kappa * pdf * (2.0 * eps);
        let second = (lambda_ln * lambda_ln - time * time) * cdf
            + kappa * pdf * ((lambda_ln - time) * (2.0 * eps));
        let third = -((lambda_ln * lambda_ln * lambda_ln + time * time * time) * cdf)
            - kappa * pdf * (3.0 * eps) * (lambda_ln * lambda_ln + time * time)
            + kappa * kappa * a0 * pdf * (3.0 * eps) * (lambda_ln + time)
            + kappa * kappa * kappa * pdf * (2.0 * eps) * (-(a0 * a0) + 1.0);
        first + second * b * 0.5 + third * b * b * (1.0 / 6.0)
    };
    main + spot * discount * half_variance * bracket_over_b * sign
}

/// The price of a lookback option under continuous monitoring.
pub fn compute_lookback_value<T: Operand>(
    strike: LookbackStrike,
    kind: OptionType,
    inputs: &LookbackInputs<T>,
) -> Result<T, Error> {
    inputs.check(strike, kind)?;
    let r = reduce(strike, kind, inputs);
    let i = inputs;
    Ok(continuous_floating(
        r.kind,
        i.spot,
        r.extremum,
        i.rate,
        i.dividend_yield,
        i.volatility,
        i.time,
        None,
    ) + r.offset)
}

/// A function of recorded inputs, for the Greeks.
fn greeks_of<F>(inputs: &LookbackInputs, f: F) -> Result<LookbackGreeks, Error>
where
    F: for<'t> Fn(&LookbackInputs<Var<'t>>) -> Result<Var<'t>, Error>,
{
    let recorder = Recorder::new();
    let lifted = inputs.lift(&recorder);
    let value = f(&lifted)?;
    let wrt = [
        lifted.spot,
        lifted.strike,
        lifted.rate,
        lifted.dividend_yield,
        lifted.volatility,
        lifted.time,
        lifted.running_extremum,
    ];
    let first = recorder.gradient_vars(value, &wrt)?;
    let gamma = recorder.gradient(first[0], &[lifted.spot])?[0].mean();
    Ok(LookbackGreeks {
        value: value.primal(),
        delta: first[0].primal(),
        gamma,
        vega: first[4].primal(),
        rho: first[2].primal(),
        dividend_rho: first[3].primal(),
        theta: -first[5].primal(),
        strike_sensitivity: first[1].primal(),
        extremum_sensitivity: first[6].primal(),
    })
}

/// The value and the Greeks of a lookback option under continuous monitoring.
pub fn compute_lookback_greeks(
    strike: LookbackStrike,
    kind: OptionType,
    inputs: &LookbackInputs,
) -> Result<LookbackGreeks, Error> {
    greeks_of(inputs, |i| compute_lookback_value(strike, kind, i))
}

/// What a tree gives besides the value: the values of `w` at the lattice nodes around the spot, for `gamma`.
struct TreeOutput<T> {
    value: T,
    nodes_z: Vec<f64>,
    nodes_w: Vec<f64>,
}

/// A floating-strike option on a binomial tree in `z = S / l`, `steps` steps, `monitored[k]` for the steps
/// `0..=steps` at which the extremum is updated.
#[allow(clippy::too_many_arguments)]
fn floating_tree<T: Operand>(
    kind: OptionType,
    spot: T,
    l: T,
    rate: T,
    dividend: T,
    volatility: T,
    time: T,
    monitored: &[bool],
    steps: usize,
) -> Result<TreeOutput<T>, Error> {
    let dt = time / steps as f64;
    let dx = volatility * dt.sqrt();
    let drift = (rate - dividend - volatility * volatility * 0.5) * dt;
    let pu = drift / dx * 0.5 + 0.5;
    if !(0.0..=1.0).contains(&pu.primal()) {
        return invalid(format!(
            "negative probability in the tree (up {})",
            pu.primal()
        ));
    }
    let pd = -pu + 1.0;
    let discount = (-rate * dt).exp();
    let z0 = spot / l;
    // the lattice is z = exp(dx j), anchored at z = 1; the spot is between two nodes (or on one), and one node more on each side
    let j0 = z0.primal().ln() / dx.primal();
    if !j0.is_finite() || j0.abs() > 1e6 {
        return invalid(format!(
            "the spot is too far from the extremum for this tree (log ratio over the step {j0})"
        ));
    }
    let (j_low, j_high) = (j0.floor() as i64, j0.ceil() as i64);
    let pad = 1;
    let lo = j_low - steps as i64 - pad;
    let hi = j_high + steps as i64 + pad;
    let len = (hi - lo + 1) as usize;
    let nodes: Vec<T> = (lo..=hi).map(|j| (dx * j as f64).exp()).collect();
    let zero = spot.like(0.0);
    let zero_index = if lo <= 0 && 0 <= hi {
        Some((0 - lo) as usize)
    } else {
        None
    };
    // the extremum is updated: a call's minimum moves down (z < 1), a put's maximum up (z > 1); the value in terms of the old extremum
    let update = |w: &mut Vec<T>| {
        if let Some(i0) = zero_index {
            let at_one = w[i0];
            for (idx, value) in w.iter_mut().enumerate() {
                let j = lo + idx as i64;
                let moved = match kind {
                    OptionType::Call => j < 0,
                    OptionType::Put => j > 0,
                };
                if moved {
                    *value = nodes[idx] * at_one;
                }
            }
        }
    };
    let mut w: Vec<T> = nodes
        .iter()
        .map(|&z| match kind {
            OptionType::Call => (z - 1.0).max(zero),
            OptionType::Put => (-z + 1.0).max(zero),
        })
        .collect();
    if monitored[steps] {
        update(&mut w);
    }
    for k in (0..steps).rev() {
        let mut next = w.clone();
        for idx in 1..len - 1 {
            next[idx] = (pd * w[idx - 1] + pu * w[idx + 1]) * discount;
        }
        if monitored[k] {
            update(&mut next);
        }
        w = next;
    }
    let (ia, ib) = ((j_low - lo) as usize, (j_high - lo) as usize);
    let value_at_root = if ia == ib {
        w[ia]
    } else {
        w[ia] + (w[ib] - w[ia]) * ((z0 - nodes[ia]) / (nodes[ib] - nodes[ia]))
    };
    let around = (ia - 1)..=(ib + 1);
    Ok(TreeOutput {
        value: value_at_root * l,
        nodes_z: around.clone().map(|i| nodes[i].primal()).collect(),
        nodes_w: around.map(|i| w[i].primal()).collect(),
    })
}

fn monitoring_steps(monitoring: &[f64], time: f64, steps: usize) -> Result<Vec<bool>, Error> {
    let mut monitored = vec![false; steps + 1];
    let mut previous = 0.0;
    for &t in monitoring {
        if !(t > previous) || t > time * (1.0 + 1e-12) {
            return invalid(format!(
                "the monitoring times must increase and lie in (0, {time}], found {t}"
            ));
        }
        previous = t;
        let k = ((t / time * steps as f64).round() as usize).clamp(1, steps);
        monitored[k] = true;
    }
    Ok(monitored)
}

fn discrete_parts<T: Operand>(
    strike: LookbackStrike,
    kind: OptionType,
    inputs: &LookbackInputs<T>,
    monitoring: &[f64],
    steps: usize,
) -> Result<(T, TreeOutput<T>, T), Error> {
    inputs.check(strike, kind)?;
    if steps < 2 {
        return invalid(format!("at least 2 time steps required, {steps} provided"));
    }
    let monitored = monitoring_steps(monitoring, inputs.time.primal(), steps)?;
    let r = reduce(strike, kind, inputs);
    let i = inputs;
    let tree = floating_tree(
        r.kind,
        i.spot,
        r.extremum,
        i.rate,
        i.dividend_yield,
        i.volatility,
        i.time,
        &monitored,
        steps,
    )?;
    Ok((tree.value + r.offset, tree, r.extremum))
}

/// The price of a lookback option monitored at `monitoring` (times in years, increasing, in `(0, time]`), on a
/// binomial tree of `steps` steps. A monitoring time is placed on the nearest step. The payoff is floored at 0
/// (it matters only when `time` is not a monitoring time).
pub fn compute_discrete_lookback_value<T: Operand>(
    strike: LookbackStrike,
    kind: OptionType,
    inputs: &LookbackInputs<T>,
    monitoring: &[f64],
    steps: usize,
) -> Result<T, Error> {
    Ok(discrete_parts(strike, kind, inputs, monitoring, steps)?.0)
}

/// The value and the Greeks of a discretely monitored lookback option on a tree. The sensitivities are the
/// derivatives of the tree by automatic differentiation; `gamma` is the second difference of the tree's values
/// at the nodes around the spot (the tree's value is piecewise linear in the spot).
pub fn compute_discrete_lookback_greeks(
    strike: LookbackStrike,
    kind: OptionType,
    inputs: &LookbackInputs,
    monitoring: &[f64],
    steps: usize,
) -> Result<LookbackGreeks, Error> {
    let recorder = Recorder::new();
    let lifted = inputs.lift(&recorder);
    let (value, tree, extremum) = discrete_parts(strike, kind, &lifted, monitoring, steps)?;
    let wrt = [
        lifted.spot,
        lifted.strike,
        lifted.rate,
        lifted.dividend_yield,
        lifted.volatility,
        lifted.time,
        lifted.running_extremum,
    ];
    let g = recorder.gradient(value, &wrt)?;
    // W'' by the divided differences over consecutive triples of nodes, averaged; V'' = W'' / extremum
    let (z, w) = (&tree.nodes_z, &tree.nodes_w);
    let second: Vec<f64> = (1..z.len() - 1)
        .map(|k| {
            let (h1, h2) = (z[k] - z[k - 1], z[k + 1] - z[k]);
            2.0 * ((w[k + 1] - w[k]) / h2 - (w[k] - w[k - 1]) / h1) / (h1 + h2)
        })
        .collect();
    let gamma = second.iter().sum::<f64>() / second.len() as f64 / extremum.primal();
    Ok(LookbackGreeks {
        value: value.primal(),
        delta: g[0].mean(),
        gamma,
        vega: g[4].mean(),
        rho: g[2].mean(),
        dividend_rho: g[3].mean(),
        theta: -g[5].mean(),
        strike_sensitivity: g[1].mean(),
        extremum_sensitivity: g[6].mean(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inputs(
        spot: f64,
        extremum: f64,
        strike: f64,
        rate: f64,
        dividend: f64,
        volatility: f64,
        time: f64,
    ) -> LookbackInputs {
        LookbackInputs {
            spot,
            running_extremum: extremum,
            strike,
            rate,
            dividend_yield: dividend,
            volatility,
            time,
        }
    }

    #[test]
    fn the_expansion_around_zero_drift_agrees_with_the_formula() {
        // both evaluations at a small drift: the expansion's error is of the order of b^2, the formula's rounding is about 1e-16 / b
        for (kind, spot, extremum) in [
            (OptionType::Call, 100.0, 85.0),
            (OptionType::Put, 100.0, 120.0),
            (OptionType::Call, 100.0, 100.0),
        ] {
            for b in [8e-5, -8e-5, 3e-5, 1e-6] {
                let (rate, dividend) = (0.03 + b, 0.03);
                let formula = continuous_floating(
                    kind,
                    spot,
                    extremum,
                    rate,
                    dividend,
                    0.3,
                    1.5,
                    Some(false),
                );
                let expansion =
                    continuous_floating(kind, spot, extremum, rate, dividend, 0.3, 1.5, Some(true));
                assert!(
                    (formula - expansion).abs() < 1e-10 * formula.abs(),
                    "{kind:?} {b}: {formula} {expansion}"
                );
            }
        }
    }

    #[test]
    fn the_price_is_continuous_through_zero_drift_and_finite_at_it() {
        for (strike, kind) in [
            (LookbackStrike::Floating, OptionType::Call),
            (LookbackStrike::Floating, OptionType::Put),
            (LookbackStrike::Fixed, OptionType::Call),
            (LookbackStrike::Fixed, OptionType::Put),
        ] {
            let extremum = if tracks_minimum(strike, kind) {
                90.0
            } else {
                110.0
            };
            let at = |dividend: f64| {
                compute_lookback_value(
                    strike,
                    kind,
                    &inputs(100.0, extremum, 100.0, 0.04, dividend, 0.25, 2.0),
                )
                .unwrap()
            };
            let zero = at(0.04);
            assert!(zero.is_finite() && zero > 0.0);
            assert!(
                (zero - at(0.04 - 1e-4)).abs() < 1e-3 * zero,
                "{strike:?} {kind:?}"
            );
            assert!(
                (zero - at(0.04 + 1e-4)).abs() < 1e-3 * zero,
                "{strike:?} {kind:?}"
            );
        }
    }

    #[test]
    fn a_fixed_strike_option_is_a_floating_one_plus_a_constant() {
        // with the extremum at the spot, a fixed call of strike K <= S equals the floating put with maximum S, plus S e^{-qT} - K e^{-rT}
        let i = inputs(100.0, 100.0, 90.0, 0.03, 0.01, 0.2, 1.0);
        let fixed = compute_lookback_value(LookbackStrike::Fixed, OptionType::Call, &i).unwrap();
        let floating =
            compute_lookback_value(LookbackStrike::Floating, OptionType::Put, &i).unwrap();
        let expected = floating + 100.0 * (-0.01f64).exp() - 90.0 * (-0.03f64).exp();
        assert!(
            (fixed - expected).abs() < 1e-12 * fixed,
            "{fixed} {expected}"
        );
    }

    #[test]
    fn invalid_inputs_are_errors() {
        let ok = inputs(100.0, 90.0, 100.0, 0.03, 0.01, 0.2, 1.0);
        let v = |i: &LookbackInputs| {
            compute_lookback_value(LookbackStrike::Floating, OptionType::Call, i)
        };
        assert!(v(&ok).is_ok());
        assert!(v(&LookbackInputs {
            running_extremum: 110.0,
            ..ok
        })
        .is_err()); // minimum above the spot
        assert!(compute_lookback_value(LookbackStrike::Floating, OptionType::Put, &ok).is_err()); // maximum below the spot
        assert!(v(&LookbackInputs {
            volatility: 0.0,
            ..ok
        })
        .is_err());
        assert!(v(&LookbackInputs { time: -1.0, ..ok }).is_err());
        assert!(compute_lookback_value(
            LookbackStrike::Fixed,
            OptionType::Call,
            &LookbackInputs {
                strike: 0.0,
                running_extremum: 110.0,
                ..ok
            }
        )
        .is_err());
        let d = |m: &[f64], steps| {
            compute_discrete_lookback_value(
                LookbackStrike::Floating,
                OptionType::Call,
                &ok,
                m,
                steps,
            )
        };
        assert!(d(&[0.5, 1.0], 50).is_ok());
        assert!(d(&[0.5, 1.0], 1).is_err());
        assert!(d(&[1.0, 0.5], 50).is_err());
        assert!(d(&[0.5, 1.5], 50).is_err());
        assert!(d(&[0.0, 1.0], 50).is_err());
    }

    #[test]
    fn the_tree_without_monitoring_is_a_vanilla_option_at_the_extremum() {
        // no monitoring date: the extremum stays, and the floating call is a call with strike m
        let i = inputs(100.0, 90.0, 100.0, 0.03, 0.01, 0.25, 1.0);
        let tree = compute_discrete_lookback_value(
            LookbackStrike::Floating,
            OptionType::Call,
            &i,
            &[],
            400,
        )
        .unwrap();
        let vanilla = crate::options::compute_european_value(
            OptionType::Call,
            &crate::options::OptionInputs {
                spot: 100.0,
                strike: 90.0,
                rate: 0.03,
                dividend_yield: 0.01,
                volatility: 0.25,
                time: 1.0,
            },
        )
        .unwrap();
        assert!((tree - vanilla).abs() < 2e-2, "{tree} {vanilla}");
    }

    #[test]
    fn homogeneity_in_the_spot_and_the_extremum() {
        let base = inputs(100.0, 90.0, 100.0, 0.03, 0.01, 0.25, 1.0);
        let scaled = LookbackInputs {
            spot: 250.0,
            running_extremum: 225.0,
            strike: 250.0,
            ..base
        };
        let monitoring = [0.25, 0.5, 0.75, 1.0];
        for (strike, kind, extremum) in [
            (LookbackStrike::Floating, OptionType::Call, 90.0),
            (LookbackStrike::Floating, OptionType::Put, 112.0),
            (LookbackStrike::Fixed, OptionType::Call, 112.0),
            (LookbackStrike::Fixed, OptionType::Put, 90.0),
        ] {
            let a = compute_discrete_lookback_value(
                strike,
                kind,
                &LookbackInputs {
                    running_extremum: extremum,
                    ..base
                },
                &monitoring,
                200,
            )
            .unwrap();
            let b = compute_discrete_lookback_value(
                strike,
                kind,
                &LookbackInputs {
                    running_extremum: extremum * 2.5,
                    ..scaled
                },
                &monitoring,
                200,
            )
            .unwrap();
            assert!(
                (b - 2.5 * a).abs() < 1e-10 * b,
                "{strike:?} {kind:?}: {a} {b}"
            );
        }
    }

    /// Floating-strike call or put, the extremum at the spot, monitored every `every` steps of a tree of `steps` steps.
    fn tree_price(kind: OptionType, steps: usize, every: usize) -> f64 {
        let i = inputs(100.0, 100.0, 100.0, 0.03, 0.01, 0.25, 1.0);
        let monitoring: Vec<f64> = (1..=steps / every)
            .map(|k| (k * every) as f64 / steps as f64)
            .collect();
        compute_discrete_lookback_value(LookbackStrike::Floating, kind, &i, &monitoring, steps)
            .unwrap()
    }

    #[test]
    fn monitoring_at_every_step_converges_to_continuous_monitoring() {
        // the error of a lookback tree falls like 1 / sqrt(steps): four times the steps halve it
        for kind in [OptionType::Call, OptionType::Put] {
            let continuous = compute_lookback_value(
                LookbackStrike::Floating,
                kind,
                &inputs(100.0, 100.0, 100.0, 0.03, 0.01, 0.25, 1.0),
            )
            .unwrap();
            let tree: Vec<f64> = [400, 1600, 6400]
                .iter()
                .map(|&n| tree_price(kind, n, 1))
                .collect();
            let error: Vec<f64> = tree.iter().map(|t| continuous - t).collect();
            assert!(
                error[0] > error[1] && error[1] > error[2] && error[2] > 0.0,
                "{kind:?} {error:?}"
            );
            for k in 0..2 {
                let ratio = error[k + 1] / error[k];
                assert!((ratio - 0.5).abs() < 0.03, "{kind:?} ratio {ratio}");
            }
            // the extrapolation in 1 / sqrt(steps)
            let extrapolated = 2.0 * tree[2] - tree[1];
            assert!(
                (extrapolated - continuous).abs() < 1e-3 * continuous,
                "{kind:?} {extrapolated} {continuous}"
            );
        }
    }

    /// Monte Carlo of the floating-strike option monitored at `dates` equally spaced times, by exact lognormal steps
    /// and antithetic pairs: the value and its standard error.
    fn monte_carlo(kind: OptionType, dates: usize, pairs: u64) -> (f64, f64) {
        let (spot, rate, dividend, volatility, time) =
            (100.0f64, 0.03f64, 0.01f64, 0.25f64, 1.0f64);
        let dt = time / dates as f64;
        let (drift, sd) = (
            (rate - dividend - 0.5 * volatility * volatility) * dt,
            volatility * dt.sqrt(),
        );
        let payoff = |sign: f64, p: u64| {
            let mut s = spot;
            let mut extremum = spot;
            for k in 0..dates {
                s *= (drift
                    + sd * sign
                        * crate::math::random::generate_standard_normal(20261122, p, k as u64))
                .exp();
                extremum = if kind == OptionType::Call {
                    extremum.min(s)
                } else {
                    extremum.max(s)
                };
            }
            match kind {
                OptionType::Call => s - extremum,
                OptionType::Put => extremum - s,
            }
        };
        let discount = (-rate * time).exp();
        let values: Vec<f64> = (0..pairs)
            .map(|p| 0.5 * discount * (payoff(1.0, p) + payoff(-1.0, p)))
            .collect();
        let mean = values.iter().sum::<f64>() / pairs as f64;
        let variance =
            values.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / (pairs as f64 - 1.0);
        (mean, (variance / pairs as f64).sqrt())
    }

    #[test]
    fn monthly_monitoring_agrees_with_an_independent_monte_carlo() {
        for kind in [OptionType::Call, OptionType::Put] {
            let (mc, standard_error) = monte_carlo(kind, 12, 200_000);
            // trees with the same 12 monitoring dates, 100 and 200 steps per month; extrapolated in 1 / sqrt(steps)
            let (coarse, fine) = (tree_price(kind, 1200, 100), tree_price(kind, 2400, 200));
            let extrapolated =
                (std::f64::consts::SQRT_2 * fine - coarse) / (std::f64::consts::SQRT_2 - 1.0);
            assert!(
                (extrapolated - mc).abs() < 4.0 * standard_error + 2e-3 * mc,
                "{kind:?}: tree {extrapolated} (fine {fine}), Monte Carlo {mc} +- {standard_error}"
            );
        }
    }

    #[test]
    fn tree_greeks_agree_with_differences_and_with_the_continuous_gamma() {
        // a seasoned call: the minimum so far is below the spot
        let i = inputs(100.0, 90.0, 100.0, 0.03, 0.01, 0.25, 1.0);
        let monitoring: Vec<f64> = (1..=400).map(|k| k as f64 / 400.0).collect();
        let g = compute_discrete_lookback_greeks(
            LookbackStrike::Floating,
            OptionType::Call,
            &i,
            &monitoring,
            400,
        )
        .unwrap();
        let value = |j: LookbackInputs| {
            compute_discrete_lookback_value(
                LookbackStrike::Floating,
                OptionType::Call,
                &j,
                &monitoring,
                400,
            )
            .unwrap()
        };
        assert!((g.value - value(i)).abs() < 1e-12);
        let h = 1e-6 * i.spot;
        let delta = (value(LookbackInputs {
            spot: i.spot + h,
            ..i
        }) - value(LookbackInputs {
            spot: i.spot - h,
            ..i
        })) / (2.0 * h);
        assert!((g.delta - delta).abs() < 1e-5, "delta {} {delta}", g.delta);
        let hv = 1e-6;
        let vega = (value(LookbackInputs {
            volatility: i.volatility + hv,
            ..i
        }) - value(LookbackInputs {
            volatility: i.volatility - hv,
            ..i
        })) / (2.0 * hv);
        assert!((g.vega - vega).abs() < 1e-4, "vega {} {vega}", g.vega);
        let continuous =
            compute_lookback_greeks(LookbackStrike::Floating, OptionType::Call, &i).unwrap();
        assert!(
            g.gamma > 0.0 && (g.gamma - continuous.gamma).abs() < 0.3 * continuous.gamma,
            "gamma {} {}",
            g.gamma,
            continuous.gamma
        );
        assert!(
            (g.delta - continuous.delta).abs() < 0.02,
            "delta {} {}",
            g.delta,
            continuous.delta
        );
    }
}
