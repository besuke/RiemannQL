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
}

impl ConversionTerms {
    fn value<T: Operand>(&self, spot: T) -> T {
        match *self {
            ConversionTerms::Fixed { ratio } => spot * ratio,
        }
    }

    /// The share price at which the bond's redemption equals the conversion value, for a soft call's trigger.
    fn reference_price(&self, redemption: f64) -> f64 {
        match *self {
            ConversionTerms::Fixed { ratio } => redemption / ratio,
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
    fn last_time(&self) -> f64 {
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
        let ConversionTerms::Fixed { ratio } = self.conversion;
        if !(ratio > 0.0) || !ratio.is_finite() {
            return invalid(format!(
                "the conversion ratio must be positive, not {ratio}"
            ));
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
            if !(x.primal() > 0.0) {
                return invalid(format!("{name} must be positive, not {}", x.primal()));
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

/// `quantlib_rounding` replicates a floating-point artifact of QuantLib 1.42: its time grid ends at `(end / steps) * steps`,
/// which can be one unit in the last place below `end`, and the test of an American conversion at the last time, `end <=
/// grid end`, then fails: the bond cannot be converted at maturity. Without it (the default), it can.
fn build_lattice<T: Operand>(
    terms: &ConvertibleTerms,
    inputs: &ConvertibleInputs<T>,
    steps: usize,
    quantlib_rounding: bool,
) -> Result<Lattice<T>, Error> {
    terms.check()?;
    inputs.check()?;
    if steps < 2 {
        return invalid(format!("at least 2 time steps required, {steps} provided"));
    }
    let ConvertibleInputs {
        spot,
        rate,
        dividend_yield,
        volatility,
        credit_spread,
    } = *inputs;
    let maturity = terms.exercise.last_time();
    let dt = maturity / steps as f64;
    let dx = volatility * dt.sqrt();
    let drift = (rate - dividend_yield - volatility * volatility * 0.5) * dt;
    let pu = drift / dx * 0.5 + 0.5;
    if pu.primal() > 1.0 {
        return invalid(format!("probability ({}) higher than one", pu.primal()));
    }
    if pu.primal() < 0.0 {
        return invalid(format!("negative ({}) probability", pu.primal()));
    }
    let pd = -pu + 1.0;
    let node_spot = |i: usize, j: usize| spot * (dx * (2.0 * j as f64 - i as f64)).exp();
    let step = |time: f64| closest_step(time, dt, steps);
    let call_steps: Vec<usize> = terms.callabilities.iter().map(|c| step(c.time)).collect();
    let coupon_steps: Vec<usize> = terms.coupons.iter().map(|c| step(c.time)).collect();
    let exercise_steps: Vec<usize> = match &terms.exercise {
        ConversionExercise::European(t) => vec![step(*t)],
        ConversionExercise::American { start, end } => vec![step(*start), step(*end)],
        ConversionExercise::Bermudan(times) => times.iter().map(|&t| step(t)).collect(),
    };
    let skipped_at_the_end = quantlib_rounding && !(maturity <= dt * steps as f64);
    let convertible_at = |i: usize| match &terms.exercise {
        ConversionExercise::American { .. } => {
            exercise_steps[0] <= i && i <= exercise_steps[1] && !(i == steps && skipped_at_the_end)
        }
        _ => exercise_steps.contains(&i),
    };
    let zero = spot.like(0.0);
    let one = spot.like(1.0);
    let conversion = terms.conversion;
    // the adjustments of a time: calls and puts, then coupons, then conversion
    let adjust = |i: usize, values: &mut [T], probability: &mut [T]| {
        let convertible = convertible_at(i);
        for (call, _) in terms
            .callabilities
            .iter()
            .zip(&call_steps)
            .filter(|(_, &s)| s == i)
        {
            match call.kind {
                CallabilityKind::Call => {
                    if let Some(trigger) = call.trigger {
                        let level = conversion.reference_price(terms.redemption) * trigger;
                        for (j, v) in values.iter_mut().enumerate() {
                            let s = node_spot(i, j);
                            if s.primal() >= level {
                                *v = conversion.value(s).max(v.like(call.price)).min(*v);
                            }
                        }
                    } else if convertible {
                        for (j, v) in values.iter_mut().enumerate() {
                            *v = conversion
                                .value(node_spot(i, j))
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
            .zip(&coupon_steps)
            .filter(|(_, &s)| s == i)
        {
            for v in values.iter_mut() {
                *v = *v + coupon.amount;
            }
        }
        if convertible {
            for (j, (v, p)) in values.iter_mut().zip(probability.iter_mut()).enumerate() {
                let payoff = conversion.value(node_spot(i, j));
                if v.primal() <= payoff.primal() {
                    *v = payoff;
                    *p = one;
                }
            }
        }
    };
    let blended = |p: T| p * rate + (-p + 1.0) * (rate + credit_spread);
    // the redemption at the end, then the adjustments of the last time, then the rates
    let mut values: Vec<T> = vec![spot.like(terms.redemption); steps + 1];
    let mut probability: Vec<T> = vec![zero; steps + 1];
    adjust(steps, &mut values, &mut probability);
    let mut rates: Vec<T> = probability.iter().map(|&p| blended(p)).collect();
    let (mut at_step_1, mut at_step_2) = ([zero; 2], [zero; 3]);
    for i in (0..steps).rev() {
        let new_probability: Vec<T> = (0..=i)
            .map(|j| pd * probability[j] + pu * probability[j + 1])
            .collect();
        let new_rates: Vec<T> = new_probability.iter().map(|&p| blended(p)).collect();
        let new_values: Vec<T> = (0..=i)
            .map(|j| {
                pd * values[j] / (rates[j] * dt + 1.0)
                    + pu * values[j + 1] / (rates[j + 1] * dt + 1.0)
            })
            .collect();
        values = new_values;
        probability = new_probability;
        rates = new_rates;
        adjust(i, &mut values, &mut probability);
        if i == 2 {
            at_step_2 = [values[0], values[1], values[2]];
        } else if i == 1 {
            at_step_1 = [values[0], values[1]];
        }
    }
    let at = |i: usize, j: usize| spot.primal() * (dx.primal() * (2.0 * j as f64 - i as f64)).exp();
    Ok(Lattice {
        value: values[0],
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
    fn events_go_to_the_nearest_step_and_the_earlier_one_on_a_tie() {
        assert_eq!(closest_step(0.0, 0.1, 10), 0);
        assert_eq!(closest_step(0.26, 0.1, 10), 3);
        assert_eq!(closest_step(0.24, 0.1, 10), 2);
        assert_eq!(closest_step(5.0, 0.1, 10), 10); // beyond the end: the last step
        assert_eq!(closest_step(-1.0, 0.1, 10), 0);
    }
}
