//! The 10¢-equivalent FX table (decisions 3C/8A, re-affirmed D28).
//!
//! Pacific's fee is a flat US$0.10 PER TICKET, charged in the LISTING's currency —
//! Ralph chose the 10¢-equivalent over USD-only and over 10-minor-units, and re-affirmed
//! it against the outside voice: ₩10 ≈ 0.7¢ would have cut the fee 93% at the Seoul
//! launch. So the box office needs an FX answer, and the answer must NEVER block a sale:
//!
//! * `rates.json` is COMMITTED beside this file and compiled in (`include_str!`) — the
//!   last-known table always serves; a rates outage cannot block checkout.
//! * A daily best-effort refresh updates it in deployment (stub below — wire later);
//!   staleness beyond 7 days logs loud and keeps serving.
//! * Rounding is HALF-UP with a floor of 1 minor unit; zero-decimal currencies (JPY,
//!   KRW, VND, …) have no cents — their minor unit IS the whole unit, so the fee is
//!   round(0.10 × rate) floored at 1 whole unit, never ×100.
//! * An UNKNOWN currency returns `None` and checkout refuses loudly — a guessed fee is
//!   worse than a refused sale.

use std::collections::HashMap;
use std::sync::OnceLock;

#[derive(serde::Deserialize)]
struct RateTable {
    /// The table's vintage (YYYY-MM-DD) — the thing `staleness_days` judges.
    as_of: String,
    /// ISO 4217 code (lowercase) → currency units per USD.
    rates: HashMap<String, f64>,
}

static TABLE: OnceLock<RateTable> = OnceLock::new();

fn table() -> &'static RateTable {
    TABLE.get_or_init(|| {
        serde_json::from_str(include_str!("rates.json"))
            .expect("rates.json is committed beside rates.rs and must parse")
    })
}

/// Stripe's zero-decimal currencies: the minor unit IS the whole unit (no cents).
fn zero_decimal(code: &str) -> bool {
    matches!(
        code,
        "bif"
            | "clp"
            | "djf"
            | "gnf"
            | "jpy"
            | "kmf"
            | "krw"
            | "mga"
            | "pyg"
            | "rwf"
            | "ugx"
            | "vnd"
            | "vuv"
            | "xaf"
            | "xof"
            | "xpf"
    )
}

/// Half-up: 14.5 → 15, 14.4 → 14. Fees are tiny positive numbers; no negative branch.
fn half_up(x: f64) -> i64 {
    (x + 0.5).floor() as i64
}

/// The 10¢-equivalent in `currency`, in MINOR units — the per-ticket application fee.
/// `None` means the table does not know this currency, and the caller must refuse
/// loudly, never guess.
pub fn fee_minor(currency: &str) -> Option<i64> {
    let code = currency.to_ascii_lowercase();
    let rate = *table().rates.get(&code)?;
    let units = 0.10_f64 * rate;
    let minor = if zero_decimal(&code) {
        units
    } else {
        units * 100.0
    };
    Some(half_up(minor).max(1))
}

/// Days between the committed table's vintage and `now_ms`. Above 7, log loud (the
/// serving decision does not change — last-known always serves).
pub fn staleness_days(now_ms: i64) -> i64 {
    now_ms / 86_400_000 - as_of_days()
}

/// The table vintage as days-since-epoch. Hand-rolled civil-date arithmetic (Hinnant's
/// days-from-civil) rather than pulling a date crate in for one subtraction.
fn as_of_days() -> i64 {
    let mut parts = table()
        .as_of
        .split('-')
        .map(|p| p.parse::<i64>().unwrap_or(0));
    let (y, m, d) = (
        parts.next().unwrap_or(1970),
        parts.next().unwrap_or(1),
        parts.next().unwrap_or(1),
    );
    days_from_civil(y, m, d)
}

fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

/// The daily best-effort refresh — A STUB, deliberately. The job fetches a fresh table,
/// rewrites the deployed rates.json, and logs loud once `staleness_days` passes 7; it
/// must stay BEST-EFFORT (a fetch failure keeps serving last-known — an FX outage can
/// never block a sale). Wire it into the plane's runtime later; committing the table
/// keeps every build serving plausible rates in the meantime.
#[allow(dead_code)] // deliberately unwired — see above
pub fn refresh_daily_stub() {}

#[cfg(test)]
mod tests {
    use super::*;

    /// USD is the identity: $0.10 is exactly 10 cents.
    #[test]
    fn usd_is_ten_cents() {
        assert_eq!(fee_minor("usd"), Some(10));
        assert_eq!(fee_minor("USD"), Some(10), "codes are case-insensitive");
    }

    /// KRW is zero-decimal: the fee is ~₩135 WHOLE units — never ×100 (₩13,500 would be
    /// a 100× overcharge, the exact bug the zero-decimal branch exists to prevent).
    #[test]
    fn krw_is_whole_won() {
        let fee = fee_minor("krw").unwrap();
        assert!((130..=150).contains(&fee), "krw fee {fee} outside ₩130–150");
    }

    /// JPY zero-decimal: ~¥15.
    #[test]
    fn jpy_is_whole_yen() {
        let fee = fee_minor("jpy").unwrap();
        assert!((14..=16).contains(&fee), "jpy fee {fee} outside ¥14–16");
    }

    /// GBP is decimal: ~8 pence.
    #[test]
    fn gbp_is_pence() {
        let fee = fee_minor("gbp").unwrap();
        assert!((7..=9).contains(&fee), "gbp fee {fee} outside 7–9p");
    }

    /// PROPERTY: every committed currency yields at least 1 minor unit — the floor means
    /// no listing currency can ever make the fee round to zero.
    #[test]
    fn every_currency_yields_at_least_one_minor_unit() {
        let t = table();
        assert!(t.rates.len() >= 30, "the table commits ~30 currencies");
        for code in t.rates.keys() {
            let fee = fee_minor(code).unwrap_or_else(|| panic!("{code} must resolve"));
            assert!(fee >= 1, "{code} fee {fee} below the 1-minor-unit floor");
        }
    }

    /// An unknown currency is a refusal, not a guess.
    #[test]
    fn unknown_currency_is_none() {
        assert_eq!(fee_minor("xxx"), None);
        assert_eq!(fee_minor(""), None);
        assert_eq!(fee_minor("btc"), None);
    }

    /// Half-up is half-UP: exactly .5 rounds away from zero.
    #[test]
    fn rounding_is_half_up_with_floor() {
        assert_eq!(half_up(14.5), 15);
        assert_eq!(half_up(14.49), 14);
        assert_eq!(
            half_up(0.4),
            0,
            "the .max(1) floor lives in fee_minor, not half_up"
        );
    }

    /// Staleness counts days from the committed vintage, whatever it currently is.
    #[test]
    fn staleness_counts_days_from_the_vintage() {
        let base = as_of_days() * 86_400_000;
        assert_eq!(staleness_days(base), 0);
        assert_eq!(staleness_days(base + 3 * 86_400_000), 3);
        assert!(
            staleness_days(base + 8 * 86_400_000) > 7,
            "the loud-log fence"
        );
    }
}
