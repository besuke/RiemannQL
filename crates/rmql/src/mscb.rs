//! Convertible bonds with a conversion price that is reset on dates and carried to the next (MSCB, P16 stage 4,
//! `docs/spec_lookback_convertible.md`). The conversion price `K` is a state: between reset dates it is fixed, and on a
//! reset date the share price `S` of the same time sets the new one, before any conversion of that date:
//!
//! | rule | `K+` |
//! |---|---|
//! | [`ResetRule::Both`] | `min(k_max, max(k_min, alpha S))`, whatever the price before |
//! | [`ResetRule::DownwardOnly`] | `min(K-, max(k_min, alpha S))` |
//!
//! Converting gives `face / K` shares, worth `face S / K`. Calls, puts and coupons are those of the convertible bonds of
//! [`crate::convertible`], which this module reuses lattice step by step (`Engine`): at a fixed `K`, the lattice in the
//! share price is the one of a convertible bond with the conversion ratio `face / K`.
//!
//! Two methods:
//!
//! - [`MscbMethod::Grid`]: a grid of conversion prices, one lattice in `S` for each, joined on the reset dates: at each node
//!   of `S` the values (the value, the probability of conversion and the discount rate) are interpolated along the
//!   conversion prices, linearly in `1 / K`, at the new price `K+(S, K-)`. The conversion prices run from the lowest to the
//!   highest that a reset can give on the lattice, with the initial price as a node.
//! - [`MscbMethod::Scaled`]: with no limits (`k_min = 0`, and `k_max` infinite for `Both`), no level of the share price or of
//!   the conversion price enters, and the value depends on `z = S / K` only. A reset is a map of `z`: `z+ = max(z, 1 / alpha)`
//!   for `DownwardOnly`, `z+ = 1 / alpha` for `Both`. This is a lattice in one dimension, the lattice of a convertible bond
//!   with the share price `S0 / K0` and the conversion ratio `face`, with the maps on the reset dates (interpolating linearly
//!   in `z` at `1 / alpha`).

// `!(x > 0.0)` is intended: it also rejects NaN
#![allow(clippy::neg_cmp_op_on_partial_ord)]

use crate::autodiff::{Operand, Recorder};
use crate::convertible::{
    Callability, ConversionExercise, ConversionTerms, ConvertibleGreeks, ConvertibleInputs,
    ConvertibleTerms, Coupon, Engine, State,
};
use crate::Error;

/// How the conversion price is set on a reset date.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ResetRule {
    /// `K+ = min(k_max, max(k_min, alpha S))`; `k_max` may be infinite.
    Both { alpha: f64, k_min: f64, k_max: f64 },
    /// `K+ = min(K-, max(k_min, alpha S))`.
    DownwardOnly { alpha: f64, k_min: f64 },
}

/// A convertible bond with a conversion price carried between reset dates; times in years from the settlement date.
#[derive(Debug, Clone, PartialEq)]
pub struct MscbTerms {
    pub redemption: f64,
    /// The face value, to which the conversion price applies: `face / K` shares are received.
    pub face: f64,
    /// The conversion price before the first reset date.
    pub initial_price: f64,
    pub exercise: ConversionExercise,
    pub coupons: Vec<Coupon>,
    pub callabilities: Vec<Callability>,
    /// Reset dates, increasing, after the settlement date and not after the last exercise time.
    pub reset_times: Vec<f64>,
    pub rule: ResetRule,
}

/// The method of the lattice (see the module).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MscbMethod {
    /// `price_nodes` conversion prices, at least 2 where a reset can change the price.
    Grid {
        price_nodes: usize,
    },
    Scaled,
}

fn invalid<T>(reason: String) -> Result<T, Error> {
    Err(Error::InvalidInput { reason })
}

impl MscbTerms {
    /// The terms as a convertible bond, for the events; its conversion is a placeholder, given to each step instead.
    fn schedule(&self) -> ConvertibleTerms {
        ConvertibleTerms {
            redemption: self.redemption,
            conversion: ConversionTerms::Fixed { ratio: 1.0 },
            exercise: self.exercise.clone(),
            coupons: self.coupons.clone(),
            callabilities: self.callabilities.clone(),
        }
    }

    fn check(&self) -> Result<(), Error> {
        for (name, x) in [("face", self.face), ("initial_price", self.initial_price)] {
            if !(x > 0.0) || !x.is_finite() {
                return invalid(format!("the {name} must be positive and finite, not {x}"));
            }
        }
        let (alpha, k_min) = match self.rule {
            ResetRule::Both {
                alpha,
                k_min,
                k_max,
            } => {
                if !(k_max > 0.0) || !(k_max >= k_min) {
                    return invalid(format!(
                        "the upper conversion price {k_max} must be positive and at least the lower one {k_min}"
                    ));
                }
                (alpha, k_min)
            }
            ResetRule::DownwardOnly { alpha, k_min } => (alpha, k_min),
        };
        if !(alpha > 0.0) || !alpha.is_finite() || !(k_min >= 0.0) || !k_min.is_finite() {
            return invalid(format!("the price ratio must be positive and the lower conversion price non-negative and finite, not {alpha}, {k_min}"));
        }
        let end = self.exercise.last_time();
        if self
            .reset_times
            .iter()
            .any(|t| !t.is_finite() || !(*t > 0.0) || *t > end * (1.0 + 1e-12))
            || self.reset_times.windows(2).any(|w| !(w[0] < w[1]))
        {
            return invalid(format!(
                "the reset times must increase and lie in (0, {end}]"
            ));
        }
        Ok(())
    }

    fn unbounded(&self) -> bool {
        match self.rule {
            ResetRule::Both { k_min, k_max, .. } => k_min == 0.0 && k_max == f64::INFINITY,
            ResetRule::DownwardOnly { k_min, .. } => k_min == 0.0,
        }
    }
}

/// The new conversion price at a node with share price `spot`, the price before being `previous`.
fn reset_price<T: Operand>(rule: ResetRule, spot: T, previous: f64) -> T {
    match rule {
        ResetRule::Both {
            alpha,
            k_min,
            k_max,
        } => (spot * alpha).max(spot.like(k_min)).min(spot.like(k_max)),
        ResetRule::DownwardOnly { alpha, k_min } => (spot * alpha)
            .max(spot.like(k_min))
            .min(spot.like(previous)),
    }
}

fn lerp<T: Operand>(a: T, b: T, w: T) -> T {
    a + (b - a) * w
}

struct Mscb<T> {
    value: T,
    at_step_1: [T; 2],
    at_step_2: [T; 3],
    spot_step_1: [f64; 2],
    spot_step_2: [f64; 3],
}

/// The steps of the reset dates; each must be a step after 0, and no two the same.
fn reset_steps<T: Operand>(terms: &MscbTerms, engine: &Engine<T>) -> Result<Vec<usize>, Error> {
    let steps: Vec<usize> = terms
        .reset_times
        .iter()
        .map(|&t| engine.step_of(t))
        .collect();
    if steps.contains(&0) || steps.windows(2).any(|w| w[0] >= w[1]) {
        return invalid(
            "each reset date needs its own step of the lattice, after the first: use more steps"
                .into(),
        );
    }
    Ok(steps)
}

/// How many standard deviations of the share price at the last reset the grid of conversion prices covers on each side. A reset
/// at a share price beyond them has a conversion price held at the end of the grid: the probability is below 1e-8.
const GRID_STANDARD_DEVIATIONS: f64 = 6.0;

/// The conversion prices of the grid, increasing, with the initial price as a node: from the lowest to the highest price a
/// reset can give at a share price within `GRID_STANDARD_DEVIATIONS` of its distribution at the last reset step (and not
/// beyond the lattice).
fn price_grid<T: Operand>(
    terms: &MscbTerms,
    engine: &Engine<T>,
    resets: &[usize],
    price_nodes: usize,
) -> Result<Vec<f64>, Error> {
    let k0 = terms.initial_price;
    let Some(&last) = resets.last() else {
        return Ok(vec![k0]);
    };
    let spot = engine.spot.primal();
    let width =
        engine.dx.primal() * (last as f64).min(GRID_STANDARD_DEVIATIONS * (last as f64).sqrt());
    let move_ = width.exp();
    let (s_low, s_high) = (spot / move_, spot * move_);
    let (low, high) = match terms.rule {
        ResetRule::Both {
            alpha,
            k_min,
            k_max,
        } => (
            (alpha * s_low).max(k_min).min(k_max),
            (alpha * s_high).max(k_min).min(k_max),
        ),
        ResetRule::DownwardOnly { alpha, k_min } => ((alpha * s_low).max(k_min).min(k0), k0),
    };
    let (low, high) = (low.min(k0), high.max(k0));
    if !(high > low * (1.0 + 1e-12)) {
        return Ok(vec![k0]);
    }
    if price_nodes < 2 {
        return invalid(format!(
            "at least 2 conversion prices required, {price_nodes} provided"
        ));
    }
    let delta = (high / low).ln() / (price_nodes - 1) as f64;
    let down = ((k0 / low).ln() / delta - 1e-9).ceil().max(0.0) as usize;
    let up = ((high / k0).ln() / delta - 1e-9).ceil().max(0.0) as usize;
    let mut prices: Vec<f64> = (1..=down)
        .rev()
        .map(|m| k0 * (-delta * m as f64).exp())
        .collect();
    prices.push(k0);
    prices.extend((1..=up).map(|m| k0 * (delta * m as f64).exp()));
    Ok(prices)
}

/// The states before a reset, from the states after it, one for each conversion price.
fn grid_reset<T: Operand>(
    rule: ResetRule,
    engine: &Engine<T>,
    i: usize,
    prices: &[f64],
    states: &[State<T>],
) -> Vec<State<T>> {
    let m = prices.len();
    if m == 1 {
        return states.to_vec();
    }
    let x: Vec<f64> = prices.iter().map(|k| 1.0 / k).collect();
    let spots: Vec<T> = (0..=i).map(|j| engine.node_spot(i, j)).collect();
    let (zero, one) = (spots[0].like(0.0), spots[0].like(1.0));
    (0..m)
        .map(|l| {
            let (mut values, mut probability, mut rates) = (
                Vec::with_capacity(i + 1),
                Vec::with_capacity(i + 1),
                Vec::with_capacity(i + 1),
            );
            for (j, &s) in spots.iter().enumerate() {
                let new_price = reset_price(rule, s, prices[l]);
                let lower = prices
                    .partition_point(|&k| k <= new_price.primal())
                    .saturating_sub(1)
                    .min(m - 2);
                // linear in 1 / K between the two prices around the new one; beyond the grid, the end price
                let w = ((-new_price.recip() + x[lower]) / (x[lower] - x[lower + 1]))
                    .max(zero)
                    .min(one);
                let (a, b) = (&states[lower], &states[lower + 1]);
                values.push(lerp(a.values[j], b.values[j], w));
                probability.push(lerp(a.probability[j], b.probability[j], w));
                rates.push(lerp(a.rates[j], b.rates[j], w));
            }
            State {
                values,
                probability,
                rates,
            }
        })
        .collect()
}

fn grid_lattice<T: Operand>(
    terms: &MscbTerms,
    inputs: &ConvertibleInputs<T>,
    steps: usize,
    price_nodes: usize,
) -> Result<Mscb<T>, Error> {
    let schedule = terms.schedule();
    let engine = Engine::new(&schedule, inputs.spot, inputs, steps, false)?;
    let resets = reset_steps(terms, &engine)?;
    let prices = price_grid(terms, &engine, &resets, price_nodes)?;
    let initial = prices
        .iter()
        .position(|&k| k == terms.initial_price)
        .expect("the initial price is a node");
    let conversions: Vec<ConversionTerms> = prices
        .iter()
        .map(|&k| ConversionTerms::Fixed {
            ratio: terms.face / k,
        })
        .collect();
    let mut states: Vec<State<T>> = conversions.iter().map(|c| engine.terminal(c)).collect();
    if resets.contains(&steps) {
        states = grid_reset(terms.rule, &engine, steps, &prices, &states);
    }
    let zero = inputs.spot.like(0.0);
    let (mut at_step_1, mut at_step_2) = ([zero; 2], [zero; 3]);
    for i in (0..steps).rev() {
        states = states
            .iter()
            .zip(&conversions)
            .map(|(state, c)| engine.step_back(i, state, c))
            .collect();
        if resets.contains(&i) {
            states = grid_reset(terms.rule, &engine, i, &prices, &states);
        }
        let v = &states[initial].values;
        if i == 2 {
            at_step_2 = [v[0], v[1], v[2]];
        } else if i == 1 {
            at_step_1 = [v[0], v[1]];
        }
    }
    let at = |i: usize, j: usize| engine.node_spot(i, j).primal();
    Ok(Mscb {
        value: states[initial].values[0],
        at_step_1,
        at_step_2,
        spot_step_1: [at(1, 0), at(1, 1)],
        spot_step_2: [at(2, 0), at(2, 1), at(2, 2)],
    })
}

/// The states before a reset in `z = S / K`, from the states after it.
fn scaled_reset<T: Operand>(
    rule: ResetRule,
    engine: &Engine<T>,
    i: usize,
    state: &State<T>,
) -> State<T> {
    let alpha = match rule {
        ResetRule::Both { alpha, .. } | ResetRule::DownwardOnly { alpha, .. } => alpha,
    };
    let target = 1.0 / alpha;
    let z: Vec<T> = (0..=i).map(|j| engine.node_spot(i, j)).collect();
    // the nodes are increasing in j: the segment around the target (the end segment beyond the nodes: a linear extrapolation)
    let upper = z.partition_point(|n| n.primal() <= target).clamp(1, i);
    let w = (-z[upper - 1] + target) / (z[upper] - z[upper - 1]);
    let at = |a: &[T]| lerp(a[upper - 1], a[upper], w);
    let (value, probability, rate) = (at(&state.values), at(&state.probability), at(&state.rates));
    let moved = |j: usize| match rule {
        ResetRule::Both { .. } => true,
        ResetRule::DownwardOnly { .. } => z[j].primal() < target,
    };
    let pick = |a: &[T], interpolated: T| -> Vec<T> {
        (0..=i)
            .map(|j| if moved(j) { interpolated } else { a[j] })
            .collect()
    };
    State {
        values: pick(&state.values, value),
        probability: pick(&state.probability, probability),
        rates: pick(&state.rates, rate),
    }
}

fn scaled_lattice<T: Operand>(
    terms: &MscbTerms,
    inputs: &ConvertibleInputs<T>,
    steps: usize,
) -> Result<Mscb<T>, Error> {
    if !terms.unbounded() {
        return invalid("the scaled lattice needs no limits on the conversion price: k_min = 0, and k_max infinite for the rule Both".into());
    }
    let schedule = terms.schedule();
    let k0 = terms.initial_price;
    let engine = Engine::new(&schedule, inputs.spot / k0, inputs, steps, false)?;
    let resets = reset_steps(terms, &engine)?;
    let conversion = ConversionTerms::Fixed { ratio: terms.face };
    let mut state = engine.terminal(&conversion);
    if resets.contains(&steps) {
        state = scaled_reset(terms.rule, &engine, steps, &state);
    }
    let zero = inputs.spot.like(0.0);
    let (mut at_step_1, mut at_step_2) = ([zero; 2], [zero; 3]);
    for i in (0..steps).rev() {
        state = engine.step_back(i, &state, &conversion);
        if resets.contains(&i) {
            state = scaled_reset(terms.rule, &engine, i, &state);
        }
        if i == 2 {
            at_step_2 = [state.values[0], state.values[1], state.values[2]];
        } else if i == 1 {
            at_step_1 = [state.values[0], state.values[1]];
        }
    }
    // the nodes are in z; the share price is K0 z
    let at = |i: usize, j: usize| engine.node_spot(i, j).primal() * k0;
    Ok(Mscb {
        value: state.values[0],
        at_step_1,
        at_step_2,
        spot_step_1: [at(1, 0), at(1, 1)],
        spot_step_2: [at(2, 0), at(2, 1), at(2, 2)],
    })
}

fn build<T: Operand>(
    terms: &MscbTerms,
    inputs: &ConvertibleInputs<T>,
    steps: usize,
    method: MscbMethod,
) -> Result<Mscb<T>, Error> {
    terms.check()?;
    match method {
        MscbMethod::Grid { price_nodes } => grid_lattice(terms, inputs, steps, price_nodes),
        MscbMethod::Scaled => scaled_lattice(terms, inputs, steps),
    }
}

/// The value of a convertible bond with a carried conversion price, on a lattice of `steps` steps.
pub fn compute_mscb_value<T: Operand>(
    terms: &MscbTerms,
    inputs: &ConvertibleInputs<T>,
    steps: usize,
    method: MscbMethod,
) -> Result<T, Error> {
    Ok(build(terms, inputs, steps, method)?.value)
}

/// The value and the Greeks, as for [`crate::convertible::compute_convertible_greeks`]. The grid of conversion prices is
/// held fixed when the share price moves; the sensitivities are the derivatives of the lattice by automatic differentiation.
pub fn compute_mscb_greeks(
    terms: &MscbTerms,
    inputs: &ConvertibleInputs,
    steps: usize,
    method: MscbMethod,
) -> Result<ConvertibleGreeks, Error> {
    let recorder = Recorder::new();
    let lifted = ConvertibleInputs {
        spot: recorder.input(inputs.spot),
        rate: recorder.input(inputs.rate),
        dividend_yield: recorder.input(inputs.dividend_yield),
        volatility: recorder.input(inputs.volatility),
        credit_spread: recorder.input(inputs.credit_spread),
    };
    let lattice = build(terms, &lifted, steps, method)?;
    let g = recorder.gradient(
        lattice.value,
        &[
            lifted.rate,
            lifted.dividend_yield,
            lifted.volatility,
            lifted.credit_spread,
        ],
    )?;
    let p2 = lattice.at_step_2.map(|v| v.primal());
    let s2 = lattice.spot_step_2;
    let delta_up = (p2[2] - p2[1]) / (s2[2] - s2[1]);
    let delta_down = (p2[1] - p2[0]) / (s2[1] - s2[0]);
    let p1 = lattice.at_step_1.map(|v| v.primal());
    let s1 = lattice.spot_step_1;
    Ok(ConvertibleGreeks {
        value: lattice.value.primal(),
        delta: (p1[1] - p1[0]) / (s1[1] - s1[0]),
        gamma: (delta_up - delta_down) / ((s2[2] - s2[0]) / 2.0),
        vega: g[2].mean(),
        rho: g[0].mean(),
        dividend_rho: g[1].mean(),
        credit_spread_sensitivity: g[3].mean(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::convertible::{compute_convertible_value, CallabilityKind};
    use crate::options::{compute_european_value, OptionInputs, OptionType};

    fn inputs(spot: f64, spread: f64) -> ConvertibleInputs {
        ConvertibleInputs {
            spot,
            rate: 0.03,
            dividend_yield: 0.01,
            volatility: 0.25,
            credit_spread: spread,
        }
    }

    fn coupons(years: u32, amount: f64) -> Vec<Coupon> {
        (1..=years)
            .map(|k| Coupon {
                time: k as f64,
                amount,
            })
            .collect()
    }

    fn terms(rule: ResetRule, reset_times: Vec<f64>, exercise: ConversionExercise) -> MscbTerms {
        MscbTerms {
            redemption: 100.0,
            face: 100.0,
            initial_price: 100.0,
            exercise,
            coupons: coupons(5, 2.0),
            callabilities: vec![],
            reset_times,
            rule,
        }
    }

    fn fixed_equivalent(t: &MscbTerms) -> ConvertibleTerms {
        ConvertibleTerms {
            redemption: t.redemption,
            conversion: ConversionTerms::Fixed {
                ratio: t.face / t.initial_price,
            },
            exercise: t.exercise.clone(),
            coupons: t.coupons.clone(),
            callabilities: t.callabilities.clone(),
        }
    }

    const GRID: MscbMethod = MscbMethod::Grid { price_nodes: 40 };

    #[test]
    fn without_resets_it_is_a_fixed_conversion_ratio() {
        let american = ConversionExercise::American {
            start: 0.0,
            end: 5.0,
        };
        let mut t = terms(
            ResetRule::DownwardOnly {
                alpha: 0.9,
                k_min: 0.0,
            },
            vec![],
            american,
        );
        t.callabilities = vec![
            Callability {
                time: 3.0,
                kind: CallabilityKind::Call,
                price: 106.0,
                trigger: Some(1.2),
            },
            Callability {
                time: 2.0,
                kind: CallabilityKind::Put,
                price: 100.0,
                trigger: None,
            },
        ];
        let i = inputs(92.0, 0.02);
        let fixed = compute_convertible_value(&fixed_equivalent(&t), &i, 200).unwrap();
        for method in [GRID, MscbMethod::Scaled] {
            let v = compute_mscb_value(&t, &i, 200, method).unwrap();
            assert!((v - fixed).abs() < 1e-10 * fixed, "{method:?}: {v} {fixed}");
        }
    }

    #[test]
    fn a_reset_that_never_changes_the_price_is_a_fixed_conversion_ratio() {
        // DownwardOnly with a huge price ratio: min(K, alpha S) = K; Both with the limits at the initial price
        let american = ConversionExercise::American {
            start: 0.0,
            end: 5.0,
        };
        let resets = vec![1.0, 2.0, 3.5];
        let i = inputs(92.0, 0.02);
        let never = terms(
            ResetRule::DownwardOnly {
                alpha: 1e6,
                k_min: 0.0,
            },
            resets.clone(),
            american.clone(),
        );
        let fixed = compute_convertible_value(&fixed_equivalent(&never), &i, 250).unwrap();
        for method in [GRID, MscbMethod::Scaled] {
            let v = compute_mscb_value(&never, &i, 250, method).unwrap();
            assert!(
                (v - fixed).abs() < 1e-10 * fixed,
                "downward {method:?}: {v} {fixed}"
            );
        }
        let pinned = terms(
            ResetRule::Both {
                alpha: 0.7,
                k_min: 100.0,
                k_max: 100.0,
            },
            resets,
            american,
        );
        let v = compute_mscb_value(&pinned, &i, 250, GRID).unwrap();
        assert!((v - fixed).abs() < 1e-10 * fixed, "pinned: {v} {fixed}");
    }

    /// The value of a European conversion at `time`, no coupon, no spread, no calls, with one reset at `tau`: the closed form
    /// of the rule Both, and an integral over the share price at `tau` for the rule DownwardOnly.
    fn european_reference(
        rule: ResetRule,
        initial_price: f64,
        spot: f64,
        tau: f64,
        time: f64,
    ) -> f64 {
        let (rate, dividend, vol, face) = (0.03f64, 0.01f64, 0.25f64, 100.0f64);
        let call = |s: f64, k: f64, t: f64| {
            compute_european_value(
                OptionType::Call,
                &OptionInputs {
                    spot: s,
                    strike: k,
                    rate,
                    dividend_yield: dividend,
                    volatility: vol,
                    time: t,
                },
            )
            .unwrap()
        };
        let base = face * (-rate * time).exp();
        match rule {
            ResetRule::Both { alpha, .. } => {
                base + (-rate * tau).exp() * (face / alpha) * call(1.0, alpha, time - tau)
            }
            ResetRule::DownwardOnly { alpha, .. } => {
                // E over S_tau of (face / K+) C(S_tau, K+), K+ = min(K0, alpha S_tau), by Simpson's rule on the standard normal
                let n = 4000;
                let (lo, hi) = (-9.0f64, 9.0f64);
                let h = (hi - lo) / n as f64;
                let f = |z: f64| {
                    let s = spot
                        * ((rate - dividend - 0.5 * vol * vol) * tau + vol * tau.sqrt() * z).exp();
                    let k = initial_price.min(alpha * s);
                    (face / k) * call(s, k, time - tau) * (-0.5 * z * z).exp()
                        / (2.0 * std::f64::consts::PI).sqrt()
                };
                let mut sum = f(lo) + f(hi);
                for k in 1..n {
                    sum += f(lo + k as f64 * h) * if k % 2 == 1 { 4.0 } else { 2.0 };
                }
                base + (-rate * tau).exp() * sum * h / 3.0
            }
        }
    }

    #[test]
    fn a_european_reset_agrees_with_a_closed_form_or_an_integral() {
        let european = ConversionExercise::European(2.0);
        for rule in [
            ResetRule::Both {
                alpha: 0.9,
                k_min: 0.0,
                k_max: f64::INFINITY,
            },
            ResetRule::DownwardOnly {
                alpha: 0.9,
                k_min: 0.0,
            },
            ResetRule::DownwardOnly {
                alpha: 0.7,
                k_min: 0.0,
            },
        ] {
            let mut t = terms(rule, vec![1.0], european.clone());
            t.coupons = vec![];
            let expected = european_reference(rule, 100.0, 100.0, 1.0, 2.0);
            let scaled =
                compute_mscb_value(&t, &inputs(100.0, 0.0), 2000, MscbMethod::Scaled).unwrap();
            assert!(
                (scaled - expected).abs() < 3e-3 * expected,
                "{rule:?} scaled: {scaled} {expected}"
            );
            let grid = compute_mscb_value(
                &t,
                &inputs(100.0, 0.0),
                400,
                MscbMethod::Grid { price_nodes: 120 },
            )
            .unwrap();
            assert!(
                (grid - expected).abs() < 1e-2 * expected,
                "{rule:?} grid: {grid} {expected}"
            );
        }
    }

    #[test]
    fn the_grid_and_the_scaled_lattice_agree_where_there_are_no_limits() {
        let american = ConversionExercise::American {
            start: 0.0,
            end: 5.0,
        };
        let mut t = terms(
            ResetRule::DownwardOnly {
                alpha: 0.9,
                k_min: 0.0,
            },
            vec![1.0, 2.0, 3.0, 4.0],
            american,
        );
        t.callabilities = vec![Callability {
            time: 3.5,
            kind: CallabilityKind::Call,
            price: 108.0,
            trigger: None,
        }];
        let i = inputs(85.0, 0.02);
        let scaled = compute_mscb_value(&t, &i, 300, MscbMethod::Scaled).unwrap();
        let mut last = f64::INFINITY;
        for nodes in [20, 40, 80, 160] {
            let grid =
                compute_mscb_value(&t, &i, 300, MscbMethod::Grid { price_nodes: nodes }).unwrap();
            let difference = (grid - scaled).abs() / scaled;
            println!(
                "PROBE nodes {nodes}: grid {grid:.6} scaled {scaled:.6} relative {difference:.2e}"
            );
            last = difference;
        }
        assert!(last < 5e-3, "{last}");
    }

    #[test]
    fn invalid_inputs_are_errors() {
        let european = ConversionExercise::European(2.0);
        let i = inputs(100.0, 0.02);
        let both = ResetRule::Both {
            alpha: 0.9,
            k_min: 0.0,
            k_max: f64::INFINITY,
        };
        let good = terms(both, vec![1.0], european.clone());
        assert!(compute_mscb_value(&good, &i, 100, GRID).is_ok());
        assert!(compute_mscb_value(&good, &i, 100, MscbMethod::Scaled).is_ok());
        let value = |t: &MscbTerms, method| compute_mscb_value(t, &i, 100, method);
        assert!(value(&terms(both, vec![0.0], european.clone()), GRID).is_err()); // not after the settlement date
        assert!(value(&terms(both, vec![2.5], european.clone()), GRID).is_err()); // after the last exercise time
        assert!(value(&terms(both, vec![1.5, 1.0], european.clone()), GRID).is_err()); // decreasing
        assert!(value(&terms(both, vec![1.0, 1.0001], european.clone()), GRID).is_err()); // the same step
        assert!(value(
            &MscbTerms {
                initial_price: 0.0,
                ..good.clone()
            },
            GRID
        )
        .is_err());
        assert!(value(
            &MscbTerms {
                face: -1.0,
                ..good.clone()
            },
            GRID
        )
        .is_err());
        assert!(value(
            &terms(
                ResetRule::Both {
                    alpha: 0.0,
                    k_min: 0.0,
                    k_max: f64::INFINITY
                },
                vec![1.0],
                european.clone()
            ),
            GRID
        )
        .is_err());
        assert!(value(
            &terms(
                ResetRule::Both {
                    alpha: 0.9,
                    k_min: 80.0,
                    k_max: 70.0
                },
                vec![1.0],
                european.clone()
            ),
            GRID
        )
        .is_err());
        for k_max in [0.0, -1.0, f64::NAN] {
            let rule = ResetRule::Both {
                alpha: 0.9,
                k_min: 0.0,
                k_max,
            };
            let invalid = terms(rule, vec![1.0], european.clone());
            let error = value(&invalid, GRID).unwrap_err().to_string();
            assert!(error.contains("upper conversion price"), "{error}");
        }
        assert!(value(&good, MscbMethod::Grid { price_nodes: 1 }).is_err());
        // the scaled lattice has no limits to give
        assert!(value(
            &terms(
                ResetRule::Both {
                    alpha: 0.9,
                    k_min: 80.0,
                    k_max: f64::INFINITY
                },
                vec![1.0],
                european.clone()
            ),
            MscbMethod::Scaled
        )
        .is_err());
        assert!(value(
            &terms(
                ResetRule::Both {
                    alpha: 0.9,
                    k_min: 0.0,
                    k_max: 150.0
                },
                vec![1.0],
                european.clone()
            ),
            MscbMethod::Scaled
        )
        .is_err());
        assert!(value(
            &terms(
                ResetRule::DownwardOnly {
                    alpha: 0.9,
                    k_min: 60.0
                },
                vec![1.0],
                european
            ),
            MscbMethod::Scaled
        )
        .is_err());
    }

    #[test]
    fn the_greeks_are_finite_and_signed() {
        let american = ConversionExercise::American {
            start: 0.0,
            end: 5.0,
        };
        let t = terms(
            ResetRule::DownwardOnly {
                alpha: 0.9,
                k_min: 50.0,
            },
            vec![1.0, 2.0, 3.0],
            american,
        );
        let i = inputs(70.0, 0.02);
        let g = compute_mscb_greeks(&t, &i, 150, MscbMethod::Grid { price_nodes: 30 }).unwrap();
        assert!(
            g.value.is_finite() && g.delta.is_finite() && g.gamma.is_finite() && g.vega.is_finite()
        );
        assert!(
            g.delta >= 0.0 && g.credit_spread_sensitivity < 0.0 && g.vega > 0.0,
            "{g:?}"
        );
        assert!(
            (g.value
                - compute_mscb_value(&t, &i, 150, MscbMethod::Grid { price_nodes: 30 }).unwrap())
            .abs()
                < 1e-12
        );
    }
}
