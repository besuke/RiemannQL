#' Lookback options
#'
#' Options whose payoff depends on the minimum or the maximum of the price over the monitoring dates, under
#' Black-Scholes-Merton with a flat continuous rate, dividend yield and volatility. With the minimum `m` and the
#' maximum `M` of the price (including the value observed so far, `running_extremum`):
#'
#' | | call | put |
#' |---|---|---|
#' | floating strike | `S - m` | `M - S` |
#' | fixed strike `K` | `max(M - K, 0)` | `max(K - m, 0)` |
#'
#' The running extremum is a minimum for a floating call and a fixed put, and a maximum for a floating put and
#' a fixed call (`minmax` of QuantLib's lookback options); a minimum must not exceed the spot, a maximum must
#' not be below it.
#'
#' `compute_lookback_option_value()` monitors continuously, in closed form (QuantLib's
#' `AnalyticContinuousFloatingLookbackEngine` and `AnalyticContinuousFixedLookbackEngine`). Where the drift
#' `rate - dividend_yield` is 0, QuantLib returns NaN or an infinite value; here the value is the limit.
#' `compute_discrete_lookback_option_value()` monitors at `monitoring_times` on a binomial tree in the ratio of the
#' price to its extremum, whose error falls like one over the square root of `steps`; a monitoring time is placed on
#' the nearest step. There is no discrete engine in QuantLib's R package. The sensitivities are derivatives by
#' automatic differentiation; the `gamma` of the tree is a second difference of its values around the spot.
#'
#' Inputs are recycled to a common length. The monitoring times are one set for all rows.
#'
#' @param lookback_type `"floating"` or `"fixed"`.
#' @param option_type `"call"` or `"put"`.
#' @param spot Positive spot.
#' @param running_extremum The minimum or the maximum observed so far (see above).
#' @param strike Strike of a fixed-strike option; `NA` for a floating strike.
#' @param rate,dividend_yield Continuous rates.
#' @param volatility Positive lognormal volatility.
#' @param time Positive time to expiry in years.
#' @param monitoring_times Increasing times in years, in `(0, time]`. An empty vector means no monitoring: the
#'   extremum stays at `running_extremum`.
#' @param steps Steps of the tree, at least 2.
#' @param on_error `"stop"` or `"na"`; see [rmql-package].
#' @return A tibble with the inputs and the columns `value`, `delta`, `gamma`, `vega`, `rho`, `dividend_rho`,
#'   `theta` (per year of calendar time), `strike_sensitivity`, `extremum_sensitivity` and `error`.
#' @examples
#' # a floating-strike call at issue: the minimum so far is the spot
#' compute_lookback_option_value("floating", "call", 100, 100, NA, 0.03, 0.01, 0.25, 1)
#' # a fixed-strike put with a minimum of 92 so far
#' compute_lookback_option_value("fixed", "put", 100, 92, 100, 0.03, 0.01, 0.25, 1)
#' # monthly monitoring on a tree of 1200 steps
#' compute_discrete_lookback_option_value("floating", "call", 100, 100, NA, 0.03, 0.01, 0.25, 1,
#'                                        monitoring_times = (1:12) / 12, steps = 1200L)
#' @export
compute_lookback_option_value <- function(lookback_type, option_type, spot, running_extremum, strike = NA_real_, rate, dividend_yield, volatility, time,
                                          on_error = "stop") {
  i <- tibble(lookback_type = as.character(lookback_type), option_type = as.character(option_type), spot = as.numeric(spot),
              running_extremum = as.numeric(running_extremum), strike = as.numeric(strike), rate = as.numeric(rate),
              dividend_yield = as.numeric(dividend_yield), volatility = as.numeric(volatility), time = as.numeric(time))
  lookback_table(.Call(wrap__compute_lookback_option_value, i$lookback_type, i$option_type, i$spot, i$running_extremum, i$strike, i$rate,
                       i$dividend_yield, i$volatility, i$time), i, on_error)
}

#' @rdname compute_lookback_option_value
#' @export
compute_discrete_lookback_option_value <- function(lookback_type, option_type, spot, running_extremum, strike = NA_real_, rate, dividend_yield, volatility,
                                                   time, monitoring_times, steps = 400L, on_error = "stop") {
  i <- tibble(lookback_type = as.character(lookback_type), option_type = as.character(option_type), spot = as.numeric(spot),
              running_extremum = as.numeric(running_extremum), strike = as.numeric(strike), rate = as.numeric(rate),
              dividend_yield = as.numeric(dividend_yield), volatility = as.numeric(volatility), time = as.numeric(time),
              steps = as.integer(steps))
  lookback_table(.Call(wrap__compute_discrete_lookback_option_value, i$lookback_type, i$option_type, i$spot, i$running_extremum, i$strike, i$rate,
                       i$dividend_yield, i$volatility, i$time, as.numeric(monitoring_times), i$steps), i, on_error)
}

lookback_table <- function(rows, inputs, on_error) {
  rows <- finish_rows(rows, on_error)
  values <- tibble::as_tibble(rows[setdiff(names(rows), "error")])
  tibble::as_tibble(c(as.list(inputs), as.list(values), list(error = rows$error)))
}
