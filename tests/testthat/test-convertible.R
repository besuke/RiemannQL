# A convertible bond through the public functions. The expected values are QuantLib R 1.42's
# (ConvertibleFixedCouponBond with BinomialCRRConvertibleEngine, 300 steps; none of these step counts is affected by the
# floating-point artifact described in the help page); the Rust crate compares 300 more cases against a file.
# Synthetic inputs: S = 100, r = 3%, q = 1%, volatility 25%, credit spread 2%, ratio 1.2, 3% semiannual coupons for 5 years.

bond <- function() {
  create_fixed_rate_bond(0L, 100, "2026-11-02", "2031-11-02", "6M", "japan", "unadjusted", "unadjusted", "backward", FALSE, 0.03,
                         "actual_365_fixed", "unadjusted", 100)
}
value <- function(exercise, dates, calls = NULL, steps = 300L, spot = 100) {
  compute_convertible_bond_value(bond(), "2026-11-02", exercise, dates, 1.2, spot, 0.03, 0.01, 0.25, 0.02, steps, calls)
}

test_that("the value agrees with QuantLib 1.42: American with a call, a soft call and a put", {
  calls <- data.frame(date = as.Date(c("2029-11-02", "2030-11-02", "2028-11-02")), kind = c("call", "call", "put"),
                      price = c(110, 105, 100), price_type = c("clean", "dirty", "dirty"), trigger = c(NA, 1.3, NA))
  v <- value("american", c("2026-11-02", "2031-11-02"), calls)
  expect_equal(v$value, 131.449570061718, tolerance = 1e-9)
  expect_named(v, c("spot", "rate", "dividend_yield", "volatility", "credit_spread", "steps", "value", "delta", "gamma", "vega", "rho",
                    "dividend_rho", "credit_spread_sensitivity", "error"))
})

test_that("a clean price gets the accrued interest on its date", {
  # QuantLib: Bond_accruedAmount at 2029-12-15 is 0.353424657534; the clean call of 112 is a dirty call of 112.353424657534
  clean <- value("european", "2031-11-02", data.frame(date = as.Date("2029-12-15"), kind = "call", price = 112, price_type = "clean"))
  dirty <- value("european", "2031-11-02", data.frame(date = as.Date("2029-12-15"), kind = "call", price = 112.353424657534, price_type = "dirty"))
  expect_equal(clean$value, 104.942125069678, tolerance = 1e-9)
  expect_equal(clean$value, dirty$value, tolerance = 1e-12)
})

test_that("a Bermudan conversion agrees with QuantLib 1.42, and market inputs are recycled", {
  v <- value("bermudan", c("2028-11-02", "2030-11-02", "2031-11-02"))
  expect_equal(v$value, 133.313992847794, tolerance = 1e-9)
  r <- compute_convertible_bond_value(bond(), "2026-11-02", "european", "2031-11-02", 1.2, spot = c(80, 100, 120), rate = 0.03,
                                      dividend_yield = 0.01, volatility = 0.25, credit_spread = 0.02, steps = 100L)
  expect_equal(nrow(r), 3L)
  expect_true(all(diff(r$value) > 0))
  expect_true(all(r$delta > 0) && all(r$vega > 0) && all(r$credit_spread_sensitivity < 0))
})

test_that("inputs are checked, and on_error = na keeps the other rows", {
  expect_error(value("european", c("2031-11-02", "2031-11-03")), "1 date")
  expect_error(value("american", "2031-11-02"), "2 dates")
  expect_error(value("asian", "2031-11-02"), "unknown exercise")
  expect_error(compute_convertible_bond_value(list(), "2026-11-02", "european", "2031-11-02", 1.2, 100, 0.03, 0.01, 0.25, 0.02), "create_fixed_rate_bond")
  expect_error(value("european", "2031-11-02", data.frame(date = as.Date("2029-11-02"), kind = "swap", price = 100)), "unknown callability")
  r <- compute_convertible_bond_value(bond(), "2026-11-02", "european", "2031-11-02", 1.2, spot = c(100, -1), rate = 0.03, dividend_yield = 0.01,
                                      volatility = 0.25, credit_spread = 0.02, steps = 100L, on_error = "na")
  expect_false(is.na(r$value[1]))
  expect_true(is.na(r$value[2]) && nzchar(r$error[2]))
})

test_that("a reset conversion price with equal limits is a fixed ratio", {
  # lower = upper = 95 for a face of 100: a conversion ratio of 100 / 95, whatever the price ratio
  fixed <- compute_convertible_bond_value(bond(), "2026-11-02", "american", c("2026-11-02", "2031-11-02"), 100 / 95, 92, 0.03, 0.01, 0.3, 0.02, 250L)
  for (alpha in c(0.5, 0.9, 1.3)) {
    mscb <- compute_reset_convertible_bond_value(bond(), "2026-11-02", "american", c("2026-11-02", "2031-11-02"), alpha, 95, 95, 92, 0.03, 0.01, 0.3, 0.02, 250L)
    expect_equal(mscb$value, fixed$value, tolerance = 1e-10)
  }
})

test_that("nonfinite market inputs and a NaN up probability are errors", {
  args <- list(bond = bond(), settlement_date = "2026-11-02", exercise = "european", exercise_dates = "2031-11-02",
               conversion_ratio = 1.2, spot = 100, rate = 0.03, dividend_yield = 0.01, volatility = 0.25,
               credit_spread = 0.02, steps = 100L)
  purrr::walk(c("spot", "volatility", "rate", "dividend_yield", "credit_spread"), function(name) {
    purrr::walk(c(NA_real_, Inf, -Inf), function(x) {
      invalid <- args
      invalid[[name]] <- x
      expect_error(do.call(compute_convertible_bond_value, invalid), paste0(name, " must.*finite"))
    })
  })

  # The market inputs are finite, but the rate difference and squared volatility both overflow.
  args$rate <- .Machine$double.xmax
  args$dividend_yield <- -.Machine$double.xmax
  args$volatility <- .Machine$double.xmax
  expect_error(do.call(compute_convertible_bond_value, args), "probability.*NaN")
})

test_that("a European reset without coupons or spread has a closed form, and an American one converts at once above lower / ratio", {
  zero_coupon <- create_fixed_rate_bond(0L, 100, "2026-11-02", "2028-11-02", "1Y", "japan", "unadjusted", "unadjusted", "backward", FALSE, 0,
                                        "actual_365_fixed", "unadjusted", 100)
  # F e^{-rT} + F [call(lower) / lower - call(lower / ratio) / lower + call(upper / ratio) / upper], T = 730 / 365
  call <- function(strike) compute_european_option_value("call", 100, strike, 0.03, 0.01, 0.25, 2)$value
  alpha <- 0.9; lower <- 80; upper <- 140
  expected <- 100 * exp(-0.03 * 2) + 100 * (call(lower) / lower - call(lower / alpha) / lower + call(upper / alpha) / upper)
  v <- compute_reset_convertible_bond_value(zero_coupon, "2026-11-02", "european", "2028-11-02", alpha, lower, upper, 100, 0.03, 0.01, 0.25, 0, 1500L)
  expect_equal(v$value, expected, tolerance = 5e-3)
  # the price ratio below 1: above lower / ratio = 88.9 the holder can convert for 100 / 0.9 = 111.1 at once
  a <- compute_reset_convertible_bond_value(bond(), "2026-11-02", "american", c("2026-11-02", "2031-11-02"), 0.9, 80, Inf, c(95, 150),
                                            0.03, 0.01, 0.25, 0.02, 200L)
  expect_true(all(a$value >= 100 / 0.9 - 1e-9))
})

test_that("a reset conversion price is checked", {
  args <- list(bond(), "2026-11-02", "european", "2031-11-02", 0.9, 80, 140, 100, 0.03, 0.01, 0.25, 0.02, 100L)
  expect_no_error(do.call(compute_reset_convertible_bond_value, args))
  expect_error(do.call(compute_reset_convertible_bond_value, replace(args, 7, 70)), "upper conversion price")
  expect_error(do.call(compute_reset_convertible_bond_value, replace(args, 5, 0)), "positive")
  soft <- data.frame(date = as.Date("2029-11-02"), kind = "call", price = 105, trigger = 1.3)
  expect_error(compute_reset_convertible_bond_value(bond(), "2026-11-02", "european", "2031-11-02", 0.9, 80, 140, 100, 0.03, 0.01, 0.25, 0.02, 100L, soft), "soft call")
})

test_that("a conversion price carried between resets: no resets or a reset that never changes the price is a fixed ratio", {
  fixed <- compute_convertible_bond_value(bond(), "2026-11-02", "american", c("2026-11-02", "2031-11-02"), 100 / 100, 92, 0.03, 0.01, 0.25, 0.02, 200L)
  for (method in c("grid", "scaled")) {
    none <- compute_carried_reset_convertible_bond_value(bond(), "2026-11-02", "american", c("2026-11-02", "2031-11-02"), as.Date(character()),
                                                         "downward_only", 0.9, 100, spot = 92, rate = 0.03, dividend_yield = 0.01, volatility = 0.25,
                                                         credit_spread = 0.02, steps = 200L, method = method)
    expect_equal(none$value, fixed$value, tolerance = 1e-10)
    never <- compute_carried_reset_convertible_bond_value(bond(), "2026-11-02", "american", c("2026-11-02", "2031-11-02"),
                                                          as.Date(c("2027-11-02", "2028-11-02")), "downward_only", 1e6, 100, spot = 92, rate = 0.03,
                                                          dividend_yield = 0.01, volatility = 0.25, credit_spread = 0.02, steps = 200L, method = method)
    expect_equal(never$value, fixed$value, tolerance = 1e-10)
  }
})

test_that("a European conversion with one reset has a closed form (the rule both), and the grid and the scaled lattice agree", {
  zero_coupon <- create_fixed_rate_bond(0L, 100, "2026-11-02", "2028-11-02", "1Y", "japan", "unadjusted", "unadjusted", "backward", FALSE, 0,
                                        "actual_365_fixed", "unadjusted", 100)
  # F e^{-rT} + e^{-r tau} (F / alpha) call(1, alpha, T - tau), T = 2, tau = 1 (the reset on 2027-11-02, 365 days)
  alpha <- 0.9
  expected <- 100 * exp(-0.03 * 2) + exp(-0.03 * 365 / 365) * (100 / alpha) * compute_european_option_value("call", 1, alpha, 0.03, 0.01, 0.25, 1)$value
  value <- function(method, steps, nodes = 40L) {
    compute_carried_reset_convertible_bond_value(zero_coupon, "2026-11-02", "european", "2028-11-02", as.Date("2027-11-02"), "both", alpha, 100,
                                                 spot = 100, rate = 0.03, dividend_yield = 0.01, volatility = 0.25, credit_spread = 0, steps = steps,
                                                 method = method, price_nodes = nodes)$value
  }
  expect_equal(value("scaled", 1500L), expected, tolerance = 5e-3)
  expect_equal(value("grid", 300L, 100L), expected, tolerance = 1e-2)
})

test_that("a carried reset conversion price is checked", {
  args <- list(bond(), "2026-11-02", "european", "2031-11-02", as.Date("2028-11-02"), "downward_only", 0.9, 100, 0, Inf, 100, 0.03, 0.01, 0.25, 0.02, 100L)
  expect_no_error(do.call(compute_carried_reset_convertible_bond_value, args))
  expect_error(do.call(compute_carried_reset_convertible_bond_value, replace(args, 6, "sideways")), "unknown reset rule")
  expect_error(do.call(compute_carried_reset_convertible_bond_value, c(args, list(method = "cubic"))), "unknown method")
  expect_error(do.call(compute_carried_reset_convertible_bond_value, c(replace(args, 9, 60), list(method = "scaled"))), "no limits")
  expect_error(do.call(compute_carried_reset_convertible_bond_value, replace(args, 5, list(as.Date("2032-01-02")))), "reset times")
  expect_error(do.call(compute_carried_reset_convertible_bond_value, replace(args, 8, -1)), "positive")
})

test_that("greeks = FALSE gives the same value and no Greeks", {
  call <- function(greeks) compute_carried_reset_convertible_bond_value(bond(), "2026-11-02", "american", c("2026-11-02", "2031-11-02"), as.Date(c("2027-11-02", "2029-11-02")),
                                                                         "downward_only", 0.9, 100, spot = 85, rate = 0.03, dividend_yield = 0.01, volatility = 0.25,
                                                                         credit_spread = 0.02, steps = 100L, price_nodes = 30L, greeks = greeks)
  with <- call(TRUE); without <- call(FALSE)
  expect_equal(without$value, with$value, tolerance = 1e-12)
  expect_true(all(is.na(without[c("delta", "gamma", "vega", "rho", "dividend_rho", "credit_spread_sensitivity")])))
  expect_true(all(is.finite(unlist(with[c("delta", "gamma", "vega", "rho", "credit_spread_sensitivity")]))))
  purrr::walk(list(NA, 1, "TRUE", logical(), c(TRUE, FALSE)), function(greeks) {
    expect_error(call(greeks), "greeks.*must be TRUE or FALSE")
  })
})
