//! AI beyond summaries: write with AI and rewrite in the composer, action items, AI search,
//! dictation and BYO auto-triage (§12.1, §12.5). Every feature runs only with consent and a
//! resolved route (`run_feature` checks both), sends only what the feature needs (§12.7 rule 2),
//! and BYO keys come from this device's keychain. Device outputs are cached in `ai_cache`.

use std::sync::Arc;
use std::time::Duration;

use futures::StreamExt;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use akatsuki_ai::transcript::{RenderAddr, ThreadForTranscript, TranscriptMessage, render_transcript, strip_quoted};
use akatsuki_ai::{AaspEvent, ROUTING, resolve};
use akatsuki_core::convex::client::{ClaimTriageArgs, SetTriageArgs};
use akatsuki_core::convex::{AiFeature, AiProviderKind};
use akatsuki_core::store::Store;
use akatsuki_core::wire::aasp::{Done, Usage};

use super::{Backend, Event, Remote};
use crate::vm::ai::{RewriteStyle, nl_search_vars, triage_items, triage_results, triages_here};

/// Where dictation stands.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DictationState {
    /// Recording since `started_ms`; `egress` says where the audio goes.
    Recording {
        /// Start (ms).
        started_ms: i64,
        /// Egress line (§12.2).
        egress: String,
    },
    /// The recording is being transcribed.
    Transcribing {
        /// Egress line.
        egress: String,
    },
    /// The words, for the caret.
    Text(String),
    /// A safe error.
    Failed(String),
}

/// How often a device without new work checks for untriaged mail.
const TRIAGE_EVERY: Duration = Duration::from_secs(60);
/// Batches per round, at most (25 conversations each).
const TRIAGE_ROUNDS: usize = 4;

fn sha256_hex(v: &Value) -> String {
    hex::encode(Sha256::digest(serde_json::to_vec(v).unwrap_or_default()))
}

impl Backend {
    fn next_request(&self) -> u64 {
        let mut seq = self.ai_seq.lock().unwrap_or_else(|e| e.into_inner());
        *seq += 1;
        *seq
    }

    /// The thread as the transcript renderer takes it: cached bodies, else snippets; quoted
    /// history stripped (§12.4).
    pub(super) fn transcript_messages(&self, thread_id: &str) -> Result<Vec<TranscriptMessage>, String> {
        let messages =
            self.store.lock().unwrap_or_else(|e| e.into_inner()).messages(thread_id).map_err(|e| e.to_string())?;
        Ok(messages
            .iter()
            .filter(|m| !m.is_draft)
            .map(|m| {
                let body =
                    self.cached_body(m.id.as_str()).and_then(|b| b.text.clone()).unwrap_or_else(|| m.snippet.clone());
                TranscriptMessage {
                    from: m.from.as_ref().map(RenderAddr::from),
                    to: m.to.iter().map(RenderAddr::from).collect(),
                    cc: m.cc.iter().map(RenderAddr::from).collect(),
                    date: m.date,
                    text: strip_quoted(&body).to_owned(),
                }
            })
            .collect())
    }

    /// Adds `transcript` to `vars`, budgeted for the feature (§12.4).
    pub(super) fn with_transcript(
        &self,
        feature: AiFeature,
        thread_id: &str,
        tz: &str,
        vars: &mut serde_json::Map<String, Value>,
    ) -> Result<(), String> {
        let pack = self.prompts.clone().ok_or("The prompt pack is not installed.")?;
        let budget = pack.transcript_budget(feature, vars).map_err(|e| e.to_string())?;
        let transcript = render_transcript(&ThreadForTranscript {
            timezone: tz.to_owned(),
            max_input_chars: Some(budget),
            messages: self.transcript_messages(thread_id)?,
        });
        vars.insert("transcript".into(), json!(transcript));
        Ok(())
    }

    /// The device cache key of a feature run, when the feature's output is cached (§16.2).
    fn cache_key(&self, feature: AiFeature, vars: &serde_json::Map<String, Value>) -> Option<String> {
        if !matches!(feature, AiFeature::Summarize | AiFeature::ExtractActions) {
            return None;
        }
        let settings = self.ai_settings()?;
        let resolved = resolve(feature, &settings, &ROUTING).ok()?;
        let version = self.prompts.as_ref()?.feature(feature)?.meta.version;
        Some(Store::ai_cache_key(
            feature.as_str(),
            &sha256_hex(&Value::Object(vars.clone())),
            version,
            &format!("{}:{}", resolved.profile.profile_id, resolved.model),
        ))
    }

    /// Runs a feature on the user's route and forwards its events as [`Event::Ai`]. Outputs of
    /// cacheable features are served from and written to the device cache.
    pub(super) fn stream_feature(
        self: &Arc<Self>,
        feature: AiFeature,
        vars: serde_json::Map<String, Value>,
    ) -> Result<u64, String> {
        let settings = self.ai_settings().ok_or("Set up an AI provider in Settings first.")?;
        let pack = self.prompts.clone().ok_or("The prompt pack is not installed.")?;
        let request = self.next_request();
        let key = self.cache_key(feature, &vars);
        if let Some(k) = &key
            && let Ok(Some(object)) = self.store.lock().unwrap_or_else(|e| e.into_inner()).ai_cached(k)
        {
            self.emit(Event::Ai {
                request,
                event: AaspEvent::Done(Done {
                    text: None,
                    object: Some(object),
                    usage: Usage { input_tokens: 0, output_tokens: 0 },
                    finish_reason: "stop".into(),
                }),
            });
            return Ok(request);
        }
        let profile_key = resolve(feature, &settings, &ROUTING).ok().and_then(|r| self.key_for(r.profile));
        let stream = akatsuki_ai::run_feature(&self.ai, &pack, &settings, feature, vars, move |_| profile_key);
        let me = self.clone();
        self.rt.spawn(async move {
            let mut stream = stream;
            while let Some(event) = stream.next().await {
                if let (Some(k), AaspEvent::Done(d)) = (&key, &event)
                    && let Some(o) = &d.object
                {
                    let _ = me.store.lock().unwrap_or_else(|e| e.into_inner()).put_ai_cached(k, o);
                }
                me.emit(Event::Ai { request, event });
            }
        });
        Ok(request)
    }

    /// Write with AI (`compose`): the instruction, the draft's own text and, for a reply or
    /// forward, the thread. The answer streams into the body.
    pub fn run_compose(
        self: &Arc<Self>,
        instruction: &str,
        draft: &str,
        thread_id: Option<&str>,
        user_name: &str,
        tz: &str,
    ) -> Result<u64, String> {
        let instruction = instruction.trim();
        if instruction.is_empty() {
            return Err("Say what to write.".into());
        }
        let mut vars = serde_json::Map::new();
        vars.insert("instruction".into(), json!(instruction));
        vars.insert("draft".into(), if draft.trim().is_empty() { Value::Null } else { json!(draft) });
        vars.insert("voice".into(), Value::Null);
        vars.insert("userName".into(), json!(user_name));
        vars.insert("transcript".into(), Value::Null);
        if let Some(t) = thread_id {
            vars.remove("transcript");
            self.with_transcript(AiFeature::Compose, t, tz, &mut vars)?;
        }
        self.stream_feature(AiFeature::Compose, vars)
    }

    /// Rewrite the selection (or the body) in a style.
    pub fn run_rewrite(self: &Arc<Self>, text: &str, style: RewriteStyle) -> Result<u64, String> {
        if text.trim().is_empty() {
            return Err("Write something to rewrite first.".into());
        }
        let mut vars = serde_json::Map::new();
        vars.insert("text".into(), json!(text));
        vars.insert("style".into(), json!(style.as_str()));
        vars.insert("voice".into(), Value::Null);
        self.stream_feature(AiFeature::Rewrite, vars)
    }

    /// Search with AI (`nl_search`): the sentence becomes a grammar string the user edits before
    /// running it (§11.2 "NL search"). Only the request and label and split names are sent.
    pub fn run_nl_search(
        self: &Arc<Self>,
        query: &str,
        tz: &str,
        splits: &[akatsuki_core::convex::client::SplitView],
    ) -> Result<u64, String> {
        if query.trim().is_empty() {
            return Err("Describe what you are looking for.".into());
        }
        let labels = self.store.lock().unwrap_or_else(|e| e.into_inner()).labels(None).unwrap_or_default();
        let vars = nl_search_vars(query.trim(), &today(tz), tz, &labels, splits);
        self.stream_feature(AiFeature::NlSearch, vars)
    }

    // ───────────── dictation ─────────────

    /// The egress line of dictation on the user's route, or why it cannot run.
    pub fn dictation_ready(&self) -> Result<String, String> {
        let settings = self.ai_settings().ok_or("Set up an AI provider in Settings first.")?;
        akatsuki_ai::ensure_consent(&settings).map_err(|e| e.to_string())?;
        let resolved = resolve(AiFeature::Dictation, &settings, &ROUTING).map_err(|e| e.to_string())?;
        match resolved.profile.kind {
            AiProviderKind::Akatsuki => {
                if !self.me_view().is_some_and(|m| m.entitlement.managed_ai) {
                    return Err("Akatsuki AI is part of Pro.".into());
                }
            }
            AiProviderKind::Openai if self.key_for(resolved.profile).is_none() => {
                return Err("Key not set on this device.".into());
            }
            AiProviderKind::Openai | AiProviderKind::OpenaiCompatible => {}
            _ => {
                return Err(
                    "Dictation needs OpenAI, Akatsuki AI or a local Whisper server: set its route in Settings.".into(),
                );
            }
        }
        Ok(resolved.egress_text())
    }

    /// Whether a recording is running.
    pub fn is_recording(&self) -> bool {
        self.recording.lock().unwrap_or_else(|e| e.into_inner()).is_some()
    }

    /// Starts recording, after checking consent and the route (nothing is recorded otherwise).
    pub fn start_dictation(self: &Arc<Self>) -> Result<(), String> {
        let egress = self.dictation_ready()?;
        let mut slot = self.recording.lock().unwrap_or_else(|e| e.into_inner());
        if slot.is_some() {
            return Ok(());
        }
        let rec =
            crate::dictation::Recording::start(&self.paths.data_dir.join("dictation")).map_err(|e| e.to_string())?;
        *slot = Some(rec);
        drop(slot);
        let session = {
            let mut s = self.dictation_seq.lock().unwrap_or_else(|e| e.into_inner());
            *s += 1;
            *s
        };
        self.emit(Event::Dictation(DictationState::Recording { started_ms: akatsuki_core::time::now_ms(), egress }));
        // §8: recordings stop at 10 minutes.
        let me = self.clone();
        self.rt.spawn(async move {
            tokio::time::sleep(crate::dictation::MAX_RECORDING).await;
            if *me.dictation_seq.lock().unwrap_or_else(|e| e.into_inner()) == session {
                me.stop_dictation();
            }
        });
        Ok(())
    }

    /// Stops recording and transcribes; the text arrives as [`DictationState::Text`].
    pub fn stop_dictation(self: &Arc<Self>) {
        let Some(rec) = self.recording.lock().unwrap_or_else(|e| e.into_inner()).take() else { return };
        *self.dictation_seq.lock().unwrap_or_else(|e| e.into_inner()) += 1;
        let egress = self.dictation_ready().unwrap_or_default();
        self.emit(Event::Dictation(DictationState::Transcribing { egress }));
        let me = self.clone();
        self.rt.spawn(async move {
            let wav = match tokio::task::spawn_blocking(move || rec.stop()).await {
                Ok(Ok(w)) => w,
                Ok(Err(e)) => return me.emit(Event::Dictation(DictationState::Failed(e.to_string()))),
                Err(_) => return me.emit(Event::Dictation(DictationState::Failed("The recording failed.".into()))),
            };
            me.transcribe_wav(wav).await;
        });
    }

    /// Transcribes a WAV recording on the dictation route.
    pub(crate) async fn transcribe_wav(self: &Arc<Self>, wav: Vec<u8>) {
        let Some(settings) = self.ai_settings() else {
            return self
                .emit(Event::Dictation(DictationState::Failed("Set up an AI provider in Settings first.".into())));
        };
        let key = resolve(AiFeature::Dictation, &settings, &ROUTING).ok().and_then(|r| self.key_for(r.profile));
        let opts = akatsuki_ai::TranscribeOptions::default();
        let state = match akatsuki_ai::dictate(&self.ai, &settings, &wav, "audio/wav", &opts, move |_| key).await {
            Ok(r) if r.text.trim().is_empty() => DictationState::Failed("No words were heard.".into()),
            Ok(r) => DictationState::Text(r.text.trim().to_owned()),
            Err(e) => DictationState::Failed(e.to_string()),
        };
        self.emit(Event::Dictation(state));
    }

    /// Stops recording and throws the audio away.
    pub fn cancel_dictation(&self) {
        if let Some(rec) = self.recording.lock().unwrap_or_else(|e| e.into_inner()).take() {
            *self.dictation_seq.lock().unwrap_or_else(|e| e.into_inner()) += 1;
            rec.cancel();
        }
    }

    // ───────────── BYO auto-triage (§12.5) ─────────────

    /// The auto-triage switch (`consent.autoTriage`). Turning it on needs AI consent first.
    pub fn set_auto_triage(self: &Arc<Self>, on: bool) {
        let Some(r) = self.remote() else { return };
        let mut settings = self.ai_settings().unwrap_or_default();
        if on && settings.consent.granted_at.is_none() {
            return self.toast("Turn on AI features with a provider first", false);
        }
        settings.consent.auto_triage = on;
        *self.ai_settings.write().unwrap_or_else(|e| e.into_inner()) = Some(settings.clone());
        self.emit(Event::AiSettings(Some(settings.clone())));
        let me = self.clone();
        self.rt.spawn(async move {
            let args = akatsuki_core::convex::client::SaveAiSettingsArgs {
                profiles: None,
                routes: None,
                consent: Some(settings.consent),
            };
            match r.api.save_ai_settings(&args).await {
                Ok(()) => me.toast(if on { "Triage is on" } else { "Triage is off" }, false),
                Err(e) => me.toast(super::safe(&e), false),
            }
        });
    }

    /// Desktop always triages when the triage route is BYO or local: claim up to 25 untriaged
    /// inbox conversations (2-minute lease), send their headers and snippets to the user's
    /// provider from this device, and store the category and "needs reply" with `ai.setTriage`.
    pub(super) async fn triage_loop(self: Arc<Self>, r: Remote) {
        tokio::time::sleep(Duration::from_secs(5)).await;
        loop {
            for _ in 0..TRIAGE_ROUNDS {
                if !self.triage_once(&r).await {
                    break;
                }
            }
            tokio::time::sleep(TRIAGE_EVERY).await;
        }
    }

    /// One batch; `true` when it triaged something (so the caller may try another).
    pub(super) async fn triage_once(self: &Arc<Self>, r: &Remote) -> bool {
        let Some(settings) = self.ai_settings() else { return false };
        if !triages_here(&settings, |id| self.has_ai_key(id)) {
            return false;
        }
        let Some(pack) = self.prompts.clone() else { return false };
        let Ok(claimed) = r.api.claim_triage(&ClaimTriageArgs { device_id: self.device_id.clone(), limit: 25 }).await
        else {
            return false;
        };
        if claimed.is_empty() {
            return false;
        }
        let Ok((text, keys)) = akatsuki_ai::transcript::render_triage_items(&triage_items(&claimed)) else {
            return false;
        };
        let mut vars = serde_json::Map::new();
        vars.insert("items".into(), json!(text));
        let profile_key = resolve(AiFeature::Triage, &settings, &ROUTING).ok().and_then(|p| self.key_for(p.profile));
        let mut stream =
            akatsuki_ai::run_feature(&self.ai, &pack, &settings, AiFeature::Triage, vars, move |_| profile_key);
        let mut model = String::from("unknown");
        let mut version = pack.feature(AiFeature::Triage).map_or(1, |f| f.meta.version);
        let mut object = None;
        while let Some(event) = stream.next().await {
            match event {
                AaspEvent::Meta(m) => {
                    model = m.model;
                    version = m.prompt_version;
                }
                AaspEvent::Done(d) => object = d.object,
                // A failed batch is claimed again once its lease expires.
                AaspEvent::Error(_) => return false,
                _ => {}
            }
        }
        let Some(object) = object else { return false };
        let results = triage_results(&keys, &object, &model, version);
        if results.is_empty() {
            return false;
        }
        r.api.set_triage(&SetTriageArgs { results }).await.is_ok()
    }
}

/// Today in the user's zone, `YYYY-MM-DD`.
pub fn today(tz: &str) -> String {
    let zone = jiff::tz::TimeZone::get(tz).unwrap_or(jiff::tz::TimeZone::UTC);
    jiff::Timestamp::now().to_zoned(zone).strftime("%Y-%m-%d").to_string()
}
