//! Compact operational notifications. Full evidence remains in the durable outbox.
//! Discord does not promise deduplication of our Idempotency-Key header.
use serde_json::{Value, json};

pub(super) fn payload(event: &Value, dedup: &str) -> Value {
    let mut lines = vec!["Sui Sentinel — cached diagnostics".to_owned()];
    for (label, value) in [
        ("Transition", &event["transition"]),
        ("Scope", &event["scope"]),
        ("Incident", &event["incident_id"]),
        ("Rule", &event["incident"]["rule_id"]),
        ("Severity", &event["incident"]["severity"]),
        ("Checkpoint", &event["incident"]["checkpoint_sequence"]),
    ] {
        let text = value
            .as_str()
            .map(str::to_owned)
            .unwrap_or_else(|| value.to_string());
        // Bound Unicode code units too, and prevent formatting/control injection.
        let text: String = text
            .chars()
            .filter(|c| !c.is_control() && !matches!(c, '`' | '*' | '@'))
            .take(100)
            .collect();
        lines.push(format!("{label}: {text}"));
    }
    lines.push(format!(
        "Delivery ID: {}",
        dedup.chars().take(64).collect::<String>()
    ));
    json!({"content":lines.join("\n"),"allowed_mentions":{"parse":[]}})
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bounded_notification_disables_mentions_and_omits_private_evidence() {
        let event = json!({"scope":"🦀".repeat(5000),"transition":"opened @everyone","incident":{"evidence":{"url":"secret"},"rule_id":"freshness"}});
        let body = payload(&event, "abc");
        assert!(body["content"].as_str().unwrap().encode_utf16().count() < 2000);
        assert_eq!(body["allowed_mentions"]["parse"], json!([]));
        assert!(!body.to_string().contains("secret"));
        assert!(
            body["content"]
                .as_str()
                .unwrap()
                .contains("Delivery ID: abc")
        );
    }
}
