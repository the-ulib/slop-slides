//! Private, revisioned local profiles. No reference recording or transcript enters a deck.
use crate::{audio, Cancellation, Error, Result};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};

pub const PROFILE_ENGINE: &str =
    "qwen-c-ef339be-base-bf16-cpu-no-kleidi-v1-sonic-b93885d-segments350-seed42-svp1";
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Profile {
    pub id: String,
    pub revision: String,
    pub name: String,
    pub reference_language: String,
    pub ready: bool,
}
#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Create {
    pub name: String,
    pub language: String,
    pub transcript: String,
    pub reference: PathBuf,
    pub authorized: bool,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct Record {
    profile: Profile,
    profile_sha256: String,
    reference_sha256: String,
    engine: String,
    model_revision: String,
    previewed_languages: Vec<String>,
    #[serde(default)]
    previous_revisions: Vec<String>,
}
#[derive(Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Registry {
    version: u32,
    profiles: Vec<Record>,
}
pub struct Store {
    root: PathBuf,
}
struct Lock(fs::File);
impl Drop for Lock {
    fn drop(&mut self) {
        let _ = FileExt::unlock(&self.0);
    }
}
fn uuid(id: &str) -> Result<()> {
    uuid::Uuid::parse_str(id)
        .map(|_| ())
        .map_err(|_| Error::msg("Invalid presenter ID."))
}
fn name(value: &str) -> Result<String> {
    let value = value.trim();
    if value.is_empty() || value.chars().count() > 80 || value.chars().any(char::is_control) {
        return Err(Error::msg("Use a presenter name from 1 to 80 characters."));
    }
    Ok(value.into())
}
fn regular(path: &Path, max: u64) -> Result<Vec<u8>> {
    let m = fs::symlink_metadata(path)?;
    if !m.is_file() || m.file_type().is_symlink() || m.len() > max {
        return Err(Error::msg("Invalid or oversized presenter file."));
    }
    Ok(fs::read(path)?)
}
fn directory(path: &Path) -> Result<()> {
    if fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_symlink() || !m.is_dir()) {
        return Err(Error::msg("Presenter storage cannot use symbolic links."));
    }
    fs::create_dir_all(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}
pub fn atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let tmp = path.with_extension(format!("{}.tmp", uuid::Uuid::new_v4()));
    let result = (|| {
        let mut options = fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&tmp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&tmp, path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(tmp);
    }
    result
}
impl Store {
    pub fn new(root: &Path) -> Self {
        Self {
            root: root.join("voices"),
        }
    }
    fn lock(&self) -> Result<Lock> {
        directory(&self.root)?;
        directory(&self.root.join("data"))?;
        let file = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(self.root.join("profiles.lock"))?;
        FileExt::lock_exclusive(&file)?;
        Ok(Lock(file))
    }
    fn registry(&self) -> Result<Registry> {
        let path = self.root.join("registry.json");
        if !path.exists() {
            return Ok(Registry {
                version: 1,
                profiles: vec![],
            });
        }
        let registry: Registry = serde_json::from_slice(&regular(&path, 1_000_000)?)
            .map_err(|e| Error::msg(format!("Cannot read saved presenters: {e}")))?;
        if registry.version != 1 || registry.profiles.len() > 1000 {
            return Err(Error::msg("Unsupported saved-presenter registry."));
        }
        for r in &registry.profiles {
            uuid(
                r.profile
                    .id
                    .strip_prefix("profile:")
                    .ok_or_else(|| Error::msg("Invalid presenter binding."))?,
            )?;
            uuid(&r.profile.revision)?;
            name(&r.profile.name)?;
        }
        Ok(registry)
    }
    fn write_registry(&self, r: &Registry) -> Result<()> {
        atomic(
            &self.root.join("registry.json"),
            &serde_json::to_vec_pretty(r).map_err(|e| Error::msg(e.to_string()))?,
        )
    }
    pub fn data(&self, token: &str) -> Result<PathBuf> {
        uuid(token)?;
        let path = self.root.join("data").join(token);
        for parent in [&self.root, &self.root.join("data"), &path] {
            if fs::symlink_metadata(parent).is_ok_and(|m| m.file_type().is_symlink()) {
                return Err(Error::msg("Presenter storage cannot use symbolic links."));
            }
        }
        Ok(path)
    }
    pub fn list(&self) -> Result<Vec<Profile>> {
        if !self.root.exists() {
            return Ok(vec![]);
        }
        let _lock = self.lock()?;
        Ok(self
            .registry()?
            .profiles
            .into_iter()
            .map(|r| {
                let mut p = r.profile.clone();
                p.ready = self.checked(&r).is_ok();
                p
            })
            .collect())
    }
    fn checked(&self, r: &Record) -> Result<PathBuf> {
        if r.engine != PROFILE_ENGINE
            || r.model_revision != crate::models::Kind::Base.pack().revision
        {
            return Err(Error::msg(
                "This presenter needs to be recreated for the installed model.",
            ));
        }
        let path = self.data(&r.profile.revision)?.join("profile.bin");
        let bytes = regular(&path, 200_000)?;
        validate_binary(&bytes)?;
        if format!("{:x}", Sha256::digest(&bytes)) != r.profile_sha256 {
            return Err(Error::msg(
                "Presenter integrity check failed. Create it again from a recording.",
            ));
        }
        Ok(path)
    }
    pub fn resolve(&self, id: &str, revision: Option<&str>) -> Result<PathBuf> {
        let _lock = self.lock()?;
        let r = self
            .registry()?
            .profiles
            .into_iter()
            .find(|r| r.profile.id == id)
            .ok_or_else(|| {
                Error::msg("Presenter is missing. Choose another voice; saved audio is kept.")
            })?;
        if Some(r.profile.revision.as_str()) != revision {
            return Err(Error::msg(
                "The presenter changed. Refresh and generate again.",
            ));
        }
        self.checked(&r)
    }
    pub fn prepare(&self, request: &Create, cancel: &Cancellation) -> Result<(Profile, PathBuf)> {
        if !request.authorized {
            return Err(Error::msg(
                "Confirm that you have permission to use this voice.",
            ));
        }
        let name = name(&request.name)?;
        if !["en", "de"].contains(&request.language.as_str())
            || request.transcript.trim().is_empty()
            || request.transcript.len() > 4096
            || request.transcript.contains('\0')
        {
            return Err(Error::msg(
                "Choose English/German and enter the exact spoken words (up to 4 KB).",
            ));
        }
        cancel.check()?;
        let bytes = regular(&request.reference, 12_000_000)?;
        let pcm = audio::decode(&bytes).map_err(|_| {
            Error::msg("Choose a PCM16 WAV recording. Microphone recordings can be used directly.")
        })?;
        let seconds = pcm.samples.len() as f64 / pcm.sample_rate as f64;
        if !(3.0..=30.0).contains(&seconds) {
            return Err(Error::msg(
                "Use a clean recording between 3 and 30 seconds; 10–20 seconds is recommended.",
            ));
        }
        let power =
            pcm.samples.iter().map(|n| (*n as f64).powi(2)).sum::<f64>() / pcm.samples.len() as f64;
        if power.sqrt() < 80.0 {
            return Err(Error::msg("The recording is too quiet. Record closer to the microphone or choose another file."));
        }
        let samples = audio::reference_pcm(pcm)?;
        cancel.check()?;
        let _lock = self.lock()?;
        let token = uuid::Uuid::new_v4().to_string();
        let path = self.data(&token)?;
        directory(&path)?;
        let profile = Profile {
            id: format!("profile:{token}"),
            revision: token,
            name,
            reference_language: request.language.clone(),
            ready: false,
        };
        let result = (|| {
            atomic(&path.join("reference.wav"), &audio::encode(&samples, 24000))?;
            atomic(
                &path.join("transcript.txt"),
                request.transcript.trim().as_bytes(),
            )?;
            Ok((profile, path.clone()))
        })();
        if result.is_err() {
            let _ = fs::remove_dir_all(path);
        }
        result
    }
    pub fn finish(&self, mut profile: Profile) -> Result<Profile> {
        let _lock = self.lock()?;
        let path = self.data(&profile.revision)?;
        let bytes = regular(&path.join("profile.bin"), 200_000)?;
        validate_binary(&bytes)?;
        profile.ready = true;
        let r = Record {
            profile: profile.clone(),
            profile_sha256: format!("{:x}", Sha256::digest(bytes)),
            reference_sha256: format!(
                "{:x}",
                Sha256::digest(regular(&path.join("reference.wav"), 2_000_000)?)
            ),
            engine: PROFILE_ENGINE.into(),
            model_revision: crate::models::Kind::Base.pack().revision,
            previewed_languages: vec![],
            previous_revisions: vec![],
        };
        atomic(
            &path.join("draft.json"),
            &serde_json::to_vec(&r).map_err(|e| Error::msg(e.to_string()))?,
        )?;
        Ok(profile)
    }
    fn draft(&self, token: &str) -> Result<Record> {
        let r: Record =
            serde_json::from_slice(&regular(&self.data(token)?.join("draft.json"), 16000)?)
                .map_err(|e| Error::msg(e.to_string()))?;
        if r.profile.revision != token {
            return Err(Error::msg("Invalid presenter draft."));
        }
        self.checked(&r)?;
        Ok(r)
    }
    pub fn draft_profile(&self, token: &str) -> Result<Profile> {
        let _lock = self.lock()?;
        Ok(self.draft(token)?.profile)
    }
    pub fn previewed(&self, token: &str, language: &str) -> Result<()> {
        let _lock = self.lock()?;
        let mut r = self.draft(token)?;
        if !r.previewed_languages.iter().any(|l| l == language) {
            r.previewed_languages.push(language.into());
        }
        atomic(
            &self.data(token)?.join("draft.json"),
            &serde_json::to_vec(&r).map_err(|e| Error::msg(e.to_string()))?,
        )
    }
    pub fn preview_file(&self, token: &str, language: &str) -> Result<PathBuf> {
        if !["en", "de"].contains(&language) {
            return Err(Error::msg("Unsupported preview language."));
        }
        let path = self.data(token)?.join(format!("preview-{language}.wav"));
        audio::decode(&regular(&path, 3_000_000)?)?;
        Ok(path)
    }
    pub fn save(&self, token: &str, replace: Option<&str>) -> Result<Profile> {
        let _lock = self.lock()?;
        let mut r = self.draft(token)?;
        let mut registry = self.registry()?;
        if !r
            .previewed_languages
            .contains(&r.profile.reference_language)
        {
            return Err(Error::msg(
                "Generate and listen to the primary-language preview before saving.",
            ));
        }
        self.preview_file(token, &r.profile.reference_language)?;
        if let Some(id) = replace {
            let old = registry
                .profiles
                .iter_mut()
                .find(|r| r.profile.id == id)
                .ok_or_else(|| Error::msg("The presenter to replace is missing."))?;
            r.previous_revisions = old.previous_revisions.clone();
            r.previous_revisions.push(old.profile.revision.clone());
            r.profile.id = id.into();
            r.profile.name = old.profile.name.clone();
            *old = r.clone();
        } else {
            if registry.profiles.len() >= 1000 {
                return Err(Error::msg("The presenter library is full."));
            }
            if registry
                .profiles
                .iter()
                .any(|p| p.profile.id == r.profile.id)
            {
                return Err(Error::msg("This presenter is already saved."));
            }
            registry.profiles.push(r.clone());
        }
        self.write_registry(&registry)?;
        let _ = fs::remove_file(self.data(token)?.join("draft.json"));
        Ok(r.profile)
    }
    pub fn discard(&self, token: &str) -> Result<()> {
        let _lock = self.lock()?;
        if self
            .registry()?
            .profiles
            .iter()
            .any(|r| r.profile.revision == token)
        {
            return Err(Error::msg("Cannot discard a saved presenter."));
        }
        let path = self.data(token)?;
        if path.exists() {
            fs::remove_dir_all(path)?;
        }
        Ok(())
    }
    pub fn rename(&self, id: &str, value: &str) -> Result<()> {
        let value = name(value)?;
        let _lock = self.lock()?;
        let mut registry = self.registry()?;
        let r = registry
            .profiles
            .iter_mut()
            .find(|r| r.profile.id == id)
            .ok_or_else(|| Error::msg("Presenter is missing."))?;
        r.profile.name = value;
        self.write_registry(&registry)
    }
    pub fn delete(&self, id: &str) -> Result<()> {
        let _lock = self.lock()?;
        let mut registry = self.registry()?;
        let r = registry
            .profiles
            .iter()
            .find(|r| r.profile.id == id)
            .cloned()
            .ok_or_else(|| Error::msg("Presenter is missing."))?;
        // Delete conditioning and references first, so a failure retains a retryable entry.
        for token in r
            .previous_revisions
            .iter()
            .chain(std::iter::once(&r.profile.revision))
        {
            let path = self.data(token)?;
            if path.exists() {
                fs::remove_dir_all(path)?;
            }
        }
        registry.profiles.retain(|r| r.profile.id != id);
        self.write_registry(&registry)?;
        Ok(())
    }
}
pub fn validate_binary(bytes: &[u8]) -> Result<()> {
    let bad = || Error::msg("Invalid or incompatible saved voice conditioning.");
    if bytes.len() < 16 || &bytes[..4] != b"SVP1" {
        return Err(bad());
    }
    let dim = u32::from_le_bytes(bytes[4..8].try_into().unwrap()) as usize;
    let text = u32::from_le_bytes(bytes[8..12].try_into().unwrap()) as usize;
    let frames = u32::from_le_bytes(bytes[12..16].try_into().unwrap()) as usize;
    if !(1..=2048).contains(&dim)
        || !(1..=4096).contains(&text)
        || !(1..=800).contains(&frames)
        || bytes.len() != 16 + dim * 4 + text + frames * 16 * 4
    {
        return Err(bad());
    }
    for value in bytes[16..16 + dim * 4].chunks_exact(4) {
        if !f32::from_le_bytes(value.try_into().unwrap()).is_finite() {
            return Err(bad());
        }
    }
    let transcript = &bytes[16 + dim * 4..16 + dim * 4 + text];
    if transcript.contains(&0) || std::str::from_utf8(transcript).is_err() {
        return Err(bad());
    }
    for code in bytes[16 + dim * 4 + text..].chunks_exact(4) {
        if !(0..2048).contains(&i32::from_le_bytes(code.try_into().unwrap())) {
            return Err(bad());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn root() -> PathBuf {
        let root = std::env::temp_dir().join(format!("profile-store-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        root
    }
    fn request(root: &Path) -> Create {
        let reference = root.join("input.wav");
        let pcm: Vec<i16> = (0..24000 * 4)
            .map(|i| ((i as f64 * 0.08).sin() * 2000.0) as i16)
            .collect();
        fs::write(&reference, audio::encode(&pcm, 24000)).unwrap();
        Create {
            name: "My voice".into(),
            language: "de".into(),
            transcript: "Die genauen Worte meiner Aufnahme.".into(),
            reference,
            authorized: true,
        }
    }
    fn binary() -> Vec<u8> {
        let mut bytes = b"SVP1".to_vec();
        bytes.extend(1u32.to_le_bytes());
        bytes.extend(5u32.to_le_bytes());
        bytes.extend(1u32.to_le_bytes());
        bytes.extend(1.0f32.to_le_bytes());
        bytes.extend(b"Hello");
        for _ in 0..16 {
            bytes.extend(4i32.to_le_bytes());
        }
        bytes
    }
    fn draft(store: &Store, request: &Create) -> Profile {
        let (profile, path) = store.prepare(request, &Cancellation::default()).unwrap();
        fs::write(path.join("profile.bin"), binary()).unwrap();
        store.finish(profile).unwrap()
    }
    fn qualify(store: &Store, profile: &Profile) {
        let path = store
            .data(&profile.revision)
            .unwrap()
            .join("preview-de.wav");
        fs::write(path, audio::encode(&[1, 2, 3], 24000)).unwrap();
        store.previewed(&profile.revision, "de").unwrap();
    }
    #[test]
    fn draft_save_restart_rename_replace_and_delete_keep_identity_but_version_audio() {
        let root = root();
        let request = request(&root);
        let store = Store::new(&root);
        let first = draft(&store, &request);
        assert!(store.list().unwrap().is_empty());
        assert!(store.save(&first.revision, None).is_err());
        qualify(&store, &first);
        let saved = store.save(&first.revision, None).unwrap();
        let reopened = Store::new(&root);
        assert_eq!(reopened.list().unwrap()[0].id, saved.id);
        assert!(reopened.resolve(&saved.id, Some(&saved.revision)).is_ok());
        reopened.rename(&saved.id, "Presenter renamed").unwrap();
        assert_eq!(reopened.list().unwrap()[0].revision, saved.revision);
        assert!(reopened.discard(&saved.revision).is_err());
        let replacement = draft(&store, &request);
        qualify(&store, &replacement);
        let updated = reopened
            .save(&replacement.revision, Some(&saved.id))
            .unwrap();
        assert_eq!(updated.id, saved.id);
        assert_ne!(updated.revision, saved.revision);
        assert_eq!(updated.name, "Presenter renamed");
        assert!(reopened.resolve(&saved.id, Some(&saved.revision)).is_err());
        assert!(reopened
            .resolve(&updated.id, Some(&updated.revision))
            .is_ok());
        let old_path = reopened.data(&saved.revision).unwrap();
        let new_path = reopened.data(&updated.revision).unwrap();
        reopened.delete(&updated.id).unwrap();
        assert!(reopened.list().unwrap().is_empty());
        assert!(!old_path.exists());
        assert!(!new_path.exists());
        assert!(request.reference.exists());
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn rejects_bad_references_consent_corruption_and_untrusted_profile_paths() {
        let root = root();
        let mut request = request(&root);
        let store = Store::new(&root);
        request.authorized = false;
        assert!(store.prepare(&request, &Cancellation::default()).is_err());
        request.authorized = true;
        let cancel = Cancellation::default();
        cancel.cancel();
        assert!(store.prepare(&request, &cancel).is_err());
        request.transcript = "".into();
        assert!(store.prepare(&request, &Cancellation::default()).is_err());
        request.transcript = "Exact words".into();
        let profile = draft(&store, &request);
        qualify(&store, &profile);
        let profile = store.save(&profile.revision, None).unwrap();
        fs::write(
            store.data(&profile.revision).unwrap().join("profile.bin"),
            b"broken",
        )
        .unwrap();
        assert!(!store.list().unwrap()[0].ready);
        assert!(store.resolve(&profile.id, Some(&profile.revision)).is_err());
        assert!(store.data("../outside").is_err());
        assert!(store
            .preview_file(&profile.revision, "../reference")
            .is_err());
        assert!(validate_binary(&binary()[..20]).is_err());
        let mut invalid = binary();
        invalid[16..20].copy_from_slice(&f32::NAN.to_le_bytes());
        assert!(validate_binary(&invalid).is_err());
        fs::write(&request.reference, audio::encode(&[0; 24000 * 4], 24000)).unwrap();
        assert!(store.prepare(&request, &Cancellation::default()).is_err());
        fs::write(&request.reference, audio::encode(&[2000; 24000 * 2], 24000)).unwrap();
        assert!(store.prepare(&request, &Cancellation::default()).is_err());
        fs::remove_dir_all(root).unwrap();
    }
    #[cfg(unix)]
    #[test]
    fn rejects_symlink_reference_and_storage_and_keeps_files_private() {
        use std::os::unix::{fs::symlink, fs::PermissionsExt};
        let root = root();
        let request = request(&root);
        let store = Store::new(&root);
        let (profile, path) = store.prepare(&request, &Cancellation::default()).unwrap();
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o700
        );
        assert_eq!(
            fs::metadata(path.join("reference.wav"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
        store.discard(&profile.revision).unwrap();
        assert!(!path.exists());
        let link = root.join("link.wav");
        symlink(&request.reference, &link).unwrap();
        assert!(store
            .prepare(
                &Create {
                    reference: link,
                    ..request
                },
                &Cancellation::default()
            )
            .is_err());
        let token = uuid::Uuid::new_v4().to_string();
        symlink(&root, root.join("voices/data").join(&token)).unwrap();
        assert!(store.data(&token).is_err());
        fs::remove_dir_all(root).unwrap();
    }
}
