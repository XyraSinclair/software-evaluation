use serde::{Deserialize, Serialize};
use std::{fmt, str::FromStr};

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct OwnerRepo {
    pub owner: String,
    pub repo: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdentityError;
impl fmt::Display for IdentityError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("invalid repository identity")
    }
}
impl std::error::Error for IdentityError {}

pub fn validate_owner(value: &str) -> Result<(), IdentityError> {
    let b = value.as_bytes();
    if !(1..=39).contains(&b.len())
        || !b.is_ascii()
        || !b[0].is_ascii_alphanumeric()
        || !b[b.len() - 1].is_ascii_alphanumeric()
        || !b.iter().all(|c| c.is_ascii_alphanumeric() || *c == b'-')
    {
        return Err(IdentityError);
    }
    Ok(())
}
pub fn validate_repo(value: &str) -> Result<(), IdentityError> {
    let b = value.as_bytes();
    if !(1..=100).contains(&b.len())
        || !b.is_ascii()
        || matches!(value, "." | "..")
        || !b
            .iter()
            .all(|c| c.is_ascii_alphanumeric() || matches!(*c, b'.' | b'_' | b'-'))
    {
        return Err(IdentityError);
    }
    Ok(())
}
impl OwnerRepo {
    pub fn new(owner: impl Into<String>, repo: impl Into<String>) -> Result<Self, IdentityError> {
        let owner = owner.into();
        let repo = repo.into();
        validate_owner(&owner)?;
        validate_repo(&repo)?;
        Ok(Self { owner, repo })
    }
    pub fn key(&self) -> String {
        format!(
            "{}/{}",
            self.owner.to_ascii_lowercase(),
            self.repo.to_ascii_lowercase()
        )
    }
}
impl FromStr for OwnerRepo {
    type Err = IdentityError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let (o, r) = s.split_once('/').ok_or(IdentityError)?;
        if r.contains('/') {
            return Err(IdentityError);
        }
        Self::new(o, r)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct GithubRepoId {
    pub owner: String,
    pub repo: String,
}
impl GithubRepoId {
    pub fn parse(owner: &str, repo: &str) -> Result<Self, IdentityError> {
        validate_owner(owner)?;
        validate_repo(repo)?;
        Ok(Self {
            owner: owner.to_owned(),
            repo: repo.to_owned(),
        })
    }
    pub fn key(&self) -> String {
        format!(
            "{}/{}",
            self.owner.to_ascii_lowercase(),
            self.repo.to_ascii_lowercase()
        )
    }
}
impl FromStr for GithubRepoId {
    type Err = IdentityError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let (o, r) = s.split_once('/').ok_or(IdentityError)?;
        if r.contains('/') {
            return Err(IdentityError);
        }
        Self::parse(o, r)
    }
}
