use std::collections::HashSet;
use std::time::Duration;

use crate::error::AppResult;

use super::operator::CloudRemote;
use super::protocol::{sync_snapshot_file, sync_snapshot_path};
use super::remote::{
    RemoteSyncPointer, SYNC_SNAPSHOTS_DIR, current_time_ms, is_legacy_sync_snapshot_path,
    load_sync_pointer, remote_path,
};

pub(super) const SYNC_SNAPSHOT_KEEP_RECENT: usize = 5;
pub(super) const SYNC_SNAPSHOT_GC_GRACE_PERIOD: Duration = Duration::from_secs(24 * 60 * 60);
/// Fixed sync documents that always occupy a gist slot: `sync/latest.redb` and
/// `sync/current.redb.enc`.
const GIST_FIXED_FILE_COUNT: usize = 2;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct SnapshotGcEntry {
    pub path: String,
    pub revision_id: String,
    pub created_at_ms: u64,
    pub deletable: bool,
}

pub(super) fn is_gist_provider(provider: &str) -> bool {
    matches!(provider, "gitee_snippet" | "github_gist")
}

pub(super) async fn cleanup_sync_snapshots(
    remote: &CloudRemote,
    remote_root: &str,
    latest: Option<&RemoteSyncPointer>,
    grace_period: Duration,
) {
    let result = collect_snapshots(remote, remote_root)
        .await
        .map(|snapshots| {
            plan_snapshot_gc(
                snapshots,
                latest.map(|pointer| pointer.revision_id.as_str()),
                current_time_ms(),
                SYNC_SNAPSHOT_KEEP_RECENT,
                grace_period,
            )
        });

    let paths = match result {
        Ok(paths) => paths,
        Err(error) => {
            tracing::warn!("Failed to plan cloud sync snapshot cleanup: {}", error);
            return;
        }
    };

    delete_snapshot_paths(remote, paths).await;
}

/// Free gist capacity for the target snapshot upload.
///
/// Only runs for backends with a hard file-count limit, and only when the
/// upcoming upload would exceed it. Reading every snapshot to rank generations is
/// expensive, so the common case stops after a single cheap remote file count.
pub(super) async fn prune_gist_snapshots_before_upload(
    remote: &CloudRemote,
    remote_root: &str,
    latest: Option<&RemoteSyncPointer>,
    target_revision: &str,
) -> AppResult<()> {
    let Some(capacity_limit) = remote.file_capacity_limit() else {
        // No hard limit: retention is handled by `cleanup_sync_snapshots`.
        return Ok(());
    };
    let Some(total_files) = remote.gist_file_count().await? else {
        return Ok(());
    };

    // Below capacity, even a new file fits without further remote inspection.
    if total_files.saturating_add(1) <= capacity_limit {
        return Ok(());
    }
    let target_exists = remote
        .exists(&sync_snapshot_path(remote_root, target_revision))
        .await?;
    let projected_total = total_files.saturating_add(usize::from(!target_exists));
    if projected_total <= capacity_limit {
        return Ok(());
    }

    tracing::info!(
        total_files,
        capacity_limit,
        "Gist is at capacity; pruning snapshots before upload"
    );
    let snapshots = collect_snapshots(remote, remote_root).await?;
    // Snapshot reads can take time. Refresh the remote head before planning any
    // deletes, while also retaining the caller's expected head or recovery candidate.
    // If the head cannot be read, return without deleting anything; the best-effort
    // wrapper still allows the upload to continue.
    let current_latest = load_sync_pointer(remote, remote_root).await?;
    let protected_revisions = [
        Some(target_revision),
        latest.map(|pointer| pointer.revision_id.as_str()),
        current_latest
            .as_ref()
            .map(|pointer| pointer.revision_id.as_str()),
    ]
    .into_iter()
    .flatten()
    .collect();
    let paths = plan_gist_capacity_prune(
        &snapshots,
        projected_total,
        capacity_limit,
        &protected_revisions,
    );

    let needed = projected_total - capacity_limit;
    let unmanaged_files = total_files.saturating_sub(snapshots.len());
    if unmanaged_files > GIST_FIXED_FILE_COUNT {
        if paths.len() < needed {
            tracing::warn!(
                total_files,
                unmanaged_files,
                freed = paths.len(),
                needed,
                "Gist holds files outside the current sync root; the snapshot prune cannot free enough capacity"
            );
        } else {
            tracing::info!(
                total_files,
                unmanaged_files,
                freed = paths.len(),
                "Gist holds files outside the current sync root; that capacity cannot be reclaimed"
            );
        }
    }

    delete_snapshot_paths(remote, paths).await;
    Ok(())
}

/// Best-effort capacity pruning for gist remotes.
///
/// Inspecting the remote can fail on its own (network, quota on reads). That must
/// never block the upload that follows, so failures are only logged.
pub(super) async fn prune_gist_snapshots_best_effort(
    remote: &CloudRemote,
    remote_root: &str,
    latest: Option<&RemoteSyncPointer>,
    target_revision: &str,
) {
    if !remote.is_gist_backend() {
        return;
    }

    if let Err(error) =
        prune_gist_snapshots_before_upload(remote, remote_root, latest, target_revision).await
    {
        tracing::warn!(
            error = %error,
            "Gist snapshot pruning did not complete; continuing with upload"
        );
    }
}

/// Pure planning half of [`prune_gist_snapshots_before_upload`].
///
/// Returns safely deletable snapshot files, oldest first, to make room for the
/// upload. Unreadable snapshots and protected pointer targets are never touched,
/// even when too few safe candidates remain to meet `capacity_limit`.
/// The projected file count grows only when the target snapshot is missing.
pub(super) fn plan_gist_capacity_prune(
    snapshots: &[SnapshotGcEntry],
    projected_total: usize,
    capacity_limit: usize,
    protected_revisions: &HashSet<&str>,
) -> Vec<String> {
    if projected_total <= capacity_limit {
        return Vec::new();
    }
    let needed = projected_total - capacity_limit;

    let mut candidates: Vec<&SnapshotGcEntry> = snapshots
        .iter()
        .filter(|snapshot| snapshot.deletable)
        .filter(|snapshot| !protected_revisions.contains(snapshot.revision_id.as_str()))
        .collect();
    candidates.sort_by_key(|snapshot| snapshot.created_at_ms);

    candidates
        .into_iter()
        .take(needed)
        .map(|snapshot| snapshot.path.clone())
        .collect()
}

async fn delete_snapshot_paths(remote: &CloudRemote, paths: Vec<String>) {
    for path in paths {
        if let Err(error) = remote.delete(&path).await {
            tracing::warn!(
                path = %path,
                error = %error,
                "Failed to delete old cloud sync snapshot"
            );
        }
    }
}

async fn collect_snapshots(
    remote: &CloudRemote,
    remote_root: &str,
) -> AppResult<Vec<SnapshotGcEntry>> {
    let prefix = remote_path(remote_root, SYNC_SNAPSHOTS_DIR);
    let paths = remote.list_files(&prefix).await?;
    let mut snapshots = Vec::new();
    for path in paths
        .into_iter()
        .filter(|path| is_legacy_sync_snapshot_path(path, remote_root))
    {
        let Some(revision_id) = snapshot_revision_from_path(&path) else {
            snapshots.push(SnapshotGcEntry {
                path,
                revision_id: String::new(),
                created_at_ms: 0,
                deletable: false,
            });
            continue;
        };
        let pointer = RemoteSyncPointer {
            schema_version: 2,
            revision_id: revision_id.clone(),
            created_at_ms: 0,
            payload_hash: String::new(),
            device_id: String::new(),
            app_version: String::new(),
        };
        match read_snapshot_for_gc(remote, remote_root, &pointer).await {
            Ok((created_at_ms, payload_hash)) => snapshots.push(SnapshotGcEntry {
                path,
                revision_id,
                created_at_ms,
                deletable: !payload_hash.is_empty(),
            }),
            Err(error) => {
                tracing::warn!(
                    path = %path,
                    error = %error,
                    "Cloud sync snapshot is not readable; keeping it during cleanup"
                );
                snapshots.push(SnapshotGcEntry {
                    path,
                    revision_id,
                    created_at_ms: 0,
                    deletable: false,
                });
            }
        }
    }
    Ok(snapshots)
}

async fn read_snapshot_for_gc(
    remote: &CloudRemote,
    remote_root: &str,
    pointer: &RemoteSyncPointer,
) -> AppResult<(u64, String)> {
    let Some(raw) = remote
        .read_if_exists(&remote_path(
            remote_root,
            &sync_snapshot_file(&pointer.revision_id),
        ))
        .await?
    else {
        return Ok((0, String::new()));
    };
    let decrypted = super::crypto::decrypt_snapshot_bytes(&raw)?;
    let snapshot = crate::core::portable_snapshot::decode_portable_snapshot(&decrypted)?;
    if snapshot.revision_id != pointer.revision_id {
        return Err(crate::error::CloudSyncError::RevisionMismatch {
            pointer_revision: pointer.revision_id.clone(),
            snapshot_revision: snapshot.revision_id,
        }
        .into());
    }
    Ok((snapshot.created_at_ms, snapshot.payload_hash))
}

/// Revisions that must survive any cleanup: the latest pointer target plus the
/// most recent `keep_recent` generations.
pub(super) fn protected_revision_ids(
    snapshots: &[SnapshotGcEntry],
    latest_revision: Option<&str>,
    keep_recent: usize,
) -> HashSet<String> {
    let mut protected: HashSet<String> = HashSet::new();
    if let Some(latest_revision) = latest_revision {
        protected.insert(latest_revision.to_string());
    }

    let mut by_age: Vec<&SnapshotGcEntry> = snapshots.iter().collect();
    by_age.sort_by_key(|snapshot| snapshot.created_at_ms);
    for snapshot in by_age.into_iter().rev().take(keep_recent) {
        protected.insert(snapshot.revision_id.clone());
    }
    protected
}

pub(super) fn plan_snapshot_gc(
    mut snapshots: Vec<SnapshotGcEntry>,
    latest_revision: Option<&str>,
    now_ms: u64,
    keep_recent: usize,
    grace_period: Duration,
) -> Vec<String> {
    let protected = protected_revision_ids(&snapshots, latest_revision, keep_recent);

    let grace_ms = u64::try_from(grace_period.as_millis()).unwrap_or(u64::MAX);
    snapshots.sort_by_key(|snapshot| snapshot.created_at_ms);
    snapshots
        .into_iter()
        .filter(|snapshot| snapshot.deletable)
        .filter(|snapshot| !protected.contains(&snapshot.revision_id))
        .filter(|snapshot| now_ms.saturating_sub(snapshot.created_at_ms) > grace_ms)
        .map(|snapshot| snapshot.path)
        .collect()
}

fn snapshot_revision_from_path(path: &str) -> Option<String> {
    let filename = path.rsplit('/').next()?;
    filename
        .strip_suffix(".redb.enc")
        .filter(|revision| !revision.is_empty())
        .map(ToString::to_string)
}

#[cfg(test)]
mod tests {
    use super::super::operator::MemoryRemote;
    use super::*;

    fn entry(revision: &str, created_at_ms: u64) -> SnapshotGcEntry {
        SnapshotGcEntry {
            path: format!("nyaterm/sync/snapshots/{revision}.redb.enc"),
            revision_id: revision.to_string(),
            created_at_ms,
            deletable: true,
        }
    }

    #[test]
    fn gc_keeps_latest_even_when_it_is_old() {
        let delete = plan_snapshot_gc(
            vec![
                entry("r1", 1),
                entry("r2", 2),
                entry("r3", 3),
                entry("r4", 4),
                entry("r5", 5),
                entry("r6", 6),
                entry("r7", 7),
            ],
            Some("r1"),
            100_000_000,
            5,
            Duration::from_secs(0),
        );

        assert!(!delete.iter().any(|path| path.contains("r1.redb.enc")));
    }

    #[test]
    fn gc_protects_recent_orphans() {
        let delete = plan_snapshot_gc(
            vec![entry("old", 1), entry("fresh", 99_000)],
            None,
            100_000,
            0,
            Duration::from_secs(2),
        );

        assert_eq!(delete, vec!["nyaterm/sync/snapshots/old.redb.enc"]);
    }

    #[test]
    fn gist_pre_upload_keep_leaves_one_slot() {
        let delete = plan_snapshot_gc(
            vec![
                entry("r1", 1),
                entry("r2", 2),
                entry("r3", 3),
                entry("r4", 4),
                entry("r5", 5),
            ],
            Some("r5"),
            100_000,
            SYNC_SNAPSHOT_KEEP_RECENT.saturating_sub(1),
            Duration::from_secs(0),
        );

        assert_eq!(delete, vec!["nyaterm/sync/snapshots/r1.redb.enc"]);
    }

    fn full_gist_snapshots() -> Vec<SnapshotGcEntry> {
        (1..=8)
            .map(|index| entry(&format!("r{index}"), index))
            .collect()
    }

    #[test]
    fn gist_capacity_prune_frees_exactly_the_needed_slot() {
        // latest + current + 8 snapshots = 10 files, so the next upload needs one
        // slot freed.
        let delete =
            plan_gist_capacity_prune(&full_gist_snapshots(), 11, 10, &HashSet::from(["r8"]));

        assert_eq!(
            delete,
            vec!["nyaterm/sync/snapshots/r1.redb.enc".to_string()]
        );
    }

    #[test]
    fn gist_capacity_prune_is_a_no_op_when_there_is_room() {
        let delete =
            plan_gist_capacity_prune(&full_gist_snapshots(), 8, 10, &HashSet::from(["r8"]));

        assert!(delete.is_empty());
    }

    #[test]
    fn gist_capacity_prune_frees_as_many_slots_as_needed() {
        // 12 files against a limit of 10: the upload needs three slots, oldest first.
        let delete =
            plan_gist_capacity_prune(&full_gist_snapshots(), 13, 10, &HashSet::from(["r8"]));

        assert_eq!(
            delete,
            vec![
                "nyaterm/sync/snapshots/r1.redb.enc".to_string(),
                "nyaterm/sync/snapshots/r2.redb.enc".to_string(),
                "nyaterm/sync/snapshots/r3.redb.enc".to_string(),
            ]
        );
    }

    #[test]
    fn gist_capacity_prune_never_drops_the_latest_pointer_snapshot() {
        // r1 is both the oldest and the pointer target: r2 goes instead.
        let delete =
            plan_gist_capacity_prune(&full_gist_snapshots(), 11, 10, &HashSet::from(["r1"]));

        assert_eq!(
            delete,
            vec!["nyaterm/sync/snapshots/r2.redb.enc".to_string()]
        );
    }

    #[test]
    fn gist_capacity_prune_keeps_unreadable_orphans() {
        let mut snapshots = vec![
            SnapshotGcEntry {
                path: "nyaterm/sync/snapshots/broken.redb.enc".to_string(),
                revision_id: "broken".to_string(),
                created_at_ms: 0,
                deletable: false,
            },
            SnapshotGcEntry {
                path: "nyaterm/sync/snapshots/.redb.enc".to_string(),
                revision_id: String::new(),
                created_at_ms: 0,
                deletable: false,
            },
        ];
        snapshots.extend(full_gist_snapshots());

        // The oldest safe generation goes first, despite the orphans' zero timestamps.
        let delete = plan_gist_capacity_prune(&snapshots, 11, 10, &HashSet::from(["r8"]));

        assert_eq!(delete, vec!["nyaterm/sync/snapshots/r1.redb.enc"]);
    }

    #[test]
    fn gist_capacity_prune_returns_empty_when_only_latest_is_deletable() {
        let mut snapshots = full_gist_snapshots();
        for snapshot in &mut snapshots {
            snapshot.deletable = snapshot.revision_id == "r8";
        }

        let delete = plan_gist_capacity_prune(&snapshots, 11, 10, &HashSet::from(["r8"]));

        assert!(delete.is_empty());
    }

    #[test]
    fn gist_capacity_prune_selects_only_safe_candidates_in_age_order() {
        let mut unreadable = entry("broken", 0);
        unreadable.deletable = false;
        let mut unsafe_snapshot = entry("unsafe", 2);
        unsafe_snapshot.deletable = false;
        let snapshots = vec![
            entry("r5", 5),
            unreadable,
            entry("latest", 1),
            entry("r3", 3),
            unsafe_snapshot,
            entry("r4", 4),
        ];

        // Four slots needed, but only three candidates are safe to delete.
        let delete = plan_gist_capacity_prune(&snapshots, 14, 10, &HashSet::from(["latest"]));

        assert_eq!(
            delete,
            vec![
                "nyaterm/sync/snapshots/r3.redb.enc".to_string(),
                "nyaterm/sync/snapshots/r4.redb.enc".to_string(),
                "nyaterm/sync/snapshots/r5.redb.enc".to_string(),
            ]
        );
    }

    const SNAPSHOT_LIST_PREFIX: &str = "nyaterm/sync/snapshots/";

    #[test]
    fn gist_capacity_prune_protects_both_heads_when_safe_candidates_are_insufficient() {
        let snapshots = vec![
            entry("r9", 1),
            entry("r8", 8),
            entry("r7", 7),
            entry("r6", 6),
        ];

        // Three slots needed, but both the expected and current head must survive.
        let delete = plan_gist_capacity_prune(&snapshots, 13, 10, &HashSet::from(["r8", "r9"]));

        assert_eq!(
            delete,
            vec![
                "nyaterm/sync/snapshots/r6.redb.enc",
                "nyaterm/sync/snapshots/r7.redb.enc",
            ]
        );
    }

    /// A gist holding a single managed file; tests set the capacity limit they
    /// need explicitly.
    fn gist_remote_with_one_file(mark_gist_backend: bool) -> (MemoryRemote, CloudRemote) {
        let memory = MemoryRemote::with_files(std::collections::HashMap::from([(
            "nyaterm/sync/latest.redb".to_string(),
            vec![1u8],
        )]));
        if mark_gist_backend {
            memory.mark_gist_backend();
        }
        let remote = CloudRemote::Memory(memory.clone());
        (memory, remote)
    }

    #[tokio::test(flavor = "current_thread")]
    async fn gist_prune_skips_the_remote_without_a_capacity_limit() {
        let (memory, remote) = gist_remote_with_one_file(true);
        memory.fail_next_list_containing(SNAPSHOT_LIST_PREFIX);

        // No limit yet: the prune must not read a single snapshot.
        prune_gist_snapshots_before_upload(&remote, "nyaterm", None, "new")
            .await
            .expect("nothing to prune without a capacity limit");

        memory.set_file_capacity_limit(1);
        assert!(
            prune_gist_snapshots_before_upload(&remote, "nyaterm", None, "new")
                .await
                .is_err(),
            "the pending injected failure proves the first call never listed snapshots"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn gist_prune_stops_after_the_cheap_count_when_there_is_room() {
        let (memory, remote) = gist_remote_with_one_file(true);
        memory.set_file_capacity_limit(10);
        memory.fail_next_list_containing(SNAPSHOT_LIST_PREFIX);

        prune_gist_snapshots_before_upload(&remote, "nyaterm", None, "new")
            .await
            .expect("one file is far below the limit");

        memory.set_file_capacity_limit(1);
        assert!(
            prune_gist_snapshots_before_upload(&remote, "nyaterm", None, "new")
                .await
                .is_err(),
            "the pending injected failure proves the first call never listed snapshots"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn gist_prune_skips_snapshot_reads_when_overwriting_at_capacity() {
        let (memory, remote) = gist_remote_with_one_file(true);
        let target = sync_snapshot_path("nyaterm", "existing");
        remote
            .write(&target, vec![1u8])
            .await
            .expect("seed corrupt target");
        memory.set_file_capacity_limit(2);
        memory.fail_next_list_containing(SNAPSHOT_LIST_PREFIX);

        prune_gist_snapshots_before_upload(&remote, "nyaterm", None, "existing")
            .await
            .expect("overwriting an existing target requires no slot or snapshot reads");

        assert_eq!(memory.file_count(), 2);
        assert_eq!(memory.file(&target), Some(vec![1u8]));
        let error = prune_gist_snapshots_before_upload(&remote, "nyaterm", None, "new")
            .await
            .expect_err("a missing target must still try to prune");
        assert!(error.to_string().contains("injected memory list failure"));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn gist_best_effort_prune_swallows_remote_failures() {
        let (memory, remote) = gist_remote_with_one_file(true);
        memory.set_file_capacity_limit(1);
        memory.fail_next_list_containing(SNAPSHOT_LIST_PREFIX);

        prune_gist_snapshots_best_effort(&remote, "nyaterm", None, "new").await;

        memory.fail_next_list_containing(SNAPSHOT_LIST_PREFIX);
        assert!(
            prune_gist_snapshots_before_upload(&remote, "nyaterm", None, "new")
                .await
                .is_err(),
            "the injected list failure must be real"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn gist_best_effort_prune_skips_non_gist_backends() {
        let (memory, remote) = gist_remote_with_one_file(false);
        memory.fail_next_list_containing(SNAPSHOT_LIST_PREFIX);

        // A non-gist backend must not even talk to the remote here.
        prune_gist_snapshots_best_effort(&remote, "nyaterm", None, "new").await;

        memory.set_file_capacity_limit(1);
        assert!(
            prune_gist_snapshots_before_upload(&remote, "nyaterm", None, "new")
                .await
                .is_err(),
            "the pending injected failure proves the best-effort call stayed local"
        );
    }

    #[test]
    fn is_gist_provider_detects_snippet_backends() {
        assert!(is_gist_provider("gitee_snippet"));
        assert!(is_gist_provider("github_gist"));
        assert!(!is_gist_provider("webdav"));
        assert!(!is_gist_provider("s3"));
    }
}
