use std::{path::PathBuf, time::Duration};

#[tauri::command]
pub async fn read_clipboard_file_paths() -> Result<Vec<String>, String> {
    tokio::time::timeout(
        Duration::from_secs(1),
        tokio::task::spawn_blocking(read_file_paths),
    )
    .await
    .map_err(|_| "File clipboard read timed out".to_string())?
    .map_err(|error| format!("File clipboard reader failed: {error}"))?
}

fn read_file_paths() -> Result<Vec<String>, String> {
    let mut clipboard = arboard::Clipboard::new().map_err(|error| error.to_string())?;
    // CF_HDROP on Windows, native NSURL pasteboard objects on macOS, URI list on Linux.
    match clipboard.get().file_list() {
        Ok(paths) => Ok(paths
            .into_iter()
            .filter_map(normalize_native_path)
            .map(|path| path.to_string_lossy().into_owned())
            .collect()),
        Err(arboard::Error::ContentNotAvailable) => {
            #[cfg(target_os = "linux")]
            if let Ok(text) = clipboard.get_text() {
                return Ok(text
                    .lines()
                    .filter_map(parse_file_uri)
                    .map(|path| path.to_string_lossy().into_owned())
                    .collect());
            }
            Ok(Vec::new())
        }
        Err(error) => Err(error.to_string()),
    }
}

fn normalize_native_path(path: PathBuf) -> Option<PathBuf> {
    #[cfg(target_os = "linux")]
    let path = match path.to_str()?.strip_prefix("localhost/") {
        Some(local) => PathBuf::from(format!("/{local}")),
        None => path,
    };
    path.is_absolute().then_some(path)
}

#[cfg(any(target_os = "linux", test))]
fn parse_file_uri(line: &str) -> Option<PathBuf> {
    let uri = line.trim().strip_prefix("file://")?;
    let path = uri
        .strip_prefix("localhost")
        .filter(|p| p.starts_with('/'))
        .unwrap_or(uri);
    if !path.starts_with('/') {
        return None;
    }
    let decoded = urlencoding::decode(path).ok()?;
    if decoded.contains('\0') {
        return None;
    }
    Some(PathBuf::from(decoded.as_ref()))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn clipboard_file_uris_preserve_absolute_localhost_paths() {
        assert_eq!(
            parse_file_uri("file://localhost/home/user/a%20b.txt"),
            Some(PathBuf::from("/home/user/a b.txt"))
        );
        assert_eq!(
            parse_file_uri("file:///home/user/a.txt"),
            Some(PathBuf::from("/home/user/a.txt"))
        );
        for line in [
            "copy",
            "cut",
            "# comment",
            "file://server/home/a",
            "relative",
            "file:///a%00b",
        ] {
            assert!(parse_file_uri(line).is_none());
        }
    }
}
