//! Screenshot context for AI post-processing.
//!
//! When the "screenshot context" setting is on, the post-processing shortcut
//! grabs either the monitor under the cursor or every active monitor *before*
//! the recording overlay appears. Multi-monitor captures keep a detailed
//! panorama for history and send each monitor to the LLM as a separate image,
//! so adding displays does not reduce the readable resolution of each one. The
//! JPEG bytes are only written to disk next to the WAV once the history entry
//! is saved, so cancelled or empty recordings leave no file behind.
//!
//! Capture itself is macOS-only for now; other platforms return no capture and
//! the pipeline transparently stays text-only.

use crate::settings::ScreenshotCaptureScope;
use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use image::codecs::jpeg::JpegEncoder;
use image::imageops::{replace, resize, FilterType};
use image::{DynamicImage, ImageFormat, Rgba, RgbaImage};
use log::{debug, error, warn};
use std::sync::Mutex;
use std::thread::JoinHandle;
use std::time::Instant;

/// Longest edge of each image sent to the LLM, in pixels.
pub const MAX_EDGE: u32 = 1600;
/// JPEG quality used for the encoded screenshot.
pub const JPEG_QUALITY: u8 = 80;

/// One display capture and its rectangle in the virtual desktop's logical
/// coordinate space. The raw image can use a higher pixel density on Retina
/// displays; composition normalizes it to the logical rectangle first.
struct MonitorCapture {
    x: i32,
    y: i32,
    logical_width: u32,
    logical_height: u32,
    image: RgbaImage,
}

/// A screenshot ready to be sent to an LLM and persisted with a history entry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CapturedScreenshot {
    jpeg: Vec<u8>,
    llm_jpegs: Vec<Vec<u8>>,
}

impl CapturedScreenshot {
    /// Wrap previously persisted JPEG bytes (history retry). Returns `None`
    /// when the bytes do not decode as a JPEG, so a corrupt file degrades to a
    /// text-only request instead of a rejected one.
    pub fn from_jpeg(bytes: Vec<u8>) -> Option<Self> {
        match image::load_from_memory_with_format(&bytes, ImageFormat::Jpeg) {
            Ok(_) => Some(Self {
                jpeg: bytes,
                llm_jpegs: Vec::new(),
            }),
            Err(err) => {
                warn!("Stored screenshot is not a valid JPEG: {err}");
                None
            }
        }
    }

    pub fn jpeg(&self) -> &[u8] {
        &self.jpeg
    }

    /// `data:` URL for the persisted panorama.
    pub fn data_url(&self) -> String {
        format!("data:image/jpeg;base64,{}", BASE64.encode(&self.jpeg))
    }

    /// One `data:` URL per monitor for OpenAI-compatible `image_url` parts.
    /// A screenshot loaded from history only has its persisted panorama, which
    /// remains detailed and is returned as a single image.
    pub fn llm_data_urls(&self) -> Vec<String> {
        let images = if self.llm_jpegs.is_empty() {
            std::slice::from_ref(&self.jpeg)
        } else {
            self.llm_jpegs.as_slice()
        };
        images
            .iter()
            .map(|jpeg| format!("data:image/jpeg;base64,{}", BASE64.encode(jpeg)))
            .collect()
    }
}

/// Dimensions after fitting `(width, height)` inside a `max_edge` square while
/// keeping the aspect ratio. Images that already fit are returned unchanged.
fn fitted_dimensions(width: u32, height: u32, max_edge: u32) -> (u32, u32) {
    let longest = width.max(height);
    if longest <= max_edge {
        return (width, height);
    }
    let scale = f64::from(max_edge) / f64::from(longest);
    (
        ((f64::from(width) * scale).round() as u32).max(1),
        ((f64::from(height) * scale).round() as u32).max(1),
    )
}

fn encode_jpeg(image: RgbaImage, max_edge: u32) -> Result<Vec<u8>, String> {
    let (width, height) = image.dimensions();
    if width == 0 || height == 0 {
        return Err("captured image is empty".to_string());
    }

    let (target_width, target_height) = fitted_dimensions(width, height, max_edge);
    let image = DynamicImage::ImageRgba8(image);
    let rgb = if (target_width, target_height) == (width, height) {
        image.to_rgb8()
    } else {
        image
            .resize_exact(target_width, target_height, FilterType::Triangle)
            .to_rgb8()
    };

    let mut jpeg = Vec::new();
    JpegEncoder::new_with_quality(&mut jpeg, JPEG_QUALITY)
        .encode_image(&rgb)
        .map_err(|err| format!("JPEG encoding failed: {err}"))?;

    Ok(jpeg)
}

/// Downscale a single raw capture to `MAX_EDGE` and encode it as JPEG.
pub fn encode_screenshot(image: RgbaImage) -> Result<CapturedScreenshot, String> {
    Ok(CapturedScreenshot {
        jpeg: encode_jpeg(image, MAX_EDGE)?,
        llm_jpegs: Vec::new(),
    })
}

/// Compose monitor captures according to their virtual desktop positions.
/// The canvas is allocated at the final LLM resolution, which avoids creating
/// a very large intermediate image for Retina or many-monitor setups.
fn compose_monitors(monitors: &[MonitorCapture]) -> Result<RgbaImage, String> {
    if monitors.is_empty() {
        return Err("no active monitors found".to_string());
    }

    let min_x = monitors
        .iter()
        .map(|monitor| i64::from(monitor.x))
        .min()
        .unwrap_or_default();
    let min_y = monitors
        .iter()
        .map(|monitor| i64::from(monitor.y))
        .min()
        .unwrap_or_default();
    let max_x = monitors
        .iter()
        .map(|monitor| i64::from(monitor.x) + i64::from(monitor.logical_width))
        .max()
        .unwrap_or_default();
    let max_y = monitors
        .iter()
        .map(|monitor| i64::from(monitor.y) + i64::from(monitor.logical_height))
        .max()
        .unwrap_or_default();
    let virtual_width =
        u32::try_from(max_x - min_x).map_err(|_| "virtual desktop width is invalid".to_string())?;
    let virtual_height = u32::try_from(max_y - min_y)
        .map_err(|_| "virtual desktop height is invalid".to_string())?;
    if virtual_width == 0 || virtual_height == 0 {
        return Err("virtual desktop is empty".to_string());
    }

    let monitor_count = u32::try_from(monitors.len()).unwrap_or(u32::MAX);
    let panorama_max_edge = MAX_EDGE.saturating_mul(monitor_count);
    let (canvas_width, canvas_height) =
        fitted_dimensions(virtual_width, virtual_height, panorama_max_edge);
    let mut canvas = RgbaImage::from_pixel(canvas_width, canvas_height, Rgba([0, 0, 0, 255]));

    for monitor in monitors {
        let left = ((i64::from(monitor.x) - min_x) * i64::from(canvas_width)
            / i64::from(virtual_width)) as u32;
        let top = ((i64::from(monitor.y) - min_y) * i64::from(canvas_height)
            / i64::from(virtual_height)) as u32;
        let right = ((i64::from(monitor.x) - min_x + i64::from(monitor.logical_width))
            * i64::from(canvas_width)
            / i64::from(virtual_width)) as u32;
        let bottom = ((i64::from(monitor.y) - min_y + i64::from(monitor.logical_height))
            * i64::from(canvas_height)
            / i64::from(virtual_height)) as u32;
        let width = right.saturating_sub(left).max(1);
        let height = bottom.saturating_sub(top).max(1);
        let normalized = resize(&monitor.image, width, height, FilterType::Triangle);
        replace(&mut canvas, &normalized, i64::from(left), i64::from(top));
    }

    Ok(canvas)
}

enum RawScreenshot {
    Single(RgbaImage),
    Multiple(Vec<MonitorCapture>),
}

impl RawScreenshot {
    fn image_count(&self) -> usize {
        match self {
            Self::Single(_) => 1,
            Self::Multiple(monitors) => monitors.len(),
        }
    }
}

fn encode_capture(raw: RawScreenshot) -> Result<CapturedScreenshot, String> {
    match raw {
        RawScreenshot::Single(image) => encode_screenshot(image),
        RawScreenshot::Multiple(monitors) => {
            let monitor_count = u32::try_from(monitors.len()).unwrap_or(u32::MAX);
            let panorama_max_edge = MAX_EDGE.saturating_mul(monitor_count);
            let panorama = compose_monitors(&monitors)?;
            let jpeg = encode_jpeg(panorama, panorama_max_edge)?;
            let llm_jpegs = monitors
                .into_iter()
                .map(|monitor| encode_jpeg(monitor.image, MAX_EDGE))
                .collect::<Result<Vec<_>, _>>()?;
            Ok(CapturedScreenshot { jpeg, llm_jpegs })
        }
    }
}

/// Grab the requested monitor scope as raw RGBA pixels.
///
/// Never prompts for permission: the Screen Recording check is a preflight, so
/// a hotkey press can't surface a system dialog. Permission is requested from
/// the settings UI when the user turns the feature on.
#[cfg(target_os = "macos")]
fn capture_screenshot(scope: ScreenshotCaptureScope) -> Result<RawScreenshot, String> {
    use objc2_core_graphics::{CGEvent, CGPreflightScreenCaptureAccess};

    if !CGPreflightScreenCaptureAccess() {
        return Err("Screen Recording permission not granted".to_string());
    }

    match scope {
        ScreenshotCaptureScope::CurrentMonitor => {
            let cursor = CGEvent::location(CGEvent::new(None).as_deref());
            let monitor = xcap::Monitor::from_point(cursor.x as i32, cursor.y as i32)
                .map_err(|err| format!("no monitor under cursor: {err}"))?;
            monitor
                .capture_image()
                .map(RawScreenshot::Single)
                .map_err(|err| format!("monitor capture failed: {err}"))
        }
        ScreenshotCaptureScope::AllMonitors => {
            let monitors = xcap::Monitor::all()
                .map_err(|err| format!("failed to list active monitors: {err}"))?;
            let mut capture_threads = Vec::with_capacity(monitors.len());
            for monitor in monitors {
                let x = monitor
                    .x()
                    .map_err(|err| format!("failed to read monitor x position: {err}"))?;
                let y = monitor
                    .y()
                    .map_err(|err| format!("failed to read monitor y position: {err}"))?;
                let logical_width = monitor
                    .width()
                    .map_err(|err| format!("failed to read monitor width: {err}"))?;
                let logical_height = monitor
                    .height()
                    .map_err(|err| format!("failed to read monitor height: {err}"))?;
                capture_threads.push(std::thread::spawn(move || {
                    let image = monitor
                        .capture_image()
                        .map_err(|err| format!("monitor capture failed: {err}"))?;
                    Ok::<_, String>(MonitorCapture {
                        x,
                        y,
                        logical_width,
                        logical_height,
                        image,
                    })
                }));
            }

            // Join every worker before returning, even after one failed, so no
            // late capture can include the overlay that appears next.
            let mut captures = Vec::with_capacity(capture_threads.len());
            let mut first_error = None;
            for thread in capture_threads {
                match thread.join() {
                    Ok(Ok(capture)) => captures.push(capture),
                    Ok(Err(err)) => {
                        first_error.get_or_insert(err);
                    }
                    Err(_) => {
                        first_error
                            .get_or_insert_with(|| "monitor capture thread panicked".to_string());
                    }
                }
            }
            match first_error {
                Some(err) => Err(err),
                None => Ok(RawScreenshot::Multiple(captures)),
            }
        }
    }
}

#[cfg(not(target_os = "macos"))]
fn capture_screenshot(_scope: ScreenshotCaptureScope) -> Result<RawScreenshot, String> {
    Err("screenshot capture is not supported on this platform".to_string())
}

/// Encoding work still running for the current dictation.
pub struct PendingScreenshot(JoinHandle<Option<CapturedScreenshot>>);

impl PendingScreenshot {
    /// Wait for encoding to finish. Blocks the calling thread.
    pub fn wait(self) -> Option<CapturedScreenshot> {
        match self.0.join() {
            Ok(screenshot) => screenshot,
            Err(_) => {
                error!("Screenshot encoding thread panicked");
                None
            }
        }
    }
}

/// Per-dictation screenshot state, managed by Tauri.
///
/// `begin_capture` grabs the displays synchronously so it completes before the
/// overlay shows. Multi-monitor grabs run in parallel. Composition, resizing,
/// and encoding then happen on a worker thread. The stop path moves the worker
/// out with `take_pending` and joins it once it needs the bytes; cancellation
/// calls `discard`, which drops the handle without joining: processing still in
/// flight finishes on its own thread and its bytes are thrown away, so nothing
/// from an abandoned dictation reaches the LLM or the disk.
#[derive(Default)]
pub struct ScreenshotContext {
    pending: Mutex<Option<PendingScreenshot>>,
}

impl ScreenshotContext {
    pub fn new() -> Self {
        Self::default()
    }

    /// Capture the requested monitor scope and start encoding it. Failures are
    /// logged and leave no pending capture (text-only post-processing).
    pub fn begin_capture(&self, scope: ScreenshotCaptureScope) {
        let started = Instant::now();
        let raw = match capture_screenshot(scope) {
            Ok(image) => image,
            Err(err) => {
                warn!("Screenshot context skipped: {err}");
                self.discard();
                return;
            }
        };
        debug!(
            "Captured {} raw screenshot image(s) ({scope:?}) in {:?}",
            raw.image_count(),
            started.elapsed()
        );

        let processing_started = Instant::now();
        let handle = std::thread::spawn(move || match encode_capture(raw) {
            Ok(screenshot) => {
                debug!(
                    "Screenshot context composed and encoded to {} bytes in {:?}",
                    screenshot.jpeg.len(),
                    processing_started.elapsed()
                );
                Some(screenshot)
            }
            Err(err) => {
                warn!("Screenshot context dropped: {err}");
                None
            }
        });

        if self
            .replace_pending(Some(PendingScreenshot(handle)))
            .is_some()
        {
            debug!("Replaced a pending screenshot from a dictation that never stopped");
        }
    }

    /// Hand the pending capture (if any) to the caller. The context is empty
    /// afterwards.
    pub fn take_pending(&self) -> Option<PendingScreenshot> {
        self.replace_pending(None)
    }

    /// Forget any pending capture. Does not wait for the encoder: the thread
    /// detaches and its result is dropped when it finishes.
    pub fn discard(&self) {
        if self.take_pending().is_some() {
            debug!("Discarded pending screenshot context");
        }
    }

    fn replace_pending(&self, next: Option<PendingScreenshot>) -> Option<PendingScreenshot> {
        let mut pending = self
            .pending
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        std::mem::replace(&mut *pending, next)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgba;

    fn solid_image(width: u32, height: u32) -> RgbaImage {
        RgbaImage::from_pixel(width, height, Rgba([40, 120, 200, 255]))
    }

    #[test]
    fn fitted_dimensions_scales_longest_edge_to_max() {
        assert_eq!(fitted_dimensions(3840, 2160, MAX_EDGE), (1600, 900));
        assert_eq!(fitted_dimensions(2000, 4000, MAX_EDGE), (800, 1600));
        assert_eq!(fitted_dimensions(5120, 2880, MAX_EDGE), (1600, 900));
    }

    #[test]
    fn fitted_dimensions_keeps_small_images() {
        assert_eq!(fitted_dimensions(1600, 1000, MAX_EDGE), (1600, 1000));
        assert_eq!(fitted_dimensions(800, 600, MAX_EDGE), (800, 600));
    }

    #[test]
    fn encode_screenshot_downscales_and_produces_jpeg() {
        let screenshot = encode_screenshot(solid_image(3200, 2000)).unwrap();

        assert!(screenshot.jpeg().starts_with(&[0xFF, 0xD8]));
        let decoded =
            image::load_from_memory_with_format(screenshot.jpeg(), ImageFormat::Jpeg).unwrap();
        assert_eq!((decoded.width(), decoded.height()), (1600, 1000));
    }

    #[test]
    fn encode_screenshot_rejects_empty_image() {
        assert!(encode_screenshot(RgbaImage::new(0, 0)).is_err());
    }

    #[test]
    fn compose_monitors_preserves_layout_with_negative_coordinates() {
        let left = MonitorCapture {
            x: -100,
            y: 0,
            logical_width: 100,
            logical_height: 100,
            image: RgbaImage::from_pixel(200, 200, Rgba([255, 0, 0, 255])),
        };
        let right = MonitorCapture {
            x: 0,
            y: 25,
            logical_width: 200,
            logical_height: 100,
            image: RgbaImage::from_pixel(200, 100, Rgba([0, 0, 255, 255])),
        };

        let composed = compose_monitors(&[left, right]).unwrap();

        assert_eq!(composed.dimensions(), (300, 125));
        assert_eq!(composed.get_pixel(50, 50), &Rgba([255, 0, 0, 255]));
        assert_eq!(composed.get_pixel(150, 75), &Rgba([0, 0, 255, 255]));
        assert_eq!(composed.get_pixel(150, 10), &Rgba([0, 0, 0, 255]));
    }

    #[test]
    fn compose_monitors_allocates_one_max_edge_per_display() {
        let monitors = vec![MonitorCapture {
            x: 0,
            y: 0,
            logical_width: 6000,
            logical_height: 2000,
            image: solid_image(3000, 1000),
        }];

        assert_eq!(
            compose_monitors(&monitors).unwrap().dimensions(),
            (1600, 533)
        );

        let monitors = vec![
            MonitorCapture {
                x: 0,
                y: 0,
                logical_width: 3000,
                logical_height: 1000,
                image: solid_image(3000, 1000),
            },
            MonitorCapture {
                x: 3000,
                y: 0,
                logical_width: 3000,
                logical_height: 1000,
                image: solid_image(3000, 1000),
            },
            MonitorCapture {
                x: 6000,
                y: 0,
                logical_width: 3000,
                logical_height: 1000,
                image: solid_image(3000, 1000),
            },
        ];
        assert_eq!(
            compose_monitors(&monitors).unwrap().dimensions(),
            (4800, 533)
        );
    }

    #[test]
    fn multi_monitor_capture_keeps_detailed_panorama_and_separate_llm_images() {
        let raw = RawScreenshot::Multiple(
            [0, 4000, 8000]
                .into_iter()
                .map(|x| MonitorCapture {
                    x,
                    y: 0,
                    logical_width: 4000,
                    logical_height: 3000,
                    image: solid_image(3200, 1800),
                })
                .collect(),
        );

        let screenshot = encode_capture(raw).unwrap();
        let panorama =
            image::load_from_memory_with_format(screenshot.jpeg(), ImageFormat::Jpeg).unwrap();
        assert_eq!((panorama.width(), panorama.height()), (4800, 1200));
        assert_eq!(screenshot.llm_jpegs.len(), 3);
        for jpeg in &screenshot.llm_jpegs {
            let monitor = image::load_from_memory_with_format(jpeg, ImageFormat::Jpeg).unwrap();
            assert_eq!((monitor.width(), monitor.height()), (1600, 900));
        }
    }

    #[test]
    fn data_url_is_base64_jpeg() {
        let screenshot = encode_screenshot(solid_image(10, 10)).unwrap();
        let url = screenshot.data_url();
        let encoded = url.strip_prefix("data:image/jpeg;base64,").unwrap();
        assert_eq!(BASE64.decode(encoded).unwrap(), screenshot.jpeg());
    }

    #[test]
    fn from_jpeg_accepts_encoded_output_and_rejects_garbage() {
        let screenshot = encode_screenshot(solid_image(10, 10)).unwrap();
        assert_eq!(
            CapturedScreenshot::from_jpeg(screenshot.jpeg().to_vec()),
            Some(screenshot)
        );
        assert!(CapturedScreenshot::from_jpeg(b"not a jpeg".to_vec()).is_none());
    }

    #[test]
    fn context_hands_out_pending_capture_once() {
        let context = ScreenshotContext::new();
        assert!(context.take_pending().is_none());

        let handle = std::thread::spawn(|| encode_screenshot(solid_image(10, 10)).ok());
        context.replace_pending(Some(PendingScreenshot(handle)));

        let pending = context.take_pending().expect("pending capture");
        assert!(context.take_pending().is_none());
        assert!(pending.wait().is_some());
    }

    #[test]
    fn discard_drops_pending_capture() {
        let context = ScreenshotContext::new();
        let handle = std::thread::spawn(|| encode_screenshot(solid_image(10, 10)).ok());
        context.replace_pending(Some(PendingScreenshot(handle)));

        context.discard();
        assert!(context.take_pending().is_none());
    }
}
