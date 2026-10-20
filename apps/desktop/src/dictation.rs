//! Dictation capture (§12.1 `dictation`): the microphone is recorded to a 16 kHz mono WAV file in
//! the data directory (0600) by the system's own recorder, then transcribed on the user's
//! dictation route by `akatsuki_ai::dictate` (managed apps/api → OpenAI, BYO OpenAI, or a local
//! Whisper server) and deleted. Recording starts only after consent and a resolved route, stops at
//! 10 minutes (§8), and the file never leaves the device except as the transcription request.
//!
//! Recorders, first found on `PATH` wins: PipeWire's `pw-record`, PulseAudio's `parecord`, ALSA's
//! `arecord`, then `ffmpeg` (PulseAudio on Linux, AVFoundation on macOS). No audio library is
//! linked into the app (D-445).

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

/// Longest recording (§8: ≤ 10 min).
pub const MAX_RECORDING: Duration = Duration::from_secs(akatsuki_ai::transcribe::AUDIO_MAX_SECONDS as u64);
/// Sample rate: what the transcription models use, and 10 minutes stay under 25 MB.
pub const SAMPLE_RATE: u32 = 16_000;

/// A recorder program.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecorderKind {
    /// PipeWire.
    PwRecord,
    /// PulseAudio.
    Parecord,
    /// ALSA.
    Arecord,
    /// ffmpeg (PulseAudio on Linux, AVFoundation on macOS).
    Ffmpeg,
}

impl RecorderKind {
    /// The program name.
    pub fn program(self) -> &'static str {
        match self {
            Self::PwRecord => "pw-record",
            Self::Parecord => "parecord",
            Self::Arecord => "arecord",
            Self::Ffmpeg => "ffmpeg",
        }
    }

    /// The recorders this platform can use, in order of preference.
    pub fn candidates() -> &'static [RecorderKind] {
        if cfg!(target_os = "linux") {
            &[Self::PwRecord, Self::Parecord, Self::Arecord, Self::Ffmpeg]
        } else if cfg!(target_os = "macos") {
            &[Self::Ffmpeg]
        } else {
            &[]
        }
    }

    /// Arguments that record 16 kHz mono 16-bit WAV to `out`.
    pub fn args(self, out: &Path) -> Vec<String> {
        let out = out.display().to_string();
        let rate = SAMPLE_RATE.to_string();
        match self {
            Self::PwRecord => {
                vec!["--rate".into(), rate, "--channels".into(), "1".into(), "--format".into(), "s16".into(), out]
            }
            Self::Parecord => vec![
                "--rate".into(),
                rate,
                "--channels".into(),
                "1".into(),
                "--format".into(),
                "s16le".into(),
                "--file-format=wav".into(),
                out,
            ],
            Self::Arecord => vec![
                "-q".into(),
                "-f".into(),
                "S16_LE".into(),
                "-r".into(),
                rate,
                "-c".into(),
                "1".into(),
                "-t".into(),
                "wav".into(),
                out,
            ],
            Self::Ffmpeg => {
                let (format, input) =
                    if cfg!(target_os = "macos") { ("avfoundation", ":0") } else { ("pulse", "default") };
                vec![
                    "-hide_banner".into(),
                    "-loglevel".into(),
                    "error".into(),
                    "-nostdin".into(),
                    "-f".into(),
                    format.into(),
                    "-i".into(),
                    input.into(),
                    "-ac".into(),
                    "1".into(),
                    "-ar".into(),
                    rate,
                    "-c:a".into(),
                    "pcm_s16le".into(),
                    "-y".into(),
                    out,
                ]
            }
        }
    }
}

/// The first recorder `exists` finds.
pub fn find_recorder(exists: impl Fn(&str) -> bool) -> Option<RecorderKind> {
    RecorderKind::candidates().iter().copied().find(|k| exists(k.program()))
}

/// Whether `program` is an executable file on `PATH`.
pub fn on_path(program: &str) -> bool {
    let Some(path) = std::env::var_os("PATH") else { return false };
    std::env::split_paths(&path).any(|dir| {
        let p = dir.join(program);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            p.metadata().is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        }
        #[cfg(not(unix))]
        {
            p.is_file()
        }
    })
}

/// Fixes the RIFF and `data` sizes of a WAV file whose recorder was stopped before it could
/// write them (some write 0 or `0xFFFFFFFF` while streaming). `None` when it is not a WAV file.
pub fn fix_wav_sizes(mut wav: Vec<u8>) -> Option<Vec<u8>> {
    if wav.len() < 12 || &wav[0..4] != b"RIFF" || &wav[8..12] != b"WAVE" {
        return None;
    }
    let total = u32::try_from(wav.len() - 8).ok()?;
    wav[4..8].copy_from_slice(&total.to_le_bytes());
    let mut at = 12;
    while at + 8 <= wav.len() {
        let id = [wav[at], wav[at + 1], wav[at + 2], wav[at + 3]];
        let size = u32::from_le_bytes([wav[at + 4], wav[at + 5], wav[at + 6], wav[at + 7]]);
        if &id == b"data" {
            let real = u32::try_from(wav.len() - at - 8).ok()?;
            if size != real {
                wav[at + 4..at + 8].copy_from_slice(&real.to_le_bytes());
            }
            return Some(wav);
        }
        at = at.checked_add(8 + size as usize + (size as usize & 1))?;
    }
    None
}

/// A WAV header for 16-bit mono PCM (tests).
#[cfg(test)]
pub fn wav_header(samples: u32) -> Vec<u8> {
    let data = samples * 2;
    let mut h = Vec::with_capacity(44);
    h.extend_from_slice(b"RIFF");
    h.extend_from_slice(&(36 + data).to_le_bytes());
    h.extend_from_slice(b"WAVEfmt ");
    h.extend_from_slice(&16u32.to_le_bytes());
    h.extend_from_slice(&1u16.to_le_bytes());
    h.extend_from_slice(&1u16.to_le_bytes());
    h.extend_from_slice(&SAMPLE_RATE.to_le_bytes());
    h.extend_from_slice(&(SAMPLE_RATE * 2).to_le_bytes());
    h.extend_from_slice(&2u16.to_le_bytes());
    h.extend_from_slice(&16u16.to_le_bytes());
    h.extend_from_slice(b"data");
    h.extend_from_slice(&data.to_le_bytes());
    h
}

/// A recording in progress.
#[derive(Debug)]
pub struct Recording {
    child: Child,
    path: PathBuf,
}

/// Why dictation could not record.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CaptureError {
    /// No recorder program.
    #[error(
        "Dictation needs a recorder: install PipeWire (pw-record), PulseAudio (parecord), ALSA (arecord) or ffmpeg."
    )]
    NoRecorder,
    /// It failed to start or stop.
    #[error("The microphone could not be recorded.")]
    Failed,
    /// Nothing was recorded.
    #[error("Nothing was recorded.")]
    Empty,
}

impl Recording {
    /// Starts recording into `dir` (created 0700; the file is 0600).
    pub fn start(dir: &Path) -> Result<Self, CaptureError> {
        let kind = find_recorder(on_path).ok_or(CaptureError::NoRecorder)?;
        std::fs::create_dir_all(dir).map_err(|_| CaptureError::Failed)?;
        let path = dir.join(format!("dictation-{}.wav", uuid::Uuid::new_v4().simple()));
        // Create the file first so it is private before any audio lands in it.
        {
            let mut opts = std::fs::OpenOptions::new();
            opts.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                opts.mode(0o600);
            }
            opts.open(&path).map_err(|_| CaptureError::Failed)?;
        }
        let child = Command::new(kind.program())
            .args(kind.args(&path))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|_| {
                let _ = std::fs::remove_file(&path);
                CaptureError::Failed
            })?;
        Ok(Self { child, path })
    }

    /// Stops the recorder (SIGINT, so it finishes the file), reads the WAV and deletes the file.
    pub fn stop(mut self) -> Result<Vec<u8>, CaptureError> {
        // SIGINT through kill(1): the workspace forbids `unsafe`, and the child is ours.
        let interrupted = cfg!(unix)
            && Command::new("kill")
                .args(["-INT", &self.child.id().to_string()])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .is_ok_and(|s| s.success());
        if !interrupted {
            let _ = self.child.kill();
        }
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            match self.child.try_wait() {
                Ok(Some(_)) => break,
                Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(30)),
                _ => {
                    let _ = self.child.kill();
                    let _ = self.child.wait();
                    break;
                }
            }
        }
        let bytes = std::fs::read(&self.path);
        let _ = std::fs::remove_file(&self.path);
        let wav = fix_wav_sizes(bytes.map_err(|_| CaptureError::Failed)?).ok_or(CaptureError::Failed)?;
        if wav.len() <= 44 + (SAMPLE_RATE as usize / 5) {
            return Err(CaptureError::Empty);
        }
        Ok(wav)
    }

    /// Stops without keeping anything.
    pub fn cancel(mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_file(&self.path);
    }
}

impl Drop for Recording {
    fn drop(&mut self) {
        // Never leave a recorder running or a recording behind.
        if matches!(self.child.try_wait(), Ok(None)) {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
        let _ = std::fs::remove_file(&self.path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recorders_are_chosen_in_order() {
        if cfg!(target_os = "linux") {
            assert_eq!(find_recorder(|p| p == "arecord" || p == "ffmpeg"), Some(RecorderKind::Arecord));
            assert_eq!(find_recorder(|_| true), Some(RecorderKind::PwRecord));
        }
        assert_eq!(find_recorder(|_| false), None);
        let args = RecorderKind::PwRecord.args(Path::new("/tmp/x.wav"));
        assert_eq!(args, ["--rate", "16000", "--channels", "1", "--format", "s16", "/tmp/x.wav"]);
        assert!(RecorderKind::Ffmpeg.args(Path::new("o.wav")).windows(2).any(|w| w == ["-ar", "16000"]));
        assert!(!on_path("definitely-not-a-recorder-akatsuki"));
    }

    #[test]
    fn wav_sizes_are_repaired() {
        let mut wav = wav_header(0);
        wav.extend(std::iter::repeat_n(0u8, 3200));
        // A streaming recorder left the sizes unset.
        wav[4..8].copy_from_slice(&u32::MAX.to_le_bytes());
        wav[40..44].copy_from_slice(&0u32.to_le_bytes());
        let fixed = fix_wav_sizes(wav).unwrap();
        assert_eq!(u32::from_le_bytes(fixed[4..8].try_into().unwrap()), 3200 + 36);
        assert_eq!(u32::from_le_bytes(fixed[40..44].try_into().unwrap()), 3200);
        assert!(fix_wav_sizes(b"OggS....".to_vec()).is_none());
        assert!(fix_wav_sizes(b"RIFF\0\0\0\0WAVE".to_vec()).is_none(), "no data chunk");
    }
}
