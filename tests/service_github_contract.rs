use software_evaluation::service::github::{
    AcquisitionError, archive_url, commit_url, repository_url, validate_repository_metadata,
};
use software_evaluation::service::identity::GithubRepoId;

fn requested() -> GithubRepoId {
    GithubRepoId::parse("Requested-Owner", "Requested.Repo").expect("valid requested identity")
}

#[test]
fn metadata_accepts_only_public_canonical_repositories_at_full_immutable_commits() {
    for commit in [
        "ABCDEF0123456789ABCDEF0123456789ABCDEF01",
        "ABCDEF0123456789ABCDEF0123456789ABCDEF0123456789ABCDEF0123456789",
    ] {
        let snapshot = validate_repository_metadata(
            &requested(),
            8675309,
            "Canonical-Owner/Canonical.Repo",
            false,
            "main",
            commit,
        )
        .expect("public canonical metadata with immutable commit");
        assert_eq!(snapshot.repository_id, 8675309);
        assert_eq!(snapshot.full_name, "Canonical-Owner/Canonical.Repo");
        assert_eq!(snapshot.identity.owner, "Canonical-Owner");
        assert_eq!(snapshot.identity.repo, "Canonical.Repo");
        assert_eq!(snapshot.commit, commit.to_ascii_lowercase());
    }
}

#[test]
fn private_or_malformed_repository_metadata_is_rejected() {
    let private_error = validate_repository_metadata(
        &requested(),
        1,
        "Canonical/Repo",
        true,
        "main",
        "0123456789abcdef0123456789abcdef01234567",
    )
    .expect_err("private repositories are ineligible");
    assert_eq!(private_error, AcquisitionError::NotPublic);

    let invalid_cases = [
        (
            0,
            "Canonical/Repo",
            "main",
            "0123456789abcdef0123456789abcdef01234567",
        ),
        (
            1,
            "not-a-full-name",
            "main",
            "0123456789abcdef0123456789abcdef01234567",
        ),
        (
            1,
            "Canonical/Repo",
            "",
            "0123456789abcdef0123456789abcdef01234567",
        ),
        (
            1,
            "Canonical/Repo",
            "main",
            "0123456789abcdef0123456789abcdef0123456",
        ),
        (
            1,
            "Canonical/Repo",
            "main",
            "g123456789abcdef0123456789abcdef01234567",
        ),
        (
            1,
            "Canonical/Repo",
            "main",
            "0123456789abcdef0123456789abcdef012345678",
        ),
    ];
    for (repository_id, full_name, branch, commit) in invalid_cases {
        let error = validate_repository_metadata(
            &requested(),
            repository_id,
            full_name,
            false,
            branch,
            commit,
        )
        .expect_err("malformed upstream metadata must fail closed");
        assert_eq!(
            error,
            AcquisitionError::InvalidMetadata,
            "invalid metadata: id={repository_id}, full_name={full_name:?}, branch={branch:?}, commit={commit:?}"
        );
    }
}

#[test]
fn acquisition_urls_are_fixed_https_hosts_and_upstream_branch_data_stays_one_segment() {
    let identity =
        GithubRepoId::parse("Canonical-Owner", "Canonical.Repo").expect("valid identity");
    assert_eq!(
        repository_url(&identity),
        "https://api.github.com/repos/Canonical-Owner/Canonical.Repo"
    );
    assert_eq!(
        commit_url(&identity, "release/next?x#fragment%encoded").expect("encode branch"),
        "https://api.github.com/repos/Canonical-Owner/Canonical.Repo/commits/release%2Fnext%3Fx%23fragment%25encoded"
    );
    assert_eq!(
        commit_url(&identity, ""),
        Err(AcquisitionError::InvalidMetadata)
    );

    let snapshot = validate_repository_metadata(
        &requested(),
        5,
        "Canonical-Owner/Canonical.Repo",
        false,
        "main",
        "0123456789abcdef0123456789abcdef01234567",
    )
    .expect("valid snapshot");
    assert_eq!(
        archive_url(&snapshot),
        "https://codeload.github.com/Canonical-Owner/Canonical.Repo/zip/0123456789abcdef0123456789abcdef01234567"
    );
}
