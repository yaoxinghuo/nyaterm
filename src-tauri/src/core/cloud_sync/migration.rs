use crate::core::portable_snapshot::PortableSnapshot;
use crate::error::{AppError, AppResult, CloudSyncError};

use super::gc::prune_gist_snapshots_best_effort;
use super::operator::CloudRemote;
use super::protocol::{
    commit_sync_pointer, pointer_from_snapshot, read_current_sync_snapshot_compat,
    read_current_sync_snapshot_compat_decoded, read_snapshot_for_pointer, upload_sync_snapshot,
    validate_decoded_snapshot_against_pointer, verify_uploaded_sync_snapshot,
};
use super::remote::RemoteSyncPointer;

#[derive(Debug)]
pub(super) enum RemoteSnapshotResolution {
    Current(PortableSnapshot),
    LegacyMigrated(PortableSnapshot),
    Inconsistent {
        pointer: RemoteSyncPointer,
        recovery_candidate: PortableSnapshot,
    },
}

pub(super) async fn resolve_remote_snapshot(
    remote: &CloudRemote,
    remote_root: &str,
    pointer: &RemoteSyncPointer,
) -> AppResult<RemoteSnapshotResolution> {
    match read_snapshot_for_pointer(remote, remote_root, pointer).await {
        Ok(snapshot) => return Ok(RemoteSnapshotResolution::Current(snapshot)),
        Err(error) if is_recoverable_pointer_snapshot_error(&error) => {
            tracing::warn!(
                error = %error,
                revision = %pointer.revision_id,
                "Remote sync pointer snapshot is not directly usable; trying compatible current snapshot"
            );
            return resolve_from_compatible_current(remote, remote_root, pointer, error).await;
        }
        Err(error) => return Err(error),
    }
}

fn is_recoverable_pointer_snapshot_error(error: &AppError) -> bool {
    matches!(
        error,
        AppError::CloudSync(
            CloudSyncError::SnapshotMissing { .. }
                | CloudSyncError::CorruptedSnapshot { .. }
                | CloudSyncError::RevisionMismatch { .. }
                | CloudSyncError::HashMismatch { .. }
        )
    )
}

async fn resolve_from_compatible_current(
    remote: &CloudRemote,
    remote_root: &str,
    pointer: &RemoteSyncPointer,
    pointer_error: AppError,
) -> AppResult<RemoteSnapshotResolution> {
    let current = match read_current_sync_snapshot_compat_decoded(remote, remote_root).await {
        Ok(Some(current)) => current,
        Ok(None) => return Err(pointer_error),
        Err(AppError::CloudSync(CloudSyncError::CorruptedSnapshot { .. })) => {
            tracing::warn!(
                revision = %pointer.revision_id,
                "Compatible current cloud sync snapshot is corrupt; keeping pointer snapshot error"
            );
            return Err(pointer_error);
        }
        Err(error) => return Err(error),
    };

    if validate_decoded_snapshot_against_pointer(pointer, &current).is_ok() {
        tracing::info!(
            revision = %pointer.revision_id,
            "Compatible current cloud sync snapshot matches latest pointer; migrating snapshot file"
        );
        migrate_legacy_snapshot(remote, remote_root, pointer, &current.snapshot).await?;
        return Ok(RemoteSnapshotResolution::LegacyMigrated(current.snapshot));
    }

    tracing::warn!(
        pointer_revision = %pointer.revision_id,
        current_revision = %current.snapshot.revision_id,
        "Remote sync pointer snapshot and compatible current snapshot are inconsistent"
    );
    Ok(RemoteSnapshotResolution::Inconsistent {
        pointer: pointer.clone(),
        recovery_candidate: current.snapshot,
    })
}

pub(super) async fn migrate_legacy_snapshot(
    remote: &CloudRemote,
    remote_root: &str,
    pointer: &RemoteSyncPointer,
    snapshot: &PortableSnapshot,
) -> AppResult<()> {
    prune_gist_snapshots_best_effort(remote, remote_root, Some(pointer), &snapshot.revision_id)
        .await;
    upload_sync_snapshot(remote, remote_root, snapshot).await?;
    verify_uploaded_sync_snapshot(remote, remote_root, pointer).await?;
    Ok(())
}

pub(super) async fn recover_current_remote_snapshot(
    remote: &CloudRemote,
    remote_root: &str,
) -> AppResult<PortableSnapshot> {
    let Some(snapshot) = read_current_sync_snapshot_compat(remote, remote_root).await? else {
        return Err(AppError::Config(
            "No current cloud sync snapshot is available for recovery".to_string(),
        ));
    };
    let pointer = pointer_from_snapshot(&snapshot);
    // Recovery only needs a free gist slot if the snapshot file is missing.
    prune_gist_snapshots_best_effort(remote, remote_root, Some(&pointer), &snapshot.revision_id)
        .await;
    upload_sync_snapshot(remote, remote_root, &snapshot).await?;
    verify_uploaded_sync_snapshot(remote, remote_root, &pointer).await?;
    commit_sync_pointer(remote, remote_root, &pointer).await?;
    Ok(snapshot)
}
