//! License-key gate ported verbatim from `yfs_key.h` of the C
//! implementation: `-y KEY` must match one of the built-in keys and that
//! key must not have expired.

use std::time::{SystemTime, UNIX_EPOCH};

const KEYS: [&str; 10] = [
    "FADEP8", "XTPV63", "23AGFQ", "CA89CS", "CDS87L", "CD34VC", "2APGFQ", "XS420X", "1409XT",
    "SD76RD",
];

/// The key a mount uses when `-y` is not given: always the first entry of
/// `KEYS`, so the default cannot drift out of sync with the table.
pub const DEFAULT_KEY: &str = KEYS[0];

/// Expiry of each key, as days since the unix epoch parsed from
/// `"%Y-%m-%d %H:%M:%S"` strings in the C version (local time there; the
/// date boundaries involved make the distinction immaterial here).
const DATES: [&str; 10] = [
    "2029-01-01 00:00:00",
    "2028-01-01 00:00:00",
    "2029-01-01 00:00:00",
    "2030-01-01 00:00:00",
    "2031-01-01 00:00:00",
    "2032-01-01 00:00:00",
    "2033-01-01 00:00:00",
    "2034-01-01 00:00:00",
    "2035-01-01 00:00:00",
    "2099-01-01 00:00:00",
];

/// Days from 1970-01-01 to year-0000-01-01 based civil-date algorithm
/// (Howard Hinnant's days_from_civil), so we do not need a date crate.
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

fn string_to_time(s: &str) -> Option<i64> {
    let (date, time) = s.split_once(' ')?;
    let mut it = date.split('-').map(|p| p.parse::<i64>().ok());
    let year = it.next()??;
    let mon = it.next()??;
    let day = it.next()??;
    let mut it = time.split(':').map(|p| p.parse::<i64>().ok());
    let h = it.next()??;
    let mi = it.next()??;
    let sec = it.next()??;
    Some(days_from_civil(year, mon, day) * 86400 + h * 3600 + mi * 60 + sec)
}

/// `key_is_good()` from the C version.
pub fn key_is_good(key: &str) -> bool {
    let idx = match KEYS.iter().position(|&k| k == key) {
        Some(i) => i,
        None => return false,
    };
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let expiry = string_to_time(DATES[idx]).unwrap_or(0);
    now <= expiry
}

#[cfg(test)]
mod tests {
    use super::{days_from_civil, key_is_good, string_to_time, DEFAULT_KEY};

    #[test]
    fn days_from_civil_known_epoch_days() {
        // guards the `if .. else .. / 400` precedence: if the division ever
        // binds only to the else block these values go wildly wrong
        assert_eq!(days_from_civil(1970, 1, 1), 0);
        assert_eq!(days_from_civil(2000, 3, 1), 11017);
        assert_eq!(days_from_civil(2027, 1, 1), 20819);
    }

    #[test]
    fn expiry_values() {
        assert_eq!(string_to_time("2027-01-01 00:00:00"), Some(20819 * 86400));
        assert_eq!(string_to_time("1970-01-01 00:00:00"), Some(0));
        assert_eq!(string_to_time("garbage"), None);
        // first key (also the default) valid until 2029, unknown keys never valid
        assert!(key_is_good("FADEP8"));
        assert_eq!(DEFAULT_KEY, "FADEP8");
        assert!(key_is_good(DEFAULT_KEY));
        assert!(!key_is_good("NOPE"));
    }
}
