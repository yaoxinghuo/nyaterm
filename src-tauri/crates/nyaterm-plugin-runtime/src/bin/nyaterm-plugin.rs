//! Package authoring CLI. Run through `pnpm plugin:pack` or directly with Cargo.
use nyaterm_plugin_runtime::manifest::{Manifest, validate_path};
use nyaterm_plugin_runtime::package::prepare;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use zip::write::SimpleFileOptions;

fn main() {
    if let Err(error) = run() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    match args.first().map(String::as_str) {
        Some("pack") if args.len() == 4 => {
            let source = PathBuf::from(&args[1]).canonicalize()?;
            let output = PathBuf::from(&args[2]);
            let manifest = Manifest::parse(&std::fs::read(source.join("manifest.json"))?, &args[3])?;
            let mut files = BTreeMap::new();
            files.insert("manifest.json".into(), std::fs::read(source.join("manifest.json"))?);
            for directory in ["ui", "assets", "bin"] {
                if source.join(directory).exists() { collect(&source, &source.join(directory), &mut files)?; }
            }
            let sdk = include_bytes!("../../../../../plugins/sdk/nyaterm.js").to_vec();
            if files.get("ui/nyaterm-sdk.js").is_some_and(|bytes| bytes != &sdk) { return Err("ui/nyaterm-sdk.js is reserved for the host SDK".into()); }
            files.insert("ui/nyaterm-sdk.js".into(), sdk);
            if files.len() > 1022 || files.values().map(Vec::len).sum::<usize>() > 256 * 1024 * 1024 { return Err("Plugin package exceeds file count or expanded size limit".into()); }
            let parent = output.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or_else(|| Path::new("."));
            std::fs::create_dir_all(parent)?;
            let mut staging = tempfile::NamedTempFile::new_in(parent)?;
            let checksums = files.iter().map(|(name, bytes)| (name.clone(), hex::encode(Sha256::digest(bytes)))).collect::<BTreeMap<_, _>>();
            {
                let mut archive = zip::ZipWriter::new(staging.as_file_mut());
                for (name, bytes) in files {
                    let options = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated)
                        .unix_permissions(if name.starts_with("bin/") { 0o755 } else { 0o644 });
                    archive.start_file(name, options)?;
                    archive.write_all(&bytes)?;
                }
                archive.start_file("checksums.json", SimpleFileOptions::default())?;
                archive.write_all(&serde_json::to_vec_pretty(&checksums)?)?;
                archive.finish()?;
            }
            // Run the real installer checks before publishing the archive.
            let staging_parent = tempfile::tempdir()?;
            let validated = prepare(staging.path(), staging_parent.path(), &args[3])?;
            staging.as_file().sync_all()?;
            staging.persist(&output).map_err(|e| e.error)?;
            println!("Packaged {} {} -> {}\nSHA-256: {}", manifest.id, manifest.version, output.display(), validated.preview.digest);
        }
        Some("inspect") if args.len() == 3 => {
            let staging = tempfile::tempdir()?;
            let package = prepare(Path::new(&args[1]), staging.path(), &args[2])?;
            println!("{}", serde_json::to_string_pretty(&package.preview)?);
        }
        _ => return Err("Usage: nyaterm-plugin pack <directory> <output.nyap> <app-version>\n       nyaterm-plugin inspect <package.nyap> <app-version>".into()),
    }
    Ok(())
}

fn collect(
    root: &Path,
    path: &Path,
    files: &mut BTreeMap<String, Vec<u8>>,
) -> Result<(), Box<dyn std::error::Error>> {
    let metadata = std::fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() {
        return Err("Plugin source cannot contain symlinks".into());
    }
    if metadata.is_dir() {
        for entry in std::fs::read_dir(path)? {
            collect(root, &entry?.path(), files)?;
        }
    } else if metadata.is_file() {
        let relative = path
            .strip_prefix(root)?
            .to_str()
            .ok_or("Non-UTF-8 plugin path")?
            .replace('\\', "/");
        validate_path(&relative)?;
        if files.len() >= 1022 || metadata.len() > 64 * 1024 * 1024 {
            return Err("Plugin source is too large".into());
        }
        if files.values().map(Vec::len).sum::<usize>() + metadata.len() as usize > 256 * 1024 * 1024
        {
            return Err("Plugin source exceeds the expanded package limit".into());
        }
        let mut bytes = Vec::new();
        std::fs::File::open(path)?
            .take(64 * 1024 * 1024 + 1)
            .read_to_end(&mut bytes)?;
        if bytes.len() > 64 * 1024 * 1024 {
            return Err("Plugin file is too large".into());
        }
        files.insert(relative, bytes);
    } else {
        return Err("Plugin source cannot contain special files".into());
    }
    Ok(())
}
