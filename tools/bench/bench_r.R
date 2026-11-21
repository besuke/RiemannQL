# Baseline timings of the calculations that the performance review names (specification, R14).
# Synthetic inputs only. It measures what a client sees: an R call, with the conversions.
#
# Usage:  Rscript tools/bench/bench_r.R [output.csv] [library]
#   output.csv  where to write the table (default: bench_r_result.csv in the working directory)
#   library     an R library that holds the rmql to measure (default: the first one that has it)
#
# Each case runs once to warm up, then `reps` times; the table has the median, minimum and maximum
# of the elapsed seconds per call (a short call is repeated inside one timing). Close other R sessions first: timings on a busy machine mean little.
# Change no input between a baseline and a later run, so that the two tables compare.

args <- commandArgs(trailingOnly = TRUE)
output <- if (length(args) >= 1L) args[[1]] else "bench_r_result.csv"
if (length(args) >= 2L) .libPaths(c(args[[2]], .libPaths()))
suppressPackageStartupMessages(library(rmql))

reps <- 7L
today <- "2026-11-02"

# ---- inputs ------------------------------------------------------------------------------------

curve_of <- function(index, rates) {
  bootstrap_ois_curve(today, today, index, "actual_365_fixed",
                      create_ois_instruments(c("ON", "1Y", "2Y", "5Y", "10Y"), rates, 2L, 1L, 0L))
}
jpy <- curve_of("tonar", c(0.002, 0.004, 0.005, 0.009, 0.014))
usd <- curve_of("sofr", c(0.040, 0.038, 0.036, 0.034, 0.033))

bond <- create_fixed_rate_bond(
  settlement_days = 2L, face_amount = 100, effective_date = "2024-03-20", termination_date = "2031-03-20",
  tenor = "6M", calendar = "japan", convention = "unadjusted", termination_convention = "unadjusted", rule = "backward",
  end_of_month = FALSE, coupon_rate = 0.015, day_count = "actual_365_fixed", payment_convention = "following",
  redemption = 100, issue_date = "2024-03-20"
)
settlement <- compute_settlement_date(bond, today)

batch_bonds <- function(n) {
  tibble::tibble(
    settlement_days = 1L, face_amount = 100, effective_date = as.Date("2020-03-20"), termination_date = as.Date("2030-03-20"),
    tenor = "6M", calendar = "japan", convention = "following", termination_convention = "following", rule = "backward",
    end_of_month = FALSE, coupon_rate = 0.015, day_count = "act_act_isma", payment_convention = "following", redemption = 100,
    issue_date = as.Date("2020-03-20"), rate = seq(0.001, 0.05, length.out = n), yield_day_count = "act_act_isma",
    compounding = "compounded", frequency = 2L, evaluation_date = as.Date(today)
  )
}
batch_curves <- function(n) {
  indices <- rep(c("tonar", "estr", "sofr", "sonia"), length.out = n)
  tibble::tibble(
    evaluation_date = as.Date(today), reference_date = as.Date(today), index = indices, curve_day_count = "actual_365_fixed",
    instruments = lapply(seq_len(n), function(i) create_ois_instruments(c("ON", "1Y", "2Y", "5Y", "10Y"), c(0.002, 0.004, 0.005, 0.009, 0.014) + i / 1e5, 2L, 1L, 0L)),
    dates = lapply(seq_len(n), function(i) as.Date(today) + c(30L, 365L, 1000L))
  )
}

mbs <- create_mbs_pass_through(100, 0.05, 0.045, refinancing_cost_rate = 0.005)
mbs_rates <- generate_hull_white_monthly_rates(200, 360, 0.02, 0.15, 0.012, seed = 3)
mbs_refinance <- pmax(mbs_rates + 0.03, 0)

smile_vols <- rbind(c(0.150, 0.130, 0.115, 0.118, 0.135), c(0.156, 0.134, 0.118, 0.121, 0.140), c(0.161, 0.138, 0.121, 0.124, 0.146))
smile <- create_fx_smile(150, jpy, usd, c(1, 2, 5), c(-0.1, -0.25), c(0.25, 0.1), smile_vols)

lmm_times <- seq(1, 6, by = 0.5)
lmm <- create_lmm(lmm_times, seq(0.02, 0.029, length.out = 10), seq(0.25, 0.205, length.out = 10), beta = 0.3, discount_start = exp(-0.02))

ctd_today <- "2026-11-04"
ctd_yen <- create_zero_curve(as.Date(ctd_today) + c(0, 365, 3652), c(0.005, 0.005, 0.012), "actual_365_fixed")
ctd_usd <- create_zero_curve(as.Date(ctd_today) + c(0, 365, 3652), c(0.009, 0.009, 0.006), "actual_365_fixed")
ctd_dates <- as.Date(ctd_today) + c(365, 1826, 3652)

time <- (1:10) / 2
principal <- c(rep(0, 9), 100)
call_price <- c(NA, rep(100, 8), NA)

# ---- cases (a named list of functions with no arguments) ----------------------------------------

cases <- list(
  ois_bootstrap_5_quotes = function() curve_of("tonar", c(0.002, 0.004, 0.005, 0.009, 0.014)),
  z_spread_one_price = function() compute_z_spread_from_price(bond, jpy, 99, "clean", "compounded", 2L, settlement, accuracy = 1e-12),
  mbs_oas_200_paths = function() compute_mbs_oas(mbs, 110, mbs_rates, mbs_refinance),
  mbs_path_values_200_paths = function() compute_mbs_path_values(mbs, mbs_rates, mbs_refinance),
  hull_white_callable_value = function() compute_hull_white_callable_bond_value(time, 1.5, principal, call_price, 0.15, 0.012, jpy),
  hull_white_callable_decisions = function() compute_hull_white_callable_bond_decisions(time, 1.5, principal, call_price, 0.15, 0.012, jpy),
  prdc_closed_form = function() compute_prdc_value(150, 0.12, jpy, usd, 1:5, rep(1, 5), 150, 0.8, 0.78, 0, 0.08, notional = 1e8),
  prdc_mc_2e5_paths_1_thread = function() compute_prdc_value(150, 0.12, jpy, usd, 1:5, rep(1, 5), 150, 0.8, 0.78, 0, 0.08, notional = 1e8, paths = 2e5, seed = 3, threads = 1L),
  prdc_mc_2e5_paths_4_threads = function() compute_prdc_value(150, 0.12, jpy, usd, 1:5, rep(1, 5), 150, 0.8, 0.78, 0, 0.08, notional = 1e8, paths = 2e5, seed = 3, threads = 4L),
  prdc_three_factor_2e4_paths = function() compute_prdc_three_factor_value(150, 0.11, jpy, usd, c(0.03, 0.01), c(0.05, 0.012), c(0.3, -0.2, 0.4), 1:5, rep(1, 5), 150, 0.8, 0.78, 0, 0.08, paths = 20000L, steps_per_year = 12L, seed = 3),
  prdc_smile_value = function() compute_prdc_smile_value(smile, 1:5, rep(1, 5), 150, 0.8, 0.78, 0, 0.08, notional = 1),
  lmm_swaption_mc_20000_paths = function() compute_lmm_swaption_value(lmm, start = c(3, 5), end = c(7, 11), strike = c(0.023, 0.026), paths = 20000L),
  ctd_time_value_5000_paths = function() compute_ctd_time_value(ctd_today, ctd_dates, list(JPY = ctd_yen, USD = ctd_usd), volatility = 0.005, mean_reversion = 0.1, paths = 5000L),
  # the pool is built on each call: compare the small batch with the large one
  batch_bonds_1_row_1_thread = local({ b <- batch_bonds(1L); function() compute_bond_values_batch(b, threads = 1L) }),
  batch_bonds_1_row_4_threads = local({ b <- batch_bonds(1L); function() compute_bond_values_batch(b, threads = 4L) }),
  batch_bonds_40_rows_4_threads = local({ b <- batch_bonds(40L); function() compute_bond_values_batch(b, threads = 4L) }),
  batch_bonds_2000_rows_1_thread = local({ b <- batch_bonds(2000L); function() compute_bond_values_batch(b, threads = 1L) }),
  batch_bonds_2000_rows_4_threads = local({ b <- batch_bonds(2000L); function() compute_bond_values_batch(b, threads = 4L) }),
  batch_curves_20_4_threads = local({ b <- batch_curves(20L); function() compute_curve_values_batch(b, threads = 4L) })
)

# ---- run ----------------------------------------------------------------------------------------

elapsed_of <- function(f) {
  start <- Sys.time()
  f()
  as.numeric(difftime(Sys.time(), start, units = "secs"))
}

# The clock has a resolution of about 10 ms. A call shorter than 20 ms is repeated, `inner` times
# in one timing, so that a timing lasts at least 0.2 s; the table has seconds per call.
time_case <- function(f) {
  invisible(f())  # warm up
  once <- elapsed_of(f)
  inner <- if (once >= 0.02) 1L else min(10000L, max(1L, as.integer(ceiling(0.2 / max(once, 1e-4)))))
  elapsed <- vapply(seq_len(reps), function(i) {
    gc(FALSE)
    elapsed_of(function() for (k in seq_len(inner)) f()) / inner
  }, numeric(1))
  c(median = median(elapsed), min = min(elapsed), max = max(elapsed), inner = inner)
}

results <- lapply(names(cases), function(name) {
  cat(sprintf("%-36s", name))
  r <- tryCatch(time_case(cases[[name]]), error = function(e) {
    cat("ERROR:", conditionMessage(e), "\n")
    NULL
  })
  if (is.null(r)) return(data.frame(case = name, median_s = NA_real_, min_s = NA_real_, max_s = NA_real_, inner = NA_integer_))
  cat(sprintf("median %10.6f s   min %10.6f   max %10.6f   (x%d per timing)\n", r[["median"]], r[["min"]], r[["max"]], as.integer(r[["inner"]])))
  data.frame(case = name, median_s = r[["median"]], min_s = r[["min"]], max_s = r[["max"]], inner = as.integer(r[["inner"]]))
})
table <- do.call(rbind, results)
table$rmql <- as.character(packageVersion("rmql"))
table$r <- paste(R.version$major, R.version$minor, sep = ".")
table$cores <- parallel::detectCores()
table$date <- format(Sys.time(), "%Y-%m-%d %H:%M:%S")
utils::write.csv(table, output, row.names = FALSE)
cat("written:", normalizePath(output, winslash = "/"), "\n")
