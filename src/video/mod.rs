//! Video / camera input.
//!
//! Captures a webcam (macOS AVFoundation / Linux v4l2, via `nokhwa`) on its own
//! thread and publishes the latest frame as RGBA into [`VideoShared`], which the
//! render loop uploads to a texture for the "camera" generator. Capture is
//! behind the `camera` feature; without it this is a no-op that simply reports
//! no frames (the camera generator then shows black).

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

/// One decoded frame, tightly-packed RGBA8, top-down.
pub struct VideoFrame {
    pub w: u32,
    pub h: u32,
    pub rgba: Vec<u8>,
}

/// Shared between the capture thread and the renderer.
#[derive(Default)]
pub struct VideoShared {
    frame: Mutex<Option<VideoFrame>>,
    /// Bumped each time a new frame is published (for cheap change detection).
    seq: AtomicU64,
    active: AtomicBool,
}

impl VideoShared {
    pub fn is_active(&self) -> bool {
        self.active.load(Ordering::Relaxed)
    }

    /// Current frame sequence number.
    pub fn seq(&self) -> u64 {
        self.seq.load(Ordering::Relaxed)
    }

    /// Publish a new frame (called by the capture thread).
    fn publish(&self, w: u32, h: u32, rgba: Vec<u8>) {
        if let Ok(mut f) = self.frame.lock() {
            *f = Some(VideoFrame { w, h, rgba });
        }
        self.seq.fetch_add(1, Ordering::Relaxed);
    }

    /// Run `f` with the latest frame, if any (the renderer uploads it).
    pub fn with_frame<R>(&self, f: impl FnOnce(&VideoFrame) -> R) -> Option<R> {
        self.frame.lock().ok().and_then(|g| g.as_ref().map(f))
    }
}

pub use engine::VideoEngine;

#[cfg(feature = "camera")]
mod engine {
    use super::*;
    use std::thread::JoinHandle;

    use nokhwa::pixel_format::RgbFormat;
    use nokhwa::utils::{ApiBackend, CameraIndex, RequestedFormat, RequestedFormatType};
    use nokhwa::Camera;

    /// Owns the capture thread; drop to stop.
    pub struct VideoEngine {
        shared: Arc<VideoShared>,
        stop: Arc<AtomicBool>,
        thread: Option<JoinHandle<()>>,
        device: String,
    }

    impl VideoEngine {
        pub fn start(device: &str) -> Self {
            // Request camera permission up front (AVFoundation needs this).
            nokhwa::nokhwa_initialize(|_granted| {});
            let mut e = VideoEngine {
                shared: Arc::new(VideoShared::default()),
                stop: Arc::new(AtomicBool::new(false)),
                thread: None,
                device: device.to_string(),
            };
            e.open(device);
            e
        }

        pub fn shared(&self) -> Arc<VideoShared> {
            self.shared.clone()
        }

        pub fn current_device(&self) -> &str {
            &self.device
        }

        /// List camera device names.
        pub fn list_devices() -> Vec<String> {
            nokhwa::query(ApiBackend::Auto)
                .map(|cams| cams.into_iter().map(|c| c.human_name()).collect())
                .unwrap_or_default()
        }

        /// Switch to a different camera (by name; empty = first available).
        pub fn set_device(&mut self, device: &str) {
            self.stop.store(true, Ordering::Relaxed);
            if let Some(h) = self.thread.take() {
                let _ = h.join();
            }
            self.shared.active.store(false, Ordering::Relaxed);
            self.stop = Arc::new(AtomicBool::new(false));
            self.device = device.to_string();
            self.open(device);
        }

        /// Resolve a device name to an index and spawn the capture thread.
        fn open(&mut self, device: &str) {
            let index = resolve_index(device);
            let shared = self.shared.clone();
            let stop = self.stop.clone();
            let handle = std::thread::Builder::new()
                .name("video-capture".into())
                .spawn(move || capture_loop(index, shared, stop))
                .ok();
            self.thread = handle;
        }
    }

    impl Drop for VideoEngine {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::Relaxed);
            if let Some(h) = self.thread.take() {
                let _ = h.join();
            }
        }
    }

    /// Map a device name to its enumerated index (default: first camera).
    fn resolve_index(device: &str) -> u32 {
        if device.is_empty() {
            return 0;
        }
        if let Ok(cams) = nokhwa::query(ApiBackend::Auto) {
            for c in cams {
                if c.human_name().contains(device) {
                    if let CameraIndex::Index(i) = c.index() {
                        return *i;
                    }
                }
            }
        }
        0
    }

    /// The capture loop: open the camera, decode frames to RGBA, publish them.
    fn capture_loop(index: u32, shared: Arc<VideoShared>, stop: Arc<AtomicBool>) {
        let format = RequestedFormat::new::<RgbFormat>(RequestedFormatType::AbsoluteHighestFrameRate);
        let mut cam = match Camera::new(CameraIndex::Index(index), format) {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!("camera {index} unavailable: {e}");
                return;
            }
        };
        if let Err(e) = cam.open_stream() {
            tracing::warn!("camera stream failed: {e}");
            return;
        }
        let name = cam.info().human_name();
        tracing::info!("video input: {name}");
        shared.active.store(true, Ordering::Relaxed);

        while !stop.load(Ordering::Relaxed) {
            match cam.frame().and_then(|b| b.decode_image::<RgbFormat>()) {
                Ok(img) => {
                    let (w, h) = (img.width(), img.height());
                    let rgb = img.as_raw();
                    let mut rgba = vec![0u8; (w * h * 4) as usize];
                    for (i, px) in rgb.chunks_exact(3).enumerate() {
                        rgba[i * 4] = px[0];
                        rgba[i * 4 + 1] = px[1];
                        rgba[i * 4 + 2] = px[2];
                        rgba[i * 4 + 3] = 255;
                    }
                    shared.publish(w, h, rgba);
                }
                Err(_) => std::thread::sleep(std::time::Duration::from_millis(10)),
            }
        }
        shared.active.store(false, Ordering::Relaxed);
    }
}

#[cfg(not(feature = "camera"))]
mod engine {
    use super::*;

    /// No-op stand-in when the `camera` feature is disabled.
    pub struct VideoEngine {
        shared: Arc<VideoShared>,
        device: String,
    }

    impl VideoEngine {
        pub fn start(_device: &str) -> Self {
            VideoEngine { shared: Arc::new(VideoShared::default()), device: String::new() }
        }
        pub fn shared(&self) -> Arc<VideoShared> {
            self.shared.clone()
        }
        pub fn current_device(&self) -> &str {
            &self.device
        }
        pub fn list_devices() -> Vec<String> {
            Vec::new()
        }
        pub fn set_device(&mut self, _device: &str) {}
    }
}
