use reqwest::header::{HeaderMap, HeaderValue};
use reqwest::StatusCode;
use serde::Deserialize;
use url::Url;
use vigia_lib::http::{classify, HttpClient, RateLimit};
use vigia_lib::providers::ProviderError;
use wiremock::matchers::{header, header_exists, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn headers(pairs: &[(&'static str, &str)]) -> HeaderMap {
    let mut map = HeaderMap::new();
    for (name, value) in pairs {
        map.insert(*name, HeaderValue::from_str(value).unwrap());
    }
    map
}

fn url(server: &MockServer, p: &str) -> Url {
    Url::parse(&format!("{}{p}", server.uri())).unwrap()
}

#[derive(Deserialize, Debug, PartialEq)]
struct Item {
    id: u64,
}

#[test]
fn ok_and_not_modified_pass_classification() {
    assert!(classify(StatusCode::OK, &HeaderMap::new(), b"").is_ok());
    assert!(classify(StatusCode::NOT_MODIFIED, &HeaderMap::new(), b"").is_ok());
}

#[test]
fn redirects_report_their_location() {
    let h = headers(&[("location", "https://elsewhere.example.test/")]);
    assert_eq!(
        classify(StatusCode::FOUND, &h, b""),
        Err(ProviderError::Redirected {
            location: "https://elsewhere.example.test/".into()
        })
    );
    assert_eq!(
        classify(StatusCode::MOVED_PERMANENTLY, &HeaderMap::new(), b""),
        Err(ProviderError::Redirected {
            location: String::new()
        })
    );
}

#[test]
fn unauthorized_not_found_and_server_errors_are_mapped() {
    let none = HeaderMap::new();
    assert_eq!(
        classify(StatusCode::UNAUTHORIZED, &none, b""),
        Err(ProviderError::Unauthorized)
    );
    assert_eq!(
        classify(StatusCode::NOT_FOUND, &none, b""),
        Err(ProviderError::NotFound)
    );
    assert_eq!(
        classify(StatusCode::BAD_GATEWAY, &none, b""),
        Err(ProviderError::Server { status: 502 })
    );
}

#[test]
fn other_statuses_are_decode_errors() {
    let err = classify(StatusCode::IM_A_TEAPOT, &HeaderMap::new(), b"").unwrap_err();
    assert_eq!(err, ProviderError::Decode("unexpected status 418".into()));
}

#[test]
fn plain_forbidden_is_not_a_rate_limit() {
    let h = headers(&[
        ("x-ratelimit-limit", "5000"),
        ("x-ratelimit-remaining", "10"),
        ("x-ratelimit-reset", "1780000000"),
    ]);
    assert_eq!(
        classify(StatusCode::FORBIDDEN, &h, b"Resource not accessible"),
        Err(ProviderError::Forbidden)
    );
}

#[test]
fn forbidden_with_an_exhausted_primary_limit_is_rate_limited() {
    let h = headers(&[
        ("x-ratelimit-limit", "5000"),
        ("x-ratelimit-remaining", "0"),
        ("x-ratelimit-reset", "1780000000"),
    ]);
    assert_eq!(
        classify(StatusCode::FORBIDDEN, &h, b""),
        Err(ProviderError::RateLimited {
            retry_after: None,
            reset_at: Some(1_780_000_000),
            remaining: Some(0),
        })
    );
}

#[test]
fn forbidden_with_retry_after_is_rate_limited() {
    let h = headers(&[("retry-after", "42")]);
    assert_eq!(
        classify(StatusCode::FORBIDDEN, &h, b""),
        Err(ProviderError::RateLimited {
            retry_after: Some(42),
            reset_at: None,
            remaining: None,
        })
    );
}

#[test]
fn forbidden_mentioning_the_secondary_limit_is_rate_limited_in_any_case() {
    let body = br#"{"message":"You have exceeded a Secondary Rate Limit."}"#;
    assert!(matches!(
        classify(StatusCode::FORBIDDEN, &HeaderMap::new(), body),
        Err(ProviderError::RateLimited { .. })
    ));
}

#[test]
fn too_many_requests_is_always_rate_limited() {
    let h = headers(&[("retry-after", "not a number")]);
    assert_eq!(
        classify(StatusCode::TOO_MANY_REQUESTS, &h, b""),
        Err(ProviderError::RateLimited {
            retry_after: None,
            reset_at: None,
            remaining: None,
        })
    );
}

#[tokio::test]
async fn get_sends_auth_and_conditional_headers_and_returns_the_body() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/items"))
        .and(header("authorization", "Bearer secret-token"))
        .and(header("if-none-match", "W/\"abc\""))
        .and(header("accept", "application/json"))
        .and(header_exists("user-agent"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("etag", "W/\"def\"")
                .insert_header("ratelimit-limit", "100")
                .insert_header("ratelimit-remaining", "99")
                .insert_header("ratelimit-reset", "1780000000")
                .set_body_json(serde_json::json!({ "id": 7 })),
        )
        .mount(&server)
        .await;

    let client = HttpClient::new("secret-token").unwrap();
    let response = client
        .get(
            &url(&server, "/items?q=1"),
            &HeaderMap::new(),
            Some("W/\"abc\""),
        )
        .await
        .unwrap();
    assert!(!response.not_modified());
    assert_eq!(response.etag().as_deref(), Some("W/\"def\""));
    assert_eq!(
        response.rate_limit(),
        Some(RateLimit {
            limit: 100,
            remaining: 99,
            reset_at: 1_780_000_000
        })
    );
    assert_eq!(response.json::<Item>().unwrap(), Item { id: 7 });
}

#[tokio::test]
async fn get_keeps_a_caller_supplied_accept_header() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(header("accept", "application/vnd.test+json"))
        .respond_with(ResponseTemplate::new(200).set_body_string("{}"))
        .mount(&server)
        .await;
    let client = HttpClient::new("tok").unwrap();
    let extra = headers(&[("accept", "application/vnd.test+json")]);
    client.get(&url(&server, "/"), &extra, None).await.unwrap();
}

#[tokio::test]
async fn get_reports_not_modified() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(304))
        .mount(&server)
        .await;
    let client = HttpClient::new("tok").unwrap();
    let response = client
        .get(&url(&server, "/"), &HeaderMap::new(), None)
        .await
        .unwrap();
    assert!(response.not_modified());
    assert!(response.etag().is_none());
}

#[tokio::test]
async fn get_does_not_follow_redirects() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(302).insert_header("location", "https://other.test/"))
        .mount(&server)
        .await;
    let client = HttpClient::new("tok").unwrap();
    let err = client
        .get(&url(&server, "/"), &HeaderMap::new(), None)
        .await
        .unwrap_err();
    assert_eq!(
        err,
        ProviderError::Redirected {
            location: "https://other.test/".into()
        }
    );
}

#[tokio::test]
async fn get_maps_a_server_error() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(503))
        .mount(&server)
        .await;
    let client = HttpClient::new("tok").unwrap();
    let err = client
        .get(&url(&server, "/"), &HeaderMap::new(), None)
        .await
        .unwrap_err();
    assert_eq!(err, ProviderError::Server { status: 503 });
}

#[tokio::test]
async fn get_reports_a_connection_failure_as_a_network_error() {
    // Nothing listens on the reserved port 1, so the connection is refused.
    let target = Url::parse("http://127.0.0.1:1/").unwrap();
    let client = HttpClient::new("tok").unwrap();
    let err = client
        .get(&target, &HeaderMap::new(), None)
        .await
        .unwrap_err();
    assert!(matches!(err, ProviderError::Network(_)));
}

#[test]
fn a_token_that_is_not_a_valid_header_value_is_unauthorized() {
    assert!(matches!(
        HttpClient::new("bad\ntoken"),
        Err(ProviderError::Unauthorized)
    ));
}

#[tokio::test]
async fn json_decode_failure_is_a_decode_error() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_string("not json"))
        .mount(&server)
        .await;
    let client = HttpClient::new("tok").unwrap();
    let response = client
        .get(&url(&server, "/"), &HeaderMap::new(), None)
        .await
        .unwrap();
    assert!(matches!(
        response.json::<Item>(),
        Err(ProviderError::Decode(_))
    ));
}

#[tokio::test]
async fn next_link_stays_on_the_base_origin() {
    let server = MockServer::start().await;
    let base = url(&server, "/");
    let next = format!("{}/page2", server.uri());
    Mock::given(method("GET"))
        .respond_with(
            ResponseTemplate::new(200).insert_header(
                "link",
                format!(
                    "<{}/last>; rel=\"last\", <{next}>; rel=\"next\"",
                    server.uri()
                )
                .as_str(),
            ),
        )
        .mount(&server)
        .await;
    let client = HttpClient::new("tok").unwrap();
    let response = client.get(&base, &HeaderMap::new(), None).await.unwrap();
    assert_eq!(response.next_link().as_deref(), Some(next.as_str()));
    assert_eq!(
        response.next_link_on(&base).unwrap().map(|u| u.to_string()),
        Some(next)
    );
}

#[tokio::test]
async fn next_link_to_another_origin_is_a_redirect_and_garbage_is_a_decode_error() {
    let server = MockServer::start().await;
    let base = url(&server, "/");
    Mock::given(method("GET"))
        .and(path("/other"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("link", "<https://evil.example.test/p2>; rel=\"next\""),
        )
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/garbage"))
        .respond_with(
            ResponseTemplate::new(200).insert_header("link", "<::not a url>; rel=\"next\""),
        )
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/none"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;
    let client = HttpClient::new("tok").unwrap();

    let other = client
        .get(&url(&server, "/other"), &HeaderMap::new(), None)
        .await
        .unwrap();
    assert_eq!(
        other.next_link_on(&base),
        Err(ProviderError::Redirected {
            location: "https://evil.example.test/p2".into()
        })
    );

    let garbage = client
        .get(&url(&server, "/garbage"), &HeaderMap::new(), None)
        .await
        .unwrap();
    assert!(matches!(
        garbage.next_link_on(&base),
        Err(ProviderError::Decode(_))
    ));

    let none = client
        .get(&url(&server, "/none"), &HeaderMap::new(), None)
        .await
        .unwrap();
    assert_eq!(none.next_link_on(&base), Ok(None));
    assert!(none.next_link().is_none());
}

#[tokio::test]
async fn a_declared_body_over_the_cap_is_rejected() {
    use vigia_lib::http::MAX_BODY_BYTES;
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(vec![b' '; MAX_BODY_BYTES + 1]))
        .mount(&server)
        .await;
    let client = HttpClient::new("tok").unwrap();
    let err = client
        .get(&url(&server, "/big"), &HeaderMap::new(), None)
        .await
        .unwrap_err();
    assert!(matches!(err, ProviderError::Decode(_)), "{err:?}");
}

#[tokio::test]
async fn a_body_at_the_cap_is_read() {
    use vigia_lib::http::MAX_BODY_BYTES;
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(vec![b' '; MAX_BODY_BYTES]))
        .mount(&server)
        .await;
    let client = HttpClient::new("tok").unwrap();
    let response = client
        .get(&url(&server, "/edge"), &HeaderMap::new(), None)
        .await
        .unwrap();
    assert_eq!(response.body.len(), MAX_BODY_BYTES);
}

/// Serves one chunked response with no Content-Length, `total` bytes long, on a local port.
fn serve_chunked(total: usize) -> Url {
    use std::io::{BufRead, BufReader, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    std::thread::spawn(move || {
        let Ok((stream, _)) = listener.accept() else {
            return;
        };
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        let mut line = String::new();
        while reader.read_line(&mut line).is_ok_and(|n| n > 2) {
            line.clear();
        }
        let mut stream = stream;
        let head = "HTTP/1.1 200 OK\r\ntransfer-encoding: chunked\r\n\r\n";
        if stream.write_all(head.as_bytes()).is_err() {
            return;
        }
        let chunk = vec![b' '; 64 * 1024];
        let mut sent = 0;
        while sent < total {
            let size = chunk.len().min(total - sent);
            let framed = format!("{size:x}\r\n");
            let ok = stream.write_all(framed.as_bytes()).is_ok()
                && stream.write_all(&chunk[..size]).is_ok()
                && stream.write_all(b"\r\n").is_ok();
            if !ok {
                // The client stopped reading at the cap.
                return;
            }
            sent += size;
        }
        let _ = stream.write_all(b"0\r\n\r\n");
    });
    Url::parse(&format!("http://{address}/stream")).unwrap()
}

#[tokio::test]
async fn a_streamed_body_stops_at_the_cap() {
    use vigia_lib::http::MAX_BODY_BYTES;
    let client = HttpClient::new("tok").unwrap();
    let err = client
        .get(&serve_chunked(MAX_BODY_BYTES * 2), &HeaderMap::new(), None)
        .await
        .unwrap_err();
    assert!(matches!(err, ProviderError::Decode(_)), "{err:?}");
}

#[tokio::test]
async fn a_streamed_body_under_the_cap_is_read() {
    let client = HttpClient::new("tok").unwrap();
    let response = client
        .get(&serve_chunked(200_000), &HeaderMap::new(), None)
        .await
        .unwrap();
    assert_eq!(response.body.len(), 200_000);
}
