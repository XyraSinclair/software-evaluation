#![cfg(unix)]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::process::Command;
use std::time::Duration;

use software_evaluation::service::dto::RepositoryProvenance;
use software_evaluation::service::worker::{WorkerError, run_child};
use tempfile::TempDir;

#[tokio::test]
async fn timed_out_worker_is_killed_and_reaped_before_returning() {
    let workspace = TempDir::new().expect("worker timeout workspace");
    let executable = workspace.path().join("blocking-worker");
    let script = "#!/bin/sh\nwhile :; do :; done\n";
    fs::write(&executable, script).expect("write blocking worker fixture");
    let mut permissions = fs::metadata(&executable)
        .expect("worker fixture metadata")
        .permissions();
    permissions.set_mode(0o700);
    fs::set_permissions(&executable, permissions).expect("make worker fixture executable");

    let error = run_child(
        &executable,
        workspace.path(),
        RepositoryProvenance {
            full_name: "fixture/repository".to_owned(),
            repository_id: 7,
            commit: "0123456789abcdef0123456789abcdef01234567".to_owned(),
            cached: false,
        },
        Duration::from_millis(500),
    )
    .await
    .expect_err("worker must exceed its deadline");
    assert!(matches!(error, WorkerError::Timeout));
    assert_eq!(error.to_string(), "worker timed out");

    let status = Command::new("pgrep")
        .args(["-f", executable.to_string_lossy().as_ref()])
        .status()
        .expect("query blocking worker process");
    assert!(!status.success(), "timed-out worker process still exists");
}
