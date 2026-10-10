//! Provider-neutral deck orchestration and immutable WAV takes.
pub mod cache;
use crate::{
    deck,
    error::{Error, Result},
    narration,
};
use serde::Serialize;
use speech_connector::{
    Artifact, Cancellation, Descriptor, FixtureProvider, QwenConnector, SpeechProvider,
};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tauri::{AppHandle, Emitter, Manager};

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Job {
    pub id: String,
    pub kind: String,
    pub deck_id: Option<String>,
    pub source_revision: Option<u64>,
    pub stage: String,
    pub completed: u64,
    pub total: u64,
    pub detail: String,
}
struct Active {
    job: Job,
    cancel: Cancellation,
}
#[derive(Default)]
struct Inner {
    active: Mutex<Option<Active>>,
    providers: Mutex<BTreeMap<String, Arc<dyn SpeechProvider>>>,
}
#[derive(Clone, Default)]
pub struct SpeechManager(Arc<Inner>);
struct Lease {
    manager: SpeechManager,
    id: String,
}
impl Drop for Lease {
    fn drop(&mut self) {
        let mut active = self.manager.0.active.lock().expect("speech job");
        if active.as_ref().is_some_and(|a| a.job.id == self.id) {
            *active = None;
        }
    }
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    providers: Vec<Descriptor>,
    job: Option<Job>,
    presenters: Vec<Presenter>,
    default_presenter: Option<DefaultPresenter>,
}
#[derive(Clone, serde::Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DefaultPresenter {
    pub provider_id: String,
    pub presenter_id: String,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Presenter {
    pub provider_id: String,
    #[serde(flatten)]
    pub profile: speech_connector::profiles::Profile,
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct Event {
    job: Job,
    error: Option<String>,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GenerationResult {
    pub generated: usize,
    pub reused: usize,
    pub superseded: usize,
}
fn root(app: &AppHandle) -> Result<PathBuf> {
    Ok(app
        .path()
        .app_data_dir()
        .map_err(|e| Error::msg(e.to_string()))?
        .join("speech"))
}
fn runtime(app: &AppHandle) -> Option<PathBuf> {
    if !cfg!(target_os = "macos") {
        return None;
    }
    let bundled = app
        .path()
        .resource_dir()
        .ok()?
        .join("speech-runtime/slopslide-speech");
    if bundled.is_file() {
        return Some(bundled);
    }
    if cfg!(debug_assertions) {
        let dev = Path::new(env!("CARGO_MANIFEST_DIR")).join("speech-runtime/slopslide-speech");
        if dev.is_file() {
            return Some(dev);
        }
    }
    None
}
impl SpeechManager {
    fn provider(&self, app: &AppHandle, id: &str) -> Result<Arc<dyn SpeechProvider>> {
        let mut providers = self.0.providers.lock().expect("speech providers");
        if providers.is_empty() {
            providers.insert(
                speech_connector::QWEN_ID.into(),
                Arc::new(QwenConnector::new(root(app)?, runtime(app))),
            );
            if cfg!(debug_assertions)
                && std::env::var("SLOPSLIDE_SPEECH_FIXTURE").as_deref() == Ok("1")
            {
                providers.insert("fixture-tone".into(), Arc::new(FixtureProvider));
            }
        }
        providers.get(id).cloned().ok_or_else(||Error::msg("This deck's speech provider is unavailable. Choose an available provider; previous recordings remain playable."))
    }
    pub fn status(&self, app: &AppHandle) -> Result<Status> {
        self.provider(app, speech_connector::QWEN_ID)?;
        // Release the registry guard before building status. Temporaries in a
        // struct initializer otherwise retain it while the next field locks it.
        let providers = self.provider_snapshot();
        Ok(Status {
            presenters: providers
                .iter()
                .map(|(id, p)| {
                    p.profiles().map(|profiles| {
                        profiles
                            .into_iter()
                            .map(|profile| Presenter {
                                provider_id: id.clone(),
                                profile,
                            })
                            .collect::<Vec<_>>()
                    })
                })
                .collect::<std::result::Result<Vec<_>, _>>()?
                .into_iter()
                .flatten()
                .collect(),
            default_presenter: read_default(&root(app)?)?,
            providers: providers.values().map(|p| p.describe()).collect(),
            job: self
                .0
                .active
                .lock()
                .expect("speech job")
                .as_ref()
                .map(|a| a.job.clone()),
        })
    }
    fn provider_snapshot(&self) -> BTreeMap<String, Arc<dyn SpeechProvider>> {
        self.0.providers.lock().expect("speech providers").clone()
    }
    fn begin(
        &self,
        id: String,
        kind: &str,
        deck_id: Option<String>,
        revision: Option<u64>,
        total: u64,
    ) -> Result<(Lease, Cancellation)> {
        uuid::Uuid::parse_str(&id).map_err(|_| Error::msg("Invalid speech job ID."))?;
        let mut active = self.0.active.lock().expect("speech job");
        if active.is_some() {
            return Err(Error::msg(
                "Another speech job is running. Wait or cancel it first.",
            ));
        }
        let cancel = Cancellation::default();
        *active = Some(Active {
            job: Job {
                id: id.clone(),
                kind: kind.into(),
                deck_id,
                source_revision: revision,
                stage: "starting".into(),
                completed: 0,
                total,
                detail: "Starting speech…".into(),
            },
            cancel: cancel.clone(),
        });
        Ok((
            Lease {
                manager: self.clone(),
                id,
            },
            cancel,
        ))
    }
    #[allow(clippy::too_many_arguments)] // A single event carries stage, counts, detail and failure.
    fn report(
        &self,
        app: &AppHandle,
        id: &str,
        stage: &str,
        completed: u64,
        total: u64,
        detail: &str,
        error: Option<String>,
    ) {
        let mut active = self.0.active.lock().expect("speech job");
        if let Some(a) = active.as_mut().filter(|a| a.job.id == id) {
            a.job.stage = stage.into();
            a.job.completed = completed;
            a.job.total = total;
            a.job.detail = detail.into();
            let _ = app.emit(
                "speech-event",
                Event {
                    job: a.job.clone(),
                    error,
                },
            );
        }
    }
    pub fn cancel(&self, id: &str) -> Result<()> {
        let active = self.0.active.lock().expect("speech job");
        if let Some(a) = active.as_ref().filter(|a| a.job.id == id) {
            a.cancel.cancel();
        }
        Ok(())
    }
    pub async fn install(
        &self,
        app: AppHandle,
        id: String,
        provider_id: String,
        source: Option<String>,
        cloning: bool,
    ) -> Result<()> {
        let provider = self.provider(&app, &provider_id)?;
        let descriptor = provider.describe();
        let (lease, cancel) = self.begin(
            id.clone(),
            "setup",
            None,
            None,
            (if cloning {
                &descriptor.clone_setup
            } else {
                &descriptor.setup
            })
            .as_ref()
            .map_or(0, |s| s.total_bytes),
        )?;
        let manager = self.clone();
        let event_app = app.clone();
        let event_id = id.clone();
        let last = Mutex::new(Instant::now() - Duration::from_secs(1));
        let progress: speech_connector::Progress = Arc::new(move |stage, n, total, detail| {
            let mut previous = last.lock().expect("progress throttle");
            if previous.elapsed() >= Duration::from_millis(150) || n == total {
                *previous = Instant::now();
                manager.report(&event_app, &event_id, stage, n, total, detail, None);
            }
        });
        let source = source.map(PathBuf::from);
        let result = if cloning {
            provider.setup_cloning(source, cancel, progress)
        } else {
            provider.setup(source, cancel, progress)
        }
        .await
        .map_err(Error::from);
        self.report(
            &app,
            &id,
            if result.is_ok() { "complete" } else { "failed" },
            0,
            0,
            if result.is_ok() {
                "Speech provider is ready."
            } else {
                "Setup stopped; retry to resume."
            },
            result.as_ref().err().map(ToString::to_string),
        );
        drop(lease);
        result
    }
    pub async fn remove(&self, app: &AppHandle, provider_id: &str) -> Result<()> {
        let provider = self.provider(app, provider_id)?;
        let (lease, _) = self.begin(uuid::Uuid::new_v4().to_string(), "remove", None, None, 0)?;
        let result = provider.remove().await.map_err(Error::from);
        drop(lease);
        result
    }
    pub async fn generate(
        &self,
        app: AppHandle,
        id: String,
        deck_id: String,
        slide: Option<String>,
        fresh: bool,
    ) -> Result<GenerationResult> {
        let dir = deck::deck_dir(&app, &deck_id)?;
        let doc = narration::load(&dir)?;
        let deck = deck::load(&dir, &deck_id)?;
        let mut sources = Vec::new();
        for s in &deck.slides {
            if slide.as_ref().map_or(!s.hidden, |id| id == &s.id)
                && doc
                    .manifest
                    .slides
                    .get(&s.id)
                    .is_some_and(|n| !n.text.trim().is_empty())
            {
                sources.push((
                    s.id.clone(),
                    cache::Source::from_manifest(&doc.manifest, &s.id)?,
                ));
            }
        }
        if sources.is_empty() {
            return Err(Error::msg("Write a script for a visible slide first."));
        }
        // Validate every slide's resolved provider before starting a batch.
        for (_, source) in &mut sources {
            let provider = self.provider(&app, &source.provider_id)?;
            let descriptor = provider.describe().for_voice(&source.presenter_id);
            source.presenter_revision = descriptor
                .voices
                .iter()
                .find(|v| v.id == source.presenter_id)
                .and_then(|v| v.revision.clone());
            descriptor.validate(&source.request())?;
        }
        let total = sources.len() as u64;
        let (lease, cancel) = self.begin(
            id.clone(),
            "generation",
            Some(deck_id),
            Some(doc.manifest.revision),
            total,
        )?;
        let result = async {
            let jobs = cache::safe_directory(&dir, "jobs")?.join(&id);
            fs::create_dir(&jobs)?;
            let _temp = Temp(jobs.clone());
            let mut result = GenerationResult {
                generated: 0,
                reused: 0,
                superseded: 0,
            };
            for (index, (slide, source)) in sources.iter().enumerate() {
                cancel.check()?;
                let provider = self.provider(&app, &source.provider_id)?;
                let descriptor = provider.describe().for_voice(&source.presenter_id);
                let key = source.key_for(&descriptor.engine_version, &descriptor.model_revision);
                let accepted = doc
                    .manifest
                    .slides
                    .get(slide)
                    .and_then(|s| s.accepted_take_id.as_deref());
                let take = if let Some(take) = cache::reusable(&dir, &key, accepted, fresh)? {
                    result.reused += 1;
                    take
                } else {
                    let manager = self.clone();
                    let event_app = app.clone();
                    let event_id = id.clone();
                    let current = index as u64;
                    let artifact = speech_connector::narration::render(
                        provider.as_ref(),
                        source.request(),
                        &jobs,
                        cancel.clone(),
                        Arc::new(move |stage, _, _, detail| {
                            manager.report(
                                &event_app,
                                &event_id,
                                stage,
                                current,
                                total,
                                &format!("Slide {} of {total} · {detail}", current + 1),
                                None,
                            );
                        }),
                    )
                    .await?;
                    let take = import_artifact(
                        &dir,
                        source.clone(),
                        &descriptor,
                        &jobs,
                        &artifact,
                        &cancel,
                    )?;
                    result.generated += 1;
                    take
                };
                let active = self.0.active.lock().expect("speech job");
                cancel.check()?;
                let latest = provider.describe();
                let unchanged_profile = latest
                    .voices
                    .iter()
                    .find(|v| v.id == source.presenter_id)
                    .and_then(|v| v.revision.as_ref())
                    == source.presenter_revision.as_ref();
                if !unchanged_profile || !cache::accept(&dir, slide, &take)? {
                    result.superseded += 1;
                }
                drop(active);
                self.report(
                    &app,
                    &id,
                    "generating",
                    index as u64 + 1,
                    total,
                    "Recording saved locally.",
                    None,
                );
            }
            Ok(result)
        }
        .await;
        self.report(
            &app,
            &id,
            if result.is_ok() { "complete" } else { "failed" },
            total,
            total,
            if result.is_ok() {
                "Audio ready."
            } else {
                "Generation stopped; previous recordings are kept."
            },
            result.as_ref().err().map(ToString::to_string),
        );
        drop(lease);
        result
    }
    fn profile_progress(&self, app: &AppHandle, id: &str) -> speech_connector::Progress {
        let manager = self.clone();
        let app = app.clone();
        let id = id.to_owned();
        Arc::new(move |stage, n, total, detail| {
            manager.report(&app, &id, stage, n, total, detail, None)
        })
    }
    pub async fn create_profile(
        &self,
        app: AppHandle,
        job_id: String,
        provider_id: String,
        request: speech_connector::profiles::Create,
    ) -> Result<speech_connector::profiles::Profile> {
        let provider = self.provider(&app, &provider_id)?;
        let (lease, cancel) = self.begin(job_id.clone(), "profile", None, None, 0)?;
        let result = provider
            .create_profile(request, cancel, self.profile_progress(&app, &job_id))
            .await
            .map_err(Error::from);
        drop(lease);
        result
    }
    pub async fn preview_profile(
        &self,
        app: AppHandle,
        job_id: String,
        provider_id: String,
        token: String,
        language: String,
    ) -> Result<()> {
        let provider = self.provider(&app, &provider_id)?;
        let (lease, cancel) = self.begin(job_id.clone(), "profile-preview", None, None, 0)?;
        let result = provider
            .preview_profile(
                &token,
                &language,
                cancel,
                self.profile_progress(&app, &job_id),
            )
            .await
            .map_err(Error::from);
        drop(lease);
        result
    }
    pub async fn profile_action(
        &self,
        app: &AppHandle,
        provider_id: &str,
        action: &str,
        id: &str,
        value: Option<&str>,
    ) -> Result<Option<speech_connector::profiles::Profile>> {
        let provider = self.provider(app, provider_id)?;
        let (lease, _) = self.begin(
            uuid::Uuid::new_v4().to_string(),
            "profile-management",
            None,
            None,
            0,
        )?;
        if ["delete", "save", "discard"].contains(&action) {
            provider.unload().await?;
        }
        let result = match action {
            "save" => provider.save_profile(id, value).map(Some),
            "discard" => provider.discard_profile(id).map(|_| None),
            "rename" => provider
                .rename_profile(id, value.unwrap_or(""))
                .map(|_| None),
            "delete" => provider.delete_profile(id).map(|_| None),
            _ => Err(speech_connector::Error::msg(
                "Unsupported presenter action.",
            )),
        }
        .map_err(Error::from);
        if result.is_ok()
            && action == "delete"
            && read_default(&root(app)?)?
                .is_some_and(|d| d.provider_id == provider_id && d.presenter_id == id)
        {
            write_default(&root(app)?, None)?;
        }
        drop(lease);
        result
    }
    pub fn set_default(&self, app: &AppHandle, choice: Option<DefaultPresenter>) -> Result<()> {
        if let Some(choice) = &choice {
            let d = self.provider(app, &choice.provider_id)?.describe();
            if !d.voices.iter().any(|v| v.id == choice.presenter_id) {
                return Err(Error::msg("Choose an available presenter first."));
            }
        }
        write_default(&root(app)?, choice)
    }
}
pub fn read_default(root: &Path) -> Result<Option<DefaultPresenter>> {
    let path = root.join("default-presenter.json");
    if !path.exists() {
        return Ok(None);
    }
    let meta = fs::symlink_metadata(&path)?;
    if meta.file_type().is_symlink() || meta.len() > 4096 {
        return Err(Error::msg("Invalid presenter preference."));
    }
    serde_json::from_slice(&fs::read(path)?).map_err(|e| Error::msg(e.to_string()))
}
fn write_default(root: &Path, choice: Option<DefaultPresenter>) -> Result<()> {
    fs::create_dir_all(root)?;
    speech_connector::profiles::atomic(
        &root.join("default-presenter.json"),
        &serde_json::to_vec(&choice).map_err(|e| Error::msg(e.to_string()))?,
    )?;
    Ok(())
}
pub fn initialize_presenter(app: &AppHandle, manager: &SpeechManager, dir: &Path) -> Result<()> {
    let Some(choice) = read_default(&root(app)?)? else {
        return Ok(());
    };
    let Ok(provider) = manager.provider(app, &choice.provider_id) else {
        return Ok(());
    };
    initialize_choice(dir, &choice, &provider.describe())
}
fn initialize_choice(dir: &Path, choice: &DefaultPresenter, d: &Descriptor) -> Result<()> {
    let Some(v) = d.voices.iter().find(|v| v.id == choice.presenter_id) else {
        return Ok(());
    };
    let mut doc = narration::load(dir)?;
    if doc.version != "missing" {
        return Ok(());
    }
    doc.manifest.speech_provider_id = choice.provider_id.clone();
    doc.manifest.presenter_id = v.id.clone();
    doc.manifest.presenter_name_snapshot = v.name.clone();
    narration::save(dir, doc.manifest, &doc.version)?;
    Ok(())
}
pub fn profile_preview_file(app: &AppHandle, token: &str, language: &str) -> Result<PathBuf> {
    Ok(speech_connector::profiles::Store::new(&root(app)?).preview_file(token, language)?)
}
#[derive(Serialize)]
pub struct Recording {
    pub id: String,
    pub path: String,
}
pub fn stage_recording(app: &AppHandle, bytes: &[u8]) -> Result<Recording> {
    if bytes.len() > 6_000_000 {
        return Err(Error::msg("Recording is too large."));
    }
    let pcm = speech_connector::audio::decode(bytes)?;
    let seconds = pcm.samples.len() as f64 / pcm.sample_rate as f64;
    if !(3.0..=30.0).contains(&seconds) {
        return Err(Error::msg("Record between 3 and 30 seconds."));
    }
    let dir = root(app)?.join("recordings");
    fs::create_dir_all(&dir)?;
    let id = uuid::Uuid::new_v4().to_string();
    let path = dir.join(format!("{id}.wav"));
    speech_connector::profiles::atomic(&path, bytes)?;
    Ok(Recording {
        id,
        path: path.to_string_lossy().into_owned(),
    })
}
pub fn release_recording(app: &AppHandle, id: &str) -> Result<()> {
    uuid::Uuid::parse_str(id).map_err(|_| Error::msg("Invalid recording ID."))?;
    let path = root(app)?.join("recordings").join(format!("{id}.wav"));
    if path.exists() {
        fs::remove_file(path)?;
    }
    Ok(())
}
pub fn reference_recording_file(app: &AppHandle, id: &str) -> Result<PathBuf> {
    uuid::Uuid::parse_str(id).map_err(|_| Error::msg("Invalid recording ID."))?;
    let path = root(app)?.join("recordings").join(format!("{id}.wav"));
    let meta = fs::symlink_metadata(&path)?;
    if !meta.is_file() || meta.file_type().is_symlink() || meta.len() > 6_000_000 {
        return Err(Error::msg("Invalid reference recording."));
    }
    Ok(path)
}
/// Only import bounded, validated artifacts from this job's private spool.
fn import_artifact(
    dir: &Path,
    source: cache::Source,
    provider: &Descriptor,
    spool: &Path,
    artifact: &Artifact,
    cancel: &Cancellation,
) -> Result<cache::Take> {
    cancel.check()?;
    if source.provider_id != provider.id {
        return Err(Error::msg(
            "Speech result has a different provider binding.",
        ));
    }
    let meta = fs::symlink_metadata(&artifact.path)?;
    if !meta.is_file()
        || meta.file_type().is_symlink()
        || meta.len() > 600 * 96000 * 4 + 4096
        || !artifact
            .path
            .canonicalize()?
            .starts_with(spool.canonicalize()?)
    {
        return Err(Error::msg(
            "Speech artifact is outside its job or exceeds the audio limit.",
        ));
    }
    let decoded = speech_connector::audio::decode(&fs::read(&artifact.path)?)?;
    if decoded.sample_rate != artifact.sample_rate || decoded.samples.len() != artifact.samples {
        return Err(Error::msg(
            "Provider audio length/format does not match its result.",
        ));
    }
    let pcm = speech_connector::audio::normalize(decoded)?;
    cancel.check()?;
    cache::publish_for(dir, source, &pcm, provider)
}
struct Temp(PathBuf);
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
pub fn takes(app: &AppHandle, id: &str) -> Result<BTreeMap<String, cache::Take>> {
    let dir = deck::deck_dir(app, id)?;
    let doc = narration::load(&dir)?;
    let mut takes = BTreeMap::new();
    for (slide, script) in doc.manifest.slides {
        if let Some(id) = script.accepted_take_id {
            if let Some(take) = cache::read(&dir, &id)? {
                takes.insert(slide, take);
            }
        }
    }
    Ok(takes)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn status_snapshot_releases_registry_before_provider_discovery() {
        let manager = SpeechManager::default();
        manager
            .0
            .providers
            .lock()
            .unwrap()
            .insert("fixture-tone".into(), Arc::new(FixtureProvider));
        let snapshot = manager.provider_snapshot();
        let guard = manager
            .0
            .providers
            .try_lock()
            .expect("status must release registry lock");
        let provider = snapshot.get("fixture-tone").unwrap();
        assert!(provider.profiles().unwrap().is_empty());
        assert_eq!(provider.describe().id, "fixture-tone");
        drop(guard);
    }
    #[test]
    fn default_presenter_initializes_only_new_decks_and_existing_choices_stay_unchanged() {
        let root = std::env::temp_dir().join(format!("presenter-default-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let choice = DefaultPresenter {
            provider_id: "fixture-tone".into(),
            presenter_id: "tone:440".into(),
        };
        write_default(&root, Some(choice.clone())).unwrap();
        assert_eq!(
            read_default(&root).unwrap().unwrap().presenter_id,
            choice.presenter_id
        );
        let first = root.join("first");
        fs::create_dir(&first).unwrap();
        initialize_choice(&first, &choice, &FixtureProvider.describe()).unwrap();
        let saved = narration::load(&first).unwrap();
        assert_eq!(saved.manifest.presenter_id, "tone:440");
        let second = root.join("second");
        fs::create_dir(&second).unwrap();
        let old = narration::save(&second, narration::Manifest::default(), "missing").unwrap();
        initialize_choice(&second, &choice, &FixtureProvider.describe()).unwrap();
        let kept = narration::load(&second).unwrap();
        assert_eq!(kept.version, old.version);
        assert_eq!(kept.manifest.presenter_id, "preset:ryan");
        write_default(&root, None).unwrap();
        assert!(read_default(&root).unwrap().is_none());
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn alternate_provider_import_is_normalized_cached_and_never_accepts_late_results() {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async {
                let dir = std::env::temp_dir()
                    .join(format!("speech-interchange-{}", uuid::Uuid::new_v4()));
                fs::create_dir_all(&dir).unwrap();
                let mut manifest = narration::Manifest {
                    speech_provider_id: "fixture-tone".into(),
                    presenter_id: "tone:440".into(),
                    pace: 1.0,
                    ..Default::default()
                };
                manifest.slides.insert(
                    "intro".into(),
                    narration::SlideNarration {
                        text: "Hello".into(),
                        ..Default::default()
                    },
                );
                let doc = narration::save(&dir, manifest, "missing").unwrap();
                let source = cache::Source::from_manifest(&doc.manifest, "intro").unwrap();
                let provider: &dyn SpeechProvider = &FixtureProvider;
                let spool = cache::safe_directory(&dir, "jobs").unwrap().join("job");
                let cancel = Cancellation::default();
                let artifact = provider
                    .synthesize(
                        source.request(),
                        &spool,
                        cancel.clone(),
                        Arc::new(|_, _, _, _| {}),
                    )
                    .await
                    .unwrap();
                let take = import_artifact(
                    &dir,
                    source.clone(),
                    &provider.describe(),
                    &spool,
                    &artifact,
                    &cancel,
                )
                .unwrap();
                assert_eq!((take.sample_rate, take.samples), (24000, 24000));
                assert!(cache::accept(&dir, "intro", &take).unwrap());
                assert_eq!(cache::find(&dir, &take.key).unwrap().unwrap().id, take.id);
                assert!(dir.join(format!("audio/{}.wav", take.id)).is_file());
                // Cancellation and mismatched provenance cannot publish a replacement.
                let cancelled = Cancellation::default();
                cancelled.cancel();
                assert!(import_artifact(
                    &dir,
                    source.clone(),
                    &provider.describe(),
                    &spool,
                    &artifact,
                    &cancelled
                )
                .is_err());
                let invalid = Artifact {
                    samples: 1,
                    ..artifact.clone()
                };
                assert!(import_artifact(
                    &dir,
                    source,
                    &provider.describe(),
                    &spool,
                    &invalid,
                    &cancel
                )
                .is_err());
                let mut doc = narration::load(&dir).unwrap();
                doc.manifest.speech_provider_id = "qwen-local".into();
                narration::save(&dir, doc.manifest, &doc.version).unwrap();
                assert!(!cache::accept(&dir, "intro", &take).unwrap());
                assert_eq!(
                    narration::load(&dir).unwrap().manifest.slides["intro"]
                        .accepted_take_id
                        .as_deref(),
                    Some(take.id.as_str())
                );
                assert!(cache::read(&dir, &take.id).unwrap().is_some());
                fs::remove_dir_all(dir).unwrap();
            });
    }
    #[test]
    fn rejects_external_and_malformed_artifacts_before_publication() {
        let dir = std::env::temp_dir().join(format!("speech-artifact-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        let spool = dir.join("spool");
        fs::create_dir(&spool).unwrap();
        let mut m = narration::Manifest::default();
        m.slides.insert(
            "intro".into(),
            narration::SlideNarration {
                text: "Hello".into(),
                ..Default::default()
            },
        );
        let source = cache::Source::from_manifest(&m, "intro").unwrap();
        let provider = QwenConnector::new(PathBuf::new(), None).describe();
        let outside = dir.join("outside.wav");
        fs::write(&outside, cache::encode_wav(&[1, 2, 3])).unwrap();
        let artifact = Artifact {
            path: outside,
            sample_rate: 24000,
            samples: 3,
        };
        assert!(import_artifact(
            &dir,
            source.clone(),
            &provider,
            &spool,
            &artifact,
            &Cancellation::default()
        )
        .is_err());
        let path = spool.join("broken.wav");
        fs::write(&path, b"broken").unwrap();
        let artifact = Artifact {
            path,
            sample_rate: 24000,
            samples: 3,
        };
        assert!(import_artifact(
            &dir,
            source,
            &provider,
            &spool,
            &artifact,
            &Cancellation::default()
        )
        .is_err());
        assert!(!dir.join("audio").exists());
        fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn only_one_job_runs_and_stale_cancellation_cannot_stop_the_next() {
        let manager = SpeechManager::default();
        let id = uuid::Uuid::new_v4().to_string();
        let (lease, cancel) = manager
            .begin(id.clone(), "generation", Some("deck".into()), Some(1), 1)
            .unwrap();
        assert!(manager
            .begin(uuid::Uuid::new_v4().to_string(), "setup", None, None, 1)
            .is_err());
        manager.cancel("stale").unwrap();
        assert!(cancel.check().is_ok());
        manager.cancel(&id).unwrap();
        assert!(cancel.check().is_err());
        assert!(*cancel.receiver().borrow());
        drop(lease);
        let (_next, next_cancel) = manager
            .begin(
                uuid::Uuid::new_v4().to_string(),
                "generation",
                None,
                None,
                1,
            )
            .unwrap();
        manager.cancel(&id).unwrap();
        assert!(next_cancel.check().is_ok());
    }
}
