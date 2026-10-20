//! Where the app talks to, where it keeps files, and the per-device settings.
//!
//! Endpoints come from `desktop.json` in the config directory, overridden by `AKATSUKI_CONVEX_URL`,
//! `AKATSUKI_CONVEX_SITE_URL`, `AKATSUKI_SYNC_URL` and `AKATSUKI_API_URL` (D-314). Only these
//! variables are read, by name; nothing here reads a `.env` file or logs a value.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::theme::ThemePref;

/// Production sync worker (BRIEF "Hostnames").
pub const DEFAULT_SYNC_URL: &str = "https://sync.akatsuki.vc";
/// Keychain service of the Better Auth session (§6.3 step 5).
pub const SESSION_SERVICE: &str = "vc.akatsuki.desktop";
/// Keychain account of the session.
pub const SESSION_ACCOUNT: &str = "session";
/// Keychain service of BYO AI keys (§12.2).
pub const AI_KEY_SERVICE: &str = "vc.akatsuki.ai";
/// App version reported to `devices.register`.
pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Server endpoints.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Endpoints {
    /// Convex deployment URL (`*.convex.cloud`, or the local harness).
    pub convex_url: Option<String>,
    /// Convex HTTP actions URL (`*.convex.site`), where Better Auth lives.
    pub convex_site_url: Option<String>,
    /// Sync worker.
    pub sync_url: Option<String>,
    /// apps/api.
    pub api_url: Option<String>,
}

impl Endpoints {
    /// File values, overridden by the environment.
    pub fn resolve(file: &Endpoints, env: impl Fn(&str) -> Option<String>) -> Self {
        let pick = |name: &str, from_file: &Option<String>| {
            env(name)
                .filter(|v| !v.trim().is_empty())
                .or_else(|| from_file.clone())
                .map(|v| v.trim_end_matches('/').to_owned())
        };
        Self {
            convex_url: pick("AKATSUKI_CONVEX_URL", &file.convex_url),
            convex_site_url: pick("AKATSUKI_CONVEX_SITE_URL", &file.convex_site_url),
            sync_url: pick("AKATSUKI_SYNC_URL", &file.sync_url),
            api_url: pick("AKATSUKI_API_URL", &file.api_url),
        }
    }

    /// Both Convex URLs are known, so sign-in can start.
    pub fn is_configured(&self) -> bool {
        self.convex_url.is_some() && self.convex_site_url.is_some()
    }

    /// The sync worker base URL.
    pub fn sync(&self) -> &str {
        self.sync_url.as_deref().unwrap_or(DEFAULT_SYNC_URL)
    }
}

/// Key map choice (`prefs.keymap`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KeymapPref {
    /// Superhuman keys.
    #[default]
    Superhuman,
    /// Gmail-style overrides.
    Gmail,
}

impl KeymapPref {
    /// The generated key map.
    pub fn keymap(self) -> akatsuki_shortcuts::Keymap {
        match self {
            KeymapPref::Superhuman => akatsuki_shortcuts::Keymap::Superhuman,
            KeymapPref::Gmail => akatsuki_shortcuts::Keymap::Gmail,
        }
    }
}

/// Reduced-motion choice.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MotionPref {
    /// Follow the desktop setting.
    #[default]
    System,
    /// Always reduce.
    Reduce,
    /// Always animate.
    Full,
}

/// List density (`prefs.density`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Density {
    /// 32 px rows.
    Compact,
    /// 44 px rows.
    #[default]
    Comfortable,
}

impl Density {
    /// Row height in px from the tokens.
    pub fn row_height(self) -> f32 {
        match self {
            Density::Compact => akatsuki_tokens::layout::ROW_HEIGHT_COMPACT,
            Density::Comfortable => akatsuki_tokens::layout::ROW_HEIGHT_COMFORTABLE,
        }
    }
}

/// Settings kept on this device (`desktop.json`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct LocalSettings {
    /// Theme.
    pub theme: ThemePref,
    /// Key map.
    pub keymap: KeymapPref,
    /// Density.
    pub density: Density,
    /// Motion.
    pub motion: MotionPref,
    /// Undo-send window (`prefs.undoSendSeconds`: 0, 5, 10, 20 or 30).
    pub undo_send_seconds: u32,
    /// Keep message bodies on disk (`prefs.cacheBodiesOnDevice`).
    pub cache_bodies: bool,
    /// Self-hosting: endpoints (the environment overrides them).
    pub endpoints: Endpoints,
}

impl Default for LocalSettings {
    fn default() -> Self {
        Self {
            theme: ThemePref::System,
            keymap: KeymapPref::Superhuman,
            density: Density::Comfortable,
            motion: MotionPref::System,
            undo_send_seconds: 10,
            cache_bodies: true,
            endpoints: Endpoints::default(),
        }
    }
}

impl LocalSettings {
    /// Reads the file; a missing or broken file gives the defaults.
    pub fn load(path: &Path) -> Self {
        std::fs::read(path).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
    }

    /// Writes the file atomically.
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(self).map_err(std::io::Error::other)?)?;
        std::fs::rename(tmp, path)
    }
}

/// File locations.
#[derive(Clone, Debug)]
pub struct Paths {
    /// Mirror, search index, caches.
    pub data_dir: PathBuf,
    /// `desktop.json`.
    pub config_file: PathBuf,
}

impl Paths {
    /// `AKATSUKI_DATA_DIR`, else the platform data and config directories.
    pub fn detect() -> Self {
        let data_dir = std::env::var_os("AKATSUKI_DATA_DIR")
            .map(PathBuf::from)
            .or_else(|| dirs::data_dir().map(|d| d.join("akatsuki")))
            .unwrap_or_else(|| std::env::temp_dir().join("akatsuki"));
        let config_file = std::env::var_os("AKATSUKI_DATA_DIR")
            .map(|d| PathBuf::from(d).join("desktop.json"))
            .or_else(|| dirs::config_dir().map(|d| d.join("akatsuki").join("desktop.json")))
            .unwrap_or_else(|| data_dir.join("desktop.json"));
        Self { data_dir, config_file }
    }

    /// The mirror.
    pub fn mirror(&self) -> PathBuf {
        self.data_dir.join("mirror.sqlite")
    }

    /// The FTS5 index (D-311).
    pub fn search(&self) -> PathBuf {
        self.data_dir.join("search.sqlite")
    }

    /// Sanitized originals opened in the browser.
    pub fn originals(&self) -> PathBuf {
        self.data_dir.join("originals")
    }
}

/// This device's id (UUIDv4, minted on first run, §1.2), kept next to the mirror.
pub fn device_id(data_dir: &Path) -> String {
    let path = data_dir.join("device-id");
    if let Ok(id) = std::fs::read_to_string(&path) {
        let id = id.trim();
        if uuid::Uuid::parse_str(id).is_ok() {
            return id.to_owned();
        }
    }
    let id = uuid::Uuid::new_v4().to_string();
    let _ = std::fs::create_dir_all(data_dir);
    let _ = std::fs::write(&path, &id);
    id
}

/// The desktop's reduced-motion setting: GNOME's `enable-animations`, or `GTK_ENABLE_ANIMATIONS=0`.
pub fn system_reduces_motion() -> bool {
    if std::env::var("GTK_ENABLE_ANIMATIONS").is_ok_and(|v| v == "0") {
        return true;
    }
    if cfg!(target_os = "linux") {
        return std::process::Command::new("gsettings")
            .args(["get", "org.gnome.desktop.interface", "enable-animations"])
            .output()
            .ok()
            .is_some_and(|o| o.status.success() && String::from_utf8_lossy(&o.stdout).trim() == "false");
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn environment_overrides_the_file() {
        let file = Endpoints {
            convex_url: Some("https://file.convex.cloud".into()),
            convex_site_url: None,
            sync_url: Some("https://sync.example/".into()),
            api_url: None,
        };
        let env = |name: &str| match name {
            "AKATSUKI_CONVEX_SITE_URL" => Some("http://127.0.0.1:3411/".to_owned()),
            "AKATSUKI_API_URL" => Some("  ".to_owned()),
            _ => None,
        };
        let e = Endpoints::resolve(&file, env);
        assert_eq!(e.convex_url.as_deref(), Some("https://file.convex.cloud"));
        assert_eq!(e.convex_site_url.as_deref(), Some("http://127.0.0.1:3411"));
        assert_eq!(e.sync(), "https://sync.example");
        assert_eq!(e.api_url, None, "blank variables are ignored");
        assert!(e.is_configured());
        assert!(!Endpoints::default().is_configured());
    }

    #[test]
    fn settings_round_trip_and_default_on_garbage() {
        let dir = std::env::temp_dir().join(format!("akatsuki-desktop-cfg-{}", uuid::Uuid::new_v4()));
        let path = dir.join("desktop.json");
        assert_eq!(LocalSettings::load(&path), LocalSettings::default());
        let s = LocalSettings { theme: ThemePref::Dark, keymap: KeymapPref::Gmail, ..LocalSettings::default() };
        s.save(&path).unwrap();
        assert_eq!(LocalSettings::load(&path), s);
        std::fs::write(&path, b"{not json").unwrap();
        assert_eq!(LocalSettings::load(&path), LocalSettings::default());
        let id = device_id(&dir);
        assert_eq!(device_id(&dir), id, "stable across runs");
        std::fs::remove_dir_all(dir).unwrap();
    }
}
