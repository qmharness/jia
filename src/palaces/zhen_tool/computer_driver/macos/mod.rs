// ── computer/macos.rs — Native macOS desktop control backend ──
//
// Uses only public Apple frameworks via the objc2 crate family.
//
// objc2 v0.3 API pattern: CF types use associated functions with Option<&T>
// as the first parameter, e.g. CGEvent::post_to_pid(pid, Some(&event)).

pub(crate) use std::ptr::NonNull;

pub(crate) use objc2_app_kit::NSWorkspace;
pub(crate) use objc2_application_services::{AXError, AXUIElement, AXValue, AXValueType};
pub(crate) use objc2_core_foundation::{
    CFArray, CFBoolean, CFRetained, CFString, CFType, CGPoint, CGRect, CGSize,
};
#[allow(deprecated)]
pub(crate) use objc2_core_graphics::{
    CGDataProvider, CGDisplayBounds, CGDisplayPixelsWide, CGEvent, CGEventFlags, CGEventType,
    CGImage, CGMainDisplayID, CGMouseButton, CGScrollEventUnit, CGWindowImageOption,
    CGWindowListCreateImage, CGWindowListOption, kCGNullWindowID,
};

pub(crate) use super::{ActionResult, AppInfo, CaptureResult, ScreenInfo, SomElement};

// SAFETY: FFI declaration for CFArrayGetValueAtIndex. This is a public
// CoreFoundation API. The function expects a valid CFArray pointer and an
// index within bounds [0, CFArrayGetCount(arr)). Callers guarantee these
// preconditions.
// Raw CF function for array access when type parameter is erased (Opaque).
unsafe extern "C" {
    #[allow(unused)]
    pub(crate) fn CFArrayGetValueAtIndex(arr: *const std::ffi::c_void, idx: isize) -> *const std::ffi::c_void;
}


// ── Submodules ───────────────────────────────────────────
mod ax;
mod screenshot;
mod input;

pub(crate) use ax::*;
pub(crate) use screenshot::*;
pub(crate) use input::*;


pub struct MacOsBackend;

impl MacOsBackend {
    pub fn capture(&self, mode: &str, app: Option<&str>) -> Result<CaptureResult, String> {
        let pid = if let Some(name) = app {
            find_app_pid(name)?
        } else {
            frontmost_pid()?
        };

        let (screenshot_b64, ax_tree, elements) = match mode {
            "ax" => {
                let (tree, elems) = ax_tree_for_pid(pid, 8)?;
                (None, Some(tree), elems)
            }
            "vision" => {
                let img = capture_screenshot()?;
                let png = cgimage_to_png_bytes(&img)?;
                (Some(base64_encode(&png)), None, Vec::new())
            }
            _ => {
                // "som" default
                let img = capture_screenshot()?;
                let (tree, elems) = ax_tree_for_pid(pid, 4)?;

                let png = if !elems.is_empty() {
                    let rendered = render_som_overlay(&img, &elems)?;
                    let mut buf = Vec::new();
                    rendered
                        .write_to(&mut std::io::Cursor::new(&mut buf), image::ImageFormat::Png)
                        .map_err(|e| format!("png encode: {e}"))?;
                    buf
                } else {
                    cgimage_to_png_bytes(&img)?
                };

                (Some(base64_encode(&png)), Some(tree), elems)
            }
        };

        Ok(CaptureResult {
            screenshot_b64,
            ax_tree,
            elements,
            app: app_info_for_pid(pid),
            screen: screen_info(),
        })
    }

    pub fn click_action(
        &self,
        input: &super::schema::ComputerUseInput,
    ) -> Result<ActionResult, String> {
        let pid = resolve_pid(input)?;
        let button = input.button.as_deref().unwrap_or("left");

        if let Some((x, y)) = input.coordinate {
            let (px, py) = if input.normalized.unwrap_or(false) {
                let screen = screen_info();
                (x * screen.width as f64, y * screen.height as f64)
            } else {
                (x, y)
            };
            click(pid, px, py, button)?;
            Ok(ActionResult {
                success: true,
                action: "click".into(),
                message: format!("Clicked at ({px:.0}, {py:.0})"),
                data: None,
            })
        } else if let Some(elem_id) = input.element {
            let (_, elements) = ax_tree_for_pid(pid, 4)?;
            let elem = elements
                .iter()
                .find(|e| e.id == elem_id)
                .ok_or_else(|| format!("Element {elem_id} not found"))?;
            let (bx, by, bw, bh) = elem.bounds;
            let cx = bx + bw / 2.0;
            let cy = by + bh / 2.0;
            click(pid, cx, cy, button)?;
            Ok(ActionResult {
                success: true,
                action: "click".into(),
                message: format!("Clicked element {elem_id} ({})", elem.role),
                data: None,
            })
        } else {
            Err("click requires 'element' or 'coordinate'".into())
        }
    }

    pub fn type_action(
        &self,
        input: &super::schema::ComputerUseInput,
    ) -> Result<ActionResult, String> {
        let pid = resolve_pid(input)?;
        let text = input.text.as_deref().ok_or("type requires 'text'")?;
        type_text(pid, text)?;
        Ok(ActionResult {
            success: true,
            action: "type".into(),
            message: format!("Typed {} chars", text.len()),
            data: None,
        })
    }

    pub fn key_action(
        &self,
        input: &super::schema::ComputerUseInput,
    ) -> Result<ActionResult, String> {
        let pid = resolve_pid(input)?;
        let keys = input.keys.as_deref().ok_or("key requires 'keys'")?;
        key_combo(pid, keys)?;
        Ok(ActionResult {
            success: true,
            action: "key".into(),
            message: format!("Pressed {keys}"),
            data: None,
        })
    }

    pub fn scroll_action(
        &self,
        input: &super::schema::ComputerUseInput,
    ) -> Result<ActionResult, String> {
        let pid = resolve_pid(input)?;
        let direction = input.direction.as_deref().unwrap_or("down");
        let amount = input.amount.unwrap_or(3);
        scroll(pid, direction, amount)?;
        Ok(ActionResult {
            success: true,
            action: "scroll".into(),
            message: format!("Scrolled {direction} × {amount}"),
            data: None,
        })
    }

    pub fn list_apps_action(&self) -> Result<ActionResult, String> {
        let apps = list_apps()?;
        Ok(ActionResult {
            success: true,
            action: "list_apps".into(),
            message: format!("{} running apps", apps.len()),
            data: Some(serde_json::to_value(&apps).map_err(|e| format!("serialize apps: {e}"))?),
        })
    }
}

fn resolve_pid(input: &super::schema::ComputerUseInput) -> Result<i32, String> {
    if let Some(ref app) = input.app {
        Ok(find_app_pid(app)?)
    } else {
        frontmost_pid()
    }
}
