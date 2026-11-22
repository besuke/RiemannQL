# Lookback options through the public functions. The continuous prices are QuantLib 1.42's
# (AnalyticContinuous*LookbackEngine); the Rust crate's tests compare 400 rows against QuantLib, and here a few values
# are checked, with the rest by identities. Synthetic inputs.

test_that("the continuous price agrees with QuantLib 1.42 on the examples of the help page", {
  # QuantLib R 1.42: S = 100, minimum so far 100, r = 3%, q = 1%, volatility 25%, one year
  v <- compute_lookback_option_value("floating", "call", 100, 100, NA, 0.03, 0.01, 0.25, 1)
  expect_equal(v$value, 19.07298, tolerance = 1e-6)
  w <- compute_lookback_option_value("fixed", "call", 100, 100, 100, 0.03, 0.01, 0.25, 1)
  expect_equal(w$value, 22.13615, tolerance = 1e-6)
  expect_named(v, c("lookback_type", "option_type", "spot", "running_extremum", "strike", "rate", "dividend_yield", "volatility", "time",
                    "value", "delta", "gamma", "vega", "rho", "dividend_rho", "theta", "strike_sensitivity", "extremum_sensitivity", "error"))
})

test_that("the value at zero drift is finite, and continuous in the drift", {
  at <- function(q) compute_lookback_option_value("floating", "call", 100, 90, NA, 0.04, q, 0.25, 2)$value
  expect_true(is.finite(at(0.04)))
  expect_lt(abs(at(0.04) - at(0.0399)), 1e-3 * at(0.04))
})

test_that("a fixed-strike option is a floating-strike one plus a constant", {
  fixed <- compute_lookback_option_value("fixed", "call", 100, 100, 90, 0.03, 0.01, 0.2, 1)$value
  floating <- compute_lookback_option_value("floating", "put", 100, 100, NA, 0.03, 0.01, 0.2, 1)$value
  expect_equal(fixed, floating + 100 * exp(-0.01) - 90 * exp(-0.03), tolerance = 1e-12)
})

test_that("the discrete price converges to the continuous one, and monitoring dates are respected", {
  continuous <- compute_lookback_option_value("floating", "call", 100, 100, NA, 0.03, 0.01, 0.25, 1)$value
  every_step <- function(n) compute_discrete_lookback_option_value("floating", "call", 100, 100, NA, 0.03, 0.01, 0.25, 1,
                                                                   monitoring_times = (1:n) / n, steps = n)$value
  e <- continuous - c(every_step(100L), every_step(400L), every_step(1600L))
  expect_true(all(diff(e) < 0) && all(e > 0))
  monthly <- compute_discrete_lookback_option_value("floating", "call", 100, 100, NA, 0.03, 0.01, 0.25, 1,
                                                    monitoring_times = (1:12) / 12, steps = 1200L)$value
  expect_lt(monthly, continuous)
  expect_error(compute_discrete_lookback_option_value("floating", "call", 100, 100, NA, 0.03, 0.01, 0.25, 1, monitoring_times = c(0.5, 0.2)), "increase")
})

test_that("inputs are checked, and on_error = na keeps the other rows", {
  expect_error(compute_lookback_option_value("floating", "call", 100, 110, NA, 0.03, 0.01, 0.25, 1), "minimum")
  expect_error(compute_lookback_option_value("floating", "put", 100, 90, NA, 0.03, 0.01, 0.25, 1), "maximum")
  expect_error(compute_lookback_option_value("other", "call", 100, 90, NA, 0.03, 0.01, 0.25, 1), "lookback strike")
  r <- compute_lookback_option_value("floating", "call", c(100, 100), c(90, 110), NA, 0.03, 0.01, 0.25, 1, on_error = "na")
  expect_false(is.na(r$value[1]))
  expect_true(is.na(r$value[2]) && nzchar(r$error[2]))
  # direct calls with vectors of different lengths are a failure with a message
  x <- .Call(rmql:::wrap__compute_lookback_option_value, "floating", c("call", "put"), 100, 90, NA_real_, 0.03, 0.01, 0.25, c(1, 2, 3))
  expect_match(x$failure, "same length")
})
