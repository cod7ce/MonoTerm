use gpui::{Context, Task};
use std::time::Duration;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const RELEASES_URL: &str = "https://github.com/cod7ce/MonoTerm/releases";

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UpdateState {
    Ready,
    Checking,
    Available,
    Current,
    Error,
    Disabled,
    Unavailable,
    InstallPending,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Deserialize)]
pub struct UpdateStatus {
    pub state: UpdateState,
    pub message: String,
    pub version: Option<String>,
    pub can_check: bool,
}

impl Default for UpdateStatus {
    fn default() -> Self {
        Self {
            state: UpdateState::Unavailable,
            message: "The updater is unavailable.".into(),
            version: None,
            can_check: false,
        }
    }
}

impl UpdateStatus {
    fn should_poll(&self) -> bool {
        !matches!(self.state, UpdateState::Disabled | UpdateState::Unavailable)
    }
}

pub struct Updater {
    pub status: UpdateStatus,
    _poll_task: Option<Task<()>>,
}

impl Updater {
    /// Register the termination handler before constructing this entity. Retain
    /// the entity in the main app so installation can finish with Settings closed.
    pub fn new(enabled: bool, cx: &mut Context<Self>) -> Self {
        initialize_updater(enabled);
        let status = updater_status();
        let poll_task = status.should_poll().then(|| {
            cx.spawn(async move |this, cx| {
                loop {
                    cx.background_executor()
                        .timer(Duration::from_millis(500))
                        .await;
                    match this.update(cx, |this, cx| {
                        this.refresh(cx);
                        this.status.should_poll()
                    }) {
                        Ok(true) => {}
                        Ok(false) | Err(_) => break,
                    }
                }
            })
        });
        Self {
            status,
            _poll_task: poll_task,
        }
    }

    pub fn check(&mut self, cx: &mut Context<Self>) {
        if self.status.can_check {
            check_for_updates();
            self.refresh(cx);
        }
    }

    fn refresh(&mut self, cx: &mut Context<Self>) {
        let mut status = updater_status();
        if status.state == UpdateState::InstallPending {
            // Sparkle has already received the user's install approval. Keep PTYs
            // alive until the actual AppKit termination callback performs cleanup.
            resume_update_install(true);
            status = updater_status();
        }
        if status != self.status {
            self.status = status;
            cx.notify();
        }
    }
}

#[cfg(target_os = "macos")]
unsafe extern "C" {
    fn mt_updater_initialize(enabled: bool);
    fn mt_updater_check();
    fn mt_updater_status() -> *mut std::ffi::c_char;
    fn mt_updater_resume_install(allowed: bool);
    fn mt_updater_set_termination_handler(handler: extern "C" fn() -> bool);
}

pub fn initialize_updater(enabled: bool) {
    // Test instances must not load Sparkle or write its shared preferences, even
    // when their caller inadvertently requests an enabled updater.
    let enabled = enabled && std::env::var_os("MONOTERM_CONFIG_DIR").is_none();
    #[cfg(target_os = "macos")]
    unsafe {
        mt_updater_initialize(enabled);
    }
    #[cfg(not(target_os = "macos"))]
    let _ = enabled;
}

pub fn check_for_updates() {
    #[cfg(target_os = "macos")]
    unsafe {
        mt_updater_check();
    }
}

pub fn resume_update_install(allowed: bool) {
    #[cfg(target_os = "macos")]
    unsafe {
        mt_updater_resume_install(allowed);
    }
    #[cfg(not(target_os = "macos"))]
    let _ = allowed;
}

type TerminationHandler = Box<dyn FnMut() -> bool>;

thread_local! {
    static UPDATE_TERMINATION_HANDLER: std::cell::RefCell<Option<TerminationHandler>> =
        std::cell::RefCell::new(None);
}

extern "C" fn prepare_update_termination() -> bool {
    // AppKit bypasses GPUI's on_app_quit observers. Cleanup must finish
    // synchronously, and missing entities, reentrant borrows and panics must
    // cancel termination without unwinding across the C ABI.
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        UPDATE_TERMINATION_HANDLER.with(|slot| {
            let Ok(mut handler) = slot.try_borrow_mut() else {
                return false;
            };
            handler.as_mut().is_some_and(|handler| handler())
        })
    }))
    .unwrap_or(false)
}

/// Call on GPUI's main thread before initialization. The closure must enter the
/// app context, clear the main TerminalApp's local sessions, and return true only
/// after cleanup succeeds. A stale app/entity must return false.
pub fn set_update_termination_handler(handler: impl FnMut() -> bool + 'static) {
    UPDATE_TERMINATION_HANDLER.with(|slot| *slot.borrow_mut() = Some(Box::new(handler)));
    #[cfg(target_os = "macos")]
    unsafe {
        mt_updater_set_termination_handler(prepare_update_termination);
    }
}

pub fn updater_status() -> UpdateStatus {
    #[cfg(target_os = "macos")]
    {
        // The bridge returns a strdup-owned snapshot. Free its allocation before
        // exposing any parsed value; no native string is borrowed by the caller.
        let pointer = unsafe { mt_updater_status() };
        if !pointer.is_null() {
            let result =
                serde_json::from_slice(unsafe { std::ffi::CStr::from_ptr(pointer).to_bytes() });
            unsafe { libc::free(pointer.cast()) };
            return result.unwrap_or_else(|error| UpdateStatus {
                state: UpdateState::Error,
                message: format!("Could not read update status: {error}"),
                version: None,
                can_check: false,
            });
        }
    }
    UpdateStatus::default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn termination_handler_fails_closed_without_unwinding_over_ffi() {
        UPDATE_TERMINATION_HANDLER.with(|slot| *slot.borrow_mut() = None);
        assert!(!prepare_update_termination());
        UPDATE_TERMINATION_HANDLER.with(|slot| *slot.borrow_mut() = Some(Box::new(|| true)));
        assert!(prepare_update_termination());
        UPDATE_TERMINATION_HANDLER.with(|slot| {
            let _active = slot.borrow_mut();
            assert!(!prepare_update_termination());
        });
        UPDATE_TERMINATION_HANDLER.with(|slot| {
            *slot.borrow_mut() = Some(Box::new(|| {
                assert!(!prepare_update_termination());
                true
            }));
        });
        assert!(prepare_update_termination());
        UPDATE_TERMINATION_HANDLER.with(|slot| {
            *slot.borrow_mut() = Some(Box::new(|| panic!("simulated GPUI borrow failure")));
        });
        assert!(!prepare_update_termination());
        UPDATE_TERMINATION_HANDLER.with(|slot| *slot.borrow_mut() = Some(Box::new(|| false)));
        assert!(!prepare_update_termination());
        UPDATE_TERMINATION_HANDLER.with(|slot| *slot.borrow_mut() = None);
    }

    #[test]
    fn native_status_contract_preserves_version_and_disables_inert_polling() {
        let status: UpdateStatus = serde_json::from_str(
            r#"{"state":"install_pending","message":"Ready","version":"0.1.1\"\né","can_check":false}"#,
        )
        .expect("valid native status JSON");
        assert_eq!(status.state, UpdateState::InstallPending);
        assert_eq!(status.version.as_deref(), Some("0.1.1\"\n\u{e9}"));
        assert!(!status.can_check);
        assert!(status.should_poll());
        for state in [UpdateState::Disabled, UpdateState::Unavailable] {
            assert!(
                !UpdateStatus {
                    state,
                    ..UpdateStatus::default()
                }
                .should_poll()
            );
        }
    }
}
