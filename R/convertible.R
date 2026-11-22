#' Convertible bond
#'
#' The value of a convertible bond on a binomial lattice with the Tsiveriotis-Fernandes treatment of credit risk,
#' as QuantLib's `ConvertibleFixedCouponBond` with `BinomialCRRConvertibleEngine`, under Black-Scholes-Merton with a
#' flat continuous rate, dividend yield and volatility. The lattice carries the value of the bond and the probability that
#' it ends in conversion at each node; a node discounts at `rate` where the bond is converted and at `rate + credit_spread`
#' where it is not, blended by that probability. The lattice runs from `settlement_date` to the last exercise date in
#' `steps` steps; dates are placed on the nearest step, the earlier one on a tie.
#'
#' The bond's cash flows are those of `bond` after `settlement_date`; its redemption is the amount paid at maturity. The
#' conversion gives `conversion_ratio` shares for the bond. A clean price of a call or a put gets the accrued interest on
#' its date added. A call with a `trigger` is a soft call: the issuer can call only where the share price is at least
#' `trigger` times the share price at which the redemption equals the conversion value. Not included: discrete
#' dividends, and stochastic rates.
#'
#' The `delta` and `gamma` come from the first steps of the lattice, as for [compute_binomial_option_value()]: a bump
#' of the spot much smaller than the lattice's spacing is noisy, since conversions and calls switch at nodes. The other
#' sensitivities are derivatives of the lattice by automatic differentiation; `credit_spread_sensitivity` is per unit of
#' spread (a basis point is 1e-4 of it). QuantLib returns the value only.
#'
#' **Difference from QuantLib 1.42.** QuantLib's time grid ends at `(end / steps) * steps`, which in floating point can be
#' one unit in the last place below the exercise end; for an American conversion it then does not allow the conversion at
#' maturity, for some numbers of steps only (about 3% of the cases tried). Here the bond can be converted at maturity.
#' The values are the same otherwise (relative 1e-11 on 291 of 300 cases tried); in the others they differ by up to 0.7%,
#' in either direction, since the right to convert changes the probabilities that blend the discount rates.
#'
#' Market inputs are recycled to a common length; the bond and the terms are one for all rows.
#'
#' @param bond A bond from [create_fixed_rate_bond()].
#' @param settlement_date The date at which the lattice starts (time 0): `Date` or ISO string.
#' @param exercise `"european"`, `"american"` or `"bermudan"`.
#' @param exercise_dates One date for a European conversion, the first and the last date for an American one, the dates
#'   for a Bermudan one. The lattice ends on the last of them.
#' @param conversion_ratio Shares received for the bond.
#' @param spot Positive share price.
#' @param rate,dividend_yield Continuous rates.
#' @param volatility Positive lognormal volatility.
#' @param credit_spread The credit spread of the issuer.
#' @param steps Steps of the lattice, at least 2.
#' @param callabilities `NULL`, or a data frame with the columns `date`, `kind` (`"call"` or `"put"`), `price` and,
#'   optionally, `price_type` (`"clean"` or `"dirty"`, default `"dirty"`) and `trigger` (`NA` for none; a call only).
#'   Callabilities on or before `settlement_date` are dropped.
#' @param day_count Day count of the times of the lattice; the default is QuantLib's engine's with a flat curve of
#'   `"actual_365_fixed"`.
#' @param on_error `"stop"` or `"na"`; see [rmql-package].
#' @return A tibble with the market inputs and the columns `value`, `delta`, `gamma`, `vega`, `rho`, `dividend_rho`,
#'   `credit_spread_sensitivity` and `error`.
#' @examples
#' bond <- create_fixed_rate_bond(0L, 100, "2026-11-02", "2031-11-02", "1Y", "japan", "unadjusted", "unadjusted",
#'                                "backward", FALSE, 0.02, "actual_365_fixed", "unadjusted", 100)
#' compute_convertible_bond_value(bond, "2026-11-02", "american", c("2026-11-02", "2031-11-02"), 1.0,
#'                                spot = 100, rate = 0.03, dividend_yield = 0.01, volatility = 0.25, credit_spread = 0.02,
#'                                steps = 300L)
#' @export
compute_convertible_bond_value <- function(bond, settlement_date, exercise, exercise_dates, conversion_ratio, spot, rate, dividend_yield, volatility,
                                           credit_spread, steps = 200L, callabilities = NULL, day_count = "actual_365_fixed", on_error = "stop") {
  convertible_table(bond, settlement_date, exercise, exercise_dates, FALSE, as.numeric(conversion_ratio), 1, 1, 1, spot, rate,
                    dividend_yield, volatility, credit_spread, steps, callabilities, day_count, on_error)
}

#' @rdname compute_convertible_bond_value
#' @param conversion_price_ratio For `compute_reset_convertible_bond_value()`: the conversion price is reset at conversion to this
#'   fraction of the share price at the same time, for example `0.9`.
#' @param lower_conversion_price,upper_conversion_price The reset price stays between them; the upper one can be `Inf`.
#' @export
compute_reset_convertible_bond_value <- function(bond, settlement_date, exercise, exercise_dates, conversion_price_ratio, lower_conversion_price,
                                                 upper_conversion_price = Inf, spot, rate, dividend_yield, volatility, credit_spread,
                                                 steps = 200L, callabilities = NULL, day_count = "actual_365_fixed", on_error = "stop") {
  convertible_table(bond, settlement_date, exercise, exercise_dates, TRUE, 1, as.numeric(conversion_price_ratio), as.numeric(lower_conversion_price),
                    as.numeric(upper_conversion_price), spot, rate, dividend_yield, volatility, credit_spread, steps, callabilities, day_count, on_error)
}

convertible_table <- function(bond, settlement_date, exercise, exercise_dates, reset, conversion_ratio, alpha, lower, upper, spot, rate, dividend_yield,
                              volatility, credit_spread, steps, callabilities, day_count, on_error) {
  if (!inherits(bond, "rmql_bond")) stop("`bond` must come from create_fixed_rate_bond().", call. = FALSE)
  calls <- as_callabilities(callabilities)
  i <- tibble(spot = as.numeric(spot), rate = as.numeric(rate), dividend_yield = as.numeric(dividend_yield), volatility = as.numeric(volatility),
              credit_spread = as.numeric(credit_spread), steps = as.integer(steps))
  rows <- checked(.Call(wrap__compute_convertible_bond_value, bond$pointer, as_serial(settlement_date), as.character(day_count), reset, conversion_ratio, alpha, lower, upper,
                as.character(exercise), as_serial(exercise_dates), as_serial(calls$date), as.character(calls$kind), as.numeric(calls$price),
                as.character(calls$price_type), as.numeric(calls$trigger), i$spot, i$rate, i$dividend_yield, i$volatility, i$credit_spread, i$steps))
  rows <- finish_rows(rows, on_error)
  values <- tibble::as_tibble(rows[setdiff(names(rows), "error")])
  tibble::as_tibble(c(as.list(i[setdiff(names(i), "steps")]), list(steps = i$steps), as.list(values), list(error = rows$error)))
}

as_callabilities <- function(callabilities) {
  calls <- callabilities
  if (is.null(calls)) {
    calls <- data.frame(date = as.Date(character()), kind = character(), price = numeric(), price_type = character(), trigger = numeric())
  } else {
    if (!all(c("date", "kind", "price") %in% names(calls))) stop("`callabilities` needs the columns date, kind and price.", call. = FALSE)
    if (is.null(calls$price_type)) calls$price_type <- "dirty"
    if (is.null(calls$trigger)) calls$trigger <- NA_real_
  }
  calls
}

#' @rdname compute_convertible_bond_value
#' @param reset_dates For `compute_carried_reset_convertible_bond_value()`: the dates on which the conversion price is reset,
#'   increasing, after `settlement_date` and not after the last exercise date. Each needs a step of its own in the lattice.
#' @param rule `"downward_only"`: the new price is `min(old price, max(lower, ratio * S))`; `"both"`:
#'   `min(upper, max(lower, ratio * S))`, whatever the old price. `S` is the share price on the reset date; the reset comes before any
#'   conversion on that date.
#' @param initial_conversion_price The conversion price until the first reset date.
#' @param method `"grid"`: a grid of `price_nodes` conversion prices, in any case; `"scaled"`: one dimension in the ratio of the
#'   share price to the conversion price, only without limits (`lower_conversion_price = 0`, and `upper_conversion_price = Inf` for
#'   `"both"`).
#' @param price_nodes Conversion prices of the grid, at least 2 (not used by `"scaled"`).
#' @param greeks `FALSE` gives the value alone, `NA` in the Greek columns, and is much faster for a grid (the lattice is not recorded
#'   for automatic differentiation).
#' @export
compute_carried_reset_convertible_bond_value <- function(bond, settlement_date, exercise, exercise_dates, reset_dates, rule = "downward_only",
                                                         conversion_price_ratio, initial_conversion_price, lower_conversion_price = 0,
                                                         upper_conversion_price = Inf, spot, rate, dividend_yield, volatility, credit_spread,
                                                         steps = 200L, method = "grid", price_nodes = 40L, greeks = TRUE,
                                                         callabilities = NULL, day_count = "actual_365_fixed", on_error = "stop") {
  if (!inherits(bond, "rmql_bond")) stop("`bond` must come from create_fixed_rate_bond().", call. = FALSE)
  calls <- as_callabilities(callabilities)
  i <- tibble(spot = as.numeric(spot), rate = as.numeric(rate), dividend_yield = as.numeric(dividend_yield), volatility = as.numeric(volatility),
              credit_spread = as.numeric(credit_spread), steps = as.integer(steps))
  rows <- checked(.Call(wrap__compute_carried_reset_convertible_bond_value, bond$pointer, as_serial(settlement_date), as.character(day_count),
                        as.character(exercise), as_serial(exercise_dates), as_serial(reset_dates), as.character(rule), as.numeric(conversion_price_ratio),
                        as.numeric(lower_conversion_price), as.numeric(upper_conversion_price), as.numeric(initial_conversion_price), as.character(method),
                        as.integer(price_nodes), isTRUE(greeks), as_serial(calls$date), as.character(calls$kind), as.numeric(calls$price), as.character(calls$price_type),
                        as.numeric(calls$trigger), i$spot, i$rate, i$dividend_yield, i$volatility, i$credit_spread, i$steps))
  rows <- finish_rows(rows, on_error)
  values <- tibble::as_tibble(rows[setdiff(names(rows), "error")])
  tibble::as_tibble(c(as.list(i[setdiff(names(i), "steps")]), list(steps = i$steps), as.list(values), list(error = rows$error)))
}
