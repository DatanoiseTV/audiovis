//! Ableton Link tempo/beat sync.
//!
//! When enabled, joins the Link session on the LAN and exposes the shared tempo
//! and beat position, which the render thread feeds into the beat clock so the
//! tempo-synced LFOs and clock phases lock to Ableton / other Link peers. Behind
//! the `link` feature (bundles the Link C++ library); a no-op otherwise.

pub use engine::LinkEngine;

#[cfg(feature = "link")]
mod engine {
    use rusty_link::{AblLink, SessionState};

    pub struct LinkEngine {
        link: AblLink,
        state: SessionState,
        enabled: bool,
    }

    impl LinkEngine {
        pub fn new() -> Self {
            LinkEngine { link: AblLink::new(120.0), state: SessionState::new(), enabled: false }
        }

        pub fn set_enabled(&mut self, on: bool) {
            if on != self.enabled {
                self.link.enable(on);
                self.link.enable_start_stop_sync(on);
                self.enabled = on;
                tracing::info!("Ableton Link {}", if on { "enabled" } else { "disabled" });
            }
        }

        pub fn enabled(&self) -> bool {
            self.enabled
        }

        pub fn peers(&self) -> u64 {
            if self.enabled {
                self.link.num_peers()
            } else {
                0
            }
        }

        /// The session tempo (BPM) and beat position at this instant, for the
        /// given quantum (bar length in beats). `None` when disabled.
        pub fn state(&mut self, quantum: f64) -> Option<(f32, f64)> {
            if !self.enabled {
                return None;
            }
            self.link.capture_app_session_state(&mut self.state);
            let now = self.link.clock_micros();
            Some((self.state.tempo() as f32, self.state.beat_at_time(now, quantum)))
        }
    }

    impl Default for LinkEngine {
        fn default() -> Self {
            Self::new()
        }
    }
}

#[cfg(not(feature = "link"))]
mod engine {
    /// No-op stand-in when the `link` feature is disabled.
    #[derive(Default)]
    pub struct LinkEngine;

    impl LinkEngine {
        pub fn new() -> Self {
            LinkEngine
        }
        pub fn set_enabled(&mut self, _on: bool) {}
        pub fn enabled(&self) -> bool {
            false
        }
        pub fn peers(&self) -> u64 {
            0
        }
        pub fn state(&mut self, _quantum: f64) -> Option<(f32, f64)> {
            None
        }
    }
}
