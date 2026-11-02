//! One bounded webhook attempt per call. Durable leases survive worker crashes;
//! delivery is at least once, so receivers must deduplicate Idempotency-Key.
use super::Store;
use anyhow::{Result, ensure};
use serde_json::Value;
use std::time::Duration;

pub struct Webhook {
    client: reqwest::Client,
    url: reqwest::Url,
    token: Option<String>,
    discord: bool,
}
#[derive(sqlx::FromRow)]
struct Claimed {
    id: i64,
    dedup_id: String,
    payload: Value,
    attempts: i32,
    lease_token: i64,
}
impl Webhook {
    pub fn new(url: &str, token: Option<String>) -> Result<Self> {
        Self::with_format(url, token, "generic")
    }

    pub fn with_format(url: &str, token: Option<String>, format: &str) -> Result<Self> {
        ensure!(
            matches!(format, "generic" | "discord"),
            "unsupported webhook format"
        );
        let discord = format == "discord";
        let mut url =
            reqwest::Url::parse(url).map_err(|_| anyhow::anyhow!("invalid webhook URL"))?;
        if discord {
            let pairs: Vec<_> = url
                .query_pairs()
                .filter(|(k, _)| k != "wait")
                .map(|(k, v)| (k.into_owned(), v.into_owned()))
                .collect();
            url.set_query(None);
            url.query_pairs_mut()
                .extend_pairs(pairs)
                .append_pair("wait", "true");
        }
        ensure!(
            matches!(url.scheme(), "https" | "http")
                && url.username().is_empty()
                && url.password().is_none(),
            "invalid webhook URL"
        );
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .connect_timeout(Duration::from_secs(3))
            .redirect(reqwest::redirect::Policy::none())
            .build()?;
        Ok(Self {
            client,
            url,
            token,
            discord,
        })
    }

    /// Returns false when no due work exists. A failed delivery is persisted,
    /// not an error to the loop; database failures propagate for supervision.
    pub async fn deliver_one(&self, store: &Store, scope: &str) -> Result<bool> {
        sqlx::query("UPDATE outbox SET dead_lettered_at=now(),last_error='delivery budget exhausted after expired lease'
            WHERE attempts>=10 AND delivered_at IS NULL AND dead_lettered_at IS NULL
            AND (lease_until IS NULL OR lease_until<now()) AND incident_id IN (SELECT id FROM incidents WHERE scope=$1)")
            .bind(scope).execute(&store.pool).await?;
        let entry: Option<Claimed> = sqlx::query_as(
            "UPDATE outbox SET lease_until=now()+interval '30 seconds',lease_token=lease_token+1,
             attempts=attempts+1,last_attempt_at=now()
             WHERE id=(SELECT o.id FROM outbox o JOIN incidents i ON i.id=o.incident_id
               WHERE i.scope=$1 AND o.delivered_at IS NULL AND o.dead_lettered_at IS NULL
               AND o.attempts<10 AND NOT EXISTS (SELECT 1 FROM outbox earlier
                 WHERE earlier.incident_id=o.incident_id AND earlier.id<o.id
                 AND earlier.delivered_at IS NULL AND earlier.dead_lettered_at IS NULL)
               AND o.next_attempt_at<=now() AND (o.lease_until IS NULL OR o.lease_until<now())
               ORDER BY o.id FOR UPDATE OF o SKIP LOCKED LIMIT 1)
             RETURNING id,dedup_id,payload,attempts,lease_token",
        )
        .bind(scope)
        .fetch_optional(&store.pool)
        .await?;
        let Some(entry) = entry else { return Ok(false) };
        let payload = if self.discord {
            super::discord::payload(&entry.payload, &entry.dedup_id)
        } else {
            entry.payload.clone()
        };
        let mut request = self
            .client
            .post(self.url.clone())
            .header("Idempotency-Key", &entry.dedup_id)
            .json(&payload);
        if let Some(token) = &self.token {
            request = request.bearer_auth(token);
        }
        let mut retry_after = None;
        let failure = match request.send().await {
            Ok(response) if response.status().is_success() => None,
            Ok(response) => {
                if response.status() == reqwest::StatusCode::TOO_MANY_REQUESTS {
                    retry_after = response
                        .headers()
                        .get(reqwest::header::RETRY_AFTER)
                        .and_then(|h| h.to_str().ok())
                        .and_then(|s| s.parse::<f64>().ok())
                        .filter(|n| n.is_finite() && *n >= 0.0)
                        .map(|n| n.ceil().clamp(1.0, 86400.0));
                }
                Some(format!("webhook HTTP {}", response.status().as_u16()))
            }
            Err(_) => Some("webhook transport error or deadline".to_owned()), // Do not persist URL/credentials.
        };
        if let Some(error) = failure {
            let seconds =
                (2_i64.pow(entry.attempts.clamp(1, 8) as u32) + entry.id.rem_euclid(7)).min(300);
            sqlx::query(
                "UPDATE outbox SET last_error=$3,lease_until=NULL,
                next_attempt_at=now()+make_interval(secs => $4),
                dead_lettered_at=CASE WHEN attempts>=10 THEN now() ELSE NULL END
                WHERE id=$1 AND lease_token=$2 AND delivered_at IS NULL",
            )
            .bind(entry.id)
            .bind(entry.lease_token)
            .bind(error)
            .bind(retry_after.map_or(seconds as f64, |retry| retry.max(seconds as f64)))
            .execute(&store.pool)
            .await?;
        } else {
            sqlx::query(
                "UPDATE outbox SET delivered_at=now(),lease_until=NULL,last_error=NULL
                WHERE id=$1 AND lease_token=$2 AND delivered_at IS NULL",
            )
            .bind(entry.id)
            .bind(entry.lease_token)
            .execute(&store.pool)
            .await?;
        }
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        diagnosis::{rules, test_support},
        runtime::{RunRecord, State},
    };
    use axum::{Router, http::StatusCode, routing::post};
    #[test]
    fn discord_waits_for_confirmation_and_never_reports_secret_urls() {
        let hook = Webhook::with_format(
            "https://discord.com/api/webhooks/test/secret?wait=false&thread_id=123",
            None,
            "discord",
        )
        .unwrap();
        assert_eq!(
            hook.url
                .query_pairs()
                .filter(|(k, _)| k == "wait")
                .collect::<Vec<_>>(),
            vec![("wait".into(), "true".into())]
        );
        assert!(
            hook.url
                .query_pairs()
                .any(|(k, v)| k == "thread_id" && v == "123")
        );
        assert!(Webhook::with_format("https://test", None, "invalid").is_err());
        let error = Webhook::new("secret malformed url", None)
            .err()
            .unwrap()
            .to_string();
        assert!(!error.contains("secret"));
    }
    #[tokio::test]
    async fn failed_send_backs_off_then_delivers_with_a_stable_idempotency_key() {
        let Some(store) = test_support::store().await else {
            return;
        };
        let scope = format!("webhook-{}", test_support::unique_suffix());
        let mut run = RunRecord::start(std::path::Path::new("/delivery/test"));
        run.finish_failed("test", &anyhow::anyhow!("failure"));
        rules::apply_capture_outcome(&store, &scope, &run)
            .await
            .unwrap();
        let calls = std::sync::Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
        let seen = calls.clone();
        let app = Router::new().route(
            "/",
            post(move |headers: axum::http::HeaderMap| {
                let seen = seen.clone();
                async move {
                    let mut seen = seen.lock().unwrap();
                    seen.push(headers["Idempotency-Key"].to_str().unwrap().into());
                    if seen.len() == 1 {
                        StatusCode::SERVICE_UNAVAILABLE
                    } else {
                        StatusCode::OK
                    }
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let webhook = Webhook::new(&format!("http://{addr}/"), None).unwrap();
        assert!(webhook.deliver_one(&store, &scope).await.unwrap());
        assert!(!webhook.deliver_one(&store, &scope).await.unwrap());
        sqlx::query("UPDATE outbox SET next_attempt_at=now() WHERE incident_id IN (SELECT id FROM incidents WHERE scope=$1)").bind(&scope).execute(&store.pool).await.unwrap();
        let (a, b) = tokio::join!(
            webhook.deliver_one(&store, &scope),
            webhook.deliver_one(&store, &scope)
        );
        assert_ne!(a.unwrap(), b.unwrap());
        assert_eq!(calls.lock().unwrap().len(), 2);
        {
            let seen = calls.lock().unwrap();
            assert_eq!(seen[0], seen[1]);
        }
        let mut recovery = RunRecord::start(std::path::Path::new("/delivery/recovered"));
        recovery.finish_ok(State::Published);
        rules::apply_capture_outcome(&store, &scope, &recovery)
            .await
            .unwrap();
        // An expired lease is reclaimed with a new token after worker loss.
        sqlx::query("UPDATE outbox SET lease_until=now()-interval '1 second',lease_token=5 WHERE delivered_at IS NULL AND incident_id IN (SELECT id FROM incidents WHERE scope=$1)")
            .bind(&scope).execute(&store.pool).await.unwrap();
        assert!(webhook.deliver_one(&store, &scope).await.unwrap());
        server.abort();
    }
}
