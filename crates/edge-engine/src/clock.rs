/// Now, as the first line of a G-code file gives it: `2026-11-10 at
/// 07:21:37`, in UTC. `SOURCE_DATE_EPOCH`, in seconds, stands in for the
/// clock when set, so the same input can be made to give the same file.
pub fn timestamp() -> String {
    let seconds = std::env::var("SOURCE_DATE_EPOCH")
        .ok()
        .and_then(|epoch| epoch.trim().parse::<u64>().ok())
        .unwrap_or_else(|| {
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |since| since.as_secs())
        });
    stamp(seconds)
}

/// A count of seconds since 1970 as a date and time of day.
fn stamp(seconds: u64) -> String {
    let (days, of_day) = (seconds / 86_400, seconds % 86_400);
    // Days since 1970 to a calendar date (Howard Hinnant's `civil_from_days`).
    let shifted = days as i64 + 719_468;
    let era = shifted.div_euclid(146_097);
    let day_of_era = shifted.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let shifted_month = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * shifted_month + 2) / 5 + 1;
    let month = if shifted_month < 10 {
        shifted_month + 3
    } else {
        shifted_month - 9
    };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02} at {:02}:{:02}:{:02}",
        of_day / 3_600,
        of_day / 60 % 60,
        of_day % 60
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seconds_since_1970_are_written_as_a_date_and_time() {
        assert_eq!(stamp(0), "1970-01-01 at 00:00:00");
        assert_eq!(stamp(1_709_164_800), "2024-02-29 at 00:00:00");
        assert_eq!(stamp(1_794_295_297), "2026-11-10 at 07:21:37");
    }
}
