//! The MS-DOS date and time fields of a zip entry, made from a moment in UTC (the readers take the fields as UTC too).

/// 1980-01-01, the first date the fields can say, and the last moment they can: 2107-12-31 23:59:58.
const FIRST_DATE: u16 = 1 << 5 | 1;
const LAST_DATE: u16 = 127 << 9 | 12 << 5 | 31;
const LAST_TIME: u16 = 23 << 11 | 59 << 5 | 29;

/// The `(date, time)` fields for `modified` (milliseconds since the Unix epoch); a moment outside 1980 to 2107, and no
/// moment at all, give the nearest the fields can say, 1980-01-01 00:00 for none.
pub fn fields(modified: Option<i64>) -> (u16, u16) {
    let Some(milliseconds) = modified else {
        return (FIRST_DATE, 0);
    };

    let seconds = milliseconds.div_euclid(1000);
    let (year, month, day) = civil_from_days(seconds.div_euclid(86_400));
    let in_day = seconds.rem_euclid(86_400);

    if year < 1980 {
        return (FIRST_DATE, 0);
    }

    if year > 2107 {
        return (LAST_DATE, LAST_TIME);
    }

    // The ranges are checked above and by the calendar: every value fits its field.
    let date = ((year - 1980) << 9) | (month << 5) | day;
    let time = ((in_day / 3600) << 11) | ((in_day % 3600 / 60) << 5) | (in_day % 60 / 2);

    (date as u16, time as u16)
}

/// The `(year, month, day)` of the proleptic Gregorian calendar `days` days after 1970-01-01.
fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let shifted = days + 719_468;
    let era = shifted.div_euclid(146_097);
    let day_of_era = shifted.rem_euclid(146_097);
    let year_of_era = (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_index + 2) / 5 + 1;
    let month = if month_index < 10 {
        month_index + 3
    } else {
        month_index - 9
    };
    let year = year_of_era + era * 400 + i64::from(month <= 2);

    (year, month, day)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::archive::testing::{DOS_DATE, DOS_TIME, MOMENT};

    #[test]
    fn a_moment_is_written_as_the_dos_fields_of_its_utc_time() {
        assert_eq!(fields(Some(MOMENT)), (DOS_DATE, DOS_TIME));
        // Leap day, and the odd second rounds down to the even one.
        assert_eq!(fields(Some(1_709_164_801_000)), (44 << 9 | 2 << 5 | 29, 0));
    }

    #[test]
    fn moments_the_fields_cannot_say_give_the_nearest_they_can() {
        assert_eq!(fields(None), (33, 0));
        assert_eq!(fields(Some(0)), (33, 0));
        assert_eq!(fields(Some(-86_400_000)), (33, 0));
        assert_eq!(fields(Some(5_000_000_000_000)), (LAST_DATE, LAST_TIME));
    }
}
