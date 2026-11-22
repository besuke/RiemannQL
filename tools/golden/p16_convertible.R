# Golden values for rmql P16 stage 2 (convertible bonds on the Tsiveriotis-Fernandes lattice) from the QuantLib R
# package Version 1.42: ConvertibleFixedCouponBond with BinomialCRRConvertibleEngine. All inputs are synthetic
# random numbers; no market data is used.
# Usage (from the rmql root): Rscript tools/golden/p16_convertible.R
# Writes tests/golden/convertible_bonds.csv. Numbers have 17 significant digits. Times are days / 365 (Actual365Fixed),
# the rates are continuous, and the settlement date is the evaluation date (settlement days 0).
# Columns: exercise, exercise_days (european: the date; american: start;end; bermudan: the dates, as days from the
# settlement), ratio, redemption, spread, spot, rate, dividend, volatility, steps,
# coupons (days:amount|...), calls (days:kind:dirty price:trigger|..., NA for no trigger, `-` for none), value.
# A call or a put given as a clean price has QuantLib's accrued interest on its date added here, from the same bond.

suppressMessages(library(QuantLib))

out_dir <- file.path("tests", "golden")
set.seed(20261123)
fmt <- function(x) sprintf("%.17g", x)
dc <- QuantLib::Actual365Fixed()
today <- QuantLib::Date(2L, "November", 2026L)
invisible(QuantLib::Settings_instance()$setEvaluationDate(today))
serial <- function(d) as.integer(QuantLib::Date_serialNumber(d))
today_serial <- serial(today)
date_of <- function(days) QuantLib::Date(today_serial + as.integer(days))

process_of <- function(spot, rate, dividend, volatility) {
  QuantLib::BlackScholesMertonProcess(
    QuantLib::QuoteHandle(QuantLib::SimpleQuote(spot)),
    QuantLib::YieldTermStructureHandle(QuantLib::FlatForward(today, dividend, dc)),
    QuantLib::YieldTermStructureHandle(QuantLib::FlatForward(today, rate, dc)),
    QuantLib::BlackVolTermStructureHandle(QuantLib::BlackConstantVol(today, QuantLib::TARGET(), volatility, dc))
  )
}

n <- 300
rows <- vector("list", n)
for (i in seq_len(n)) {
  years <- sample(c(1, 2, 3, 4, 5, 7), 1)
  months <- sample(c(6, 12), 1)
  maturity <- QuantLib::Date(2L, "November", as.integer(2026 + years))
  schedule <- QuantLib::Schedule(today, maturity, QuantLib::Period(as.integer(months), "Months"), QuantLib::TARGET(), "Unadjusted", "Unadjusted", "Backward", FALSE)
  coupon_rate <- runif(1, 0, 0.06)
  redemption <- sample(c(100, 100, 105), 1)
  ratio <- runif(1, 0.3, 2)
  spot <- runif(1, 40, 150)
  spread <- runif(1, 0, 0.05)
  rate <- runif(1, -0.005, 0.06)
  dividend <- runif(1, 0, 0.04)
  volatility <- runif(1, 0.1, 0.6)
  steps <- sample(c(20L, 50L, 100L, 157L, 300L), 1)
  maturity_days <- serial(maturity) - today_serial
  coupon_days <- (months / 12 * 365.25) # only for choosing dates below; the schedule's own dates are read from the bond
  plain <- QuantLib::ConvertibleFixedCouponBond(QuantLib::AmericanExercise(today, maturity), ratio, QuantLib::CallabilitySchedule(), today, 0L, c(coupon_rate), dc, schedule, redemption)
  cash <- QuantLib::Bond_cashflows(plain)
  m <- QuantLib::Leg___len__(cash)
  flow_days <- vapply(seq_len(m) - 1L, function(k) serial(QuantLib::CashFlow_date(QuantLib::Leg___getitem__(cash, k))) - today_serial, 0)
  flow_amount <- vapply(seq_len(m) - 1L, function(k) QuantLib::CashFlow_amount(QuantLib::Leg___getitem__(cash, k)), 0)
  coupon_idx <- seq_len(m - 1L) # the last flow is the redemption
  coupons <- paste(flow_days[coupon_idx], vapply(flow_amount[coupon_idx], fmt, ""), sep = ":", collapse = "|")
  schedule_days <- flow_days[coupon_idx]

  kind <- sample(c("european", "american", "bermudan"), 1)
  if (kind == "european") {
    exercise_days <- maturity_days
    exercise <- QuantLib::EuropeanExercise(maturity)
  } else if (kind == "american") {
    start <- sample(c(0L, schedule_days[schedule_days < maturity_days]), 1)
    exercise_days <- c(start, maturity_days)
    exercise <- QuantLib::AmericanExercise(date_of(start), maturity)
  } else {
    k <- min(length(schedule_days), sample(2:4, 1))
    exercise_days <- sort(sample(schedule_days, k))
    dv <- QuantLib::DateVector()
    for (d in exercise_days) QuantLib::DateVector_push_back(dv, date_of(d))
    exercise <- QuantLib::BermudanExercise(dv)
  }

  # calls and puts on schedule dates (QuantLib requires them not after maturity)
  n_calls <- sample(0:3, 1, prob = c(0.3, 0.3, 0.25, 0.15))
  calls <- QuantLib::CallabilitySchedule()
  call_text <- character(0)
  if (n_calls > 0) {
    call_days <- sort(sample(schedule_days, min(n_calls, length(schedule_days))))
    for (d in call_days) {
      call_kind <- sample(c("Call", "Put"), 1, prob = c(0.65, 0.35))
      price_type <- sample(c("Clean", "Dirty"), 1)
      price <- runif(1, 95, 130)
      trigger <- if (call_kind == "Call" && runif(1) < 0.3) runif(1, 1.0, 1.6) else NA
      if (is.na(trigger)) {
        QuantLib::CallabilitySchedule_push_back(calls, QuantLib::Callability(QuantLib::BondPrice(price, price_type), call_kind, date_of(d)))
      } else {
        QuantLib::CallabilitySchedule_push_back(calls, QuantLib::SoftCallability(QuantLib::BondPrice(price, price_type), date_of(d), trigger))
      }
      accrued <- if (price_type == "Clean") QuantLib::Bond_accruedAmount(plain, date_of(d)) else 0
      call_text <- c(call_text, paste(d, tolower(call_kind), fmt(price + accrued), if (is.na(trigger)) "NA" else fmt(trigger), sep = ":"))
    }
  }
  bond <- QuantLib::ConvertibleFixedCouponBond(exercise, ratio, calls, today, 0L, c(coupon_rate), dc, schedule, redemption)
  engine <- QuantLib::BinomialCRRConvertibleEngine(process_of(spot, rate, dividend, volatility), steps, QuantLib::QuoteHandle(QuantLib::SimpleQuote(spread)), QuantLib::DividendSchedule())
  QuantLib::Instrument_setPricingEngine(bond, engine)
  value <- QuantLib::Instrument_NPV(bond)
  rows[[i]] <- c(kind, paste(exercise_days, collapse = ";"), fmt(ratio), fmt(redemption), fmt(spread), fmt(spot), fmt(rate), fmt(dividend), fmt(volatility),
                 steps, coupons, if (length(call_text)) paste(call_text, collapse = "|") else "-", fmt(value))
}

path <- file.path(out_dir, "convertible_bonds.csv")
writeLines(c(paste0("# QuantLib R package ", packageVersion("QuantLib"), "; synthetic inputs; tools/golden/p16_convertible.R"),
             "exercise,exercise_days,ratio,redemption,spread,spot,rate,dividend,volatility,steps,coupons,calls,value"), path)
for (r in rows) write(paste(r, collapse = ","), path, append = TRUE)
values <- as.numeric(vapply(rows, function(r) r[length(r)], ""))
cat("wrote", path, "with", n, "rows;", sum(!is.finite(values)), "non-finite values; range", range(values), "\n")
