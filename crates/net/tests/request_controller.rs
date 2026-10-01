//! Integration tests for the public RequestController API.

use std::{
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    thread,
    time::Duration,
};

use futures_util::StreamExt;
use net::{CacheMode, Config, Request, RequestController, RequestError, StreamingResponse};
use reqwest::{Method, StatusCode, header::HeaderValue};

#[test]
fn fetch_sends_headers_and_body_to_a_local_server() {
    // This verifies the public request path without depending on DNS,
    // internet access, or a particular remote server implementation.
    let server = TestServer::start(1, |request| {
        assert!(request.contains("x-test: integration"));
        assert!(request.ends_with("request body"));
        b"HTTP/1.1 201 Created\r\nContent-Length: 7\r\nConnection: close\r\n\r\ncreated".to_vec()
    });

    let mut request = Request::get(server.url());
    request.method = Method::POST;
    request
        .headers
        .insert("x-test", HeaderValue::from_static("integration"));
    request.body = Some(b"request body".to_vec());
    request.cache_mode = CacheMode::NoStore;

    let controller = RequestController::new(Config::default()).unwrap();
    let response = runtime().block_on(controller.fetch(request)).unwrap();

    server.join();
    assert_eq!(response.status, StatusCode::CREATED);
    assert_eq!(response.body, b"created");
    assert!(!response.from_cache);
}

#[test]
fn fetch_stream_yields_body_chunks_before_response_completion() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        read_request(&mut stream);
        stream
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 11\r\nConnection: close\r\n\r\nhello")
            .unwrap();
        stream.flush().unwrap();
        thread::sleep(Duration::from_millis(100));
        let _ = stream.write_all(b" world");
    });

    let controller = RequestController::new(Config::default()).unwrap();
    let first_chunk = runtime().block_on(async {
        let mut response = controller.fetch_stream(Request::get(url)).await.unwrap();
        response.body.next().await.unwrap().unwrap().to_vec()
    });

    server.join().unwrap();
    assert_eq!(first_chunk, b"hello");
}

#[test]
fn incomplete_stream_forwards_error_and_is_not_cached() {
    let hits = Arc::new(AtomicUsize::new(0));
    let observed_hits = Arc::clone(&hits);
    let server = TestServer::start(2, move |_| {
        if observed_hits.fetch_add(1, Ordering::SeqCst) == 0 {
            // The connection closes before the advertised body is complete.
            b"HTTP/1.1 200 OK\r\nCache-Control: max-age=60\r\nContent-Length: 11\r\nConnection: close\r\n\r\nshort".to_vec()
        } else {
            b"HTTP/1.1 200 OK\r\nCache-Control: max-age=60\r\nContent-Length: 5\r\nConnection: close\r\n\r\nvalid".to_vec()
        }
    });
    let controller = RequestController::new(Config::default()).unwrap();

    let (partial, retry) = runtime().block_on(async {
        let mut response = controller
            .fetch_stream(Request::get(server.url()))
            .await
            .unwrap();
        let mut partial = Vec::new();
        let mut error = None;
        while let Some(chunk) = response.body.next().await {
            match chunk {
                Ok(chunk) => partial.extend_from_slice(&chunk),
                Err(error_value) => {
                    error = Some(error_value);
                    break;
                }
            }
        }

        let retry = controller.fetch(Request::get(server.url())).await.unwrap();
        (partial, (error, retry))
    });

    server.join();
    assert_eq!(partial, b"short");
    assert!(matches!(retry.0, Some(RequestError::Transport(_))));
    assert!(!retry.1.from_cache);
    assert_eq!(retry.1.body, b"valid");
    assert_eq!(hits.load(Ordering::SeqCst), 2);
}

#[test]
fn dropping_a_stream_releases_its_scheduler_permit_and_does_not_cache() {
    let hits = Arc::new(AtomicUsize::new(0));
    let observed_hits = Arc::clone(&hits);
    let server = TestServer::start(2, move |_| {
        observed_hits.fetch_add(1, Ordering::SeqCst);
        b"HTTP/1.1 200 OK\r\nCache-Control: max-age=60\r\nContent-Length: 4\r\nConnection: close\r\n\r\nbody".to_vec()
    });
    let controller = RequestController::new(Config {
        max_in_flight: 1,
        ..Config::default()
    })
    .unwrap();

    runtime().block_on(async {
        let mut response = controller
            .fetch_stream(Request::get(server.url()))
            .await
            .unwrap();
        assert_eq!(
            response.body.next().await.unwrap().unwrap().as_ref(),
            b"body"
        );
        drop(response);

        let retry = controller.fetch(Request::get(server.url())).await.unwrap();
        assert!(!retry.from_cache);
        assert_eq!(retry.body, b"body");
    });

    server.join();
    assert_eq!(hits.load(Ordering::SeqCst), 2);
}

#[test]
fn empty_stream_body_can_be_consumed_and_cached() {
    let server = TestServer::start(1, |_| {
        b"HTTP/1.1 204 No Content\r\nCache-Control: max-age=60\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_vec()
    });
    let controller = RequestController::new(Config::default()).unwrap();

    let (first, second) = runtime().block_on(async {
        let first = controller
            .fetch_stream(Request::get(server.url()))
            .await
            .unwrap();
        let first_body = collect_body(first).await.unwrap();

        let second = controller
            .fetch_stream(Request::get(server.url()))
            .await
            .unwrap();
        let second_from_cache = second.from_cache;
        let second_body = collect_body(second).await.unwrap();
        ((first_body, second_from_cache), second_body)
    });

    server.join();
    assert!(first.0.is_empty());
    assert!(first.1);
    assert!(second.is_empty());
}

#[test]
fn fully_consumed_stream_is_cached() {
    let hits = Arc::new(AtomicUsize::new(0));
    let observed_hits = Arc::clone(&hits);
    let server = TestServer::start(1, move |_| {
        observed_hits.fetch_add(1, Ordering::SeqCst);
        b"HTTP/1.1 200 OK\r\nCache-Control: max-age=60\r\nContent-Length: 6\r\nConnection: close\r\n\r\ncached".to_vec()
    });
    let controller = RequestController::new(Config::default()).unwrap();

    runtime().block_on(async {
        let mut response = controller
            .fetch_stream(Request::get(server.url()))
            .await
            .unwrap();
        let mut body = Vec::new();
        while let Some(chunk) = response.body.next().await {
            body.extend_from_slice(&chunk.unwrap());
        }
        assert_eq!(body, b"cached");

        let cached = controller.fetch(Request::get(server.url())).await.unwrap();
        assert!(cached.from_cache);
        assert_eq!(cached.body, b"cached");
    });

    server.join();
    assert_eq!(hits.load(Ordering::SeqCst), 1);
}

#[test]
fn no_store_streams_are_not_cached() {
    let hits = Arc::new(AtomicUsize::new(0));
    let observed_hits = Arc::clone(&hits);
    let server = TestServer::start(2, move |_| {
        observed_hits.fetch_add(1, Ordering::SeqCst);
        b"HTTP/1.1 200 OK\r\nCache-Control: max-age=60\r\nContent-Length: 5\r\nConnection: close\r\n\r\nfresh".to_vec()
    });
    let controller = RequestController::new(Config::default()).unwrap();

    runtime().block_on(async {
        let mut request = Request::get(server.url());
        request.cache_mode = CacheMode::NoStore;
        let first = controller.fetch_stream(request.clone()).await.unwrap();
        let second = controller.fetch_stream(request).await.unwrap();
        assert!(!first.from_cache);
        assert!(!second.from_cache);
        assert_eq!(collect_body(first).await.unwrap(), b"fresh");
        assert_eq!(collect_body(second).await.unwrap(), b"fresh");
    });

    server.join();
    assert_eq!(hits.load(Ordering::SeqCst), 2);
}

#[test]
fn reload_stream_bypasses_cache_and_caches_the_refresh() {
    let hits = Arc::new(AtomicUsize::new(0));
    let observed_hits = Arc::clone(&hits);
    let server = TestServer::start(2, move |_| {
        let body = if observed_hits.fetch_add(1, Ordering::SeqCst) == 0 {
            "first"
        } else {
            "second"
        };
        format!(
            "HTTP/1.1 200 OK\r\nCache-Control: max-age=60\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(), body
        )
        .into_bytes()
    });
    let controller = RequestController::new(Config::default()).unwrap();

    let (first, refreshed, cached) = runtime().block_on(async {
        let request = Request::get(server.url());
        let first = collect_body(controller.fetch_stream(request.clone()).await.unwrap())
            .await
            .unwrap();

        let mut reload = request;
        reload.cache_mode = CacheMode::Reload;
        let refreshed_response = controller.fetch_stream(reload).await.unwrap();
        let refreshed_from_cache = refreshed_response.from_cache;
        let refreshed = collect_body(refreshed_response).await.unwrap();
        let cached_response = controller
            .fetch_stream(Request::get(server.url()))
            .await
            .unwrap();
        let cached_from_cache = cached_response.from_cache;
        let cached = collect_body(cached_response).await.unwrap();
        (
            first,
            (refreshed_from_cache, refreshed),
            (cached_from_cache, cached),
        )
    });

    server.join();
    assert_eq!(first, b"first");
    assert!(!refreshed.0);
    assert_eq!(refreshed.1, b"second");
    assert!(cached.0);
    assert_eq!(cached.1, b"second");
    assert_eq!(hits.load(Ordering::SeqCst), 2);
}

#[test]
fn only_if_cached_returns_a_streaming_cache_hit() {
    let server = TestServer::start(1, |_| {
        b"HTTP/1.1 200 OK\r\nCache-Control: max-age=60\r\nContent-Length: 6\r\nX-Cache-Test: yes\r\nConnection: close\r\n\r\ncached".to_vec()
    });
    let controller = RequestController::new(Config::default()).unwrap();

    let (from_cache, body, header) = runtime().block_on(async {
        let request = Request::get(server.url());
        collect_body(controller.fetch_stream(request.clone()).await.unwrap())
            .await
            .unwrap();

        let mut only_cached = request;
        only_cached.cache_mode = CacheMode::OnlyIfCached;
        let response = controller.fetch_stream(only_cached).await.unwrap();
        let from_cache = response.from_cache;
        let header = response.headers["x-cache-test"]
            .to_str()
            .unwrap()
            .to_owned();
        let body = collect_body(response).await.unwrap();
        (from_cache, body, header)
    });

    server.join();
    assert!(from_cache);
    assert_eq!(body, b"cached");
    assert_eq!(header, "yes");
}

#[test]
fn streamed_redirect_exposes_the_final_url_and_response() {
    let target_listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let target_url = format!("http://{}", target_listener.local_addr().unwrap());
    let target_server = thread::spawn(move || {
        let (mut stream, _) = target_listener.accept().unwrap();
        read_request(&mut stream);
        stream
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\nConnection: close\r\n\r\nfinal")
            .unwrap();
    });

    let redirect_listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let redirect_url = format!("http://{}", redirect_listener.local_addr().unwrap());
    let redirect_target = target_url.clone();
    let redirect_server = thread::spawn(move || {
        let (mut stream, _) = redirect_listener.accept().unwrap();
        read_request(&mut stream);
        let response = format!(
            "HTTP/1.1 302 Found\r\nLocation: {redirect_target}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
        );
        stream.write_all(response.as_bytes()).unwrap();
    });

    let controller = RequestController::new(Config::default()).unwrap();
    let response = runtime().block_on(async {
        controller
            .fetch_stream(Request::get(redirect_url))
            .await
            .unwrap()
    });
    let final_url = response.url.clone();
    let body = runtime().block_on(collect_body(response)).unwrap();

    redirect_server.join().unwrap();
    target_server.join().unwrap();
    assert_eq!(final_url, format!("{target_url}/"));
    assert_eq!(body, b"final");
}

#[test]
fn unsupported_schemes_are_rejected_without_network_io() {
    let controller = RequestController::new(Config::default()).unwrap();
    let error = runtime()
        .block_on(controller.fetch(Request::get("ftp://example.test/file")))
        .unwrap_err();

    assert!(matches!(error, RequestError::UnsupportedScheme(scheme) if scheme == "ftp"));
}

#[test]
fn no_store_requests_are_not_cached() {
    let hits = Arc::new(AtomicUsize::new(0));
    let observed_hits = Arc::clone(&hits);
    let server = TestServer::start(2, move |_| {
        observed_hits.fetch_add(1, Ordering::SeqCst);
        b"HTTP/1.1 200 OK\r\nCache-Control: max-age=60\r\nContent-Length: 4\r\nConnection: close\r\n\r\nfresh".to_vec()
    });

    let controller = RequestController::new(Config::default()).unwrap();
    runtime().block_on(async {
        let mut request = Request::get(server.url());
        request.cache_mode = CacheMode::NoStore;
        controller.fetch(request.clone()).await.unwrap();
        controller.fetch(request).await.unwrap();
    });

    server.join();
    assert_eq!(hits.load(Ordering::SeqCst), 2);
}

#[test]
fn clear_cache_forces_a_new_network_request() {
    let hits = Arc::new(AtomicUsize::new(0));
    let observed_hits = Arc::clone(&hits);
    let server = TestServer::start(2, move |_| {
        observed_hits.fetch_add(1, Ordering::SeqCst);
        b"HTTP/1.1 200 OK\r\nCache-Control: max-age=60\r\nContent-Length: 5\r\nConnection: close\r\n\r\nhello".to_vec()
    });

    let controller = RequestController::new(Config::default()).unwrap();
    runtime().block_on(async {
        let request = Request::get(server.url());
        let first = controller.fetch(request.clone()).await.unwrap();
        assert!(!first.from_cache);
        let cached = controller.fetch(request.clone()).await.unwrap();
        assert!(cached.from_cache);
        controller.clear_cache();
        let refreshed = controller.fetch(request).await.unwrap();
        assert!(!refreshed.from_cache);
    });

    server.join();
    assert_eq!(hits.load(Ordering::SeqCst), 2);
}

#[test]
fn reload_bypasses_cached_response_and_caches_the_refresh() {
    let hits = Arc::new(AtomicUsize::new(0));
    let observed_hits = Arc::clone(&hits);
    let server = TestServer::start(2, move |_| {
        let body = if observed_hits.fetch_add(1, Ordering::SeqCst) == 0 {
            "first"
        } else {
            "second"
        };
        format!(
            "HTTP/1.1 200 OK\r\nCache-Control: max-age=60\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(), body
        )
        .into_bytes()
    });

    let controller = RequestController::new(Config::default()).unwrap();
    let response = runtime().block_on(async {
        let request = Request::get(server.url());
        controller.fetch(request.clone()).await.unwrap();
        let mut reload = request;
        reload.cache_mode = CacheMode::Reload;
        let refreshed = controller.fetch(reload).await.unwrap();
        let cached = controller.fetch(Request::get(server.url())).await.unwrap();
        (refreshed, cached)
    });

    server.join();
    assert_eq!(response.0.body, b"second");
    assert!(!response.0.from_cache);
    assert_eq!(response.1.body, b"second");
    assert!(response.1.from_cache);
    assert_eq!(hits.load(Ordering::SeqCst), 2);
}

#[test]
fn only_if_cached_returns_a_cached_response_without_network_io() {
    let server = TestServer::start(1, |_| {
        b"HTTP/1.1 200 OK\r\nCache-Control: max-age=60\r\nContent-Length: 6\r\nConnection: close\r\n\r\ncached".to_vec()
    });
    let controller = RequestController::new(Config::default()).unwrap();

    let cached = runtime().block_on(async {
        let request = Request::get(server.url());
        controller.fetch(request.clone()).await.unwrap();
        let mut only_cached = request;
        only_cached.cache_mode = CacheMode::OnlyIfCached;
        controller.fetch(only_cached).await.unwrap()
    });

    server.join();
    assert!(cached.from_cache);
    assert_eq!(cached.body, b"cached");
}

#[test]
fn non_cacheable_post_requests_are_sent_each_time() {
    let hits = Arc::new(AtomicUsize::new(0));
    let observed_hits = Arc::clone(&hits);
    let server = TestServer::start(2, move |_| {
        observed_hits.fetch_add(1, Ordering::SeqCst);
        b"HTTP/1.1 200 OK\r\nCache-Control: max-age=60\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok".to_vec()
    });
    let controller = RequestController::new(Config::default()).unwrap();

    runtime().block_on(async {
        let mut request = Request::get(server.url());
        request.method = Method::POST;
        request.body = Some(b"payload".to_vec());
        controller.fetch(request.clone()).await.unwrap();
        controller.fetch(request).await.unwrap();
    });

    server.join();
    assert_eq!(hits.load(Ordering::SeqCst), 2);
}

#[test]
fn cloned_controllers_share_the_response_cache() {
    let server = TestServer::start(1, |_| {
        b"HTTP/1.1 200 OK\r\nCache-Control: max-age=60\r\nContent-Length: 6\r\nConnection: close\r\n\r\nshared".to_vec()
    });
    let controller = RequestController::new(Config::default()).unwrap();
    let clone = controller.clone();

    let (first, second) = runtime().block_on(async {
        let request = Request::get(server.url());
        let first = controller.fetch(request.clone()).await.unwrap();
        let second = clone.fetch(request).await.unwrap();
        (first, second)
    });

    server.join();
    assert!(!first.from_cache);
    assert!(second.from_cache);
}

#[test]
fn scheduler_limits_active_transport_requests() {
    let active = Arc::new(AtomicUsize::new(0));
    let maximum = Arc::new(AtomicUsize::new(0));
    let observed_active = Arc::clone(&active);
    let observed_maximum = Arc::clone(&maximum);
    let server = TestServer::start(2, move |_| {
        let now = observed_active.fetch_add(1, Ordering::SeqCst) + 1;
        observed_maximum.fetch_max(now, Ordering::SeqCst);
        thread::sleep(Duration::from_millis(50));
        observed_active.fetch_sub(1, Ordering::SeqCst);
        b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok".to_vec()
    });

    let config = Config {
        max_in_flight: 1,
        ..Config::default()
    };
    let controller = RequestController::new(config).unwrap();
    runtime().block_on(async {
        let first_controller = controller.clone();
        let second_controller = controller.clone();
        let first_url = server.url();
        let second_url = server.url();
        let first =
            tokio::spawn(async move { first_controller.fetch(Request::get(first_url)).await });
        let second =
            tokio::spawn(async move { second_controller.fetch(Request::get(second_url)).await });
        first.await.unwrap().unwrap();
        second.await.unwrap().unwrap();
    });

    server.join();
    assert_eq!(maximum.load(Ordering::SeqCst), 1);
}

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Runtime::new().expect("Tokio runtime should initialize")
}

async fn collect_body(mut response: StreamingResponse) -> Result<Vec<u8>, RequestError> {
    let mut body = Vec::new();
    while let Some(chunk) = response.body.next().await {
        body.extend_from_slice(&chunk?);
    }
    Ok(body)
}

struct TestServer {
    url: String,
    thread: thread::JoinHandle<()>,
}

impl TestServer {
    /// Starts a minimal HTTP/1.1 server that accepts exactly `connections`
    /// requests, making each test's expected network activity explicit.
    fn start<F>(connections: usize, response: F) -> Self
    where
        F: Fn(&str) -> Vec<u8> + Send + Sync + 'static,
    {
        let listener = TcpListener::bind("127.0.0.1:0").expect("server should bind");
        let url = format!("http://{}", listener.local_addr().unwrap());
        let response = Arc::new(response);
        let thread = thread::spawn(move || {
            for _ in 0..connections {
                let (mut stream, _) = listener.accept().expect("server should accept request");
                let request = read_request(&mut stream);
                stream
                    .write_all(response(&request).as_slice())
                    .expect("server should write response");
            }
        });
        Self { url, thread }
    }

    fn url(&self) -> String {
        self.url.clone()
    }

    fn join(self) {
        self.thread.join().expect("server should exit");
    }
}

fn read_request(stream: &mut TcpStream) -> String {
    // Read through the request body so assertions inspect the complete request
    // even when the operating system splits it across multiple TCP reads.
    let mut bytes = Vec::new();
    let mut buffer = [0; 1024];
    loop {
        let count = stream
            .read(&mut buffer)
            .expect("server should read request");
        if count == 0 {
            break;
        }
        bytes.extend_from_slice(&buffer[..count]);
        if let Some(header_end) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
            let headers = String::from_utf8_lossy(&bytes[..header_end]);
            let body_length = headers
                .lines()
                .find_map(|line| {
                    line.split_once(':').and_then(|(name, value)| {
                        name.eq_ignore_ascii_case("content-length")
                            .then_some(value.trim())
                    })
                })
                .and_then(|length| length.parse::<usize>().ok())
                .unwrap_or(0);
            if bytes.len() >= header_end + 4 + body_length {
                break;
            }
        }
    }
    String::from_utf8_lossy(&bytes).into_owned()
}
