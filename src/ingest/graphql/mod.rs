//! Checkpoint-scoped GraphQL reads with bounded requests and transient retries.

mod telemetry;
use std::sync::atomic::Ordering::Relaxed;
pub use telemetry::Transport;

use anyhow::{Context, Result, bail, ensure};
use reqwest::{Client, StatusCode, Url};
use serde_json::{Value, json};
use std::{collections::HashSet, sync::Arc, time::Duration};
use tokio::sync::Semaphore;

use crate::{
    config::MAINNET_CHAIN,
    model::{Checkpoint, Object, integer},
};

const OBJECT_FIELDS: &str = "address version digest owner { __typename ... on AddressOwner { address { address } } ... on ObjectOwner { address { address } } ... on ConsensusAddressOwner { address { address } } } asMoveObject { contents { type { repr } json } }";

#[derive(Clone)]
pub struct Graphql {
    client: Client,
    endpoint: Url,
    slots: Arc<Semaphore>,
    counters: Arc<telemetry::Counters>,
}

impl Graphql {
    pub fn new(endpoint: &str, api_key: Option<&str>, concurrency: usize) -> Result<Self> {
        Self::with_auth_header(
            endpoint,
            api_key,
            concurrency,
            &std::env::var("SUI_API_KEY_HEADER").unwrap_or_else(|_| "authorization".into()),
        )
    }

    pub fn with_auth_header(
        endpoint: &str,
        api_key: Option<&str>,
        concurrency: usize,
        header_name: &str,
    ) -> Result<Self> {
        ensure!(
            (1..=32).contains(&concurrency),
            "concurrency must be 1..=32"
        );
        let endpoint = Url::parse(endpoint).context("invalid GraphQL endpoint")?;
        ensure!(
            endpoint.scheme() == "https"
                || (endpoint.scheme() == "http"
                    && matches!(
                        endpoint.host_str(),
                        Some("127.0.0.1" | "localhost" | "[::1]")
                    )),
            "GraphQL requires HTTPS except on localhost"
        );
        ensure!(
            endpoint.username().is_empty() && endpoint.password().is_none(),
            "use SUI_API_KEY rather than URL credentials"
        );
        let mut headers = reqwest::header::HeaderMap::new();
        if let Some(key) = api_key.filter(|key| !key.is_empty()) {
            let name = reqwest::header::HeaderName::from_bytes(header_name.as_bytes())
                .context("invalid API key header name")?;
            ensure!(
                matches!(name.as_str(), "authorization" | "x-token" | "x-api-key"),
                "unsupported API key header name"
            );
            let value = if name == reqwest::header::AUTHORIZATION {
                format!("Bearer {key}")
            } else {
                key.to_owned()
            };
            let mut header = value
                .parse::<reqwest::header::HeaderValue>()
                .context("invalid API key header")?;
            header.set_sensitive(true);
            headers.insert(name, header);
        }
        let client = Client::builder()
            .default_headers(headers)
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(45))
            .redirect(reqwest::redirect::Policy::none())
            .build()?;
        Ok(Self {
            client,
            endpoint,
            slots: Arc::new(Semaphore::new(concurrency)),
            counters: Arc::new(telemetry::Counters::default()),
        })
    }

    pub fn transport(&self) -> Transport {
        self.counters.snapshot()
    }

    pub async fn request(&self, query: &str, variables: Value) -> Result<Value> {
        const MAX_RESPONSE_BYTES: usize = 64 * 1024 * 1024;
        for attempt in 0..4 {
            let permit = self.slots.acquire().await?;
            self.counters.attempts.fetch_add(1, Relaxed);
            if attempt > 0 {
                self.counters.retries.fetch_add(1, Relaxed);
            }
            let mut retry_after = None;
            let result: Result<Value> = async {
                let mut response = self
                    .client
                    .post(self.endpoint.clone())
                    .json(&json!({"query":query,"variables":variables}))
                    .send()
                    .await
                    .map_err(|e| e.without_url())?;
                if response.status() == StatusCode::TOO_MANY_REQUESTS {
                    self.counters.throttled.fetch_add(1, Relaxed);
                }
                retry_after = response
                    .headers()
                    .get(reqwest::header::RETRY_AFTER)
                    .and_then(|h| h.to_str().ok())
                    .and_then(|s| s.parse::<u64>().ok());
                response
                    .error_for_status_ref()
                    .map_err(|e| e.without_url())?;
                ensure!(
                    response.status().is_success(),
                    "unexpected GraphQL redirect"
                );
                let mut bytes = Vec::new();
                while let Some(chunk) = response.chunk().await.map_err(|e| e.without_url())? {
                    ensure!(
                        chunk.len() <= MAX_RESPONSE_BYTES - bytes.len(),
                        "GraphQL response exceeds 64 MiB limit"
                    );
                    self.counters
                        .response_bytes
                        .fetch_add(chunk.len() as u64, Relaxed);
                    bytes.extend_from_slice(&chunk);
                }
                let mut body: Value =
                    serde_json::from_slice(&bytes).context("invalid GraphQL JSON")?;
                // A GraphQL error can accompany partial data. Never ingest that data.
                ensure!(
                    body.get("errors")
                        .is_none_or(|v| v.as_array().is_some_and(Vec::is_empty)),
                    // Provider errors may reflect authentication headers/URLs.
                    // Keep untrusted response text out of logs and incident evidence.
                    "GraphQL returned errors; partial data rejected"
                );
                ensure!(
                    body.get("data").is_some_and(|v| !v.is_null()),
                    "missing GraphQL data"
                );
                Ok(body["data"].take())
            }
            .await;
            drop(permit);
            if result.is_err() {
                self.counters.failed_attempts.fetch_add(1, Relaxed);
            }
            if self.transport().attempts.is_multiple_of(50) {
                crate::runtime::progress::write("collecting", Some(self.transport()), None)?;
            }
            match result {
                Ok(data) => return Ok(data),
                Err(error) => {
                    let retryable = error.downcast_ref::<reqwest::Error>().is_some_and(|e| {
                        e.is_timeout()
                            || e.is_connect()
                            || e.is_body()
                            || e.is_decode()
                            || e.status().is_some_and(|s| {
                                s == StatusCode::TOO_MANY_REQUESTS || s.is_server_error()
                            })
                    });
                    if !retryable || attempt == 3 {
                        return Err(error.context("GraphQL request failed"));
                    }
                    let delay = retry_after
                        .map(|seconds| Duration::from_secs(seconds.min(10)))
                        .unwrap_or_else(|| Duration::from_millis(250 * (1 << attempt)));
                    tokio::time::sleep(delay).await;
                }
            }
        }
        unreachable!()
    }

    pub async fn checkpoint(&self, sequence: Option<u64>) -> Result<Checkpoint> {
        let data = self.request("query($cp: UInt53) { chainIdentifier checkpoint(sequenceNumber:$cp) { sequenceNumber timestamp digest } }", json!({"cp":sequence})).await?;
        ensure!(
            data["chainIdentifier"] == MAINNET_CHAIN,
            "endpoint is not Sui mainnet"
        );
        let checkpoint: Checkpoint =
            serde_json::from_value(data["checkpoint"].clone()).context("checkpoint unavailable")?;
        if let Some(sequence) = sequence {
            ensure!(
                checkpoint.sequence_number == sequence,
                "checkpoint mismatch"
            );
        }
        Ok(checkpoint)
    }

    async fn scoped(&self, checkpoint: u64, query: &str, variables: Value) -> Result<Value> {
        let mut data = self.request(query, variables).await?;
        ensure!(
            data["checkpoint"].is_object(),
            "checkpoint scope unavailable (provider indexing lag or expired retention)"
        );
        ensure!(
            integer(&data["checkpoint"], "sequenceNumber")? == checkpoint,
            "checkpoint mismatch"
        );
        ensure!(
            data["checkpoint"]
                .get("query")
                .is_some_and(|v| !v.is_null()),
            "checkpoint scope unavailable (possibly outside provider retention)"
        );
        Ok(data["checkpoint"]["query"].take())
    }

    pub async fn object(&self, checkpoint: u64, address: &str) -> Result<Object> {
        let query = format!(
            "query($cp: UInt53!, $address: SuiAddress!) {{ checkpoint(sequenceNumber:$cp) {{ sequenceNumber query {{ object(address:$address) {{ {OBJECT_FIELDS} }} }} }} }}"
        );
        let mut data = self
            .scoped(
                checkpoint,
                &query,
                json!({"cp":checkpoint,"address":address}),
            )
            .await?;
        let object = Object::from_graphql(
            data.get_mut("object")
                .context("missing object response")?
                .take(),
            true,
        )
        .with_context(|| format!("reading object {address}"))?;
        ensure!(
            object.address == address,
            "response returned a different object address"
        );
        Ok(object)
    }

    pub async fn objects(
        &self,
        checkpoint: u64,
        move_type: &str,
        cursor: Option<&str>,
        size: u32,
    ) -> Result<Page> {
        let query = format!(
            "query($cp: UInt53!, $type: String!, $after: String, $size: Int!) {{ checkpoint(sequenceNumber:$cp) {{ sequenceNumber query {{ objects(filter:{{type:$type}}, first:$size, after:$after) {{ pageInfo {{ hasNextPage endCursor }} nodes {{ {OBJECT_FIELDS} }} }} }} }} }}"
        );
        let mut data = self
            .scoped(
                checkpoint,
                &query,
                json!({"cp":checkpoint,"type":move_type,"after":cursor,"size":size}),
            )
            .await?;
        Page::parse(
            data.get_mut("objects")
                .context("missing objects response")?
                .take(),
            false,
        )
    }

    pub async fn obligations(
        &self,
        checkpoint: u64,
        table: &str,
        cursor: Option<&str>,
        size: u32,
    ) -> Result<Page> {
        // ObjectTable is an embedded UID, so use address(), not object().
        let query = "query($cp: UInt53!, $table: SuiAddress!, $after: String, $size: Int!) { checkpoint(sequenceNumber:$cp) { sequenceNumber query { address(address:$table) { dynamicFields(first:$size,after:$after) { pageInfo { hasNextPage endCursor } nodes { value { __typename ... on MoveObject { address version digest contents { type { repr } json } } } } } } } } }";
        let mut data = self
            .scoped(
                checkpoint,
                query,
                json!({"cp":checkpoint,"table":table,"after":cursor,"size":size}),
            )
            .await?;
        Page::parse(
            data.get_mut("address")
                .and_then(|v| v.get_mut("dynamicFields"))
                .context("missing dynamic fields response")?
                .take(),
            true,
        )
    }
}

pub struct Page {
    pub objects: Vec<Object>,
    pub next: Option<String>,
}

impl Page {
    fn parse(mut value: Value, dynamic: bool) -> Result<Self> {
        let nodes = value
            .get_mut("nodes")
            .and_then(Value::as_array_mut)
            .map(std::mem::take)
            .context("missing page nodes")?;
        let more = value["pageInfo"]["hasNextPage"]
            .as_bool()
            .context("missing hasNextPage")?;
        let next = if more {
            ensure!(!nodes.is_empty(), "empty nonterminal page");
            Some(
                value["pageInfo"]["endCursor"]
                    .as_str()
                    .filter(|v| !v.is_empty())
                    .context("missing continuation cursor")?
                    .to_owned(),
            )
        } else {
            None
        };
        let objects = nodes
            .into_iter()
            .map(|mut v| {
                if dynamic {
                    ensure!(
                        v["value"]["__typename"] == "MoveObject",
                        "expected dynamic object field"
                    );
                    Object::from_graphql(v["value"].take(), false)
                } else {
                    Object::from_graphql(v, true)
                }
            })
            .collect::<Result<_>>()?;
        Ok(Self { objects, next })
    }
}

/// Detect duplicate objects and cursor loops; never silently deduplicate evidence.
#[derive(Default)]
pub struct Pagination {
    ids: HashSet<String>,
    cursors: HashSet<String>,
    pub cursor: Option<String>,
    pub pages: u64,
}

impl Pagination {
    pub fn accept(&mut self, page: &Page) -> Result<()> {
        for object in &page.objects {
            ensure!(
                self.ids.insert(object.address.clone()),
                "duplicate object {}",
                object.address
            );
        }
        if let Some(next) = &page.next
            && !self.cursors.insert(next.clone())
        {
            bail!("pagination cursor repeated");
        }
        self.cursor.clone_from(&page.next);
        self.pages += 1;
        Ok(())
    }

    pub fn count(&self) -> u64 {
        self.ids.len() as u64
    }
}

#[cfg(test)]
mod tests;
