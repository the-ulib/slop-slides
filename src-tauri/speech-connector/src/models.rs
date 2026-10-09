//! One pinned, verified data-only pack. Downloads/imports stage separately and resume.
use crate::{Error, Result};
use fs2::FileExt;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};

#[derive(Deserialize)]
pub struct Pack {
    pub model: String,
    pub revision: String,
    pub files: Vec<PackFile>,
}
#[derive(Deserialize)]
pub struct PackFile {
    pub path: String,
    pub bytes: u64,
    pub sha256: String,
}
pub fn pack() -> Pack {
    serde_json::from_str(include_str!("../data/custom-voice-pack.json")).expect("pinned pack")
}
pub fn total_bytes() -> u64 {
    pack().files.iter().map(|f| f.bytes).sum()
}
pub fn location(root: &Path) -> PathBuf {
    root.join("models/custom-voice")
}
pub fn installed(root: &Path) -> bool {
    let dir = location(root);
    fs::read_to_string(dir.join("verified.txt")).ok().as_deref() == Some(&pack().revision)
        && pack().files.iter().all(|f| {
            fs::metadata(dir.join(&f.path)).is_ok_and(|m| m.len() == f.bytes && m.is_file())
        })
}
// Explicit unlock also releases a lock briefly inherited by a concurrently
// spawning subprocess; closing only the parent's FD can leave that lock alive.
pub struct PackLock(fs::File);
impl Drop for PackLock {
    fn drop(&mut self) {
        let _ = FileExt::unlock(&self.0);
    }
}
pub fn lock(root: &Path, shared: bool) -> Result<PackLock> {
    fs::create_dir_all(root)?;
    let file = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(root.join("pack.lock"))?;
    let result = if shared {
        FileExt::try_lock_shared(&file)
    } else {
        FileExt::try_lock_exclusive(&file)
    };
    result.map_err(|e| {
        if e.kind() == std::io::ErrorKind::WouldBlock {
            Error::msg(
                "The speech pack is in use by another application. Try again after it finishes.",
            )
        } else {
            Error::msg(format!("Could not lock the speech pack: {e}"))
        }
    })?;
    Ok(PackLock(file))
}
fn cancelled(cancel: &AtomicBool) -> Result<()> {
    if cancel.load(Ordering::Relaxed) {
        Err(Error::msg("Cancelled. You can resume setup later."))
    } else {
        Ok(())
    }
}
pub fn verify_file(path: &Path, expected: &PackFile, cancel: &AtomicBool) -> Result<()> {
    if fs::symlink_metadata(path)?.file_type().is_symlink() {
        return Err(Error::msg("Model files must not be symbolic links."));
    }
    let mut file = fs::File::open(path)?;
    if file.metadata()?.len() != expected.bytes {
        return Err(Error::msg(format!("Incorrect size for {}.", expected.path)));
    }
    let mut hash = Sha256::new();
    let mut buf = vec![0; 1024 * 1024];
    loop {
        cancelled(cancel)?;
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hash.update(&buf[..n]);
    }
    if format!("{:x}", hash.finalize()) != expected.sha256 {
        return Err(Error::msg(format!(
            "Checksum mismatch for {}. Retry setup or choose the pinned pack.",
            expected.path
        )));
    }
    Ok(())
}
pub fn verify(root: &Path, cancel: &AtomicBool) -> Result<()> {
    for f in pack().files {
        verify_file(&location(root).join(&f.path), &f, cancel)?;
    }
    Ok(())
}
enum Input {
    Local(fs::File),
    Remote(reqwest::Response),
}
async fn cancellable<T>(
    future: impl std::future::Future<Output = std::result::Result<T, reqwest::Error>>,
    cancel: &AtomicBool,
) -> Result<T> {
    tokio::pin!(future);
    loop {
        cancelled(cancel)?;
        tokio::select! {
            result = &mut future => return result.map_err(|e| Error::msg(format!("Download failed: {e}. Retry to resume."))),
            _ = tokio::time::sleep(Duration::from_millis(100)) => {},
        }
    }
}
/// Never exposes a partial pack as installed. Exact revision URLs, size and SHA-256 checks.
pub fn install(
    root: &Path,
    source: Option<&Path>,
    cancel: &AtomicBool,
    progress: &dyn Fn(u64, &str),
) -> Result<()> {
    install_pack(root, source, cancel, progress, pack())
}
fn install_pack(
    root: &Path,
    source: Option<&Path>,
    cancel: &AtomicBool,
    progress: &dyn Fn(u64, &str),
    manifest: Pack,
) -> Result<()> {
    let _guard = lock(root, false)?;
    if installed(root) {
        verify(root, cancel)?;
        return Ok(());
    }
    let staging = root.join("models/custom-voice.install");
    fs::create_dir_all(&staging)?;
    let client = if source.is_none() {
        Some(
            reqwest::Client::builder()
                .https_only(true)
                .connect_timeout(Duration::from_secs(20))
                .timeout(Duration::from_secs(1800))
                .build()
                .map_err(|e| Error::msg(e.to_string()))?,
        )
    } else {
        None
    };
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let mut completed = 0;
    for file in &manifest.files {
        cancelled(cancel)?;
        let dest = staging.join(&file.path);
        fs::create_dir_all(dest.parent().expect("pack file parent"))?;
        if verify_file(&dest, file, cancel).is_ok() {
            completed += file.bytes;
            progress(completed, &file.path);
            continue;
        }
        cancelled(cancel)?;
        let mut offset = fs::metadata(&dest).map_or(0, |m| m.len());
        if offset >= file.bytes {
            fs::remove_file(&dest)?;
            offset = 0;
        }
        let mut input: Input = if let Some(dir) = source {
            let input_path = dir.join(&file.path);
            if fs::symlink_metadata(&input_path)?.file_type().is_symlink() {
                return Err(Error::msg("Choose a pack containing regular model files."));
            }
            let mut input = fs::File::open(&input_path)?;
            if input.metadata()?.len() != file.bytes {
                return Err(Error::msg(format!(
                    "Wrong pack: {} has a different size.",
                    file.path
                )));
            }
            input.seek(SeekFrom::Start(offset))?;
            Input::Local(input)
        } else {
            let url = format!(
                "https://huggingface.co/{}/resolve/{}/{}",
                manifest.model, manifest.revision, file.path
            );
            let mut request = client.as_ref().expect("download client").get(url);
            if offset > 0 {
                request = request.header(reqwest::header::RANGE, format!("bytes={offset}-"));
            }
            let response = runtime
                .block_on(cancellable(request.send(), cancel))?
                .error_for_status()
                .map_err(|e| Error::msg(format!("Download failed: {e}. Retry to resume.")))?;
            if offset > 0 && response.status() != reqwest::StatusCode::PARTIAL_CONTENT {
                offset = 0;
            }
            if response.status() == reqwest::StatusCode::PARTIAL_CONTENT {
                let expected =
                    format!("bytes {offset}-{} /{}", file.bytes - 1, file.bytes).replace(" /", "/");
                if response
                    .headers()
                    .get(reqwest::header::CONTENT_RANGE)
                    .and_then(|h| h.to_str().ok())
                    != Some(&expected)
                {
                    return Err(Error::msg("Unexpected download range. Retry setup."));
                }
            }
            Input::Remote(response)
        };
        let mut output = fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(offset == 0)
            .open(&dest)?;
        output.seek(SeekFrom::Start(offset))?;
        let mut received = offset;
        let mut buffer = vec![0; 1024 * 1024];
        progress(completed + received, &file.path);
        loop {
            cancelled(cancel)?;
            let bytes = match &mut input {
                Input::Local(input) => {
                    let n = input.read(&mut buffer)?;
                    buffer[..n].to_vec()
                }
                Input::Remote(input) => runtime
                    .block_on(cancellable(input.chunk(), cancel))?
                    .map(|b| b.to_vec())
                    .unwrap_or_default(),
            };
            if bytes.is_empty() {
                break;
            }
            received += bytes.len() as u64;
            if received > file.bytes {
                return Err(Error::msg("Download exceeds pinned model size."));
            }
            output.write_all(&bytes)?;
            progress(completed + received, &file.path);
        }
        output.sync_all()?;
        drop(output);
        progress(completed + received, "Verifying model files…");
        if let Err(e) = verify_file(&dest, file, cancel) {
            if !cancel.load(Ordering::Relaxed) && received == file.bytes {
                let _ = fs::remove_file(&dest);
            }
            return Err(e);
        }
        completed += file.bytes;
    }
    cancelled(cancel)?;
    fs::write(staging.join("verified.txt"), manifest.revision)?;
    let dest = location(root);
    if dest.exists() {
        fs::remove_dir_all(&dest)?;
    }
    fs::rename(staging, dest)?;
    Ok(())
}
pub fn remove(root: &Path) -> Result<()> {
    let _guard = lock(root, false)?;
    for dir in [location(root), root.join("models/custom-voice.install")] {
        if dir.exists() {
            fs::remove_dir_all(dir)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn shared_worker_leases_exclude_pack_mutation_until_all_are_released() {
        let root = std::env::temp_dir().join(format!("speech-lock-{}", uuid::Uuid::new_v4()));
        let first = lock(&root, true).unwrap();
        let second = lock(&root, true).unwrap();
        assert!(lock(&root, false).is_err());
        drop(first);
        assert!(lock(&root, false).is_err());
        drop(second);
        let mutation = lock(&root, false).unwrap();
        assert!(lock(&root, true).is_err());
        drop(mutation);
        assert!(lock(&root, true).is_ok());
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn resumes_partial_import_and_does_not_publish_cancelled_setup() {
        let root = std::env::temp_dir().join(format!("speech-install-{}", uuid::Uuid::new_v4()));
        let source = root.join("input");
        fs::create_dir_all(&source).unwrap();
        fs::write(source.join("weights"), b"model").unwrap();
        let staging = root.join("models/custom-voice.install");
        fs::create_dir_all(&staging).unwrap();
        fs::write(staging.join("weights"), b"mo").unwrap();
        let manifest = || Pack {
            model: "fixture".into(),
            revision: "fixture-revision".into(),
            files: vec![PackFile {
                path: "weights".into(),
                bytes: 5,
                sha256: format!("{:x}", Sha256::digest(b"model")),
            }],
        };
        let cancel = AtomicBool::new(false);
        assert!(install_pack(
            &root,
            Some(&source),
            &cancel,
            &|_, _| cancel.store(true, Ordering::Relaxed),
            manifest()
        )
        .is_err());
        assert!(!location(&root).exists());
        assert_eq!(fs::read(staging.join("weights")).unwrap(), b"mo");
        cancel.store(false, Ordering::Relaxed);
        install_pack(&root, Some(&source), &cancel, &|_, _| {}, manifest()).unwrap();
        assert_eq!(fs::read(location(&root).join("weights")).unwrap(), b"model");
        assert!(!staging.exists());
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn cancels_a_stalled_network_future() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            let cancel = std::sync::Arc::new(AtomicBool::new(false));
            let trigger = cancel.clone();
            tokio::spawn(async move {
                tokio::time::sleep(Duration::from_millis(10)).await;
                trigger.store(true, Ordering::Relaxed);
            });
            let result = tokio::time::timeout(
                Duration::from_secs(1),
                cancellable(
                    std::future::pending::<std::result::Result<(), reqwest::Error>>(),
                    &cancel,
                ),
            )
            .await
            .unwrap();
            assert!(result.unwrap_err().to_string().contains("Cancelled"));
        });
    }
    #[test]
    fn verifies_sizes_and_hashes_and_obeys_cancellation() {
        let path = std::env::temp_dir().join(format!("speech-pack-{}", uuid::Uuid::new_v4()));
        fs::write(&path, b"model").unwrap();
        let mut file = PackFile {
            path: "model".into(),
            bytes: 5,
            sha256: format!("{:x}", Sha256::digest(b"model")),
        };
        let cancel = AtomicBool::new(false);
        assert!(verify_file(&path, &file, &cancel).is_ok());
        file.bytes = 4;
        assert!(verify_file(&path, &file, &cancel).is_err());
        file.bytes = 5;
        file.sha256 = "bad".into();
        assert!(verify_file(&path, &file, &cancel).is_err());
        cancel.store(true, Ordering::Relaxed);
        assert!(verify_file(&path, &file, &cancel).is_err());
        fs::remove_file(path).unwrap();
    }
    #[test]
    fn pinned_pack_has_only_safe_paths_and_expected_size() {
        let pack = pack();
        assert_eq!(total_bytes(), 2_498_383_610);
        assert_eq!(pack.files.len(), 11);
        for f in pack.files {
            assert!(!f.path.starts_with('/') && !f.path.contains(".."));
            assert_eq!(f.sha256.len(), 64);
        }
    }
}
