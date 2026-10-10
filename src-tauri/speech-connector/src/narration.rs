//! Portable narration controls. Markers are compiled here, never spoken by a model.
use crate::{
    audio, Artifact, Cancellation, Error, Progress, Result, SpeechProvider, SynthesisRequest,
};
use serde::{Deserialize, Serialize};
use std::{fs, path::Path};

pub const FORMAT_VERSION: u32 = 1;
pub const MAX_PAUSE_MS: u32 = 60_000;
pub const MAX_MARKERS: usize = 100;
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Controls {
    pub format_version: u32,
    pub max_pause_ms: u32,
    pub max_markers: usize,
    pub supports_tone: bool,
}
impl Default for Controls {
    fn default() -> Self {
        Self {
            format_version: FORMAT_VERSION,
            max_pause_ms: MAX_PAUSE_MS,
            max_markers: MAX_MARKERS,
            supports_tone: false,
        }
    }
}
#[derive(Clone, Debug, PartialEq)]
pub enum Part {
    Speech(String),
    Pause(u32),
}
#[derive(Debug)]
pub struct Plan {
    pub parts: Vec<Part>,
    pub pause_ms: u32,
    pub marked: bool,
}
impl Plan {
    pub fn has_speech(&self) -> bool {
        self.parts.iter().any(|p| matches!(p, Part::Speech(_)))
    }
}
pub fn parse(text: &str) -> Result<Plan> {
    if text.len() > 100_000 {
        return Err(Error::msg("Narration must be below 100 KB."));
    }
    let mut plan = Plan {
        parts: Vec::new(),
        pause_ms: 0,
        marked: false,
    };
    let mut rest = text;
    if text.char_indices().any(|(i, c)| {
        c == '<'
            && text[i + 1..]
                .trim_start_matches('/')
                .starts_with(char::is_alphabetic)
    }) {
        return Err(Error::msg(
            "SSML is not supported. Use [pause:800ms] for an explicit pause.",
        ));
    }
    let mut count = 0;
    while let Some(open) = rest.char_indices().find_map(|(i, c)| {
        (c == '[' && rest[i + 1..].trim_start().starts_with(char::is_alphabetic)).then_some(i)
    }) {
        if !rest[..open].trim().is_empty() {
            plan.parts
                .push(Part::Speech(rest[..open].trim().to_owned()));
        }
        let close = rest[open..]
            .find(']')
            .map(|i| open + i)
            .ok_or_else(|| Error::msg("Incomplete narration marker. Use [pause:800ms]."))?;
        let marker = &rest[open + 1..close];
        let number = marker.strip_prefix("pause:").and_then(|s| s.strip_suffix("ms"))
            .ok_or_else(|| Error::msg(format!("Unsupported narration marker [{marker}]. Only [pause:800ms] is supported; tone instructions are unavailable for this provider.")))?;
        let ms = if !number.is_empty() && number.bytes().all(|b| b.is_ascii_digit()) {
            number.parse::<u32>().ok()
        } else {
            None
        }
        .filter(|n| *n > 0 && *n <= MAX_PAUSE_MS)
        .ok_or_else(|| {
            Error::msg(
                "Pause duration must be an integer from 1 to 60000 ms, for example [pause:800ms].",
            )
        })?;
        count += 1;
        if count > MAX_MARKERS {
            return Err(Error::msg("Use at most 100 pause markers per slide."));
        }
        plan.pause_ms += ms;
        if plan.pause_ms > 600_000 {
            return Err(Error::msg(
                "Total explicit pauses must not exceed ten minutes.",
            ));
        }
        plan.parts.push(Part::Pause(ms));
        plan.marked = true;
        rest = &rest[close + 1..];
    }
    if !rest.trim().is_empty() {
        plan.parts.push(Part::Speech(rest.trim().to_owned()));
    }
    Ok(plan)
}
/// Exact inserted silence follows pace processing. Provider-created pauses stay
/// in the audio; there is no extra automatic join gap at an explicit marker.
/// Plain scripts take the original synthesis path without changing their PCM.
pub async fn render(
    provider: &dyn SpeechProvider,
    request: SynthesisRequest,
    spool: &Path,
    cancel: Cancellation,
    progress: Progress,
) -> Result<Artifact> {
    provider.describe().validate(&request)?;
    let plan = parse(&request.text)?;
    if !plan.marked {
        return provider.synthesize(request, spool, cancel, progress).await;
    }
    let mut pcm = Vec::new();
    for part in plan.parts {
        cancel.check()?;
        match part {
            Part::Pause(ms) => append(&mut pcm, &vec![0; ms as usize * 24])?,
            Part::Speech(text) => {
                let artifact = provider
                    .synthesize(
                        SynthesisRequest {
                            text,
                            ..request.clone()
                        },
                        spool,
                        cancel.clone(),
                        progress.clone(),
                    )
                    .await?;
                let result = (|| {
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
                    let decoded = audio::decode(&fs::read(&artifact.path)?)?;
                    if decoded.sample_rate != artifact.sample_rate
                        || decoded.samples.len() != artifact.samples
                    {
                        return Err(Error::msg("Provider audio does not match its result."));
                    }
                    let samples = audio::normalize(decoded)?;
                    cancel.check()?;
                    append(&mut pcm, &samples)
                })();
                // Only remove artifacts confirmed to belong to our private spool.
                if artifact
                    .path
                    .canonicalize()
                    .ok()
                    .zip(spool.canonicalize().ok())
                    .is_some_and(|(p, root)| p.starts_with(root))
                {
                    let _ = fs::remove_file(&artifact.path);
                }
                result?;
            }
        }
    }
    cancel.check()?;
    fs::create_dir_all(spool)?;
    let path = spool.join(format!("{}.wav", uuid::Uuid::new_v4()));
    fs::write(&path, audio::encode(&pcm, 24000))?;
    Ok(Artifact {
        path,
        samples: pcm.len(),
        sample_rate: 24000,
    })
}
fn append(pcm: &mut Vec<i16>, samples: &[i16]) -> Result<()> {
    if pcm.len() + samples.len() > 600 * 24000 {
        return Err(Error::msg(
            "Recording including pauses exceeds ten minutes. Shorten the script.",
        ));
    }
    pcm.extend_from_slice(samples);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{FixtureProvider, SpeechProvider};
    use std::sync::{Arc, Mutex};
    #[test]
    fn parser_uses_the_same_cases_as_the_editor() {
        let cases: Vec<serde_json::Value> =
            serde_json::from_str(include_str!("../data/narration-cases.json")).unwrap();
        for case in cases {
            let result = parse(case["text"].as_str().unwrap());
            assert_eq!(
                result.is_err(),
                case["error"].as_bool().unwrap_or(false),
                "{case}"
            );
            if let Ok(plan) = result {
                let pauses: Vec<u32> = plan
                    .parts
                    .iter()
                    .filter_map(|p| {
                        if let Part::Pause(ms) = p {
                            Some(*ms)
                        } else {
                            None
                        }
                    })
                    .collect();
                assert_eq!(serde_json::json!(pauses), case["pauses"]);
                assert_eq!(plan.pause_ms, pauses.iter().sum::<u32>());
            }
        }
        assert!(parse(&format!("Hello {}", "[pause:1ms]".repeat(101))).is_err());
        assert!(parse(&format!("Hello {}", "[pause:60000ms]".repeat(11))).is_err());
        assert_eq!(
            parse("Source [1]. [pause:800ms] Evidence [2].")
                .unwrap()
                .parts[0],
            Part::Speech("Source [1].".into())
        );
    }
    struct Recorder(Mutex<Vec<String>>);
    impl SpeechProvider for Recorder {
        fn describe(&self) -> crate::Descriptor {
            FixtureProvider.describe()
        }
        fn synthesize<'a>(
            &'a self,
            request: SynthesisRequest,
            spool: &'a Path,
            cancel: Cancellation,
            progress: Progress,
        ) -> crate::Task<'a, Artifact> {
            self.0.lock().unwrap().push(request.text.clone());
            FixtureProvider.synthesize(request, spool, cancel, progress)
        }
    }
    #[test]
    fn renderer_never_speaks_markers_and_inserts_exact_silence_after_pace() {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async {
                for pace in [1.0, 2.0] {
                    let spool = std::env::temp_dir()
                        .join(format!("narration-pause-{}", uuid::Uuid::new_v4()));
                    let provider = Recorder(Mutex::new(vec![]));
                    let request = SynthesisRequest {
                        text: "First. [pause:800ms] Second.".into(),
                        language: "en".into(),
                        voice_id: "tone:440".into(),
                        voice_revision: None,
                        pace,
                    };
                    let artifact = render(
                        &provider,
                        request.clone(),
                        &spool,
                        Cancellation::default(),
                        Arc::new(|_, _, _, _| {}),
                    )
                    .await
                    .unwrap();
                    let pcm = audio::decode(&fs::read(&artifact.path).unwrap()).unwrap();
                    let part = (24000.0 / pace) as usize;
                    assert_eq!(pcm.samples.len(), part * 2 + 19200);
                    assert!(pcm.samples[part..part + 19200].iter().all(|s| *s == 0));
                    assert_eq!(*provider.0.lock().unwrap(), vec!["First.", "Second."]);
                    assert_eq!(fs::read_dir(&spool).unwrap().count(), 1);
                    assert!(provider
                        .synthesize(
                            request,
                            &spool,
                            Cancellation::default(),
                            Arc::new(|_, _, _, _| {})
                        )
                        .await
                        .is_err());
                    fs::remove_dir_all(spool).unwrap();
                }
            });
    }
    #[test]
    fn plain_render_preserves_original_audio_and_invalid_input_never_calls_provider() {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async {
                let spool =
                    std::env::temp_dir().join(format!("narration-plain-{}", uuid::Uuid::new_v4()));
                let provider = Recorder(Mutex::new(vec![]));
                let request = SynthesisRequest {
                    text: "A normal sentence.".into(),
                    language: "en".into(),
                    voice_id: "tone:440".into(),
                    voice_revision: None,
                    pace: 1.0,
                };
                let original = provider
                    .synthesize(
                        request.clone(),
                        &spool,
                        Cancellation::default(),
                        Arc::new(|_, _, _, _| {}),
                    )
                    .await
                    .unwrap();
                let rendered = render(
                    &provider,
                    request.clone(),
                    &spool,
                    Cancellation::default(),
                    Arc::new(|_, _, _, _| {}),
                )
                .await
                .unwrap();
                assert_eq!(
                    fs::read(original.path).unwrap(),
                    fs::read(rendered.path).unwrap()
                );
                provider.0.lock().unwrap().clear();
                for text in [
                    "[pause:800ms]",
                    "Hello [tone:confident]",
                    "Hello [pause:800ms",
                ] {
                    assert!(render(
                        &provider,
                        SynthesisRequest {
                            text: text.into(),
                            ..request.clone()
                        },
                        &spool,
                        Cancellation::default(),
                        Arc::new(|_, _, _, _| {})
                    )
                    .await
                    .is_err());
                }
                let cancel = Cancellation::default();
                cancel.cancel();
                assert!(render(
                    &provider,
                    SynthesisRequest {
                        text: "First [pause:800ms] Last".into(),
                        ..request
                    },
                    &spool,
                    cancel,
                    Arc::new(|_, _, _, _| {})
                )
                .await
                .is_err());
                assert!(provider.0.lock().unwrap().is_empty());
                let mut pcm = vec![0; 600 * 24000];
                assert!(append(&mut pcm, &[1]).is_err());
                fs::remove_dir_all(spool).unwrap();
            });
    }
}
