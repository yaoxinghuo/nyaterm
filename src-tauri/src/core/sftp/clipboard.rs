use super::*;
use std::future::Future;

static MOVE_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

pub(super) fn normalize_remote_path(path: &str) -> AppResult<String> {
    if !path.starts_with('/') || path.contains('\0') {
        return Err(AppError::Config(format!(
            "Invalid absolute remote path: {path}"
        )));
    }
    let mut parts = Vec::new();
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            _ => parts.push(part),
        }
    }
    Ok(format!("/{}", parts.join("/")))
}

pub(super) fn validate_remote_copy_destination(
    source: &str,
    target_dir: &str,
    name: &str,
    directory: bool,
    same_endpoint: bool,
) -> AppResult<()> {
    if name.is_empty() || name == "." || name == ".." || name.contains('/') || name.contains('\0') {
        return Err(AppError::Config("Invalid destination file name".into()));
    }
    let source = normalize_remote_path(source)?;
    let target = normalize_remote_path(target_dir)?;
    if source == "/" {
        return Err(AppError::Config(
            "Cannot copy or cut the remote root".into(),
        ));
    }
    if same_endpoint
        && (normalize_remote_path(&join_remote_child_path(&target, name))? == source
            || (directory && (target == source || target.starts_with(&format!("{source}/")))))
    {
        return Err(AppError::Config(
            "Destination is the source or inside the source directory".into(),
        ));
    }
    Ok(())
}

pub(super) async fn same_remote_endpoint(
    manager: &SessionManager,
    source: &str,
    target: &str,
) -> AppResult<bool> {
    if source == target {
        return Ok(true);
    }
    let sessions = manager.sessions.lock().await;
    let identity = |id: &str| -> AppResult<Option<(String, u16, String)>> {
        let session = sessions
            .get(id)
            .ok_or_else(|| AppError::SessionNotFound(id.into()))?;
        Ok(session
            .ssh_config
            .as_ref()
            .and_then(|cfg| cfg.downcast_ref::<crate::core::ssh::SshConfig>())
            .map(|cfg| (cfg.host.to_lowercase(), cfg.port, cfg.username.clone())))
    };
    let source = identity(source)?;
    Ok(source.is_some() && source == identity(target)?)
}

// Skip and cancellation are not successful copies, even when returned as Ok.
async fn delete_only_after_copied(
    outcome: AppResult<CopyEntryOutcome>,
    delete: impl Future<Output = AppResult<()>>,
) -> AppResult<CopyEntryOutcome> {
    let outcome = outcome?;
    if outcome == CopyEntryOutcome::Copied {
        delete.await?;
    }
    Ok(outcome)
}

async fn ensure_no_symlink_ancestors(fs: &dyn RemoteFs, path: &str) -> AppResult<()> {
    let path = normalize_remote_path(path)?;
    let mut current = String::new();
    for part in path.split('/').filter(|part| !part.is_empty()) {
        current.push('/');
        current.push_str(part);
        if fs.stat(&current).await?.is_symlink {
            return Err(AppError::Config(format!(
                "Cut through symbolic links is not supported: {current}"
            )));
        }
    }
    Ok(())
}

async fn move_inventory(fs: &dyn RemoteFs, root: &str) -> AppResult<Vec<(String, bool, u64, u64)>> {
    let mut inventory = Vec::new();
    let mut stack = vec![(root.to_string(), String::new())];
    while let Some((path, relative)) = stack.pop() {
        let props = fs.stat(&path).await?;
        if props.is_symlink {
            return Err(AppError::Config(format!(
                "Cut of symbolic links is not supported: {path}"
            )));
        }
        inventory.push((relative.clone(), props.is_dir, props.size, props.mtime));
        if props.is_dir {
            for entry in fs.list_dir(&path).await? {
                if entry.name == "." || entry.name == ".." {
                    continue;
                }
                stack.push((
                    join_remote_child_path(&path, &entry.name),
                    format!("{relative}/{}", entry.name),
                ));
            }
        }
    }
    inventory.sort();
    Ok(inventory)
}

pub async fn move_file_entry(
    app: tauri::AppHandle,
    manager: Arc<SessionManager>,
    mut request: CopyFileEntryRequest,
) -> AppResult<CopyEntryOutcome> {
    validate_copy_session_generations(&manager, &request).await?;
    if request.source.kind != CopyEndpointKind::Remote
        || request.target.kind != CopyEndpointKind::Remote
    {
        return Err(AppError::Config("Cut requires remote endpoints".into()));
    }
    ensure_local_session_kind(&manager, &request.source.session_id, &request.source.kind).await?;
    request.source.path = normalize_remote_path(&request.source.path)?;
    request.target.path = normalize_remote_path(&request.target.path)?;
    ensure_local_session_kind(&manager, &request.target.session_id, &request.target.kind).await?;
    let transfer_id = request
        .transfer_id
        .clone()
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    request.transfer_id = Some(transfer_id.clone());
    let retained =
        transfer::RetainedTransfer::new(transfer::create_child_file_transfer_controller(
            Some(transfer_id),
            &request.source.session_id,
            request.file_name.clone(),
            &request.source.path,
            &request.target.path,
            "copy",
            None,
        ));
    // Serialize move conflict resolution so a mixed tree selection cannot race rename candidates.
    let _move_guard = MOVE_LOCK.lock().await;
    retained.ready().await?;
    let source_auto = get_or_create_auto_fs(&manager, &request.source.session_id).await?;
    let target_auto = get_or_create_auto_fs(&manager, &request.target.session_id).await?;
    let source_guard = source_auto.backend().await?;
    let target_guard = target_auto.backend().await?;
    let source_fs = source_guard.as_ref().unwrap().as_ref();
    let target_fs = target_guard.as_ref().unwrap().as_ref();
    ensure_no_symlink_ancestors(source_fs, &request.source.path).await?;
    ensure_no_symlink_ancestors(target_fs, &request.target.path).await?;
    let before = move_inventory(source_fs, &request.source.path).await?;
    request.is_directory = before[0].1;
    validate_remote_copy_destination(
        &request.source.path,
        &request.target.path,
        &request.file_name,
        request.is_directory,
        same_remote_endpoint(
            &manager,
            &request.source.session_id,
            &request.target.session_id,
        )
        .await?,
    )?;
    let source_path = request.source.path.clone();
    let source_session = request.source.session_id.clone();
    let target_session = request.target.session_id.clone();
    // Resolve conflicts once, then use the exact resolved destination for verification.
    let settings = crate::config::load_app_settings(&app)
        .map(|s| s.transfer)
        .unwrap_or_default();
    let strategy = request
        .duplicate_strategy_override
        .as_deref()
        .unwrap_or(&settings.duplicate_strategy);
    let destination = join_remote_child_path(&request.target.path, &request.file_name);
    let resolved = resolve_remote_copy_target_generic(
        &app,
        &manager,
        target_fs,
        &request.target.session_id,
        &destination,
        &request.file_name,
        strategy,
    )
    .await?;
    let Some(resolved) = resolved else {
        emit_copy_cancelled(
            &app,
            &source_session,
            request.file_name,
            &source_path,
            &destination,
            request.is_directory,
            request.transfer_id,
        );
        return Ok(CopyEntryOutcome::Skipped);
    };
    if let Ok(props) = target_fs.stat(&resolved.path).await {
        if props.is_symlink {
            return Err(AppError::Config("Cannot cut onto a symbolic link".into()));
        }
        if props.is_dir {
            move_inventory(target_fs, &resolved.path).await?;
        }
    }
    request.file_name = file_name_from_path(&resolved.path);
    request.duplicate_strategy_override = Some("overwrite".into());
    retained.ready().await?;
    let outcome = copy_file_entry_with_outcome(app, manager.clone(), request).await;
    delete_only_after_copied(outcome, async {
        retained.ready().await?;
        ensure_local_session_kind(&manager, &source_session, &CopyEndpointKind::Remote).await?;
        ensure_local_session_kind(&manager, &target_session, &CopyEndpointKind::Remote).await?;
        if !Arc::ptr_eq(
            &source_auto,
            &get_or_create_auto_fs(&manager, &source_session).await?,
        ) || !Arc::ptr_eq(
            &target_auto,
            &get_or_create_auto_fs(&manager, &target_session).await?,
        ) {
            return Err(AppError::Channel(
                "Session reconnected during copy; source was kept".into(),
            ));
        }
        if move_inventory(source_fs, &source_path).await? != before {
            return Err(AppError::Channel(
                "Source changed during copy; source was kept".into(),
            ));
        }
        for (relative, directory, size, _) in &before {
            retained.ready().await?;
            let target_path = format!("{}{relative}", resolved.path);
            ensure_no_symlink_ancestors(target_fs, &target_path).await?;
            let props = target_fs.stat(&target_path).await?;
            if props.is_dir != *directory || (!directory && props.size != *size) {
                return Err(AppError::Channel(format!(
                    "Copy verification failed: {target_path}; source was kept"
                )));
            }
        }
        retained.ready().await?;
        source_fs.remove_file(&source_path).await
    })
    .await
}

pub async fn find_missing_remote_entries(
    manager: Arc<SessionManager>,
    session_id: &str,
    paths: Vec<String>,
) -> AppResult<Vec<String>> {
    ensure_local_session_kind(&manager, session_id, &CopyEndpointKind::Remote).await?;
    let auto = get_or_create_auto_fs(&manager, session_id).await?;
    let guard = auto.backend().await?;
    let fs = guard.as_ref().unwrap();
    let mut missing = Vec::new();
    for path in paths {
        match fs.stat(&path).await {
            Ok(_) => {}
            Err(error) if is_remote_delete_not_found(&error) => missing.push(path),
            Err(error) => return Err(error),
        }
    }
    Ok(missing)
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Fixture(std::path::PathBuf);
    impl Fixture {
        fn new() -> Self {
            let root =
                std::env::temp_dir().join(format!("nyaterm-cut-test-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir(&root).unwrap();
            std::fs::write(root.join("source"), b"source content").unwrap();
            std::fs::write(root.join("target"), b"existing target").unwrap();
            Self(root)
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    #[tokio::test]
    async fn clipboard_cut_skip_and_ask_skip_preserve_source_and_target() {
        // Both duplicate=skip and ask->Skip resolve to the same explicit outcome.
        for _strategy in ["skip", "ask_skip"] {
            let fixture = Fixture::new();
            delete_only_after_copied(Ok(CopyEntryOutcome::Skipped), async {
                std::fs::remove_file(fixture.0.join("source"))?;
                Ok(())
            })
            .await
            .unwrap();
            assert_eq!(
                std::fs::read(fixture.0.join("source")).unwrap(),
                b"source content"
            );
            assert_eq!(
                std::fs::read(fixture.0.join("target")).unwrap(),
                b"existing target"
            );
        }
    }
    #[tokio::test]
    async fn clipboard_cut_completed_copy_removes_source_and_keeps_target() {
        let fixture = Fixture::new();
        std::fs::copy(fixture.0.join("source"), fixture.0.join("target")).unwrap();
        delete_only_after_copied(Ok(CopyEntryOutcome::Copied), async {
            std::fs::remove_file(fixture.0.join("source"))?;
            Ok(())
        })
        .await
        .unwrap();
        assert!(!fixture.0.join("source").exists());
        assert_eq!(
            std::fs::read(fixture.0.join("target")).unwrap(),
            b"source content"
        );
    }
    #[tokio::test]
    async fn clipboard_cut_failed_copy_preserves_source() {
        let fixture = Fixture::new();
        assert!(
            delete_only_after_copied(
                Err(AppError::Channel("network disconnected".into())),
                async {
                    std::fs::remove_file(fixture.0.join("source"))?;
                    Ok(())
                }
            )
            .await
            .is_err()
        );
        assert!(fixture.0.join("source").exists());
    }
    #[tokio::test]
    async fn clipboard_cut_delete_failure_is_not_reported_as_moved() {
        assert!(
            delete_only_after_copied(Ok(CopyEntryOutcome::Copied), async {
                Err(AppError::Channel(
                    "permission denied deleting source".into(),
                ))
            })
            .await
            .is_err()
        );
    }
    #[test]
    fn clipboard_destination_boundaries() {
        for target in ["/a", "/a/b", "/a//b/", "/c/../a/./b"] {
            assert!(validate_remote_copy_destination("/a/", target, "a", true, true).is_err());
        }
        assert!(validate_remote_copy_destination("/a", "/abc", "a", true, true).is_ok());
        assert!(validate_remote_copy_destination("/a", "/a/b", "a", true, false).is_ok());
        assert!(validate_remote_copy_destination("/a", "/", "a", false, true).is_err());
    }
    #[tokio::test]
    async fn clipboard_cut_skip_cancel_and_failure_keep_source() {
        for outcome in [
            Ok(CopyEntryOutcome::Skipped),
            Ok(CopyEntryOutcome::Cancelled),
            Err(AppError::Channel("copy failed".into())),
        ] {
            let called = std::sync::atomic::AtomicBool::new(false);
            let _ = delete_only_after_copied(outcome, async {
                called.store(true, std::sync::atomic::Ordering::SeqCst);
                Ok(())
            })
            .await;
            assert!(!called.load(std::sync::atomic::Ordering::SeqCst));
        }
    }
    #[tokio::test]
    async fn clipboard_cut_copied_deletes_source() {
        let called = std::sync::atomic::AtomicBool::new(false);
        assert_eq!(
            delete_only_after_copied(Ok(CopyEntryOutcome::Copied), async {
                called.store(true, std::sync::atomic::Ordering::SeqCst);
                Ok(())
            })
            .await
            .unwrap(),
            CopyEntryOutcome::Copied
        );
        assert!(called.load(std::sync::atomic::Ordering::SeqCst));
    }
}
