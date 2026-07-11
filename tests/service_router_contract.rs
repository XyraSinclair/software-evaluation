use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use axum::Router;
use axum::body::Body;
use http::{Request, StatusCode, header};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use software_evaluation::service::app::{AppState, ServiceConfig, cache_key, router};
use software_evaluation::service::github::{AcquisitionError, RemoteSource, RepositorySnapshot};
use software_evaluation::service::identity::GithubRepoId;
use tempfile::TempDir;
use tokio::sync::Notify;
use tower::ServiceExt;

#[derive(Default)]
struct BlockingSource {
    resolve_calls: AtomicUsize,
    download_calls: AtomicUsize,
    entered: Notify,
    release: Notify,
}

#[async_trait]
impl RemoteSource for BlockingSource {
    async fn resolve(
        &self,
        _identity: &GithubRepoId,
    ) -> Result<RepositorySnapshot, AcquisitionError> {
        self.resolve_calls.fetch_add(1, Ordering::SeqCst);
        self.entered.notify_one();
        self.release.notified().await;
        Err(AcquisitionError::NotFound)
    }

    async fn download(
        &self,
        _snapshot: &RepositorySnapshot,
        _destination: &Path,
        _max_bytes: u64,
    ) -> Result<(), AcquisitionError> {
        self.download_calls.fetch_add(1, Ordering::SeqCst);
        Err(AcquisitionError::Upstream)
    }
}

fn test_router(source: Arc<dyn RemoteSource>) -> (TempDir, Router) {
    let cache = TempDir::new().expect("router cache directory");
    let state = AppState::new(
        ServiceConfig {
            cache_dir: cache.path().to_owned(),
            workers: 1,
            queue_capacity: 4,
            worker_timeout: Duration::from_secs(1),
        },
        source,
    )
    .expect("construct test app state");
    (cache, router(state))
}

fn request(method: &str, uri: &str, body: Option<Value>) -> Request<Body> {
    let bytes = body.map_or_else(Vec::new, |value| {
        serde_json::to_vec(&value).expect("encode request")
    });
    let mut builder = Request::builder().method(method).uri(uri);
    if !bytes.is_empty() {
        builder = builder.header(header::CONTENT_TYPE, "application/json");
    }
    builder.body(Body::from(bytes)).expect("build HTTP request")
}

async fn send(app: &Router, request: Request<Body>) -> (StatusCode, http::HeaderMap, Value) {
    let response = app.clone().oneshot(request).await.expect("router response");
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("collect response body")
        .to_bytes();
    let value = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or_else(|error| {
            panic!(
                "response was not a JSON contract body ({error}): {}",
                String::from_utf8_lossy(&bytes)
            )
        })
    };
    (status, headers, value)
}

#[tokio::test]
async fn health_and_readiness_are_available_without_repository_acquisition() {
    let source = Arc::new(BlockingSource::default());
    let (_cache, app) = test_router(source.clone());

    assert_eq!(
        send(&app, request("GET", "/healthz", None)).await.0,
        StatusCode::OK
    );
    assert_eq!(
        send(&app, request("GET", "/readyz", None)).await.0,
        StatusCode::OK
    );
    assert_eq!(source.resolve_calls.load(Ordering::SeqCst), 0);
    assert_eq!(source.download_calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn invalid_identity_and_extra_fields_are_rejected_before_outbound_calls() {
    let source = Arc::new(BlockingSource::default());
    let (_cache, app) = test_router(source.clone());
    let cases = [
        (
            json!({"owner":"-invalid", "repo":"repo"}),
            "invalid_repository",
            "owner or repository name is invalid",
        ),
        (
            json!({"owner":"owner", "repo":"repo/main"}),
            "invalid_repository",
            "owner or repository name is invalid",
        ),
        (
            json!({"owner":"owner", "repo":"repo", "ref":"main"}),
            "invalid_request",
            "request body must be exactly an owner and repository name",
        ),
        (
            json!({"owner":"owner", "repo":"repo", "url":"https://github.com/owner/repo"}),
            "invalid_request",
            "request body must be exactly an owner and repository name",
        ),
    ];

    for (body, code, message) in cases {
        let (status, _, response) = send(&app, request("POST", "/v1/analyses", Some(body))).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(
            response,
            json!({"error":{"code":code,"message":message,"retryable":false}})
        );
    }
    assert_eq!(source.resolve_calls.load(Ordering::SeqCst), 0);
    assert_eq!(source.download_calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn start_deduplicates_case_insensitive_identity_and_get_exposes_monotonic_states() {
    let source = Arc::new(BlockingSource::default());
    let (_cache, app) = test_router(source.clone());

    let (status, headers, started) = send(
        &app,
        request(
            "POST",
            "/v1/analyses",
            Some(json!({"owner":"Octo-Cat", "repo":"Repo.Name"})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED);
    assert_eq!(
        headers
            .get(header::RETRY_AFTER)
            .and_then(|v| v.to_str().ok()),
        Some("2")
    );
    assert_eq!(started["state"], "queued");
    let analysis_id = started["analysis_id"]
        .as_str()
        .expect("opaque analysis ID")
        .to_owned();

    source.entered.notified().await;
    let (status, _, duplicate) = send(
        &app,
        request(
            "POST",
            "/v1/analyses",
            Some(json!({"owner":"octo-cat", "repo":"repo.name"})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED);
    assert_eq!(duplicate["analysis_id"], analysis_id);
    assert_eq!(duplicate["state"], "resolving");
    assert_eq!(source.resolve_calls.load(Ordering::SeqCst), 1);

    let path = format!("/v1/analyses/{analysis_id}");
    let (_, _, resolving) = send(&app, request("GET", &path, None)).await;
    assert_eq!(resolving["state"], "resolving");

    source.release.notify_one();
    let failed = loop {
        let (_, _, current) = send(&app, request("GET", &path, None)).await;
        if current["state"] == "failed" {
            break current;
        }
        tokio::task::yield_now().await;
    };
    assert_eq!(
        failed["error"],
        json!({"code":"repository_not_found","message":"repository was not found","retryable":false})
    );
    assert_eq!(source.download_calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn queue_capacity_rejects_only_distinct_work_beyond_the_bound() {
    let source = Arc::new(BlockingSource::default());
    let cache = TempDir::new().expect("bounded queue cache");
    let state = AppState::new(
        ServiceConfig {
            cache_dir: cache.path().to_owned(),
            workers: 1,
            queue_capacity: 1,
            worker_timeout: Duration::from_secs(1),
        },
        source.clone(),
    )
    .expect("construct bounded queue state");
    let app = router(state);

    let first = send(
        &app,
        request(
            "POST",
            "/v1/analyses",
            Some(json!({"owner":"owner", "repo":"one"})),
        ),
    )
    .await;
    assert_eq!(first.0, StatusCode::ACCEPTED);
    source.entered.notified().await;

    let second = send(
        &app,
        request(
            "POST",
            "/v1/analyses",
            Some(json!({"owner":"owner", "repo":"two"})),
        ),
    )
    .await;
    assert_eq!(second.0, StatusCode::ACCEPTED);
    assert_eq!(second.2["state"], "queued");

    let third = send(
        &app,
        request(
            "POST",
            "/v1/analyses",
            Some(json!({"owner":"owner", "repo":"three"})),
        ),
    )
    .await;
    assert_eq!(third.0, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(
        third.2,
        json!({"error":{"code":"queue_full","message":"analysis queue is full","retryable":true}})
    );

    source.release.notify_one();
    source.release.notify_one();
}

#[tokio::test]
async fn unknown_analysis_id_has_one_stable_path_free_error() {
    let source = Arc::new(BlockingSource::default());
    let (_cache, app) = test_router(source);

    for id in ["not-a-uuid", "00000000-0000-4000-8000-000000000000"] {
        let (status, _, body) =
            send(&app, request("GET", &format!("/v1/analyses/{id}"), None)).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(
            body,
            json!({"error":{"code":"analysis_not_found","message":"analysis was not found","retryable":false}})
        );
        let encoded = body.to_string();
        assert!(!encoded.contains("/tmp"));
        assert!(!encoded.contains(".seval-cache"));
    }
}

#[test]
fn cache_identity_separates_repository_and_full_commit() {
    let commit_a = "0123456789abcdef0123456789abcdef01234567";
    let commit_b = "1123456789abcdef0123456789abcdef01234567";
    let base = cache_key(42, commit_a);

    assert_ne!(
        base,
        cache_key(43, commit_a),
        "numeric GitHub repository ID is part of cache identity"
    );
    assert_ne!(
        base,
        cache_key(42, commit_b),
        "full immutable commit is part of cache identity"
    );
    assert_eq!(
        base,
        cache_key(42, commit_a),
        "same analysis bundle identity is deterministic"
    );
}
