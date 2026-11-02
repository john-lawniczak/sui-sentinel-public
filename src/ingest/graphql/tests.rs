use super::*;
use std::{
    io::{BufRead, BufReader, Read, Write},
    net::TcpListener,
    thread,
};

fn server(responses: Vec<(u16, Value)>) -> (String, thread::JoinHandle<Vec<Value>>) {
    server_framed(
        responses
            .into_iter()
            .map(|(status, body)| (status, body, 0))
            .collect(),
    )
}

fn server_framed(responses: Vec<(u16, Value, usize)>) -> (String, thread::JoinHandle<Vec<Value>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let handle = thread::spawn(move || {
        let mut requests = Vec::new();
        for (status, response, missing_bytes) in responses {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut reader = BufReader::new(&mut stream);
            let mut length = 0;
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                if line == "\r\n" {
                    break;
                }
                if let Some(value) = line.to_lowercase().strip_prefix("content-length:") {
                    length = value.trim().parse().unwrap();
                }
            }
            let mut bytes = vec![0; length];
            reader.read_exact(&mut bytes).unwrap();
            requests.push(serde_json::from_slice(&bytes).unwrap());
            let body = response.to_string();
            write!(stream,"HTTP/1.1 {status} Test\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len() + missing_bytes).unwrap();
        }
        requests
    });
    (endpoint, handle)
}

#[tokio::test]
async fn retries_transient_http_without_changing_checkpoint() {
    let response = json!({"data":{"checkpoint":{"sequenceNumber":42,"query":{"objects":{"nodes":[],"pageInfo":{"hasNextPage":false,"endCursor":null}}}}}});
    let (url, handle) = server(vec![
        (429, json!({})),
        (503, json!({})),
        (200, response.clone()),
    ]);
    let client = Graphql::new(&url, None, 1).unwrap();
    assert!(
        client
            .objects(42, "type", None, 50)
            .await
            .unwrap()
            .objects
            .is_empty()
    );
    let counters = client.transport();
    assert_eq!(
        (
            counters.attempts,
            counters.retries,
            counters.throttled,
            counters.failed_attempts
        ),
        (3, 2, 1, 2)
    );
    assert_eq!(counters.response_bytes, response.to_string().len() as u64);
    let requests = handle.join().unwrap();
    assert_eq!(requests[0], requests[1]);
    assert_eq!(requests[0]["variables"]["cp"], 42);
    assert!(
        requests[0]["query"]
            .as_str()
            .unwrap()
            .contains("checkpoint(sequenceNumber:$cp)")
    );
}

#[tokio::test]
async fn rejects_partial_graphql_data_and_wrong_checkpoint() {
    for response in [
        json!({"errors":[{"message":"retention expired"}],"data":{"checkpoint":{"sequenceNumber":42}}}),
        json!({"data":{"checkpoint":{"sequenceNumber":43,"query":{}}}}),
        json!({"data":{"checkpoint":{"sequenceNumber":42,"query":null}}}),
    ] {
        let (url, handle) = server(vec![(200, response)]);
        let client = Graphql::new(&url, None, 1).unwrap();
        assert!(client.objects(42, "type", None, 50).await.is_err());
        handle.join().unwrap();
    }
}

#[tokio::test]
async fn retries_disconnected_response_body_without_ingesting_it() {
    let response = json!({"data":{"ok":true}});
    let (url, handle) = server_framed(vec![(200, response.clone(), 10), (200, response, 0)]);
    let client = Graphql::new(&url, None, 1).unwrap();
    assert_eq!(
        client.request("query", json!({})).await.unwrap(),
        json!({"ok":true})
    );
    assert_eq!(handle.join().unwrap().len(), 2);
}

#[tokio::test]
async fn permanent_http_errors_do_not_retry_or_leak_url_credentials() {
    let (url, handle) = server(vec![(401, json!({}))]);
    let client = Graphql::new(&format!("{url}/?api_key=secret"), None, 1).unwrap();
    let error = client.request("query", json!({})).await.unwrap_err();
    assert!(!format!("{error:#}").contains("secret"));
    assert_eq!(handle.join().unwrap().len(), 1);
}

#[tokio::test]
async fn graphql_errors_do_not_leak_reflected_credentials_or_extensions() {
    let (url, handle) = server(vec![(
        200,
        json!({
            "errors": [{"message": "invalid x-token reflected-secret", "extensions": {
                "url": "https://provider.invalid/private-endpoint", "debug": "x".repeat(4096)
            }}],
            "data": {"ok": true}
        }),
    )]);
    let client = Graphql::with_auth_header(&url, Some("reflected-secret"), 1, "x-token").unwrap();
    let error = client.request("query", json!({})).await.unwrap_err();
    let diagnostic = format!("{error:#}");
    assert!(diagnostic.contains("partial data rejected"));
    assert!(!diagnostic.contains("reflected-secret"));
    assert!(!diagnostic.contains("private-endpoint"));
    assert!(diagnostic.len() < 200);
    assert_eq!(handle.join().unwrap().len(), 1);
}

#[test]
fn rejects_missing_or_empty_continuation_pages() {
    for value in [
        json!(1),
        json!([]),
        json!({}),
        json!({"nodes":[],"pageInfo":{"hasNextPage":true,"endCursor":"next"}}),
        json!({"nodes":[{}],"pageInfo":{"hasNextPage":true}}),
    ] {
        assert!(Page::parse(value, false).is_err());
    }
}

#[test]
fn rejects_duplicate_ids_and_cursor_cycles() {
    let object = Object {
        address: "id".into(),
        version: 1,
        digest: "digest".into(),
        move_type: "type".into(),
        contents: json!({}),
        owner: None,
    };
    let page = Page {
        objects: vec![object],
        next: Some("cursor".into()),
    };
    let mut pagination = Pagination::default();
    pagination.accept(&page).unwrap();
    assert!(pagination.accept(&page).is_err());
    assert!(
        pagination
            .accept(&Page {
                objects: vec![],
                next: Some("cursor".into())
            })
            .is_err()
    );
}

#[tokio::test]
async fn quicknode_header_is_sent_without_bearer_prefix() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = axum::Router::new().route(
        "/graphql",
        axum::routing::post(|headers: axum::http::HeaderMap| async move {
            assert_eq!(headers["x-token"], "test-credential");
            assert!(!headers.contains_key("authorization"));
            "{\"data\":{\"ok\":true}}"
        }),
    );
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let client = Graphql::with_auth_header(
        &format!("http://{addr}/graphql"),
        Some("test-credential"),
        1,
        "x-token",
    )
    .unwrap();
    assert_eq!(
        client
            .request("query { ok }", serde_json::json!({}))
            .await
            .unwrap()["ok"],
        true
    );
    assert!(Graphql::with_auth_header("https://example.invalid", Some("test"), 1, "Host").is_err());
    server.abort();
}
