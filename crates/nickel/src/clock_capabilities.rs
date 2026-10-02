//! Public local wallclock service. Packages own formatting and presentation.
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub(crate) fn snapshot() -> serde_json::Value {
    let local = jiff::Zoned::now();
    let milliseconds = local.timestamp().as_millisecond();
    serde_json::json!({"unixMilliseconds":milliseconds.div_euclid(60_000) * 60_000,
        "utcOffsetMinutes":local.offset().seconds() / 60})
}

pub(crate) fn until_next_minute() -> Duration {
    let elapsed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    Duration::from_secs(60)
        .saturating_sub(Duration::new(
            elapsed.as_secs() % 60,
            elapsed.subsec_nanos(),
        ))
        .max(Duration::from_millis(1))
}
