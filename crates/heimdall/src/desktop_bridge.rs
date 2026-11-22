//! Real-time Heimdall desktop integration workers:
//! - Deduplicated Android -> Windows 11 native Toast notification forwarding
//! - Hash-guarded Windows -> Android automatic clipboard synchronization

use std::collections::hash_map::DefaultHasher;
use std::collections::HashSet;
use std::hash::{Hash, Hasher};

use bifrost_bridge::{
    dispatch_windows_toast, list_active_notifications, read_host_clipboard_text,
    sync_clipboard_to_android, AdbClient, AndroidNotification,
};
use tracing::{debug, warn};

const MAX_SEEN_NOTIFICATIONS: usize = 512;

/// Stateful deduplicator and hash tracker for Heimdall's background desktop bridge.
#[derive(Debug, Default)]
pub struct DesktopBridgeWorker {
    notifications_seeded: bool,
    seen_notifications: HashSet<String>,
    last_clipboard_hash: Option<u64>,
}

impl DesktopBridgeWorker {
    /// Polls active Android notifications and dispatches newly arrived items to Windows 11.
    ///
    /// On the very first successful poll after startup, existing Android notifications are
    /// seeded into the deduplication set without spamming stale toasts.
    pub fn sync_notifications(&mut self, adb: &AdbClient, serial: &str) {
        let active = match list_active_notifications(adb, serial) {
            Ok(list) => list,
            Err(err) => {
                debug!(error = %err, "skipping notification poll");
                return;
            }
        };

        let fresh = self.collect_new_notifications(&active);
        for notif in fresh {
            if let Err(err) = dispatch_windows_toast(&notif) {
                warn!(
                    package = %notif.package_name,
                    error = %err,
                    "failed forwarding Android notification to Windows 11"
                );
            }
        }
    }

    /// Pure deduplication filter returning only newly observed notifications with user-visible content.
    pub fn collect_new_notifications(
        &mut self,
        active: &[AndroidNotification],
    ) -> Vec<AndroidNotification> {
        if !self.notifications_seeded {
            for notif in active {
                self.seen_notifications
                    .insert(notification_signature(notif));
            }
            self.notifications_seeded = true;
            return Vec::new();
        }

        if self.seen_notifications.len() > MAX_SEEN_NOTIFICATIONS {
            self.seen_notifications.clear();
        }

        let mut fresh = Vec::new();
        for notif in active {
            // Skip silent system notifications that carry neither title nor text.
            if notif.title.is_none() && notif.text.is_none() {
                continue;
            }
            let sig = notification_signature(notif);
            if self.seen_notifications.insert(sig) {
                fresh.push(notif.clone());
            }
        }
        fresh
    }

    /// Reads the Windows host Unicode clipboard and syncs to Android only when its content changes.
    pub fn sync_clipboard(&mut self, adb: &AdbClient, serial: &str) {
        let text = match read_host_clipboard_text() {
            Ok(Some(t)) if !t.is_empty() => t,
            _ => return,
        };

        if !self.should_sync_clipboard_text(&text) {
            return;
        }

        if let Err(err) = sync_clipboard_to_android(adb, serial) {
            debug!(error = %err, "automatic clipboard sync to Android failed");
        }
    }

    /// Returns `true` and records the new hash only when `text` differs from the previously synced text.
    pub fn should_sync_clipboard_text(&mut self, text: &str) -> bool {
        let hash = hash_text(text);
        if self.last_clipboard_hash == Some(hash) {
            return false;
        }
        self.last_clipboard_hash = Some(hash);
        true
    }
}

fn notification_signature(notif: &AndroidNotification) -> String {
    format!(
        "{}|{}|{}|{}",
        notif.key,
        notif.package_name,
        notif.title.as_deref().unwrap_or(""),
        notif.text.as_deref().unwrap_or("")
    )
}

fn hash_text(text: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    text.hash(&mut hasher);
    hasher.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seeds_initial_notifications_and_deduplicates_subsequent_polls() {
        let mut worker = DesktopBridgeWorker::default();
        let initial = vec![AndroidNotification {
            key: "k1".to_string(),
            package_name: "org.fdroid.fdroid".to_string(),
            title: Some("Updates ready".to_string()),
            text: Some("2 updates".to_string()),
        }];

        // First poll seeds without dispatching stale notifications.
        assert!(worker.collect_new_notifications(&initial).is_empty());
        // Second poll with same notification dispatches nothing.
        assert!(worker.collect_new_notifications(&initial).is_empty());

        // New notification arrives -> dispatched once.
        let mut next = initial;
        next.push(AndroidNotification {
            key: "k2".to_string(),
            package_name: "org.signal.app".to_string(),
            title: Some("Alice".to_string()),
            text: Some("Hello from Android".to_string()),
        });
        let fresh = worker.collect_new_notifications(&next);
        assert_eq!(fresh.len(), 1);
        assert_eq!(fresh[0].key, "k2");
        assert!(worker.collect_new_notifications(&next).is_empty());
    }

    #[test]
    fn clipboard_hash_prevents_redundant_syncs() {
        let mut worker = DesktopBridgeWorker::default();
        assert!(worker.should_sync_clipboard_text("hello"));
        assert!(!worker.should_sync_clipboard_text("hello"));
        assert!(worker.should_sync_clipboard_text("world"));
    }
}
