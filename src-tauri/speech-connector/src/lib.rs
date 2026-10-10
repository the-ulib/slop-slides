//! Speech data and lifecycle only: no Tauri, slides, manifests or deck paths.
pub mod audio;
pub mod models;
pub mod narration;
pub mod profiles;
mod qwen;
mod segments;
mod worker;
pub use qwen::QwenConnector;
use serde::{Deserialize, Serialize};
use std::{
    future::Future,
    path::{Path, PathBuf},
    pin::Pin,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};
use tokio::sync::watch;

pub const QWEN_ID: &str = "qwen-local";
pub const QWEN_ENGINE: &str =
    "qwen-c-ef339be-bf16-cpu-no-kleidi-v1-sonic-b93885d-segments350-seed42";
pub const NORMALIZATION: &str = "pcm16-mono-24k-v1";
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error("{0}")]
    Message(String),
}
impl Error {
    pub fn msg(s: impl Into<String>) -> Self {
        Self::Message(s.into())
    }
}
pub type Result<T> = std::result::Result<T, Error>;
pub type Task<'a, T> = Pin<Box<dyn Future<Output = Result<T>> + Send + 'a>>;
pub type Progress = Arc<dyn Fn(&str, u64, u64, &str) + Send + Sync>;
#[derive(Clone)]
pub struct Cancellation {
    flag: Arc<AtomicBool>,
    sender: watch::Sender<bool>,
}
impl Default for Cancellation {
    fn default() -> Self {
        let (sender, _) = watch::channel(false);
        Self {
            flag: Arc::new(AtomicBool::new(false)),
            sender,
        }
    }
}
impl Cancellation {
    pub fn cancel(&self) {
        self.flag.store(true, Ordering::Relaxed);
        self.sender.send_replace(true);
    }
    pub fn check(&self) -> Result<()> {
        if self.flag.load(Ordering::Relaxed) {
            Err(Error::msg(
                "Audio generation cancelled. Previous recordings are kept.",
            ))
        } else {
            Ok(())
        }
    }
    pub fn receiver(&self) -> watch::Receiver<bool> {
        self.sender.subscribe()
    }
    pub fn flag(&self) -> &AtomicBool {
        &self.flag
    }
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Voice {
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revision: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ready: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_revision: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub engine_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reference_language: Option<String>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Pace {
    pub min: f64,
    pub max: f64,
    pub default: f64,
    pub choices: Vec<f64>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Setup {
    pub total_bytes: u64,
    pub detail: String,
    pub import_title: Option<String>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Descriptor {
    pub id: String,
    pub label: String,
    pub contract_version: u32,
    pub processing: String,
    pub engine_version: String,
    pub model_revision: String,
    pub ready: bool,
    pub available: bool,
    pub unavailable_reason: Option<String>,
    pub voices: Vec<Voice>,
    pub languages: Vec<String>,
    pub pace: Pace,
    pub supports_cloning: bool,
    #[serde(default)]
    pub clone_ready: bool,
    #[serde(default)]
    pub clone_setup: Option<Setup>,
    pub setup: Option<Setup>,
    pub voice_hint: Option<String>,
    #[serde(default)]
    pub narration_controls: narration::Controls,
    #[serde(default)]
    pub narration_guidance: String,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SynthesisRequest {
    pub text: String,
    pub language: String,
    pub voice_id: String,
    pub pace: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub voice_revision: Option<String>,
}
impl Descriptor {
    pub fn for_voice(&self, id: &str) -> Self {
        let mut d = self.clone();
        if let Some(v) = self.voices.iter().find(|v| v.id == id) {
            d.ready = v.ready.unwrap_or(self.ready);
            if let Some(revision) = &v.model_revision {
                d.model_revision = revision.clone();
            }
            if let Some(engine) = &v.engine_version {
                d.engine_version = engine.clone();
            }
        }
        d
    }
    pub fn validate(&self, r: &SynthesisRequest) -> Result<()> {
        if self.contract_version != 1 || !self.available {
            return Err(Error::msg(
                self.unavailable_reason
                    .clone()
                    .unwrap_or_else(|| "Speech provider unavailable.".into()),
            ));
        }
        if !self.for_voice(&r.voice_id).ready {
            return Err(Error::msg("Set up the selected speech provider first."));
        }
        if !self.voices.iter().any(|v| v.id == r.voice_id) {
            return Err(Error::msg(
                "Choose an available presenter for this speech provider.",
            ));
        }
        if self
            .voices
            .iter()
            .find(|v| v.id == r.voice_id)
            .and_then(|v| v.revision.as_ref())
            != r.voice_revision.as_ref()
        {
            return Err(Error::msg(
                "The presenter changed. Refresh and generate again.",
            ));
        }
        if !self.languages.contains(&r.language) {
            return Err(Error::msg(
                "This speech provider does not support the selected language.",
            ));
        }
        if !r.pace.is_finite() || r.pace < self.pace.min || r.pace > self.pace.max {
            return Err(Error::msg(
                "Speaking pace is outside this provider's supported range.",
            ));
        }
        let plan = narration::parse(&r.text)?;
        if plan.marked && (self.narration_controls.format_version != narration::FORMAT_VERSION
            || plan.parts.iter().filter(|p| matches!(p, narration::Part::Pause(_))).count() > self.narration_controls.max_markers
            || plan.parts.iter().any(|p| matches!(p, narration::Part::Pause(ms) if *ms > self.narration_controls.max_pause_ms))) {
            return Err(Error::msg("Pause markers exceed this connector's supported controls."));
        }
        if !plan.has_speech() {
            return Err(Error::msg("Write spoken text as well as pause markers."));
        }
        if r.text.trim().is_empty() || r.text.len() > 100_000 {
            return Err(Error::msg("Speech requires nonempty text below 100 KB."));
        }
        Ok(())
    }
    pub fn validate_plain(&self, r: &SynthesisRequest) -> Result<()> {
        self.validate(r)?;
        if narration::parse(&r.text)?.marked {
            return Err(Error::msg("Use narration::render to synthesize pause markers; raw synthesis accepts spoken text only."));
        }
        Ok(())
    }
}
#[derive(Clone, Debug)]
pub struct Artifact {
    pub path: PathBuf,
    pub sample_rate: u32,
    pub samples: usize,
}
/// Adapters own speech; the caller owns spool lifetime and accepted recordings.
/// Polling/job IDs are host concerns for now; an MCP wrapper can map this task to jobs.
pub trait SpeechProvider: Send + Sync {
    fn describe(&self) -> Descriptor;
    fn synthesize<'a>(
        &'a self,
        request: SynthesisRequest,
        spool: &'a Path,
        cancel: Cancellation,
        progress: Progress,
    ) -> Task<'a, Artifact>;
    fn setup(
        &self,
        _source: Option<PathBuf>,
        _cancel: Cancellation,
        _progress: Progress,
    ) -> Task<'_, ()> {
        Box::pin(async { Err(Error::msg("This provider does not support local setup.")) })
    }
    fn remove(&self) -> Task<'_, ()> {
        Box::pin(async {
            Err(Error::msg(
                "This provider has no removable local resources.",
            ))
        })
    }
    fn setup_cloning(
        &self,
        _source: Option<PathBuf>,
        _cancel: Cancellation,
        _progress: Progress,
    ) -> Task<'_, ()> {
        Box::pin(async { Err(Error::msg("This provider does not support cloning setup.")) })
    }
    fn unload(&self) -> Task<'_, ()> {
        Box::pin(async { Ok(()) })
    }
    fn profiles(&self) -> Result<Vec<profiles::Profile>> {
        Ok(vec![])
    }
    fn create_profile(
        &self,
        _request: profiles::Create,
        _cancel: Cancellation,
        _progress: Progress,
    ) -> Task<'_, profiles::Profile> {
        Box::pin(async {
            Err(Error::msg(
                "This provider does not support saved presenters.",
            ))
        })
    }
    fn preview_profile<'a>(
        &'a self,
        _token: &'a str,
        _language: &'a str,
        _cancel: Cancellation,
        _progress: Progress,
    ) -> Task<'a, ()> {
        Box::pin(async {
            Err(Error::msg(
                "This provider does not support presenter previews.",
            ))
        })
    }
    fn save_profile(&self, _token: &str, _replace: Option<&str>) -> Result<profiles::Profile> {
        Err(Error::msg("Saved presenters are unsupported."))
    }
    fn discard_profile(&self, _token: &str) -> Result<()> {
        Err(Error::msg("Saved presenters are unsupported."))
    }
    fn rename_profile(&self, _id: &str, _name: &str) -> Result<()> {
        Err(Error::msg("Saved presenters are unsupported."))
    }
    fn delete_profile(&self, _id: &str) -> Result<()> {
        Err(Error::msg("Saved presenters are unsupported."))
    }
}
/// Development-only interchange fixture. It produces a short tone, never fake speech.
pub struct FixtureProvider;
impl SpeechProvider for FixtureProvider {
    fn describe(&self) -> Descriptor {
        Descriptor {
            id: "fixture-tone".into(),
            label: "Test tone (not speech)".into(),
            contract_version: 1,
            processing: "test".into(),
            engine_version: "fixture-tone-v1".into(),
            model_revision: "1".into(),
            ready: true,
            available: true,
            unavailable_reason: None,
            voices: vec![Voice {
                id: "tone:440".into(),
                name: "440 Hz test tone".into(),
                revision: None, ready: None, model_revision: None, engine_version: None, reference_language: None,
            }],
            languages: vec!["en".into(), "de".into()],
            pace: Pace {
                min: 0.5,
                max: 2.0,
                default: 1.0,
                choices: vec![0.5, 1.0, 1.5, 2.0],
            },
            supports_cloning: false,
            clone_ready: false, clone_setup: None,
            setup: None,
            narration_controls: narration::Controls::default(),
            narration_guidance: "Development fixture: audio is a tone, not real speech. Explicit pauses use the shared renderer; tonal instructions are unsupported.".into(),
            voice_hint: Some(
                "Development fixture: generates a test tone to verify provider interchange.".into(),
            ),
        }
    }
    fn synthesize<'a>(
        &'a self,
        request: SynthesisRequest,
        spool: &'a Path,
        cancel: Cancellation,
        progress: Progress,
    ) -> Task<'a, Artifact> {
        Box::pin(async move {
            self.describe().validate_plain(&request)?;
            cancel.check()?;
            let rate = 16000;
            let samples = (rate as f64 / request.pace).round() as usize;
            let pcm: Vec<i16> = (0..samples)
                .map(|n| {
                    ((n as f64 * 440.0 * std::f64::consts::TAU / rate as f64).sin() * 4000.0) as i16
                })
                .collect();
            std::fs::create_dir_all(spool)?;
            let path = spool.join(format!("{}.wav", uuid::Uuid::new_v4()));
            std::fs::write(&path, audio::encode(&pcm, rate))?;
            cancel.check()?;
            progress("generating", 1, 1, "Test tone generated (not speech).");
            Ok(Artifact {
                path,
                sample_rate: rate,
                samples,
            })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn validates_provider_voice_language_pace_and_cancellation() {
        let d = FixtureProvider.describe();
        let r = SynthesisRequest {
            text: "Hello".into(),
            language: "en".into(),
            voice_id: "tone:440".into(),
            voice_revision: None,
            pace: 1.0,
        };
        assert!(d.validate(&r).is_ok());
        for changed in [
            SynthesisRequest {
                voice_id: "preset:ryan".into(),
                ..r.clone()
            },
            SynthesisRequest {
                language: "fr".into(),
                ..r.clone()
            },
            SynthesisRequest {
                pace: f64::NAN,
                ..r.clone()
            },
            SynthesisRequest {
                pace: 3.0,
                ..r.clone()
            },
        ] {
            assert!(d.validate(&changed).is_err());
        }
        let cancel = Cancellation::default();
        cancel.cancel();
        assert!(cancel.check().is_err());
        assert!(*cancel.receiver().borrow());
    }
    #[test]
    fn fixture_runs_through_contract_without_a_deck_or_model() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async {
            let spool =
                std::env::temp_dir().join(format!("speech-fixture-{}", uuid::Uuid::new_v4()));
            let provider: &dyn SpeechProvider = &FixtureProvider;
            let r = SynthesisRequest {
                text: "Test".into(),
                language: "de".into(),
                voice_id: "tone:440".into(),
                voice_revision: None,
                pace: 1.0,
            };
            let a = provider
                .synthesize(
                    r.clone(),
                    &spool,
                    Cancellation::default(),
                    Arc::new(|_, _, _, _| {}),
                )
                .await
                .unwrap();
            assert_eq!((a.sample_rate, a.samples), (16000, 16000));
            assert_eq!(
                audio::normalize(audio::decode(&std::fs::read(a.path).unwrap()).unwrap())
                    .unwrap()
                    .len(),
                24000
            );
            let cancel = Cancellation::default();
            cancel.cancel();
            assert!(provider
                .synthesize(r, &spool, cancel, Arc::new(|_, _, _, _| {}))
                .await
                .is_err());
            assert_eq!(std::fs::read_dir(&spool).unwrap().count(), 1);
            std::fs::remove_dir_all(spool).unwrap();
        });
    }
}
