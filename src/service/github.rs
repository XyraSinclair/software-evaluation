use crate::service::identity::GithubRepoId;
use async_trait::async_trait;
use futures_util::StreamExt;
use percent_encoding::{AsciiSet, CONTROLS, utf8_percent_encode};
use reqwest::{Client, StatusCode, header};
use serde::Deserialize;
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};
use thiserror::Error;
use tokio::{fs::File, io::AsyncWriteExt};

#[derive(Debug, Clone)]
pub struct RepositorySnapshot {
    pub identity: GithubRepoId,
    pub full_name: String,
    pub repository_id: u64,
    pub commit: String,
}
#[derive(Debug, Error, PartialEq, Eq)]
pub enum AcquisitionError {
    #[error("repository was not found")]
    NotFound,
    #[error("repository is not public")]
    NotPublic,
    #[error("GitHub returned invalid metadata")]
    InvalidMetadata,
    #[error("GitHub request failed")]
    Upstream,
    #[error("archive exceeds byte limits")]
    TooLarge,
    #[error("archive write failed")]
    Io,
}

const SEGMENT: &AsciiSet = &CONTROLS.add(b'/').add(b'?').add(b'#').add(b'%');
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GithubClientPolicy {
    pub follows_redirects: bool,
    pub uses_proxy_environment: bool,
    pub api_host: &'static str,
    pub archive_host: &'static str,
}
pub const GITHUB_CLIENT_POLICY: GithubClientPolicy = GithubClientPolicy {
    follows_redirects: false,
    uses_proxy_environment: false,
    api_host: "api.github.com",
    archive_host: "codeload.github.com",
};
pub fn repository_url(id: &GithubRepoId) -> String {
    format!("https://api.github.com/repos/{}/{}", id.owner, id.repo)
}
pub fn commit_url(id: &GithubRepoId, branch: &str) -> Result<String, AcquisitionError> {
    if branch.is_empty() || branch.len() > 255 || branch.bytes().any(|b| b == 0) {
        return Err(AcquisitionError::InvalidMetadata);
    }
    Ok(format!(
        "https://api.github.com/repos/{}/{}/commits/{}",
        id.owner,
        id.repo,
        utf8_percent_encode(branch, SEGMENT)
    ))
}
pub fn archive_url(snapshot: &RepositorySnapshot) -> String {
    format!(
        "https://codeload.github.com/{}/{}/zip/{}",
        snapshot.identity.owner, snapshot.identity.repo, snapshot.commit
    )
}
pub fn validate_repository_metadata(
    _requested: &GithubRepoId,
    repository_id: u64,
    full_name: &str,
    private: bool,
    default_branch: &str,
    commit: &str,
) -> Result<RepositorySnapshot, AcquisitionError> {
    if private {
        return Err(AcquisitionError::NotPublic);
    }
    if repository_id == 0 || default_branch.is_empty() || default_branch.len() > 255 {
        return Err(AcquisitionError::InvalidMetadata);
    }
    let identity: GithubRepoId = full_name
        .parse()
        .map_err(|_| AcquisitionError::InvalidMetadata)?;
    if !matches!(commit.len(), 40 | 64) || !commit.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(AcquisitionError::InvalidMetadata);
    }
    Ok(RepositorySnapshot {
        identity,
        full_name: full_name.to_owned(),
        repository_id,
        commit: commit.to_ascii_lowercase(),
    })
}

#[async_trait]
pub trait RemoteSource: Send + Sync {
    async fn resolve(
        &self,
        identity: &GithubRepoId,
    ) -> Result<RepositorySnapshot, AcquisitionError>;
    async fn download(
        &self,
        snapshot: &RepositorySnapshot,
        destination: &Path,
        max_bytes: u64,
    ) -> Result<(), AcquisitionError>;
}
#[derive(Clone)]
pub struct GithubClient {
    client: Client,
    token: Option<Arc<str>>,
}
#[derive(Deserialize)]
struct RepoMeta {
    id: u64,
    full_name: String,
    private: bool,
    default_branch: String,
}
#[derive(Deserialize)]
struct CommitMeta {
    sha: String,
}

pub fn new_github_client(token: Option<String>) -> Result<GithubClient, AcquisitionError> {
    let client = Client::builder()
        .https_only(true)
        .redirect(reqwest::redirect::Policy::none())
        .no_proxy()
        .user_agent("sevald/0.1")
        .build()
        .map_err(|_| AcquisitionError::Upstream)?;
    Ok(GithubClient {
        client,
        token: token.map(Arc::from),
    })
}
impl GithubClient {
    fn request(&self, url: String) -> reqwest::RequestBuilder {
        let r = self
            .client
            .get(url)
            .header(header::ACCEPT, "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28");
        match &self.token {
            Some(t) => r.bearer_auth(t),
            None => r,
        }
    }
    async fn json<T: serde::de::DeserializeOwned>(
        &self,
        url: String,
    ) -> Result<T, AcquisitionError> {
        let r = self
            .request(url)
            .send()
            .await
            .map_err(|_| AcquisitionError::Upstream)?;
        if r.status() == StatusCode::NOT_FOUND {
            return Err(AcquisitionError::NotFound);
        }
        if !r.status().is_success() {
            return Err(AcquisitionError::Upstream);
        }
        r.json()
            .await
            .map_err(|_| AcquisitionError::InvalidMetadata)
    }
}
#[async_trait]
impl RemoteSource for GithubClient {
    async fn resolve(&self, id: &GithubRepoId) -> Result<RepositorySnapshot, AcquisitionError> {
        let meta: RepoMeta = self.json(repository_url(id)).await?;
        let canonical: GithubRepoId = meta
            .full_name
            .parse()
            .map_err(|_| AcquisitionError::InvalidMetadata)?;
        let commit: CommitMeta = self
            .json(commit_url(&canonical, &meta.default_branch)?)
            .await?;
        validate_repository_metadata(
            id,
            meta.id,
            &meta.full_name,
            meta.private,
            &meta.default_branch,
            &commit.sha,
        )
    }
    async fn download(
        &self,
        s: &RepositorySnapshot,
        destination: &Path,
        max: u64,
    ) -> Result<(), AcquisitionError> {
        let url = archive_url(s);
        let r = self
            .client
            .get(url)
            .send()
            .await
            .map_err(|_| AcquisitionError::Upstream)?;
        if !r.status().is_success() {
            return Err(AcquisitionError::Upstream);
        }
        if r.content_length().is_some_and(|n| n > max) {
            return Err(AcquisitionError::TooLarge);
        }
        let mut file = File::create(destination)
            .await
            .map_err(|_| AcquisitionError::Io)?;
        let mut n = 0u64;
        let mut stream = r.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|_| AcquisitionError::Upstream)?;
            n = n
                .checked_add(chunk.len() as u64)
                .ok_or(AcquisitionError::TooLarge)?;
            if n > max {
                return Err(AcquisitionError::TooLarge);
            }
            file.write_all(&chunk)
                .await
                .map_err(|_| AcquisitionError::Io)?
        }
        Ok(())
    }
}
pub fn archive_path(dir: &Path) -> PathBuf {
    dir.join("repository.zip")
}
