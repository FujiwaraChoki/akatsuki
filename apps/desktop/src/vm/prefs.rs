//! Settings sync (§4.2 `me.updatePrefs`): the per-device copy in `desktop.json` and the synced
//! `prefs` of the user's profile. Theme, density, key map, the undo-send window and "keep bodies
//! on this device" are synced; motion and endpoints stay on the device.
//!
//! At sign-in the server's prefs win, except for fields changed on this device that have not
//! reached the server yet (`pending`), so a choice made offline or just before a restart survives
//! (the web's D-400 rule). Every change on the device sends only the fields that changed.

use akatsuki_core::convex::client::PrefsPatch;
use akatsuki_core::convex::schema;
use serde::{Deserialize, Serialize};

use crate::config::{Density, KeymapPref, LocalSettings};
use crate::theme::ThemePref;

/// The synced fields as the device holds them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Synced {
    /// Theme.
    pub theme: ThemePref,
    /// Density.
    pub density: Density,
    /// Key map.
    pub keymap: KeymapPref,
    /// Undo send seconds.
    pub undo_send_seconds: u32,
    /// Keep bodies on this device.
    pub cache_bodies: bool,
}

impl Synced {
    /// The synced part of the device settings.
    pub fn of(s: &LocalSettings) -> Self {
        Self {
            theme: s.theme,
            density: s.density,
            keymap: s.keymap,
            undo_send_seconds: s.undo_send_seconds,
            cache_bodies: s.cache_bodies,
        }
    }

    /// The synced part of the server's prefs.
    pub fn from_prefs(p: &schema::Prefs) -> Self {
        Self {
            theme: match p.theme {
                schema::ThemePref::System => ThemePref::System,
                schema::ThemePref::Light => ThemePref::Light,
                schema::ThemePref::Dark => ThemePref::Dark,
            },
            density: match p.density {
                schema::Density::Compact => Density::Compact,
                schema::Density::Comfortable => Density::Comfortable,
            },
            keymap: match p.keymap {
                schema::Keymap::Superhuman => KeymapPref::Superhuman,
                schema::Keymap::Gmail => KeymapPref::Gmail,
            },
            undo_send_seconds: if [0, 5, 10, 20, 30].contains(&p.undo_send_seconds) { p.undo_send_seconds } else { 10 },
            cache_bodies: p.cache_bodies_on_device,
        }
    }

    /// Writes these fields into the device settings.
    pub fn apply_to(self, s: &mut LocalSettings) {
        s.theme = self.theme;
        s.density = self.density;
        s.keymap = self.keymap;
        s.undo_send_seconds = self.undo_send_seconds;
        s.cache_bodies = self.cache_bodies;
    }
}

/// Which synced fields changed on the device and are not on the server yet.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Pending {
    /// Theme.
    pub theme: bool,
    /// Density.
    pub density: bool,
    /// Key map.
    pub keymap: bool,
    /// Undo send.
    pub undo_send_seconds: bool,
    /// Body cache.
    pub cache_bodies: bool,
}

impl Pending {
    /// Nothing waits.
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }

    /// Marks what differs between `before` and `after`.
    pub fn mark(&mut self, before: &Synced, after: &Synced) {
        self.theme |= before.theme != after.theme;
        self.density |= before.density != after.density;
        self.keymap |= before.keymap != after.keymap;
        self.undo_send_seconds |= before.undo_send_seconds != after.undo_send_seconds;
        self.cache_bodies |= before.cache_bodies != after.cache_bodies;
    }
}

/// The `me.updatePrefs` patch for the pending fields of `local`.
pub fn patch(local: &Synced, pending: &Pending) -> PrefsPatch {
    PrefsPatch {
        theme: pending.theme.then_some(match local.theme {
            ThemePref::System => schema::ThemePref::System,
            ThemePref::Light => schema::ThemePref::Light,
            ThemePref::Dark => schema::ThemePref::Dark,
        }),
        density: pending.density.then_some(match local.density {
            Density::Compact => schema::Density::Compact,
            Density::Comfortable => schema::Density::Comfortable,
        }),
        keymap: pending.keymap.then_some(match local.keymap {
            KeymapPref::Superhuman => schema::Keymap::Superhuman,
            KeymapPref::Gmail => schema::Keymap::Gmail,
        }),
        undo_send_seconds: pending.undo_send_seconds.then_some(local.undo_send_seconds),
        cache_bodies_on_device: pending.cache_bodies.then_some(local.cache_bodies),
        ..PrefsPatch::default()
    }
}

/// Merges the server's prefs into the device's: server values win for every field that is not
/// pending on this device.
pub fn merge(local: &Synced, server: &Synced, pending: &Pending) -> Synced {
    Synced {
        theme: if pending.theme { local.theme } else { server.theme },
        density: if pending.density { local.density } else { server.density },
        keymap: if pending.keymap { local.keymap } else { server.keymap },
        undo_send_seconds: if pending.undo_send_seconds { local.undo_send_seconds } else { server.undo_send_seconds },
        cache_bodies: if pending.cache_bodies { local.cache_bodies } else { server.cache_bodies },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn server(theme: schema::ThemePref, undo: u32) -> schema::Prefs {
        schema::Prefs {
            undo_send_seconds: undo,
            remote_images: schema::RemoteImages::Block,
            read_receipts: schema::ReadReceiptPrefs { enabled: false, warning_acknowledged_at: None },
            cache_bodies_on_device: false,
            density: schema::Density::Compact,
            theme,
            keymap: schema::Keymap::Gmail,
            unified_inbox: true,
            notifications: schema::NotificationPref::Off,
        }
    }

    #[test]
    fn server_prefs_win_unless_the_device_changed_them() {
        let local = Synced::of(&LocalSettings { theme: ThemePref::Dark, ..LocalSettings::default() });
        let from_server = Synced::from_prefs(&server(schema::ThemePref::Light, 30));
        let merged = merge(&local, &from_server, &Pending::default());
        assert_eq!(merged, from_server, "a fresh device follows the account");
        assert_eq!(merged.density, Density::Compact);
        assert_eq!(merged.keymap, KeymapPref::Gmail);
        assert!(!merged.cache_bodies);

        let mut pending = Pending::default();
        pending.mark(&Synced::of(&LocalSettings::default()), &local);
        assert!(pending.theme && !pending.density);
        let merged = merge(&local, &from_server, &pending);
        assert_eq!(merged.theme, ThemePref::Dark, "a change not yet synced survives the sign-in");
        assert_eq!(merged.undo_send_seconds, 30);
    }

    #[test]
    fn patches_carry_only_changed_fields() {
        let before = Synced::of(&LocalSettings::default());
        let mut after = before;
        after.undo_send_seconds = 5;
        after.keymap = KeymapPref::Gmail;
        let mut pending = Pending::default();
        pending.mark(&before, &after);
        let p = patch(&after, &pending);
        assert_eq!(serde_json::to_value(&p).unwrap(), serde_json::json!({"undoSendSeconds": 5, "keymap": "gmail"}));
        assert!(patch(&after, &Pending::default()).is_empty());
        let mut s = LocalSettings::default();
        after.apply_to(&mut s);
        assert_eq!(s.undo_send_seconds, 5);
        assert_eq!(Synced::from_prefs(&server(schema::ThemePref::System, 7)).undo_send_seconds, 10, "bad values");
    }
}
