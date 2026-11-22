# Golden values for rmql P16 stage 1 (continuously monitored lookback options) from the QuantLib R package
# Version 1.42. All inputs are synthetic random numbers; no market data is used.
# Usage (from the rmql root): Rscript tools/golden/p16_lookback.R
# Writes tests/golden/option_lookback.csv. Numbers have 17 significant digits; time is days / 365 and the
# rates are continuous, so QuantLib's flat curves reduce to the numeric inputs of rmql.
# `extremum` is the minimum so far for a floating call and a fixed put, the maximum so far for a floating put
# and a fixed call (QuantLib's `minmax`). Some rows have a drift b = r - q of +-1e-5 (see below).

suppressMessages(library(QuantLib))

out_dir <- file.path("tests", "golden")
set.seed(20261122)
fmt <- function(x) sprintf("%.17g", x)
day_count <- QuantLib::Actual365Fixed()
calendar <- QuantLib::NullCalendar()
today <- QuantLib::Date(2L, "November", 2026L)
invisible(QuantLib::Settings_instance()$setEvaluationDate(today))

process_of <- function(spot, rate, dividend, volatility) {
  QuantLib::BlackScholesMertonProcess(
    QuantLib::QuoteHandle(QuantLib::SimpleQuote(spot)),
    QuantLib::YieldTermStructureHandle(QuantLib::FlatForward(today, dividend, day_count)),
    QuantLib::YieldTermStructureHandle(QuantLib::FlatForward(today, rate, day_count)),
    QuantLib::BlackVolTermStructureHandle(QuantLib::BlackConstantVol(today, calendar, volatility, day_count))
  )
}
expiry <- function(days) QuantLib::Date(as.integer(QuantLib::Date_serialNumber(today)) + as.integer(days))

price <- function(strike_type, kind, spot, extremum, strike, rate, dividend, volatility, days) {
  process <- process_of(spot, rate, dividend, volatility)
  exercise <- QuantLib::EuropeanExercise(expiry(days))
  if (strike_type == "floating") {
    option <- QuantLib::ContinuousFloatingLookbackOption(extremum, QuantLib::FloatingTypePayoff(if (kind == "call") "Call" else "Put"), exercise)
    QuantLib::Instrument_setPricingEngine(option, QuantLib::AnalyticContinuousFloatingLookbackEngine(process))
  } else {
    option <- QuantLib::ContinuousFixedLookbackOption(extremum, QuantLib::PlainVanillaPayoff(if (kind == "call") "Call" else "Put", strike), exercise)
    QuantLib::Instrument_setPricingEngine(option, QuantLib::AnalyticContinuousFixedLookbackEngine(process))
  }
  QuantLib::Instrument_NPV(option)
}

n <- 400
rows <- data.frame(
  strike_type = sample(c("floating", "fixed"), n, TRUE), kind = sample(c("call", "put"), n, TRUE),
  spot = runif(n, 50, 150), rate = runif(n, -0.01, 0.08), dividend = runif(n, 0, 0.05),
  volatility = runif(n, 0.05, 0.8), days = sample(30:3650, n, TRUE), stringsAsFactors = FALSE
)
# drift b = r - q of 1e-5 or -1e-5 in some rows: QuantLib 1.42 returns NaN or Inf where b is exactly 0 (the formulas divide
# by b), so the value at b = 0 is checked by its continuity against these rows
near <- sample(n, 40)
rows$dividend[near] <- rows$rate[near] - ifelse(seq_along(near) %% 2 == 0, 1e-5, -1e-5)
# the extremum so far: below the spot for a minimum (floating call, fixed put), above for a maximum; sometimes the spot itself
rows$min_type <- (rows$strike_type == "floating" & rows$kind == "call") | (rows$strike_type == "fixed" & rows$kind == "put")
at_spot <- runif(n) < 0.25
rows$extremum <- ifelse(at_spot, rows$spot, ifelse(rows$min_type, rows$spot * runif(n, 0.6, 1), rows$spot * runif(n, 1, 1.5)))
# the strike of a fixed option: either side of the extremum
rows$strike <- ifelse(rows$strike_type == "fixed", rows$spot * runif(n, 0.6, 1.4), rows$spot)
rows$value <- vapply(seq_len(n), function(i) with(rows[i, ], price(strike_type, kind, spot, extremum, strike, rate, dividend, volatility, days)), 0)

path <- file.path(out_dir, "option_lookback.csv")
writeLines(c(paste0("# QuantLib R package ", packageVersion("QuantLib"), "; synthetic inputs; tools/golden/p16_lookback.R"),
             "strike_type,kind,spot,extremum,strike,rate,dividend,volatility,days,value"), path)
for (i in seq_len(n)) {
  r <- rows[i, ]
  write(paste(c(r$strike_type, r$kind, fmt(r$spot), fmt(r$extremum), fmt(r$strike), fmt(r$rate), fmt(r$dividend), fmt(r$volatility), r$days, fmt(r$value)), collapse = ","), path, append = TRUE)
}
cat("wrote", path, "with", n, "rows;", sum(!is.finite(rows$value)), "non-finite values;", sum(rows$rate == rows$dividend), "rows with rate = dividend\n")
