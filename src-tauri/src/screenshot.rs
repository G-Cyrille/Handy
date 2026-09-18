//! Screenshot context for AI post-processing.
//!
//! When the "screenshot context" setting is on, the post-processing shortcut
//! grabs either the monitor under the cursor or every active monitor *before*
//! the recording overlay appears. Multi-monitor captures keep a detailed
//! panorama for history and send each monitor to the LLM as a separate image,
//! so adding displays does not reduce the readable resolution of each one.
//! An optional enlarged pointer marks the cursor at capture time in both outputs. The
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

/// Draw an enlarged mouse pointer with its hotspot at the captured position.
/// Scale it for both the per-monitor LLM image and the logical-size panorama.
fn mark_cursor(monitor: &mut MonitorCapture, cursor: (f64, f64)) {
    let local_x = cursor.0 - f64::from(monitor.x);
    let local_y = cursor.1 - f64::from(monitor.y);
    if !(0.0..f64::from(monitor.logical_width)).contains(&local_x)
        || !(0.0..f64::from(monitor.logical_height)).contains(&local_y)
    {
        return;
    }
    let (width, height) = monitor.image.dimensions();
    if width == 0 || height == 0 {
        return;
    }
    let tip = (
        local_x * f64::from(width) / f64::from(monitor.logical_width),
        local_y * f64::from(height) / f64::from(monitor.logical_height),
    );
    let scale = (f64::from(width.max(height)) / f64::from(MAX_EDGE))
        .max(f64::from(width) / f64::from(monitor.logical_width))
        .max(f64::from(height) / f64::from(monitor.logical_height))
        .max(1.0);
    let points = [
        (0.0, 0.0),
        (0.0, 30.0),
        (8.0, 23.0),
        (14.0, 36.0),
        (20.0, 33.0),
        (14.0, 21.0),
        (25.0, 21.0),
    ]
    .map(|(x, y)| (tip.0 + x * scale, tip.1 + y * scale));
    let min_x = tip.0.max(0.0) as u32;
    let min_y = tip.1.max(0.0) as u32;
    let max_x = ((tip.0 + 26.0 * scale).ceil() as u32).min(width - 1);
    let max_y = ((tip.1 + 37.0 * scale).ceil() as u32).min(height - 1);
    for y in min_y..=max_y {
        for x in min_x..=max_x {
            let mut inside = false;
            for i in 0..points.len() {
                let (ax, ay) = points[i];
                let (bx, by) = points[(i + 1) % points.len()];
                if (ay > f64::from(y)) != (by > f64::from(y))
                    && f64::from(x) < (bx - ax) * (f64::from(y) - ay) / (by - ay) + ax
                {
                    inside = !inside;
                }
            }
            if inside {
                monitor.image.put_pixel(x, y, Rgba([20, 20, 20, 255]));
            }
        }
    }
    for i in 0..points.len() {
        draw_segment(
            &mut monitor.image,
            points[i],
            points[(i + 1) % points.len()],
            1.5 * scale,
            Rgba([255, 255, 255, 255]),
        );
    }
}

/// Clip the field's desktop rectangle to each display before converting to pixels.
fn mark_focused_field(monitor: &mut MonitorCapture, bounds: (f64, f64, f64, f64)) {
    let (x, y, width, height) = bounds;
    let left = (x - f64::from(monitor.x)).max(0.0);
    let top = (y - f64::from(monitor.y)).max(0.0);
    let right = (x + width - f64::from(monitor.x)).min(f64::from(monitor.logical_width));
    let bottom = (y + height - f64::from(monitor.y)).min(f64::from(monitor.logical_height));
    if left >= right || top >= bottom || monitor.image.width() == 0 || monitor.image.height() == 0 {
        return;
    }
    let sx = f64::from(monitor.image.width()) / f64::from(monitor.logical_width);
    let sy = f64::from(monitor.image.height()) / f64::from(monitor.logical_height);
    let points = [
        (left * sx, top * sy),
        (right * sx - 1.0, top * sy),
        (right * sx - 1.0, bottom * sy - 1.0),
        (left * sx, bottom * sy - 1.0),
    ];
    let scale = sx
        .max(sy)
        .max(f64::from(monitor.image.width().max(monitor.image.height())) / f64::from(MAX_EDGE));
    for (radius, color) in [
        (3.0 * scale, Rgba([255, 255, 255, 255])),
        (1.5 * scale, Rgba([230, 40, 70, 255])),
    ] {
        for i in 0..4 {
            draw_segment(
                &mut monitor.image,
                points[i],
                points[(i + 1) % 4],
                radius,
                color,
            );
        }
    }
}

fn draw_segment(
    image: &mut RgbaImage,
    start: (f64, f64),
    end: (f64, f64),
    radius: f64,
    color: Rgba<u8>,
) {
    let min_x = (start.0.min(end.0) - radius).max(0.0) as u32;
    let max_x = ((start.0.max(end.0) + radius).ceil() as u32).min(image.width() - 1);
    let min_y = (start.1.min(end.1) - radius).max(0.0) as u32;
    let max_y = ((start.1.max(end.1) + radius).ceil() as u32).min(image.height() - 1);
    let vx = end.0 - start.0;
    let vy = end.1 - start.1;
    let squared_length = vx * vx + vy * vy;
    for y in min_y..=max_y {
        for x in min_x..=max_x {
            let px = f64::from(x) - start.0;
            let py = f64::from(y) - start.1;
            let t = ((px * vx + py * vy) / squared_length).clamp(0.0, 1.0);
            if (px - t * vx).powi(2) + (py - t * vy).powi(2) <= radius * radius {
                image.put_pixel(x, y, color);
            }
        }
    }
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

    /// Persist separate monitor images beside the panorama for history and retries.
    pub fn save_monitor_images(&self, panorama_path: &std::path::Path) -> Result<(), String> {
        if self.llm_jpegs.is_empty() {
            return Ok(());
        }
        let images: Vec<_> = self
            .llm_jpegs
            .iter()
            .map(|jpeg| BASE64.encode(jpeg))
            .collect();
        let bytes = serde_json::to_vec(&images).map_err(|err| err.to_string())?;
        std::fs::write(panorama_path.with_extension("images.json"), bytes)
            .map_err(|err| err.to_string())
    }

    /// Older entries have only a panorama. Missing or corrupt sidecars use that fallback.
    pub fn load_monitor_images(&mut self, panorama_path: &std::path::Path) {
        let load = || -> Option<Vec<Vec<u8>>> {
            let bytes = std::fs::read(panorama_path.with_extension("images.json")).ok()?;
            let images: Vec<String> = serde_json::from_slice(&bytes).ok()?;
            images
                .into_iter()
                .map(|encoded| {
                    let jpeg = BASE64.decode(encoded).ok()?;
                    image::load_from_memory_with_format(&jpeg, ImageFormat::Jpeg).ok()?;
                    Some(jpeg)
                })
                .collect()
        };
        self.llm_jpegs = load().unwrap_or_default();
    }

    /// Panorama first, followed by individual monitors when several were captured.
    pub fn history_data_urls(&self) -> Vec<String> {
        let mut images = vec![self.data_url()];
        if self.llm_jpegs.len() > 1 {
            images.extend(self.llm_data_urls());
        }
        images
    }

    pub fn history_image(&self, index: usize) -> Option<&[u8]> {
        if index == 0 {
            Some(&self.jpeg)
        } else if self.llm_jpegs.len() > 1 {
            self.llm_jpegs.get(index - 1).map(Vec::as_slice)
        } else {
            None
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
    /// Older history entries without a monitor sidecar fall back to the panorama.
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
fn capture_screenshot(
    scope: ScreenshotCaptureScope,
    cursor_highlight: bool,
    focus_highlight: bool,
) -> Result<RawScreenshot, String> {
    use objc2_core_graphics::{CGEvent, CGPreflightScreenCaptureAccess};

    if !CGPreflightScreenCaptureAccess() {
        return Err("Screen Recording permission not granted".to_string());
    }

    let focused_bounds = focus_highlight
        .then(crate::screenshot_focus::focused_text_bounds)
        .flatten();
    let cursor = CGEvent::location(CGEvent::new(None).as_deref());
    let cursor_position = (cursor.x, cursor.y);
    match scope {
        ScreenshotCaptureScope::CurrentMonitor => {
            let monitor = xcap::Monitor::from_point(cursor.x as i32, cursor.y as i32)
                .map_err(|err| format!("no monitor under cursor: {err}"))?;
            let image = monitor
                .capture_image()
                .map_err(|err| format!("monitor capture failed: {err}"))?;
            if !cursor_highlight && focused_bounds.is_none() {
                return Ok(RawScreenshot::Single(image));
            }
            let mut capture = MonitorCapture {
                x: monitor.x().map_err(|err| err.to_string())?,
                y: monitor.y().map_err(|err| err.to_string())?,
                logical_width: monitor.width().map_err(|err| err.to_string())?,
                logical_height: monitor.height().map_err(|err| err.to_string())?,
                image,
            };
            if let Some(bounds) = focused_bounds {
                mark_focused_field(&mut capture, bounds);
            }
            if cursor_highlight {
                mark_cursor(&mut capture, cursor_position);
            }
            Ok(RawScreenshot::Single(capture.image))
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
                    let mut capture = MonitorCapture {
                        x,
                        y,
                        logical_width,
                        logical_height,
                        image,
                    };
                    if let Some(bounds) = focused_bounds {
                        mark_focused_field(&mut capture, bounds);
                    }
                    if cursor_highlight {
                        mark_cursor(&mut capture, cursor_position);
                    }
                    Ok::<_, String>(capture)
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
fn capture_screenshot(
    _scope: ScreenshotCaptureScope,
    _cursor_highlight: bool,
    _focus_highlight: bool,
) -> Result<RawScreenshot, String> {
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
    pub fn begin_capture(
        &self,
        scope: ScreenshotCaptureScope,
        cursor_highlight: bool,
        focus_highlight: bool,
    ) {
        let started = Instant::now();
        let raw = match capture_screenshot(scope, cursor_highlight, focus_highlight) {
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
    fn monitor_sidecar_roundtrip_and_legacy_fallback() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("capture.jpg");
        let capture = CapturedScreenshot {
            jpeg: encode_jpeg(solid_image(200, 100), MAX_EDGE).unwrap(),
            llm_jpegs: vec![encode_jpeg(solid_image(100, 100), MAX_EDGE).unwrap(); 2],
        };
        capture.save_monitor_images(&path).unwrap();
        let mut restored = CapturedScreenshot::from_jpeg(capture.jpeg.clone()).unwrap();
        restored.load_monitor_images(&path);
        assert_eq!(restored, capture);
        assert_eq!(restored.history_data_urls().len(), 3);
        assert!(restored.history_image(3).is_none());
        std::fs::write(path.with_extension("images.json"), b"corrupt").unwrap();
        restored.load_monitor_images(&path);
        assert_eq!(restored.history_data_urls().len(), 1);
        assert_eq!(restored.llm_data_urls(), vec![restored.data_url()]);
        std::fs::remove_file(path.with_extension("images.json")).unwrap();
        restored.load_monitor_images(&path);
        assert_eq!(restored.history_data_urls().len(), 1);
    }

    #[test]
    fn focused_field_clips_to_retina_monitor_without_filling_the_field() {
        let mut monitor = MonitorCapture {
            x: -800,
            y: 0,
            logical_width: 800,
            logical_height: 600,
            image: solid_image(1600, 1200),
        };
        let original = monitor.image.clone();
        mark_focused_field(&mut monitor, (10.0, 20.0, 100.0, 100.0));
        assert_eq!(monitor.image, original);
        mark_focused_field(&mut monitor, (-850.0, 100.0, 200.0, 100.0));
        assert_eq!(monitor.image.get_pixel(100, 200), &Rgba([230, 40, 70, 255]));
        assert_eq!(
            monitor.image.get_pixel(100, 300),
            original.get_pixel(100, 300)
        );
    }

    #[test]
    fn cursor_highlight_maps_retina_and_negative_desktop_coordinates() {
        let mut monitor = MonitorCapture {
            x: -800,
            y: -200,
            logical_width: 800,
            logical_height: 600,
            image: solid_image(1600, 1200),
        };
        let original = monitor.image.clone();
        mark_cursor(&mut monitor, (0.0, 100.0));
        assert_eq!(monitor.image, original, "cursor on another display");
        mark_cursor(&mut monitor, (-600.0, -50.0));
        assert_eq!(
            monitor.image.get_pixel(400, 300),
            &Rgba([255, 255, 255, 255])
        );
        assert_eq!(
            monitor.image.get_pixel(1200, 900),
            original.get_pixel(1200, 900)
        );
    }

    #[test]
    fn cursor_highlight_survives_panorama_and_llm_encoding() {
        let mut monitor = MonitorCapture {
            x: 0,
            y: 0,
            logical_width: 800,
            logical_height: 600,
            image: solid_image(3200, 2400),
        };
        mark_cursor(&mut monitor, (400.0, 300.0));
        let screenshot = encode_capture(RawScreenshot::Multiple(vec![monitor])).unwrap();
        for jpeg in [&screenshot.jpeg, &screenshot.llm_jpegs[0]] {
            let decoded = image::load_from_memory(jpeg).unwrap().to_rgb8();
            assert!(
                decoded
                    .pixels()
                    .filter(|p| p[0] < 40 && p[1] < 40 && p[2] < 40)
                    .count()
                    > 100
            );
            assert!(
                decoded
                    .pixels()
                    .filter(|p| p[0] > 200 && p[1] > 200 && p[2] > 200)
                    .count()
                    > 50
            );
        }
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
