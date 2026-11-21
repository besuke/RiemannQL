hw_curve <- function() {
  bootstrap_ois_curve("2026-11-02", "2026-11-02", "tonar", "actual_365_fixed",
                  create_ois_instruments(c("1Y", "5Y", "10Y", "20Y"), c(0.01, 0.015, 0.02, 0.022), 2L, 1L, 0L))
}

# The border with Rust: a count, a thread number or a vector of the wrong kind is an error with a
# message, never a huge allocation or a panic. Synthetic inputs.

test_that("negative, NA and infinite counts are failures", {
  w <- c(0.5, 0.5)
  mu <- c(0.01, 0.02)
  sigma <- matrix(c(0.04, 0.01, 0.01, 0.09), 2)
  expect_error(compute_monte_carlo_risk(w, mu, sigma, scenarios = Inf), "scenarios")
  expect_error(compute_monte_carlo_risk(w, mu, sigma, scenarios = -5), "scenarios")
  expect_error(compute_monte_carlo_risk(w, mu, sigma, scenarios = NaN), "scenarios")
  expect_error(compute_monte_carlo_risk(w, mu, sigma, scenarios = 1000, batches = -1L), "batches")
  expect_error(compute_monte_carlo_risk(w, mu, sigma, scenarios = 1000, batches = NA_integer_), "Must not be NA|batches")
  expect_error(generate_sobol_sequence(-1L, 10), "dimension")
  expect_error(generate_sobol_sequence(2L, Inf), "points")
  expect_error(generate_sobol_sequence(20L, 9e7), "points \\* dimension")
})

test_that("a negative or NA number of threads is a failure, not a huge pool", {
  w <- c(0.5, 0.5)
  mu <- c(0.01, 0.02)
  sigma <- matrix(c(0.04, 0.01, 0.01, 0.09), 2)
  expect_error(compute_monte_carlo_risk(w, mu, sigma, scenarios = 1000, threads = -1L), "threads")
  expect_error(compute_monte_carlo_risk(w, mu, sigma, scenarios = 1000, threads = NA_integer_), "Must not be NA|threads")
  expect_error(compute_monte_carlo_risk(w, mu, sigma, scenarios = 1000, threads = 100000L), "threads")
  expect_error(simulate_credit_losses(0.02, 100, 0.45, 0.2, scenarios = 100, threads = -3L), "threads")
  expect_no_error(compute_monte_carlo_risk(w, mu, sigma, scenarios = 1000, threads = 1L))
})

test_that("the traffic light does not turn a negative count into zero", {
  expect_error(compute_traffic_light(-1L, 250L), "exceptions|non-negative")
})

test_that("an NA logical in a swaption or an XCCY row is a failure, not TRUE", {
  curve <- hw_curve()
  swaptions <- tibble::tibble(payer = NA, maturity = 1, payment_times = list(2:3), coupons = list(c(0.02, 0.02)), price = 0.01)
  expect_error(calibrate_hull_white(curve, swaptions = swaptions), "missing value")
})

test_that("vectors of different lengths are a failure with a message, not an out-of-range index", {
  today <- rmql:::as_serial("2026-11-02")
  r <- .Call(rmql:::wrap__bootstrap_ois_curve, today, today, "tonar", "actual_365_fixed",
             c("1Y", "5Y", "10Y"), c(0.01, 0.015), c(2L, 2L, 2L), c(1L, 1L, 1L), c(0L, 0L, 0L), "log_linear", 1e-12)
  expect_match(r$failure, "same length")
  expect_match(r$failure, "tenor: 3")
})
