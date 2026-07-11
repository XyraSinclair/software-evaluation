use crate::service::{
    archive::{self, ArchiveLimits},
    dto::*,
    github::{AcquisitionError, RemoteSource, archive_path},
    identity::GithubRepoId,
    worker,
};
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Path, State},
    http::{HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    fs,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::sync::{Mutex, Semaphore};
use uuid::Uuid;

const BUNDLE: &str = concat!(env!("CARGO_PKG_VERSION"), ":static-five:v2:dup-40-5-100");
#[derive(Debug, Clone)]
pub struct ServiceConfig {
    pub cache_dir: PathBuf,
    pub workers: usize,
    pub queue_capacity: usize,
    pub worker_timeout: Duration,
}
impl Default for ServiceConfig {
    fn default() -> Self {
        Self {
            cache_dir: PathBuf::from(".seval-cache"),
            workers: 2,
            queue_capacity: 32,
            worker_timeout: Duration::from_secs(120),
        }
    }
}
#[derive(Clone)]
pub struct AppState {
    inner: Arc<Inner>,
}
struct Inner {
    config: ServiceConfig,
    source: Arc<dyn RemoteSource>,
    jobs: Mutex<HashMap<Uuid, Job>>,
    active: Mutex<HashMap<String, Uuid>>,
    semaphore: Arc<Semaphore>,
    queued: AtomicUsize,
}
#[derive(Clone)]
struct Job {
    id: Uuid,
    state: JobState,
    created: u64,
    updated: u64,
    result: Option<CompactResult>,
    error: Option<ErrorEnvelope>,
}
#[derive(Debug)]
pub struct ServiceError;
impl AppState {
    pub fn new(config: ServiceConfig, source: Arc<dyn RemoteSource>) -> Result<Self, ServiceError> {
        if config.workers == 0 || config.queue_capacity == 0 {
            return Err(ServiceError);
        }
        fs::create_dir_all(&config.cache_dir).map_err(|_| ServiceError)?;
        Ok(Self {
            inner: Arc::new(Inner {
                semaphore: Arc::new(Semaphore::new(config.workers)),
                config,
                source,
                jobs: Mutex::new(HashMap::new()),
                active: Mutex::new(HashMap::new()),
                queued: AtomicUsize::new(0),
            }),
        })
    }
}
pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/healthz", get(health))
        .route("/readyz", get(ready))
        .route("/v1/analyses", post(create))
        .route("/v1/analyses/{analysis_id}", get(fetch))
        .layer(DefaultBodyLimit::max(1024))
        .with_state(state)
}
async fn health() -> StatusCode {
    StatusCode::OK
}
async fn ready(State(s): State<AppState>) -> StatusCode {
    if s.inner.queued.load(Ordering::Relaxed) < s.inner.config.queue_capacity {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    }
}
fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
fn public(job: &Job) -> JobResponse {
    JobResponse {
        analysis_id: job.id.to_string(),
        state: job.state,
        created_at_ms: job.created,
        updated_at_ms: job.updated,
        result: job.result.clone(),
        error: job.error.clone(),
    }
}
fn err(status: StatusCode, code: &'static str, message: &'static str, retryable: bool) -> Response {
    (
        status,
        Json(ErrorBody {
            error: ErrorEnvelope {
                code,
                message,
                retryable,
            },
        }),
    )
        .into_response()
}
async fn create(
    State(state): State<AppState>,
    payload: Result<Json<AnalysisRequest>, axum::extract::rejection::JsonRejection>,
) -> Response {
    let Json(req) = match payload {
        Ok(v) => v,
        Err(_) => {
            return err(
                StatusCode::BAD_REQUEST,
                "invalid_request",
                "request body must be exactly an owner and repository name",
                false,
            );
        }
    };
    let id = match GithubRepoId::parse(&req.owner, &req.repo) {
        Ok(v) => v,
        Err(_) => {
            return err(
                StatusCode::BAD_REQUEST,
                "invalid_repository",
                "owner or repository name is invalid",
                false,
            );
        }
    };
    let key = id.key();
    if let Some(existing) = state.inner.active.lock().await.get(&key).copied() {
        if let Some(job) = state.inner.jobs.lock().await.get(&existing) {
            let status = if job.state.terminal() {
                StatusCode::OK
            } else {
                StatusCode::ACCEPTED
            };
            return response(status, public(job));
        }
    }
    let queued = state.inner.queued.fetch_add(1, Ordering::SeqCst);
    if queued >= state.inner.config.queue_capacity {
        state.inner.queued.fetch_sub(1, Ordering::SeqCst);
        return err(
            StatusCode::TOO_MANY_REQUESTS,
            "queue_full",
            "analysis queue is full",
            true,
        );
    }
    let uid = Uuid::new_v4();
    let stamp = now();
    let job = Job {
        id: uid,
        state: JobState::Queued,
        created: stamp,
        updated: stamp,
        result: None,
        error: None,
    };
    state.inner.jobs.lock().await.insert(uid, job.clone());
    state.inner.active.lock().await.insert(key.clone(), uid);
    tokio::spawn(run(state.clone(), uid, key, id));
    response(StatusCode::ACCEPTED, public(&job))
}
fn response(status: StatusCode, body: JobResponse) -> Response {
    let mut r = (status, Json(body)).into_response();
    if status == StatusCode::ACCEPTED {
        r.headers_mut()
            .insert(header::RETRY_AFTER, HeaderValue::from_static("2"));
    }
    r
}
async fn fetch(State(state): State<AppState>, Path(raw): Path<String>) -> Response {
    let id = match Uuid::parse_str(&raw) {
        Ok(v) => v,
        Err(_) => {
            return err(
                StatusCode::NOT_FOUND,
                "analysis_not_found",
                "analysis was not found",
                false,
            );
        }
    };
    match state.inner.jobs.lock().await.get(&id) {
        Some(j) => response(StatusCode::OK, public(j)),
        None => err(
            StatusCode::NOT_FOUND,
            "analysis_not_found",
            "analysis was not found",
            false,
        ),
    }
}
async fn set(state: &AppState, id: Uuid, next: JobState) {
    if let Some(j) = state.inner.jobs.lock().await.get_mut(&id) {
        j.state = next;
        j.updated = now()
    }
}
async fn fail(state: &AppState, id: Uuid, key: &str, e: AcquisitionError) {
    let (code, msg, retry) = match e {
        AcquisitionError::NotFound => ("repository_not_found", "repository was not found", false),
        AcquisitionError::NotPublic => ("repository_not_public", "repository is not public", false),
        AcquisitionError::TooLarge => (
            "archive_too_large",
            "repository archive exceeds safety limits",
            false,
        ),
        _ => ("upstream_failure", "repository acquisition failed", true),
    };
    if let Some(j) = state.inner.jobs.lock().await.get_mut(&id) {
        j.state = JobState::Failed;
        j.updated = now();
        j.error = Some(ErrorEnvelope {
            code,
            message: msg,
            retryable: retry,
        })
    }
    state.inner.active.lock().await.remove(key);
}
pub fn cache_key(repository_id: u64, commit: &str) -> String {
    let mut h = Sha256::new();
    h.update(format!("{repository_id}:{commit}:{BUNDLE}"));
    format!("{:x}.json", h.finalize())
}
fn cache_path(dir: &std::path::Path, repo: u64, commit: &str) -> PathBuf {
    dir.join(cache_key(repo, commit))
}
async fn run(state: AppState, id: Uuid, key: String, identity: GithubRepoId) {
    let permit = state.inner.semaphore.clone().acquire_owned().await;
    state.inner.queued.fetch_sub(1, Ordering::SeqCst);
    let _permit = match permit {
        Ok(p) => p,
        Err(_) => return,
    };
    set(&state, id, JobState::Resolving).await;
    let snapshot = match state.inner.source.resolve(&identity).await {
        Ok(s) => s,
        Err(e) => return fail(&state, id, &key, e).await,
    };
    let cache = cache_path(
        &state.inner.config.cache_dir,
        snapshot.repository_id,
        &snapshot.commit,
    );
    if let Ok(bytes) = tokio::fs::read(&cache).await {
        if let Ok(mut result) = serde_json::from_slice::<CompactResult>(&bytes) {
            result.repository.cached = true;
            complete(&state, id, key, result).await;
            return;
        }
    }
    let temp = match tempfile::Builder::new().prefix("sevald-").tempdir() {
        Ok(t) => t,
        Err(_) => return fail(&state, id, &key, AcquisitionError::Io).await,
    };
    let zip = archive_path(temp.path());
    set(&state, id, JobState::Downloading).await;
    if let Err(e) = state
        .inner
        .source
        .download(&snapshot, &zip, ArchiveLimits::default().compressed_bytes)
        .await
    {
        return fail(&state, id, &key, e).await;
    }
    set(&state, id, JobState::Extracting).await;
    let dest = temp.path().join("source");
    let root = match tokio::task::spawn_blocking(move || {
        archive::extract_zip(&zip, &dest, ArchiveLimits::default())
    })
    .await
    {
        Ok(Ok(p)) => p,
        _ => return fail(&state, id, &key, AcquisitionError::InvalidMetadata).await,
    };
    set(&state, id, JobState::Analyzing).await;
    let provenance = RepositoryProvenance {
        full_name: snapshot.full_name,
        repository_id: snapshot.repository_id,
        commit: snapshot.commit,
        cached: false,
    };
    let exe = match std::env::current_exe() {
        Ok(e) => e,
        Err(_) => return fail(&state, id, &key, AcquisitionError::Io).await,
    };
    let result =
        match worker::run_child(&exe, &root, provenance, state.inner.config.worker_timeout).await {
            Ok(r) => r,
            Err(_) => return fail(&state, id, &key, AcquisitionError::Upstream).await,
        };
    if let Ok(bytes) = serde_json::to_vec(&result) {
        let tmp = cache.with_extension(format!("{}.tmp", Uuid::new_v4()));
        if tokio::fs::write(&tmp, &bytes).await.is_ok() {
            let _ = tokio::fs::rename(tmp, &cache).await;
        }
    }
    complete(&state, id, key, result).await;
}
async fn complete(state: &AppState, id: Uuid, key: String, result: CompactResult) {
    if let Some(j) = state.inner.jobs.lock().await.get_mut(&id) {
        j.state = if result.failed_instruments == 0 {
            JobState::Completed
        } else {
            JobState::CompletedPartial
        };
        j.updated = now();
        j.result = Some(result)
    }
    state.inner.active.lock().await.remove(&key);
}
