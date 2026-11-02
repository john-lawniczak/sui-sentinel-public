//! Exporter self-health metrics (temp.md's Monitor Operations dashboard
//! row: "Is Sentinel functioning?"), kept separate from the report-content
//! metrics in `export::metrics` - these answer whether the exporter itself
//! is loading generations, not what the latest loaded report says. A pure
//! function over plain values so it can be unit-tested without touching
//! `Registry`'s lock or filesystem access.

use time::OffsetDateTime;

use super::super::metrics::{family, sample};

/// Missing data is represented by omitting a sample, never by a fabricated
/// `0` or `-1` value - a scraper sees the metric family's `HELP`/`TYPE`
/// header either way, but no series until there is a real observation.
pub(super) fn render(
    ready: bool,
    generation_loaded_at: Option<OffsetDateTime>,
    last_reload_attempted_at: Option<OffsetDateTime>,
    last_reload_failed: Option<bool>,
) -> String {
    let mut out = String::new();
    family(
        &mut out,
        "exporter_ready",
        "Whether the current generation satisfies observation freshness, coverage and scope policy.",
        [sample([], u64::from(ready))],
    );
    family(
        &mut out,
        "exporter_generation_loaded_at_seconds",
        "Unix time the currently-served generation was loaded; absent until the first load.",
        generation_loaded_at
            .map(|t| sample([], t.unix_timestamp()))
            .into_iter()
            .collect::<Vec<_>>(),
    );
    family(
        &mut out,
        "exporter_last_reload_attempt_seconds",
        "Unix time of the most recent reload attempt (success, no-op, or failure); absent before the first attempt.",
        last_reload_attempted_at
            .map(|t| sample([], t.unix_timestamp()))
            .into_iter()
            .collect::<Vec<_>>(),
    );
    family(
        &mut out,
        "exporter_last_reload_failed",
        "1 if the most recent reload attempt failed (the last good generation is still being served); 0 if it succeeded or found nothing new; absent before the first attempt.",
        last_reload_failed
            .map(|failed| sample([], u64::from(failed)))
            .into_iter()
            .collect::<Vec<_>>(),
    );
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn has_sample_line(text: &str, metric: &str, value: &str) -> bool {
        text.lines().any(|line| line == format!("{metric} {value}"))
    }

    fn has_any_sample_line(text: &str, metric: &str) -> bool {
        text.lines()
            .any(|line| !line.starts_with('#') && line.starts_with(metric))
    }

    #[test]
    fn omits_samples_before_any_reload_attempt() {
        let text = render(false, None, None, None);
        assert!(has_sample_line(&text, "sui_sentinel_exporter_ready", "0"));
        assert!(!has_any_sample_line(
            &text,
            "sui_sentinel_exporter_last_reload_attempt_seconds"
        ));
        assert!(!has_any_sample_line(
            &text,
            "sui_sentinel_exporter_last_reload_failed"
        ));
        assert!(!has_any_sample_line(
            &text,
            "sui_sentinel_exporter_generation_loaded_at_seconds"
        ));
    }

    #[test]
    fn reports_readiness_and_last_reload_outcome_independently() {
        let now = OffsetDateTime::now_utc();

        // An attempt happened but nothing ever loaded (e.g. pointer missing
        // every time so far): not ready, but the attempt itself is visible.
        let text = render(false, None, Some(now), Some(false));
        assert!(has_sample_line(&text, "sui_sentinel_exporter_ready", "0"));
        assert!(has_sample_line(
            &text,
            "sui_sentinel_exporter_last_reload_failed",
            "0"
        ));
        assert!(has_any_sample_line(
            &text,
            "sui_sentinel_exporter_last_reload_attempt_seconds"
        ));

        // Loaded once: ready, with a load timestamp.
        let text = render(true, Some(now), Some(now), Some(false));
        assert!(has_sample_line(&text, "sui_sentinel_exporter_ready", "1"));
        assert!(has_any_sample_line(
            &text,
            "sui_sentinel_exporter_generation_loaded_at_seconds"
        ));
        assert!(has_sample_line(
            &text,
            "sui_sentinel_exporter_last_reload_failed",
            "0"
        ));

        // The caller degrades readiness on reload failure while retaining load history.
        let text = render(false, Some(now), Some(now), Some(true));
        assert!(
            has_sample_line(&text, "sui_sentinel_exporter_ready", "0"),
            "degraded readiness must remain visible while historical data is served"
        );
        assert!(has_sample_line(
            &text,
            "sui_sentinel_exporter_last_reload_failed",
            "1"
        ));
    }
}
