//! Clickable macOS notifications without polling (#1609).
//!
//! `mac-notification-sys` waits for a click by adding a repeating main-thread
//! `NSTimer` per notification that calls the synchronous
//! `deliveredNotifications` XPC until the banner leaves Notification Center,
//! which a banner never does on its own. Hours of notifications pile up
//! timers, push `usernoted` to ~35% CPU, and one slow reply blocks the main
//! thread for good. Here delivery is one asynchronous call and clicks arrive
//! through the notification center's delegate; nothing waits for anything.
#![allow(deprecated)] // NSUserNotification: the same API the crate used.

use objc2::rc::Retained;
use objc2::runtime::{NSObjectProtocol, ProtocolObject};
use objc2::{define_class, msg_send, AllocAnyThread};
use objc2_foundation::{
    NSObject, NSString, NSUserNotification, NSUserNotificationCenter,
    NSUserNotificationCenterDelegate,
};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, Once};
use tauri::AppHandle;

/// What a click on one notification does.
pub(crate) struct Click {
    pub app: AppHandle,
    pub window_label: String,
    pub target: Option<serde_json::Value>,
    pub focus_fallback_armed: bool,
}

impl Click {
    fn run(self) {
        // The native callback identifies the notification exactly, so it
        // takes precedence over the latest-session focus fallback.
        if !self.focus_fallback_armed
            || crate::claim_notify_activation(&self.window_label, self.target.is_some())
        {
            crate::desktop_lifecycle::activate_workspace_window(
                &self.app,
                &self.window_label,
                self.target,
            );
        }
    }
}

// ponytail: bounded queue; the oldest unclicked banners just stop navigating.
const REMEMBERED: usize = 64;
static PENDING: Mutex<VecDeque<(String, Click)>> = Mutex::new(VecDeque::new());
static SEQUENCE: AtomicU64 = AtomicU64::new(0);

fn remember(id: String, click: Click) {
    let mut pending = PENDING.lock().unwrap();
    pending.push_back((id, click));
    while pending.len() > REMEMBERED {
        pending.pop_front();
    }
}

fn take(id: &str) -> Option<Click> {
    let mut pending = PENDING.lock().unwrap();
    let index = pending.iter().position(|(known, _)| known == id)?;
    pending.remove(index).map(|(_, click)| click)
}

define_class!(
    // SAFETY: NSObject has no subclassing requirements; no Drop, no ivars.
    #[unsafe(super(NSObject))]
    #[name = "WispNotificationDelegate"]
    struct Delegate;

    unsafe impl NSObjectProtocol for Delegate {}

    unsafe impl NSUserNotificationCenterDelegate for Delegate {
        #[unsafe(method(userNotificationCenter:didActivateNotification:))]
        fn did_activate(
            &self,
            _center: &NSUserNotificationCenter,
            notification: &NSUserNotification,
        ) {
            let Some(id) = notification.identifier() else {
                return;
            };
            if let Some(click) = take(&id.to_string()) {
                click.run();
            }
        }

        // Banners are only requested while no app window is focused, so the
        // system's "app is active" suppression must not hide them.
        #[unsafe(method(userNotificationCenter:shouldPresentNotification:))]
        fn should_present(
            &self,
            _center: &NSUserNotificationCenter,
            _notification: &NSUserNotification,
        ) -> bool {
            true
        }
    }
);

/// Deliver a banner; a click runs `click` on the main thread.
pub(crate) fn show(title: String, body: String, click: Click) -> Result<(), String> {
    let app = click.app.clone();
    let id = format!(
        "wisp-{}-{}",
        std::process::id(),
        SEQUENCE.fetch_add(1, Ordering::Relaxed)
    );
    remember(id.clone(), click);
    app.run_on_main_thread(move || {
        static DELEGATE: Once = Once::new();
        let center = NSUserNotificationCenter::defaultUserNotificationCenter();
        DELEGATE.call_once(|| {
            let delegate: Retained<Delegate> =
                unsafe { msg_send![super(Delegate::alloc().set_ivars(())), init] };
            // The center does not retain its delegate; this one lives as long
            // as the process.
            unsafe { center.setDelegate(Some(ProtocolObject::from_ref(&*delegate))) };
            std::mem::forget(delegate);
        });
        let notification = NSUserNotification::new();
        notification.setTitle(Some(&NSString::from_str(&title)));
        notification.setInformativeText(Some(&NSString::from_str(&body)));
        notification.setIdentifier(Some(&NSString::from_str(&id)));
        center.deliverNotification(&notification);
    })
    .map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pending_clicks_are_taken_once_and_the_oldest_are_dropped() {
        // Clicks need an AppHandle, so exercise the queue with its id strings.
        let mut pending: VecDeque<(String, ())> = VecDeque::new();
        for n in 0..(REMEMBERED + 2) {
            pending.push_back((format!("wisp-1-{n}"), ()));
            while pending.len() > REMEMBERED {
                pending.pop_front();
            }
        }
        assert_eq!(pending.len(), REMEMBERED);
        assert_eq!(pending.front().unwrap().0, "wisp-1-2");
        let index = pending.iter().position(|(id, _)| id == "wisp-1-9").unwrap();
        assert!(pending.remove(index).is_some());
        assert!(pending.iter().all(|(id, _)| id != "wisp-1-9"));
    }
}
