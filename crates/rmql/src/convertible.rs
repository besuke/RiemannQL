//! Convertible bonds on a binomial lattice with the Tsiveriotis-Fernandes treatment of credit risk, under
//! Black-Scholes-Merton with a flat continuous rate, dividend yield and volatility (P16 stage 2,
//! `docs/spec_lookback_convertible.md`). Every value is a function generic over [`Operand`], so its
//! sensitivities come from automatic differentiation.
//!
//! The method and its conventions follow QuantLib v1.42 (modified BSD license; see THIRD_PARTY_NOTICES):
//! `ql/pricingengines/bond/binomialconvertibleengine.hpp`, `ql/pricingengines/bond/discretizedconvertible.{hpp,cpp}`,
//! `ql/methods/lattices/tflattice.hpp` and `ql/instruments/bonds/convertiblebonds.cpp` (Copyright (C) 2005, 2006
//! Theo Boafo; (C) 2006, 2007 StatPro Italia srl; the lattice file: (C) 2006 StatPro Italia srl).
//!
//! The lattice carries the value `V` of the whole bond and the probability `p` that it ends in conversion, at each
//! node. A node's discount rate blends the risk-free rate `r` and the rate with the credit spread `s`:
//! `p r + (1 - p)(r + s)`, and one step back is `V_j = p_d V_j / (1 + rate_j dt) + p_u V_{j+1} / (1 + rate_{j+1} dt)`:
//! simple compounding per step, and the rates of the nodes one step later. On each node time, the adjustments are
//! applied in this order: calls and puts, coupons, conversion. The new probability of a step back is taken before the
//! adjustments of that time, so the rate array of a time does not see the conversions made on that time (as in
//! QuantLib).
//!
//! Differences from QuantLib: the inputs are numbers (times in years from the settlement date, flat rates), not
//! term structures and dates (see [`create_convertible_terms`] for the dates); there are no discrete dividends; a
//! lattice of at least 2 steps is required (QuantLib accepts 1); only the Cox-Ross-Rubinstein lattice is built; only
//! `theta` is not returned among the Greeks.

// `!(x > 0.0)` is intended: it also rejects NaN
#![allow(clippy::neg_cmp_op_on_partial_ord)]

use crate::autodiff::{Operand, Recorder};
use crate::cashflows::CashFlow;
use crate::instruments::{FixedRateBond, PriceType};
use crate::time::{Date, DayCount};
use crate::Error;

/// What the holder receives on conversion.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ConversionTerms {
    /// `ratio` shares for the bond.
    Fixed { ratio: f64 },
    /// The conversion price is reset at conversion, by the share price at the same time (a bond with a downward
    /// adjustment clause, MSCB): `K(S) = min(k_max, max(k_min, alpha S))`, and `face / K(S)` shares are received, worth
    /// `face S / K(S) = face max(S / k_max, min(S / k_min, 1 / alpha))`. `k_max` may be infinite. No conversion price
    /// is carried from one date to the next: that is stage 4 of `docs/spec_lookback_convertible.md`.
    Reset {
        face: f64,
        alpha: f64,
        k_min: f64,
        k_max: f64,
    },
}

impl ConversionTerms {
    fn value<T: Operand>(&self, spot: T) -> T {
        match *self {
            ConversionTerms::Fixed { ratio } => spot * ratio,
            ConversionTerms::Reset {
                face,
                alpha,
                k_min,
                k_max,
            } => (spot / k_max).max((spot / k_min).min(spot.like(1.0 / alpha))) * face,
        }
    }

    /// The share price at which the bond's redemption equals the conversion value, for a soft call's trigger; only a
    /// fixed ratio has one.
    fn reference_price(&self, redemption: f64) -> Option<f64> {
        match *self {
            ConversionTerms::Fixed { ratio } => Some(redemption / ratio),
            ConversionTerms::Reset { .. } => None,
        }
    }
}

/// When the bond can be converted; times in years from the settlement date.
#[derive(Debug, Clone, PartialEq)]
pub enum ConversionExercise {
    European(f64),
    American { start: f64, end: f64 },
    Bermudan(Vec<f64>),
}

impl ConversionExercise {
    /// The end of the lattice: the last exercise time.
    pub(crate) fn last_time(&self) -> f64 {
        match self {
            ConversionExercise::European(t) => *t,
            ConversionExercise::American { end, .. } => *end,
            ConversionExercise::Bermudan(times) => {
                times.iter().copied().fold(f64::NEG_INFINITY, f64::max)
            }
        }
    }
}

/// The issuer's call or the holder's put.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CallabilityKind {
    Call,
    Put,
}

/// A call or a put at `time` for `price` (a dirty price, in the units of the bond's redemption). A call with a
/// `trigger` is a soft call: it can be made only where the share price is at least `trigger` times the share price at
/// which redemption equals the conversion value.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Callability {
    pub time: f64,
    pub kind: CallabilityKind,
    pub price: f64,
    pub trigger: Option<f64>,
}

/// A coupon at `time` of `amount`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Coupon {
    pub time: f64,
    pub amount: f64,
}

/// A convertible bond in terms of times in years from the settlement date.
#[derive(Debug, Clone, PartialEq)]
pub struct ConvertibleTerms {
    pub redemption: f64,
    pub conversion: ConversionTerms,
    pub exercise: ConversionExercise,
    pub coupons: Vec<Coupon>,
    pub callabilities: Vec<Callability>,
}

/// The market inputs: flat continuous rates, one volatility and a credit spread.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ConvertibleInputs<T = f64> {
    pub spot: T,
    pub rate: T,
    pub dividend_yield: T,
    pub volatility: T,
    pub credit_spread: T,
}

/// A value and its sensitivities. `delta` and `gamma` come from the first steps of the lattice (Hull's formulas, as
/// for the vanilla tree); the others are the derivatives of the lattice by automatic differentiation.
/// `credit_spread_sensitivity` is per unit of spread (a basis point is 1e-4 of it).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ConvertibleGreeks {
    pub value: f64,
    pub delta: f64,
    pub gamma: f64,
    pub vega: f64,
    pub rho: f64,
    pub dividend_rho: f64,
    pub credit_spread_sensitivity: f64,
}

fn invalid<T>(reason: String) -> Result<T, Error> {
    Err(Error::InvalidInput { reason })
}

impl ConvertibleTerms {
    fn check(&self) -> Result<(), Error> {
        if !(self.redemption >= 0.0) || !self.redemption.is_finite() {
            return invalid(format!(
                "the redemption must be non-negative, not {}",
                self.redemption
            ));
        }
        match self.conversion {
            ConversionTerms::Fixed { ratio } => {
                if !(ratio > 0.0) || !ratio.is_finite() {
                    return invalid(format!(
                        "the conversion ratio must be positive, not {ratio}"
                    ));
                }
            }
            ConversionTerms::Reset {
                face,
                alpha,
                k_min,
                k_max,
            } => {
                if !(face > 0.0)
                    || !face.is_finite()
                    || !(alpha > 0.0)
                    || !alpha.is_finite()
                    || !(k_min > 0.0)
                    || !k_min.is_finite()
                {
                    return invalid(format!("the face, the price ratio and the lower conversion price must be positive and finite, not {face}, {alpha}, {k_min}"));
                }
                if !(k_max >= k_min) {
                    return invalid(format!("the upper conversion price {k_max} must not be below the lower one {k_min}"));
                }
                if self.callabilities.iter().any(|c| c.trigger.is_some()) {
                    return invalid("a soft call needs a fixed conversion ratio, which a reset conversion price does not have".into());
                }
            }
        }
        let end = self.exercise.last_time();
        if !(end > 0.0) || !end.is_finite() {
            return invalid(format!(
                "the last exercise time must be positive, not {end}"
            ));
        }
        match &self.exercise {
            ConversionExercise::American { start, end } if !(start <= end) => {
                return invalid(format!(
                    "the exercise period starts at {start}, after its end {end}"
                ));
            }
            ConversionExercise::Bermudan(times)
                if times.is_empty() || times.iter().any(|t| !t.is_finite()) =>
            {
                return invalid("give the exercise times of a Bermudan conversion".into());
            }
            _ => {}
        }
        if self
            .coupons
            .iter()
            .any(|c| !c.time.is_finite() || !c.amount.is_finite())
            || self
                .callabilities
                .iter()
                .any(|c| !c.time.is_finite() || !c.price.is_finite())
        {
            return invalid("the coupons and callabilities must be finite".into());
        }
        if self.callabilities.iter().any(|c| {
            c.trigger.is_some_and(|t| !(t > 0.0))
                || (c.trigger.is_some() && c.kind == CallabilityKind::Put)
        }) {
            return invalid("a trigger must be positive, and only a call has one".into());
        }
        Ok(())
    }
}

impl<T: Operand> ConvertibleInputs<T> {
    fn check(&self) -> Result<(), Error> {
        for (name, x) in [("spot", self.spot), ("volatility", self.volatility)] {
            let x = x.primal();
            if !(x > 0.0) || !x.is_finite() {
                return invalid(format!("{name} must be positive and finite, not {x}"));
            }
        }
        for (name, x) in [
            ("rate", self.rate),
            ("dividend_yield", self.dividend_yield),
            ("credit_spread", self.credit_spread),
        ] {
            let x = x.primal();
            if !x.is_finite() {
                return invalid(format!("{name} must be finite, not {x}"));
            }
        }
        Ok(())
    }
}

/// The index of the lattice time nearest to `time`, the earlier one on a tie (QuantLib's `TimeGrid::closestIndex`).
fn closest_step(time: f64, dt: f64, steps: usize) -> usize {
    if !(time > 0.0) {
        return 0;
    }
    let x = time / dt;
    let k = x.floor();
    let index = if x - k > 0.5 { k + 1.0 } else { k };
    (index as usize).min(steps)
}

struct Lattice<T> {
    value: T,
    at_step_1: [T; 2],
    at_step_2: [T; 3],
    spot_step_1: [f64; 2],
    spot_step_2: [f64; 3],
}

/// The values at the nodes of one time of the lattice: the value of the bond, the probability that it ends in conversion
/// and the discount rate of the node, which is the blend of the probability taken before the adjustments of the time.
#[derive(Clone)]
pub(crate) struct State<T> {
    pub values: Vec<T>,
    pub probability: Vec<T>,
    pub rates: Vec<T>,
}

/// The lattice of a convertible bond with its events, one step at a time, so that a conversion can be given to each step
/// (a conversion price that is reset, `mscb.rs`). `spot` is the share price at the first node; the conversion given to a step
/// is used there, and the conversion of the terms is not (only `check` looks at it).
pub(crate) struct Engine<'a, T: Operand> {
    pub steps: usize,
    pub dt: f64,
    pub dx: T,
    pub spot: T,
    pu: T,
    pd: T,
    rate: T,
    credit_spread: T,
    terms: &'a ConvertibleTerms,
    call_steps: Vec<usize>,
    coupon_steps: Vec<usize>,
    exercise_steps: Vec<usize>,
    skipped_at_the_end: bool,
}

impl<'a, T: Operand> Engine<'a, T> {
    /// `quantlib_rounding` replicates a floating-point artifact of QuantLib 1.42: its time grid ends at `(end / steps) * steps`,
    /// which can be one unit in the last place below `end`, and the test of an American conversion at the last time, `end <=
    /// grid end`, then fails: the bond cannot be converted at maturity. Without it (the default), it can.
    pub(crate) fn new(
        terms: &'a ConvertibleTerms,
        spot: T,
        inputs: &ConvertibleInputs<T>,
        steps: usize,
        quantlib_rounding: bool,
    ) -> Result<Self, Error> {
        terms.check()?;
        inputs.check()?;
        if steps < 2 {
            return invalid(format!("at least 2 time steps required, {steps} provided"));
        }
        let ConvertibleInputs {
            rate,
            dividend_yield,
            volatility,
            credit_spread,
            ..
        } = *inputs;
        let maturity = terms.exercise.last_time();
        let dt = maturity / steps as f64;
        let dx = volatility * dt.sqrt();
        let drift = (rate - dividend_yield - volatility * volatility * 0.5) * dt;
        let pu = drift / dx * 0.5 + 0.5;
        // `!contains` also rejects NaN
        if !(0.0..=1.0).contains(&pu.primal()) {
            return invalid(format!(
                "the probability of an up step ({}) must be in [0, 1]",
                pu.primal()
            ));
        }
        let pd = -pu + 1.0;
        let step = |time: f64| closest_step(time, dt, steps);
        let exercise_steps: Vec<usize> = match &terms.exercise {
            ConversionExercise::European(t) => vec![step(*t)],
            ConversionExercise::American { start, end } => vec![step(*start), step(*end)],
            ConversionExercise::Bermudan(times) => times.iter().map(|&t| step(t)).collect(),
        };
        Ok(Engine {
            steps,
            dt,
            dx,
            spot,
            pu,
            pd,
            rate,
            credit_spread,
            terms,
            call_steps: terms.callabilities.iter().map(|c| step(c.time)).collect(),
            coupon_steps: terms.coupons.iter().map(|c| step(c.time)).collect(),
            exercise_steps,
            skipped_at_the_end: quantlib_rounding && !(maturity <= dt * steps as f64),
        })
    }

    /// The step of the lattice nearest to a time.
    pub(crate) fn step_of(&self, time: f64) -> usize {
        closest_step(time, self.dt, self.steps)
    }

    /// The share price at node `j` of step `i`.
    pub(crate) fn node_spot(&self, i: usize, j: usize) -> T {
        self.spot * (self.dx * (2.0 * j as f64 - i as f64)).exp()
    }

    fn convertible_at(&self, i: usize) -> bool {
        match &self.terms.exercise {
            ConversionExercise::American { .. } => {
                self.exercise_steps[0] <= i
                    && i <= self.exercise_steps[1]
                    && !(i == self.steps && self.skipped_at_the_end)
            }
            _ => self.exercise_steps.contains(&i),
        }
    }

    fn blended(&self, p: T) -> T {
        p * self.rate + (-p + 1.0) * (self.rate + self.credit_spread)
    }

    /// The adjustments of a time: calls and puts, then coupons, then conversion.
    fn adjust(
        &self,
        i: usize,
        conversion: &ConversionTerms,
        values: &mut [T],
        probability: &mut [T],
    ) {
        let terms = self.terms;
        let one = self.spot.like(1.0);
        let convertible = self.convertible_at(i);
        for (call, _) in terms
            .callabilities
            .iter()
            .zip(&self.call_steps)
            .filter(|(_, &s)| s == i)
        {
            match call.kind {
                CallabilityKind::Call => {
                    if let Some(trigger) = call.trigger {
                        // `check` allows a trigger only with a fixed ratio
                        let level = conversion
                            .reference_price(terms.redemption)
                            .unwrap_or(f64::INFINITY)
                            * trigger;
                        for (j, v) in values.iter_mut().enumerate() {
                            let s = self.node_spot(i, j);
                            if s.primal() >= level {
                                *v = conversion.value(s).max(v.like(call.price)).min(*v);
                            }
                        }
                    } else if convertible {
                        for (j, v) in values.iter_mut().enumerate() {
                            *v = conversion
                                .value(self.node_spot(i, j))
                                .max(v.like(call.price))
                                .min(*v);
                        }
                    } else {
                        for v in values.iter_mut() {
                            *v = v.like(call.price).min(*v);
                        }
                    }
                }
                CallabilityKind::Put => {
                    for v in values.iter_mut() {
                        *v = (*v).max(v.like(call.price));
                    }
                }
            }
        }
        for (coupon, _) in terms
            .coupons
            .iter()
            .zip(&self.coupon_steps)
            .filter(|(_, &s)| s == i)
        {
            for v in values.iter_mut() {
                *v = *v + coupon.amount;
            }
        }
        if convertible {
            for (j, (v, p)) in values.iter_mut().zip(probability.iter_mut()).enumerate() {
                let payoff = conversion.value(self.node_spot(i, j));
                if v.primal() <= payoff.primal() {
                    *v = payoff;
                    *p = one;
                }
            }
        }
    }

    /// The redemption at the end, then the adjustments of the last time, then the rates.
    pub(crate) fn terminal(&self, conversion: &ConversionTerms) -> State<T> {
        let mut values: Vec<T> = vec![self.spot.like(self.terms.redemption); self.steps + 1];
        let mut probability: Vec<T> = vec![self.spot.like(0.0); self.steps + 1];
        self.adjust(self.steps, conversion, &mut values, &mut probability);
        let rates = probability.iter().map(|&p| self.blended(p)).collect();
        State {
            values,
            probability,
            rates,
        }
    }

    /// One step back to time `i` (from the state at `i + 1`), and the adjustments of `i`. The new rates are made from the new
    /// probability before the adjustments, so a conversion of time `i` is not in them.
    pub(crate) fn step_back(
        &self,
        i: usize,
        state: &State<T>,
        conversion: &ConversionTerms,
    ) -> State<T> {
        let (pd, pu, dt) = (self.pd, self.pu, self.dt);
        let mut probability: Vec<T> = (0..=i)
            .map(|j| pd * state.probability[j] + pu * state.probability[j + 1])
            .collect();
        let rates: Vec<T> = probability.iter().map(|&p| self.blended(p)).collect();
        let mut values: Vec<T> = (0..=i)
            .map(|j| {
                pd * state.values[j] / (state.rates[j] * dt + 1.0)
                    + pu * state.values[j + 1] / (state.rates[j + 1] * dt + 1.0)
            })
            .collect();
        self.adjust(i, conversion, &mut values, &mut probability);
        State {
            values,
            probability,
            rates,
        }
    }
}

fn build_lattice<T: Operand>(
    terms: &ConvertibleTerms,
    inputs: &ConvertibleInputs<T>,
    steps: usize,
    quantlib_rounding: bool,
) -> Result<Lattice<T>, Error> {
    let engine = Engine::new(terms, inputs.spot, inputs, steps, quantlib_rounding)?;
    let zero = inputs.spot.like(0.0);
    let mut state = engine.terminal(&terms.conversion);
    let (mut at_step_1, mut at_step_2) = ([zero; 2], [zero; 3]);
    for i in (0..steps).rev() {
        state = engine.step_back(i, &state, &terms.conversion);
        if i == 2 {
            at_step_2 = [state.values[0], state.values[1], state.values[2]];
        } else if i == 1 {
            at_step_1 = [state.values[0], state.values[1]];
        }
    }
    let at = |i: usize, j: usize| engine.node_spot(i, j).primal();
    Ok(Lattice {
        value: state.values[0],
        at_step_1,
        at_step_2,
        spot_step_1: [at(1, 0), at(1, 1)],
        spot_step_2: [at(2, 0), at(2, 1), at(2, 2)],
    })
}

/// The value of a convertible bond on a lattice of `steps` steps from the settlement date to the last exercise time.
pub fn compute_convertible_value<T: Operand>(
    terms: &ConvertibleTerms,
    inputs: &ConvertibleInputs<T>,
    steps: usize,
) -> Result<T, Error> {
    Ok(build_lattice(terms, inputs, steps, false)?.value)
}

/// The value as QuantLib 1.42 computes it, for reconciliation: an American conversion is not allowed at the last time
/// where QuantLib's time grid ends below the exercise end in floating point (see `build_lattice`). A defect of QuantLib's,
/// not a feature: use [`compute_convertible_value`].
#[doc(hidden)]
pub fn compute_convertible_value_like_quantlib<T: Operand>(
    terms: &ConvertibleTerms,
    inputs: &ConvertibleInputs<T>,
    steps: usize,
) -> Result<T, Error> {
    Ok(build_lattice(terms, inputs, steps, true)?.value)
}

/// The value and the Greeks of a convertible bond. QuantLib returns only the value.
pub fn compute_convertible_greeks(
    terms: &ConvertibleTerms,
    inputs: &ConvertibleInputs,
    steps: usize,
) -> Result<ConvertibleGreeks, Error> {
    let recorder = Recorder::new();
    let lifted = ConvertibleInputs {
        spot: recorder.input(inputs.spot),
        rate: recorder.input(inputs.rate),
        dividend_yield: recorder.input(inputs.dividend_yield),
        volatility: recorder.input(inputs.volatility),
        credit_spread: recorder.input(inputs.credit_spread),
    };
    let lattice = build_lattice(terms, &lifted, steps, false)?;
    let wrt = [
        lifted.rate,
        lifted.dividend_yield,
        lifted.volatility,
        lifted.credit_spread,
    ];
    let g = recorder.gradient(lattice.value, &wrt)?;
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

/// When the bond can be converted, as dates.
#[derive(Debug, Clone, PartialEq)]
pub enum DatedExercise {
    European(Date),
    American { start: Date, end: Date },
    Bermudan(Vec<Date>),
}

/// A call or a put as a date and a price.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DatedCallability {
    pub date: Date,
    pub kind: CallabilityKind,
    pub price: f64,
    pub price_type: PriceType,
    pub trigger: Option<f64>,
}

/// The terms of a convertible bond from a fixed-rate bond: its coupons after `settlement`, its redemption, the times of
/// the dates in years from `settlement` by `day_count`, and a clean price of a call or a put made dirty by the accrued
/// interest on its date (as QuantLib's `ConvertibleBond::setupArguments`). A callability on or before `settlement` is
/// dropped. The bond's cash flows are used as they are: QuantLib's convertible bonds fix the notional at 100.
pub fn create_convertible_terms(
    bond: &FixedRateBond,
    settlement: Date,
    day_count: &DayCount,
    conversion: ConversionTerms,
    exercise: &DatedExercise,
    callabilities: &[DatedCallability],
) -> Result<ConvertibleTerms, Error> {
    let time = |d: Date| day_count.year_fraction(settlement, d, None, None);
    let mut coupons = Vec::new();
    let mut redemption = None;
    for flow in &bond.cashflows {
        match flow {
            CashFlow::FixedRateCoupon(_) => {
                if flow.date() > settlement {
                    coupons.push(Coupon {
                        time: time(flow.date())?,
                        amount: flow.amount()?,
                    });
                }
            }
            CashFlow::Redemption { .. } => redemption = Some(flow.amount()?),
        }
    }
    let redemption = redemption.ok_or_else(|| Error::InvalidInput {
        reason: "the bond has no redemption".into(),
    })?;
    let exercise = match exercise {
        DatedExercise::European(d) => ConversionExercise::European(time(*d)?),
        DatedExercise::American { start, end } => ConversionExercise::American {
            start: time(*start)?,
            end: time(*end)?,
        },
        DatedExercise::Bermudan(dates) => {
            ConversionExercise::Bermudan(dates.iter().map(|&d| time(d)).collect::<Result<_, _>>()?)
        }
    };
    let mut calls = Vec::new();
    for c in callabilities.iter().filter(|c| c.date > settlement) {
        let accrued = if c.price_type == PriceType::Clean {
            bond.accrued_amount(c.date)?
        } else {
            0.0
        };
        calls.push(Callability {
            time: time(c.date)?,
            kind: c.kind,
            price: c.price + accrued,
            trigger: c.trigger,
        });
    }
    Ok(ConvertibleTerms {
        redemption,
        conversion,
        exercise,
        coupons,
        callabilities: calls,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inputs(
        spot: f64,
        rate: f64,
        dividend: f64,
        volatility: f64,
        spread: f64,
    ) -> ConvertibleInputs {
        ConvertibleInputs {
            spot,
            rate,
            dividend_yield: dividend,
            volatility,
            credit_spread: spread,
        }
    }

    fn annual_coupons(years: u32, amount: f64) -> Vec<Coupon> {
        (1..=years)
            .map(|k| Coupon {
                time: k as f64,
                amount,
            })
            .collect()
    }

    fn terms(
        ratio: f64,
        years: u32,
        coupon: f64,
        exercise: ConversionExercise,
    ) -> ConvertibleTerms {
        ConvertibleTerms {
            redemption: 100.0,
            conversion: ConversionTerms::Fixed { ratio },
            exercise,
            coupons: annual_coupons(years, coupon),
            callabilities: vec![],
        }
    }

    #[test]
    fn without_conversion_the_value_is_the_discounted_cash_flows_at_the_spread_rate() {
        // a tiny ratio is never worth converting: every node discounts at r + s, one simple step at a time
        let steps = 120;
        let t = terms(
            1e-9,
            3,
            4.0,
            ConversionExercise::American {
                start: 0.0,
                end: 3.0,
            },
        );
        let (rate, spread) = (0.03, 0.02);
        let v =
            compute_convertible_value(&t, &inputs(100.0, rate, 0.01, 0.3, spread), steps).unwrap();
        let dt = 3.0 / steps as f64;
        let factor = |k: usize| (1.0 + (rate + spread) * dt).powi(-(k as i32));
        let expected: f64 =
            (1..=3).map(|y| 4.0 * factor(y * 40)).sum::<f64>() + 100.0 * factor(steps);
        assert!((v - expected).abs() < 1e-10 * expected, "{v} {expected}");
    }

    #[test]
    fn a_european_conversion_without_coupons_or_spread_is_a_bond_plus_a_call() {
        // with no spread, no coupon and no calls: the redemption F or kappa S_T, F e^{-rT} + kappa call(S, F / kappa)
        let (spot, rate, dividend, vol, ratio) = (100.0, 0.03, 0.01, 0.25, 0.9);
        let t = ConvertibleTerms {
            coupons: vec![],
            ..terms(ratio, 0, 0.0, ConversionExercise::European(2.0))
        };
        let v =
            compute_convertible_value(&t, &inputs(spot, rate, dividend, vol, 0.0), 2000).unwrap();
        let call = crate::options::compute_european_value(
            crate::options::OptionType::Call,
            &crate::options::OptionInputs {
                spot,
                strike: 100.0 / ratio,
                rate,
                dividend_yield: dividend,
                volatility: vol,
                time: 2.0,
            },
        )
        .unwrap();
        let expected = 100.0 * (-rate * 2.0f64).exp() + ratio * call;
        assert!((v - expected).abs() < 3e-3 * expected, "{v} {expected}");
    }

    #[test]
    fn properties_of_the_value() {
        let t = terms(
            1.0,
            5,
            2.0,
            ConversionExercise::American {
                start: 0.0,
                end: 5.0,
            },
        );
        let v = |spot, spread| {
            compute_convertible_value(&t, &inputs(spot, 0.03, 0.01, 0.25, spread), 200).unwrap()
        };
        assert!(v(80.0, 0.02) < v(100.0, 0.02) && v(100.0, 0.02) < v(130.0, 0.02)); // increasing in the spot
        assert!(v(100.0, 0.05) < v(100.0, 0.0)); // decreasing in the credit spread
        let straight = compute_convertible_value(
            &terms(
                1e-9,
                5,
                2.0,
                ConversionExercise::American {
                    start: 0.0,
                    end: 5.0,
                },
            ),
            &inputs(100.0, 0.03, 0.01, 0.25, 0.02),
            200,
        )
        .unwrap();
        assert!(v(100.0, 0.02) >= straight); // the option to convert has a value
    }

    #[test]
    fn a_call_lowers_the_value_and_a_put_raises_it() {
        let base = terms(
            1.0,
            5,
            2.0,
            ConversionExercise::American {
                start: 0.0,
                end: 5.0,
            },
        );
        let value = |t: &ConvertibleTerms| {
            compute_convertible_value(t, &inputs(100.0, 0.03, 0.01, 0.25, 0.02), 200).unwrap()
        };
        let none = value(&base);
        let call = ConvertibleTerms {
            callabilities: vec![Callability {
                time: 3.0,
                kind: CallabilityKind::Call,
                price: 105.0,
                trigger: None,
            }],
            ..base.clone()
        };
        let put = ConvertibleTerms {
            callabilities: vec![Callability {
                time: 3.0,
                kind: CallabilityKind::Put,
                price: 110.0,
                trigger: None,
            }],
            ..base.clone()
        };
        let soft = ConvertibleTerms {
            callabilities: vec![Callability {
                time: 3.0,
                kind: CallabilityKind::Call,
                price: 105.0,
                trigger: Some(1.3),
            }],
            ..base
        };
        assert!(value(&call) < none && value(&put) > none);
        assert!(value(&call) <= value(&soft) && value(&soft) <= none); // a soft call is a weaker call
    }

    #[test]
    fn greeks_agree_with_differences() {
        let t = ConvertibleTerms {
            callabilities: vec![Callability {
                time: 4.0,
                kind: CallabilityKind::Call,
                price: 108.0,
                trigger: None,
            }],
            ..terms(
                1.1,
                5,
                2.0,
                ConversionExercise::American {
                    start: 0.0,
                    end: 5.0,
                },
            )
        };
        let i = inputs(95.0, 0.03, 0.01, 0.25, 0.02);
        let g = compute_convertible_greeks(&t, &i, 400).unwrap();
        let v = |j: ConvertibleInputs| compute_convertible_value(&t, &j, 400).unwrap();
        assert!((g.value - v(i)).abs() < 1e-12);
        let h = 1e-6;
        let vega = (v(ConvertibleInputs {
            volatility: i.volatility + h,
            ..i
        }) - v(ConvertibleInputs {
            volatility: i.volatility - h,
            ..i
        })) / (2.0 * h);
        let rho = (v(ConvertibleInputs {
            rate: i.rate + h,
            ..i
        }) - v(ConvertibleInputs {
            rate: i.rate - h,
            ..i
        })) / (2.0 * h);
        let cs = (v(ConvertibleInputs {
            credit_spread: i.credit_spread + h,
            ..i
        }) - v(ConvertibleInputs {
            credit_spread: i.credit_spread - h,
            ..i
        })) / (2.0 * h);
        assert!(
            (g.vega - vega).abs() < 1e-4 * (1.0 + vega.abs()),
            "vega {} {vega}",
            g.vega
        );
        assert!(
            (g.rho - rho).abs() < 1e-4 * (1.0 + rho.abs()),
            "rho {} {rho}",
            g.rho
        );
        assert!(
            (g.credit_spread_sensitivity - cs).abs() < 1e-4 * (1.0 + cs.abs()),
            "cs {} {cs}",
            g.credit_spread_sensitivity
        );
        // the delta is a difference over the first step (about +-2.4% of the spot here). A bump of the spot much smaller than the
        // lattice's spacing is noisy, since conversions and calls switch at nodes; a bump of the size of the first step agrees
        let hs = 0.05 * i.spot;
        let delta = (v(ConvertibleInputs {
            spot: i.spot + hs,
            ..i
        }) - v(ConvertibleInputs {
            spot: i.spot - hs,
            ..i
        })) / (2.0 * hs);
        assert!((g.delta - delta).abs() < 0.02, "delta {} {delta}", g.delta);
        assert!(g.gamma.is_finite() && g.delta > 0.0 && g.credit_spread_sensitivity < 0.0);
    }

    #[test]
    fn invalid_inputs_are_errors() {
        let good = terms(1.0, 5, 2.0, ConversionExercise::European(5.0));
        let i = inputs(100.0, 0.03, 0.01, 0.25, 0.02);
        assert!(compute_convertible_value(&good, &i, 100).is_ok());
        assert!(compute_convertible_value(&good, &i, 1).is_err());
        assert!(
            compute_convertible_value(&good, &inputs(0.0, 0.03, 0.01, 0.25, 0.02), 100).is_err()
        );
        assert!(
            compute_convertible_value(&good, &inputs(100.0, 0.03, 0.01, 0.0, 0.02), 100).is_err()
        );
        assert!(compute_convertible_value(
            &terms(0.0, 5, 2.0, ConversionExercise::European(5.0)),
            &i,
            100
        )
        .is_err());
        assert!(compute_convertible_value(
            &terms(1.0, 5, 2.0, ConversionExercise::European(0.0)),
            &i,
            100
        )
        .is_err());
        assert!(compute_convertible_value(
            &terms(
                1.0,
                5,
                2.0,
                ConversionExercise::American {
                    start: 3.0,
                    end: 1.0
                }
            ),
            &i,
            100
        )
        .is_err());
        assert!(compute_convertible_value(
            &terms(1.0, 5, 2.0, ConversionExercise::Bermudan(vec![])),
            &i,
            100
        )
        .is_err());
        let put_with_trigger = ConvertibleTerms {
            callabilities: vec![Callability {
                time: 3.0,
                kind: CallabilityKind::Put,
                price: 100.0,
                trigger: Some(1.2),
            }],
            ..good
        };
        assert!(compute_convertible_value(&put_with_trigger, &i, 100).is_err());
    }

    #[test]
    fn nonfinite_market_inputs_are_errors() {
        let terms = terms(1.0, 5, 2.0, ConversionExercise::European(5.0));
        let valid = inputs(100.0, 0.03, 0.01, 0.25, 0.02);
        for x in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            for (name, input) in [
                ("spot", ConvertibleInputs { spot: x, ..valid }),
                (
                    "volatility",
                    ConvertibleInputs {
                        volatility: x,
                        ..valid
                    },
                ),
                ("rate", ConvertibleInputs { rate: x, ..valid }),
                (
                    "dividend_yield",
                    ConvertibleInputs {
                        dividend_yield: x,
                        ..valid
                    },
                ),
                (
                    "credit_spread",
                    ConvertibleInputs {
                        credit_spread: x,
                        ..valid
                    },
                ),
            ] {
                let error = compute_convertible_value(&terms, &input, 100)
                    .unwrap_err()
                    .to_string();
                assert!(
                    error.contains(&format!("{name} must")) && error.contains("finite"),
                    "{error}"
                );
            }
        }
    }

    #[test]
    fn an_up_probability_that_overflows_to_nan_is_an_error() {
        let terms = terms(1.0, 5, 2.0, ConversionExercise::European(5.0));
        // Both the rate difference and the squared volatility overflow, so the drift is NaN.
        let input = inputs(100.0, f64::MAX, -f64::MAX, f64::MAX, 0.02);
        let error = compute_convertible_value(&terms, &input, 100)
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("probability") && error.contains("NaN"),
            "{error}"
        );
    }

    #[test]
    fn events_go_to_the_nearest_step_and_the_earlier_one_on_a_tie() {
        assert_eq!(closest_step(0.0, 0.1, 10), 0);
        assert_eq!(closest_step(0.26, 0.1, 10), 3);
        assert_eq!(closest_step(0.24, 0.1, 10), 2);
        assert_eq!(closest_step(5.0, 0.1, 10), 10); // beyond the end: the last step
        assert_eq!(closest_step(-1.0, 0.1, 10), 0);
    }

    fn reset(face: f64, alpha: f64, k_min: f64, k_max: f64) -> ConversionTerms {
        ConversionTerms::Reset {
            face,
            alpha,
            k_min,
            k_max,
        }
    }

    #[test]
    fn the_conversion_value_of_a_reset_is_the_value_of_the_reset_price() {
        // K(S) = min(120, max(60, 0.9 S)): 50 -> 60 (the lower), 100 -> 90, 200 -> 120 (the upper)
        let terms = reset(100.0, 0.9, 60.0, 120.0);
        for (spot, price) in [
            (50.0, 60.0),
            (65.0, 60.0),
            (100.0, 90.0),
            (140.0, 120.0),
            (200.0, 120.0),
        ] {
            assert!(
                (terms.value(spot) - 100.0 * spot / price).abs() < 1e-12 * terms.value(spot),
                "{spot}: {} {}",
                terms.value(spot),
                100.0 * spot / price
            );
        }
        // without an upper price: face min(S / k_min, 1 / alpha)
        let open = reset(100.0, 0.9, 60.0, f64::INFINITY);
        for spot in [30.0, 60.0, 66.0, 67.0, 90.0, 500.0] {
            assert!(
                (open.value(spot) - 100.0 * (spot / 60.0).min(1.0 / 0.9)).abs()
                    < 1e-12 * open.value(spot),
                "{spot}"
            );
        }
    }

    #[test]
    fn a_reset_with_equal_prices_is_a_fixed_ratio() {
        // k_min = k_max = K: face S / K, a conversion ratio face / K
        let callabilities = vec![
            Callability {
                time: 3.0,
                kind: CallabilityKind::Call,
                price: 106.0,
                trigger: None,
            },
            Callability {
                time: 2.0,
                kind: CallabilityKind::Put,
                price: 100.0,
                trigger: None,
            },
        ];
        let exercise = ConversionExercise::American {
            start: 0.0,
            end: 5.0,
        };
        let i = inputs(92.0, 0.03, 0.01, 0.3, 0.02);
        for alpha in [0.5, 0.9, 1.0, 1.3] {
            let fixed = ConvertibleTerms {
                callabilities: callabilities.clone(),
                ..terms(100.0 / 95.0, 5, 2.0, exercise.clone())
            };
            let mscb = ConvertibleTerms {
                conversion: reset(100.0, alpha, 95.0, 95.0),
                ..fixed.clone()
            };
            let (a, b) = (
                compute_convertible_value(&fixed, &i, 250).unwrap(),
                compute_convertible_value(&mscb, &i, 250).unwrap(),
            );
            assert!((a - b).abs() < 1e-10 * a, "alpha {alpha}: {a} {b}");
        }
    }

    #[test]
    fn a_european_reset_without_coupons_or_spread_has_a_closed_form() {
        // payoff F max(1, g(S_T)), g = S / k_min - (S / k_min - 1 / alpha)+ + (S / k_max - 1 / alpha)+ for k_min <= k_max, alpha <= 1:
        // F e^{-rT} + F [call(k_min) / k_min - call(k_min / alpha) / k_min + call(k_max / alpha) / k_max]
        let (spot, rate, dividend, vol, time, face) = (100.0, 0.03, 0.01, 0.25, 2.0, 100.0);
        let call = |strike: f64| {
            crate::options::compute_european_value(
                crate::options::OptionType::Call,
                &crate::options::OptionInputs {
                    spot,
                    strike,
                    rate,
                    dividend_yield: dividend,
                    volatility: vol,
                    time,
                },
            )
            .unwrap()
        };
        for (alpha, k_min, k_max) in [(0.9, 80.0, 140.0), (0.8, 90.0, 110.0), (0.95, 70.0, 1e9)] {
            let t = ConvertibleTerms {
                coupons: vec![],
                ..terms(1.0, 0, 0.0, ConversionExercise::European(time))
            };
            let t = ConvertibleTerms {
                conversion: reset(face, alpha, k_min, k_max),
                ..t
            };
            let v = compute_convertible_value(&t, &inputs(spot, rate, dividend, vol, 0.0), 2500)
                .unwrap();
            let expected = face * (-rate * time).exp()
                + face
                    * (call(k_min) / k_min - call(k_min / alpha) / k_min
                        + call(k_max / alpha) / k_max);
            assert!(
                (v - expected).abs() < 3e-3 * expected,
                "alpha {alpha}: {v} {expected}"
            );
        }
    }

    #[test]
    fn a_reset_conversion_price_is_checked() {
        let base = terms(1.0, 5, 2.0, ConversionExercise::European(5.0));
        let i = inputs(100.0, 0.03, 0.01, 0.25, 0.02);
        let value = |c: ConversionTerms| {
            compute_convertible_value(
                &ConvertibleTerms {
                    conversion: c,
                    ..base.clone()
                },
                &i,
                100,
            )
        };
        assert!(value(reset(100.0, 0.9, 80.0, 140.0)).is_ok());
        assert!(value(reset(100.0, 0.9, 80.0, f64::INFINITY)).is_ok());
        assert!(value(reset(100.0, 0.9, 80.0, 70.0)).is_err());
        assert!(value(reset(100.0, 0.0, 80.0, 140.0)).is_err());
        assert!(value(reset(100.0, 0.9, 0.0, 140.0)).is_err());
        assert!(value(reset(0.0, 0.9, 80.0, 140.0)).is_err());
        assert!(value(reset(100.0, 0.9, 80.0, f64::NAN)).is_err());
        let soft = ConvertibleTerms {
            conversion: reset(100.0, 0.9, 80.0, 140.0),
            callabilities: vec![Callability {
                time: 3.0,
                kind: CallabilityKind::Call,
                price: 105.0,
                trigger: Some(1.3),
            }],
            ..base
        };
        assert!(compute_convertible_value(&soft, &i, 100).is_err());
    }

    #[test]
    fn the_greeks_of_a_reset_are_finite_and_signed() {
        let t = ConvertibleTerms {
            conversion: reset(100.0, 0.9, 80.0, f64::INFINITY),
            ..terms(
                1.0,
                5,
                2.0,
                ConversionExercise::American {
                    start: 0.0,
                    end: 5.0,
                },
            )
        };
        // the spot is below k_min / alpha = 88.9: above it the holder converts at once for face / alpha, whatever the spot
        let g = compute_convertible_greeks(&t, &inputs(70.0, 0.03, 0.01, 0.25, 0.02), 300).unwrap();
        assert!(
            g.value.is_finite() && g.delta.is_finite() && g.gamma.is_finite() && g.vega.is_finite()
        );
        assert!(
            g.delta >= 0.0 && g.credit_spread_sensitivity < 0.0 && g.vega > 0.0,
            "{g:?}"
        );
    }
}
