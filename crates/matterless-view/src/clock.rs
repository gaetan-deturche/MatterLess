//! What time it is where the reader is.
//!
//! Every timestamp the store holds is UTC milliseconds, and every clock and
//! date separator has to be drawn in local time. The app asks the webview,
//! which knows; a native window has to ask the system, because std does not
//! model a timezone at all.

/// Minutes to add to UTC to get the reader's local time.
///
/// Positive east of Greenwich, matching what a browser's
/// `-getTimezoneOffset()` reports and what `PlanOptions` expects. Answers zero
/// wherever the system cannot be asked: UTC is a wrong clock but a coherent
/// one, and it is what the window drew before it asked at all.
pub fn utc_offset_minutes() -> i32 {
    offset().unwrap_or(0)
}

#[cfg(windows)]
fn offset() -> Option<i32> {
    use windows::Win32::System::Time::{GetTimeZoneInformation, TIME_ZONE_INFORMATION};
    /// What `GetTimeZoneInformation` answers. Unknown means the zone has no
    /// seasonal change, not that the call failed; invalid is the failure.
    const UNKNOWN: u32 = 0;
    const STANDARD: u32 = 1;
    const DAYLIGHT: u32 = 2;

    let mut zone = TIME_ZONE_INFORMATION::default();
    // The seasonal bias is separate from the standing one, and only the
    // returned id says which of them is in force right now.
    let extra = match unsafe { GetTimeZoneInformation(&mut zone) } {
        UNKNOWN => 0,
        STANDARD => zone.StandardBias,
        DAYLIGHT => zone.DaylightBias,
        _ => return None,
    };
    // Windows states the bias as UTC = local + bias, so the sign flips here.
    Some(-(zone.Bias + extra))
}

#[cfg(not(windows))]
fn offset() -> Option<i32> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Whatever the machine says, it has to be a real offset: the extremes in
    /// use are UTC-12 and UTC+14, and a nonsense value here would silently move
    /// every message to the wrong day.
    #[test]
    fn the_offset_is_one_a_timezone_could_have() {
        let minutes = utc_offset_minutes();
        assert!(
            (-12 * 60..=14 * 60).contains(&minutes),
            "{minutes} is not an offset any timezone uses"
        );
        // Every real zone is a whole number of quarter hours.
        assert_eq!(minutes % 15, 0, "{minutes} is not a quarter-hour offset");
    }
}
