// Sets LOGIX_ENGINE_VERSION, the engine version reported by get_engine_info
// and --version. Release builds pass it in the LOGIX_ENGINE_VERSION env var
// (the release tag, yyyy-mm-dd or yyyy-mm-dd-N). Other builds use today's
// UTC date.

use std::time::{SystemTime, UNIX_EPOCH};

fn main() {
    println!("cargo:rerun-if-env-changed=LOGIX_ENGINE_VERSION");
    let version = match std::env::var("LOGIX_ENGINE_VERSION") {
        Ok(v) if !v.is_empty() => {
            if !is_release_tag(&v) {
                panic!("LOGIX_ENGINE_VERSION must be yyyy-mm-dd or yyyy-mm-dd-N, got {v:?}");
            }
            v
        }
        _ => today_utc(),
    };
    println!("cargo:rustc-env=LOGIX_ENGINE_VERSION={version}");
}

fn is_release_tag(v: &str) -> bool {
    let (date, suffix) = match v.get(10..) {
        Some("") => (v, None),
        Some(rest) => match rest.strip_prefix('-') {
            Some(n) => (&v[..10], Some(n)),
            None => return false,
        },
        None => return false,
    };
    let b = date.as_bytes();
    let date_ok = b.len() == 10
        && b[4] == b'-'
        && b[7] == b'-'
        && b.iter()
            .enumerate()
            .all(|(i, c)| i == 4 || i == 7 || c.is_ascii_digit());
    let suffix_ok = suffix.is_none_or(|n| {
        !n.is_empty() && !n.starts_with('0') && n.bytes().all(|c| c.is_ascii_digit())
    });
    date_ok && suffix_ok
}

fn today_utc() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock before 1970")
        .as_secs();
    let (y, m, d) = civil_from_days((secs / 86_400) as i64);
    format!("{y:04}-{m:02}-{d:02}")
}

// Days since 1970-01-01 to a calendar date (Howard Hinnant's algorithm).
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = yoe + era * 400 + i64::from(m <= 2);
    (y, m, d)
}
