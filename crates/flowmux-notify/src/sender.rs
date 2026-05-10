// SPDX-License-Identifier: GPL-3.0-or-later
//! Send desktop notifications via `org.freedesktop.Notifications`.
//!
//! On Linux this is the standard FDO spec implemented by GNOME Shell,
//! KDE plasma, dunst, mako, etc. — the equivalent of macOS
//! UserNotifications used by cmux.

use flowmux_core::{Notification, NotificationLevel};
use std::collections::HashMap;
use zbus::{proxy, zvariant::Value, Connection};

/// Object path the Unity LauncherEntry signal is broadcast on. Dock
/// implementations (Ubuntu Dock, Dash-to-Dock, KDE Plasma, plank) match
/// on the interface name + `com.canonical.Unity.LauncherEntry::Update`
/// member, so the path itself just needs to be unique-ish and stable
/// across emissions for the same app.
const LAUNCHER_ENTRY_PATH: &str = "/com/canonical/unity/launcherentry/flowmux";
const LAUNCHER_ENTRY_INTERFACE: &str = "com.canonical.Unity.LauncherEntry";
const LAUNCHER_ENTRY_MEMBER: &str = "Update";

/// Basename of the installed desktop file (`com.flowmux.App.desktop`)
/// without the `.desktop` extension. Used as:
///
/// 1. The `desktop-entry` hint on every FDO `Notify` call so GNOME
///    Shell, KDE Plasma and `notification-daemon` can group flowmux
///    toasts under the same launcher icon and — crucially — clear
///    the dock indicator the moment we issue `CloseNotification`.
///    Mismatching this string against the real desktop file name
///    makes GNOME Shell associate the toast with a non-existent app
///    id, and the message-tray dot then survives even after every
///    pending toast has been withdrawn (the exact "badge stays after
///    ack" symptom this constant guards against).
/// 2. The icon name (FDO `app_icon` argument) so the toast image
///    matches the dock icon. Falls back to the system theme when the
///    flowmux icon is not installed.
///
/// Keep this in lockstep with `crates/flowmux/src/main.rs::APP_ID`
/// and `resources/desktop/com.flowmux.App.desktop`.
pub const DESKTOP_FILE_BASENAME: &str = "com.flowmux.App";

/// FDO Notifications proxy. Spec: <https://specifications.freedesktop.org/notification-spec/>.
#[proxy(
    interface = "org.freedesktop.Notifications",
    default_service = "org.freedesktop.Notifications",
    default_path = "/org/freedesktop/Notifications"
)]
trait FdoNotifications {
    #[allow(clippy::too_many_arguments)]
    fn notify(
        &self,
        app_name: &str,
        replaces_id: u32,
        app_icon: &str,
        summary: &str,
        body: &str,
        actions: Vec<&str>,
        hints: std::collections::HashMap<&str, Value<'_>>,
        expire_timeout: i32,
    ) -> zbus::Result<u32>;

    fn close_notification(&self, id: u32) -> zbus::Result<()>;
}

#[derive(Clone)]
pub struct DesktopNotifier {
    conn: Connection,
}

impl DesktopNotifier {
    pub async fn connect() -> zbus::Result<Self> {
        Ok(Self {
            conn: Connection::session().await?,
        })
    }

    pub async fn send(&self, n: &Notification) -> zbus::Result<u32> {
        let proxy = FdoNotificationsProxy::new(&self.conn).await?;
        let mut hints = std::collections::HashMap::new();
        hints.insert("urgency", Value::U8(urgency_for(n.level)));
        // MUST match the installed `.desktop` basename so GNOME Shell /
        // KDE Plasma group the toast under flowmux's launcher icon —
        // otherwise the dock dot survives `CloseNotification` and the
        // user is left with a stuck badge after acknowledging.
        hints.insert("desktop-entry", Value::Str(DESKTOP_FILE_BASENAME.into()));
        proxy
            .notify(
                "flowmux",
                0,
                DESKTOP_FILE_BASENAME,
                &n.title,
                &n.body,
                vec![],
                hints,
                expire_for(n.level),
            )
            .await
    }

    /// Tell the FDO notification daemon to close (and silently
    /// withdraw) the notification with `desktop_id`. We use this to
    /// drop dock/launcher counters once the user has acknowledged the
    /// alert in flowmux's own bell popover — without it, AttentionNeeded
    /// toasts (which we send with `expire_timeout = 0`) would otherwise
    /// linger forever in the GNOME / KDE notification center.
    pub async fn close(&self, desktop_id: u32) -> zbus::Result<()> {
        let proxy = FdoNotificationsProxy::new(&self.conn).await?;
        proxy.close_notification(desktop_id).await
    }

    /// Publish the unread-notification count to the dock badge via the
    /// `com.canonical.Unity.LauncherEntry::Update` D-Bus signal. Ubuntu
    /// Dock, Dash-to-Dock, KDE Plasma and plank all listen for this
    /// signal. `count <= 0` hides the badge by sending
    /// `count-visible = false`.
    ///
    /// `app_uri` should be `application://<desktop-file-name>.desktop`
    /// (e.g. `application://com.flowmux.App.desktop`) so the dock can
    /// associate the badge with our launcher icon.
    pub async fn update_launcher_count(&self, app_uri: &str, count: i64) -> zbus::Result<()> {
        let visible = count > 0;
        let mut props: HashMap<&str, Value<'_>> = HashMap::new();
        props.insert("count", Value::I64(count.max(0)));
        props.insert("count-visible", Value::Bool(visible));
        // `urgent = true` makes some docks bounce / glow the icon. We
        // mirror visibility so the icon goes back to neutral once the
        // count hits zero.
        props.insert("urgent", Value::Bool(visible));
        self.conn
            .emit_signal(
                None::<&str>,
                LAUNCHER_ENTRY_PATH,
                LAUNCHER_ENTRY_INTERFACE,
                LAUNCHER_ENTRY_MEMBER,
                &(app_uri, props),
            )
            .await
    }
}

fn urgency_for(level: NotificationLevel) -> u8 {
    // FDO urgency levels: 0 = low, 1 = normal, 2 = critical.
    match level {
        NotificationLevel::Info => 0,
        NotificationLevel::AttentionNeeded => 1,
        NotificationLevel::Error => 2,
    }
}

fn expire_for(level: NotificationLevel) -> i32 {
    // -1 lets the desktop apply its default; critical sticks until dismissed.
    match level {
        NotificationLevel::Error | NotificationLevel::AttentionNeeded => 0,
        NotificationLevel::Info => -1,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The `desktop-entry` hint, the FDO `app_icon` argument, and the
    /// LauncherEntry `app_uri` all need to key off the same `.desktop`
    /// basename so GNOME Shell / KDE Plasma / Ubuntu Dock / Dash-to-Dock
    /// route the notification, the icon, and the badge to the same app.
    /// A drift between any of these is exactly the "badge stays after
    /// the user acknowledges" symptom that recurred three times before
    /// — pin the literal here so a future rename surfaces in CI before
    /// it ships.
    #[test]
    fn desktop_file_basename_matches_installed_desktop_file() {
        assert_eq!(
            DESKTOP_FILE_BASENAME, "com.flowmux.App",
            "DESKTOP_FILE_BASENAME must match resources/desktop/<basename>.desktop \
             and the GApplication application_id; otherwise the dock badge survives ack",
        );
    }

    #[test]
    fn urgency_for_levels_maps_to_fdo_codes() {
        // FDO spec: 0=low, 1=normal, 2=critical. Pin the mapping so a
        // future refactor that flips Info ↔ AttentionNeeded does not
        // silently downgrade agent toasts.
        assert_eq!(urgency_for(NotificationLevel::Info), 0);
        assert_eq!(urgency_for(NotificationLevel::AttentionNeeded), 1);
        assert_eq!(urgency_for(NotificationLevel::Error), 2);
    }

    /// AttentionNeeded and Error both intentionally pick `expire_timeout =
    /// 0` so the toast lingers in the message tray until flowmux itself
    /// withdraws it via `CloseNotification`. If a refactor changed this
    /// to a positive timeout, GNOME would auto-expire the toast after a
    /// few seconds and the dock badge would *also* clear on its own —
    /// which sounds nice but masks the bell-popover sweep / workspace
    /// activation sweep, hiding real regressions.
    #[test]
    fn expire_for_attention_and_error_returns_zero_so_caller_must_close_explicitly() {
        assert_eq!(expire_for(NotificationLevel::AttentionNeeded), 0);
        assert_eq!(expire_for(NotificationLevel::Error), 0);
        assert_eq!(expire_for(NotificationLevel::Info), -1);
    }
}
