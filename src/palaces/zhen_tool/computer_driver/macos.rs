// ── computer/macos.rs — Native macOS desktop control backend ──
//
// Uses only public Apple frameworks via the objc2 crate family.
//
// objc2 v0.3 API pattern: CF types use associated functions with Option<&T>
// as the first parameter, e.g. CGEvent::post_to_pid(pid, Some(&event)).

use std::ptr::NonNull;

use objc2_app_kit::NSWorkspace;
use objc2_application_services::{AXError, AXUIElement, AXValue, AXValueType};
use objc2_core_foundation::{
    CFArray, CFBoolean, CFRetained, CFString, CFType, CGPoint, CGRect, CGSize,
};
#[allow(deprecated)]
use objc2_core_graphics::{
    CGDataProvider, CGDisplayBounds, CGDisplayPixelsWide, CGEvent, CGEventFlags, CGEventType,
    CGImage, CGMainDisplayID, CGMouseButton, CGScrollEventUnit, CGWindowImageOption,
    CGWindowListCreateImage, CGWindowListOption, kCGNullWindowID,
};

use super::ActionResult;
use super::AppInfo;
use super::CaptureResult;
use super::ScreenInfo;
use super::SomElement;

// SAFETY: FFI declaration for CFArrayGetValueAtIndex. This is a public
// CoreFoundation API. The function expects a valid CFArray pointer and an
// index within bounds [0, CFArrayGetCount(arr)). Callers guarantee these
// preconditions.
// Raw CF function for array access when type parameter is erased (Opaque).
unsafe extern "C" {
    fn CFArrayGetValueAtIndex(arr: *const std::ffi::c_void, idx: isize) -> *const std::ffi::c_void;
}

// ── AX safe wrappers ──
// All AX attribute access uses raw pointers; these helpers encapsulate the unsafety.

fn ax_attr_cftype(element: &AXUIElement, attr: &'static str) -> Option<CFRetained<CFType>> {
    let cf_str = CFString::from_static_str(attr);
    let mut ptr: *const CFType = std::ptr::null();
    // SAFETY: copy_attribute_value writes a retained CFType pointer.
    // We check AXError::Success and ptr.is_null() before constructing
    // CFRetained. NonNull::new_unchecked is safe because we verified
    // the pointer is non-null. The returned CFRetained takes ownership.
    unsafe {
        if element.copy_attribute_value(&cf_str, NonNull::from(&mut ptr)) == AXError::Success
            && !ptr.is_null()
        {
            Some(CFRetained::from_raw(NonNull::new_unchecked(
                ptr as *mut CFType,
            )))
        } else {
            None
        }
    }
}

fn ax_attr_str(element: &AXUIElement, attr: &'static str) -> Option<String> {
    ax_attr_cftype(element, attr)
        .and_then(|v| v.downcast::<CFString>().ok())
        .map(|s| s.to_string())
}

fn ax_attr_bool(element: &AXUIElement, attr: &'static str) -> Option<bool> {
    ax_attr_cftype(element, attr)
        .and_then(|v| v.downcast::<CFBoolean>().ok())
        .map(|b| b.as_bool())
}

fn ax_attr_point(element: &AXUIElement, attr: &'static str) -> Option<(f64, f64)> {
    let v = ax_attr_cftype(element, attr)?;
    let ax_val = v.downcast::<AXValue>().ok()?;
    // SAFETY: r#type() returns the AXValueType enum. The AXValue was
    // obtained via a successful downcast, guaranteeing it's a valid AXValue.
    match unsafe { ax_val.r#type() } {
        AXValueType::CGPoint => {
            let mut pt = CGPoint::ZERO;
            // SAFETY: value() writes to the provided buffer. We match the
            // AXValueType to the buffer type (CGPoint for CGPoint, CGSize
            // for CGSize). The buffer is stack-allocated with sufficient size.
            unsafe {
                if ax_val.value(AXValueType::CGPoint, NonNull::from(&mut pt).cast()) {
                    return Some((pt.x, pt.y));
                }
            }
            None
        }
        AXValueType::CGSize => {
            let mut sz = CGSize::ZERO;
            // SAFETY: See above — buffer type matches AXValueType.
            unsafe {
                if ax_val.value(AXValueType::CGSize, NonNull::from(&mut sz).cast()) {
                    return Some((sz.width, sz.height));
                }
            }
            None
        }
        _ => None,
    }
}

fn ax_attr_children(element: &AXUIElement) -> Vec<CFRetained<AXUIElement>> {
    let v = match ax_attr_cftype(element, "AXChildren") {
        Some(v) => v,
        None => return Vec::new(),
    };
    let arr = match v.downcast::<CFArray>() {
        Ok(a) => a,
        Err(_) => return Vec::new(),
    };

    let count = arr.len();
    let mut children = Vec::with_capacity(count);
    let arr_ptr = CFRetained::as_ptr(&arr).cast::<std::ffi::c_void>().as_ptr();
    for i in 0..count {
        // SAFETY: CFArrayGetValueAtIndex returns a pointer at index i.
        // i is in bounds [0, count) where count = CFArrayGetCount(arr).
        // The returned pointer is non-owned; we call CFRetain via
        // CFRetained::retain to take a reference.
        unsafe {
            let item_ptr = CFArrayGetValueAtIndex(arr_ptr, i as isize);
            if !item_ptr.is_null() {
                children.push(CFRetained::retain(NonNull::new_unchecked(
                    item_ptr as *mut AXUIElement,
                )));
            }
        }
    }
    children
}

// ── AX tree ──

fn ax_app_for_pid(pid: i32) -> CFRetained<AXUIElement> {
    // SAFETY: AXUIElement::new_application(pid) creates an accessibility
    // object for the process. pid is a valid process ID obtained from the
    // system. Returns a valid retained AXUIElement or null (handled by caller).
    unsafe { AXUIElement::new_application(pid) }
}

fn ax_tree_for_pid(pid: i32, max_depth: u32) -> Result<(String, Vec<SomElement>), String> {
    let app = ax_app_for_pid(pid);
    let mut text = String::new();
    let mut elements = Vec::new();
    let mut next_id: u32 = 0;

    // Try focused window first, fall back to app element
    let mut found_children = false;
    if let Some(focused_win) = ax_attr_cftype(&app, "AXFocusedWindow")
        && let Ok(win_element) = focused_win.downcast::<AXUIElement>()
    {
        render_ax_node(
            &win_element,
            0,
            max_depth,
            &mut text,
            &mut elements,
            &mut next_id,
        );
        found_children = true;
    }
    if !found_children
        && let Some(main_win) = ax_attr_cftype(&app, "AXMainWindow")
        && let Ok(win_element) = main_win.downcast::<AXUIElement>()
    {
        render_ax_node(
            &win_element,
            0,
            max_depth,
            &mut text,
            &mut elements,
            &mut next_id,
        );
        found_children = true;
    }
    if !found_children {
        // Render from app root
        text.push_str(&format!(
            "App: {}\n",
            ax_attr_str(&app, "AXTitle").unwrap_or_default()
        ));
        render_ax_children(&app, 1, max_depth, &mut text, &mut elements, &mut next_id);
    }

    Ok((text, elements))
}

/// Cap interactive elements to prevent explosion on complex UIs (e.g. large tables).
const MAX_ELEMENTS: usize = 200;

fn render_ax_children(
    element: &AXUIElement,
    depth: u32,
    max_depth: u32,
    text: &mut String,
    elements: &mut Vec<SomElement>,
    next_id: &mut u32,
) {
    if depth > max_depth || elements.len() >= MAX_ELEMENTS {
        return;
    }
    for child in &ax_attr_children(element) {
        if elements.len() >= MAX_ELEMENTS {
            break;
        }
        render_ax_node(child, depth, max_depth, text, elements, next_id);
    }
}

fn render_ax_node(
    element: &AXUIElement,
    depth: u32,
    max_depth: u32,
    text: &mut String,
    elements: &mut Vec<SomElement>,
    next_id: &mut u32,
) {
    if depth > max_depth {
        return;
    }

    let role = ax_attr_str(element, "AXRole").unwrap_or_else(|| "unknown".into());
    let title = ax_attr_str(element, "AXTitle").unwrap_or_default();
    let desc = ax_attr_str(element, "AXDescription").unwrap_or_default();
    let value = ax_attr_str(element, "AXValue").unwrap_or_default();
    let enabled = ax_attr_bool(element, "AXEnabled").unwrap_or(true);

    // Layout-noise nodes: don't emit a line, but recurse into children
    let skip = matches!(role.as_str(), "AXGroup" | "AXLayoutArea" | "AXLayoutItem");

    let interactive = matches!(
        role.as_str(),
        "AXButton"
            | "AXTextField"
            | "AXTextArea"
            | "AXCheckBox"
            | "AXRadioButton"
            | "AXPopUpButton"
            | "AXMenuButton"
            | "AXMenuItem"
            | "AXMenu"
            | "AXSlider"
            | "AXTabGroup"
            | "AXComboBox"
            | "AXScrollBar"
            | "AXToolbar"
            | "AXLink"
            | "AXImage"
            | "AXStaticText"
            | "AXHeading"
            | "AXCell"
            | "AXRow"
            | "AXTable"
            | "AXList"
            | "AXOutline"
            | "AXBrowser"
            | "AXColorWell"
            | "AXStepper"
            | "AXSegmentedControl"
            | "AXDisclosureTriangle"
            | "AXHandle"
            | "AXWindow"
            | "AXDrawer"
            | "AXSheet"
            | "AXGrowArea"
            | "AXSearchField"
    );

    if skip {
        render_ax_children(element, depth, max_depth, text, elements, next_id);
        return;
    }

    if interactive {
        let id = *next_id;
        *next_id += 1;
        let position = ax_attr_point(element, "AXPosition").unwrap_or((0.0, 0.0));
        let size = ax_attr_point(element, "AXSize").unwrap_or((0.0, 0.0));

        let indent = "  ".repeat(depth as usize);
        text.push_str(&format!("{indent}[{id}] {role}"));
        if !title.is_empty() {
            text.push_str(&format!(" \"{title}\""));
        }
        if !desc.is_empty() && desc != title {
            text.push_str(&format!(" ({desc})"));
        }
        if !value.is_empty() {
            text.push_str(&format!(" = \"{value}\""));
        }
        if !enabled {
            text.push_str(" (disabled)");
        }
        text.push('\n');

        elements.push(SomElement {
            id,
            role,
            label: if title.is_empty() { None } else { Some(title) },
            value: if value.is_empty() { None } else { Some(value) },
            bounds: (position.0, position.1, size.0, size.1),
            enabled,
        });
    } else {
        let indent = "  ".repeat(depth as usize);
        text.push_str(&format!("{indent}{role}"));
        if !title.is_empty() {
            text.push_str(&format!(" \"{title}\""));
        }
        if !value.is_empty() {
            text.push_str(&format!(" = \"{value}\""));
        }
        text.push('\n');
    }

    // Recurse children
    render_ax_children(element, depth + 1, max_depth, text, elements, next_id);
}

// ── Screen capture ──

fn screen_info() -> ScreenInfo {
    let main_id = CGMainDisplayID();
    let bounds = CGDisplayBounds(main_id);
    let pixels_wide = CGDisplayPixelsWide(main_id);
    let scale = if bounds.size.width > 0.0 {
        pixels_wide as f64 / bounds.size.width
    } else {
        return ScreenInfo {
            width: 0,
            height: 0,
            scale: 1.0,
        };
    };
    ScreenInfo {
        width: bounds.size.width as u32,
        height: bounds.size.height as u32,
        scale,
    }
}

#[allow(deprecated)]
fn capture_screenshot() -> Result<CFRetained<CGImage>, String> {
    let info = screen_info();
    let screen_bounds = CGRect::new(
        CGPoint::new(0.0, 0.0),
        CGSize::new(info.width as f64, info.height as f64),
    );
    let image = CGWindowListCreateImage(
        screen_bounds,
        CGWindowListOption::OptionOnScreenOnly,
        kCGNullWindowID,
        CGWindowImageOption::NominalResolution,
    );
    image.ok_or_else(|| {
        "CGWindowListCreateImage returned null \
         (is Accessibility permission granted in System Settings > \
         Privacy & Security > Accessibility?)"
            .into()
    })
}

fn cgimage_to_image_buffer(
    image: &CGImage,
) -> Result<image::ImageBuffer<image::Rgba<u8>, Vec<u8>>, String> {
    let width = CGImage::width(Some(image)) as u32;
    let height = CGImage::height(Some(image)) as u32;
    let bpc = CGImage::bits_per_component(Some(image));
    let bpp = CGImage::bits_per_pixel(Some(image));
    if bpc != 8 || bpp != 32 {
        return Err(format!("unsupported pixel format: {bpc}bpc {bpp}bpp"));
    }
    let provider = CGImage::data_provider(Some(image)).ok_or("no data provider")?;
    let cf_data = CGDataProvider::data(Some(&provider)).ok_or("no data")?;
    let raw = cf_data.to_vec();
    image::ImageBuffer::from_raw(width, height, raw)
        .ok_or("failed to construct image buffer".into())
}

fn cgimage_to_png_bytes(image: &CGImage) -> Result<Vec<u8>, String> {
    let img_buf = cgimage_to_image_buffer(image)?;
    let mut png = Vec::new();
    img_buf
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .map_err(|e| format!("png encode: {e}"))?;
    Ok(png)
}

// ── Base64 ──

fn base64_encode(data: &[u8]) -> String {
    base64::Engine::encode(&base64::engine::general_purpose::STANDARD, data)
}

// ── Event helpers ──

fn modifier_mask(name: &str) -> CGEventFlags {
    match name {
        "cmd" | "command" => CGEventFlags::MaskCommand,
        "shift" => CGEventFlags::MaskShift,
        "option" | "opt" | "alt" => CGEventFlags::MaskAlternate,
        "ctrl" | "control" => CGEventFlags::MaskControl,
        _ => CGEventFlags::empty(),
    }
}

fn virtual_keycode(name: &str) -> Option<u16> {
    match name.to_lowercase().as_str() {
        "return" | "enter" => Some(0x24),
        "space" => Some(0x31),
        "tab" => Some(0x30),
        "escape" | "esc" => Some(0x35),
        "delete" | "backspace" => Some(0x33),
        "forwarddelete" => Some(0x75),
        "left" | "arrowleft" => Some(0x7B),
        "right" | "arrowright" => Some(0x7C),
        "down" | "arrowdown" => Some(0x7D),
        "up" | "arrowup" => Some(0x7E),
        "home" => Some(0x73),
        "end" => Some(0x77),
        "pageup" => Some(0x74),
        "pagedown" => Some(0x79),
        "f1" => Some(0x7A),
        "f2" => Some(0x78),
        "f3" => Some(0x63),
        "f4" => Some(0x76),
        "f5" => Some(0x60),
        "f6" => Some(0x61),
        "f7" => Some(0x62),
        "f8" => Some(0x64),
        "f9" => Some(0x65),
        "f10" => Some(0x6D),
        "f11" => Some(0x67),
        "f12" => Some(0x6F),
        "a" => Some(0x00),
        "b" => Some(0x0B),
        "c" => Some(0x08),
        "d" => Some(0x02),
        "e" => Some(0x0E),
        "f" => Some(0x03),
        "g" => Some(0x05),
        "h" => Some(0x04),
        "i" => Some(0x22),
        "j" => Some(0x26),
        "k" => Some(0x28),
        "l" => Some(0x25),
        "m" => Some(0x2E),
        "n" => Some(0x2D),
        "o" => Some(0x1F),
        "p" => Some(0x23),
        "q" => Some(0x0C),
        "r" => Some(0x0F),
        "s" => Some(0x01),
        "t" => Some(0x11),
        "u" => Some(0x20),
        "v" => Some(0x09),
        "w" => Some(0x0D),
        "x" => Some(0x07),
        "y" => Some(0x10),
        "z" => Some(0x06),
        "0" => Some(0x1D),
        "1" => Some(0x12),
        "2" => Some(0x13),
        "3" => Some(0x14),
        "4" => Some(0x15),
        "5" => Some(0x17),
        "6" => Some(0x16),
        "7" => Some(0x1A),
        "8" => Some(0x1C),
        "9" => Some(0x19),
        "-" | "_" => Some(0x1B),
        "=" | "+" => Some(0x18),
        "[" | "{" => Some(0x21),
        "]" | "}" => Some(0x1E),
        "\\" | "|" => Some(0x2A),
        ";" | ":" => Some(0x29),
        "'" | "\"" => Some(0x27),
        "," | "<" => Some(0x2B),
        "." | ">" => Some(0x2F),
        "/" | "?" => Some(0x2C),
        "`" | "~" => Some(0x32),
        _ => None,
    }
}

// ── App discovery ──

fn frontmost_pid() -> Result<i32, String> {
    let ws = NSWorkspace::sharedWorkspace();
    ws.frontmostApplication()
        .ok_or("no frontmost app".into())
        .map(|a| a.processIdentifier())
}

fn find_app_pid(name_or_bundle: &str) -> Result<i32, String> {
    let ws = NSWorkspace::sharedWorkspace();
    let apps = ws.runningApplications();
    let lower = name_or_bundle.to_lowercase();

    for app in apps.iter() {
        let name = app
            .localizedName()
            .map(|n| n.to_string().to_lowercase())
            .unwrap_or_default();
        let bundle = app.bundleIdentifier().map(|b| b.to_string().to_lowercase());
        if name.contains(&lower) || bundle.as_deref() == Some(&lower) {
            return Ok(app.processIdentifier());
        }
    }
    Err(format!("App '{name_or_bundle}' not found"))
}

fn app_info_for_pid(pid: i32) -> AppInfo {
    let ws = NSWorkspace::sharedWorkspace();
    let apps = ws.runningApplications();
    let frontmost_pid = ws.frontmostApplication().map(|a| a.processIdentifier());

    for app in apps.iter() {
        if app.processIdentifier() == pid {
            return AppInfo {
                name: app
                    .localizedName()
                    .map(|n| n.to_string())
                    .unwrap_or_else(|| "Unknown".into()),
                bundle_id: app.bundleIdentifier().map(|b| b.to_string()),
                pid,
                is_frontmost: frontmost_pid == Some(pid),
            };
        }
    }
    AppInfo {
        name: "Unknown".into(),
        bundle_id: None,
        pid,
        is_frontmost: false,
    }
}

// ── Event dispatch ──

fn click(pid: i32, x: f64, y: f64, button: &str) -> Result<(), String> {
    let (down_type, up_type, btn) = match button {
        "right" => (
            CGEventType::RightMouseDown,
            CGEventType::RightMouseUp,
            CGMouseButton::Right,
        ),
        "center" => (
            CGEventType::OtherMouseDown,
            CGEventType::OtherMouseUp,
            CGMouseButton::Center,
        ),
        _ => (
            CGEventType::LeftMouseDown,
            CGEventType::LeftMouseUp,
            CGMouseButton::Left,
        ),
    };

    let pt = CGPoint::new(x, y);

    let down = CGEvent::new_mouse_event(None, down_type, pt, btn)
        .ok_or("failed to create mouse down event")?;
    CGEvent::post_to_pid(pid, Some(&down));
    std::thread::sleep(std::time::Duration::from_millis(50));

    let up = CGEvent::new_mouse_event(None, up_type, pt, btn)
        .ok_or("failed to create mouse up event")?;
    CGEvent::post_to_pid(pid, Some(&up));

    Ok(())
}

fn key_combo(pid: i32, keys: &str) -> Result<(), String> {
    let parts: Vec<&str> = keys.split('+').map(|s| s.trim()).collect();
    if parts.is_empty() {
        return Err("empty key combo".into());
    }

    let mut modifiers = CGEventFlags::empty();
    let mut main_key: Option<&str> = None;

    for part in &parts {
        let mask = modifier_mask(part);
        if !mask.is_empty() {
            modifiers |= mask;
        } else if main_key.is_some() {
            return Err(format!("Multiple main keys in combo: '{keys}'"));
        } else {
            main_key = Some(part);
        }
    }

    let main = main_key.ok_or_else(|| format!("No main key in combo: '{keys}'"))?;
    let vk = virtual_keycode(main).ok_or_else(|| format!("Unknown key: '{main}'"))?;

    let down =
        CGEvent::new_keyboard_event(None, vk, true).ok_or("failed to create keyboard event")?;
    if !modifiers.is_empty() {
        CGEvent::set_flags(Some(&down), modifiers);
    }
    CGEvent::post_to_pid(pid, Some(&down));
    std::thread::sleep(std::time::Duration::from_millis(30));

    let up =
        CGEvent::new_keyboard_event(None, vk, false).ok_or("failed to create keyboard event")?;
    if !modifiers.is_empty() {
        CGEvent::set_flags(Some(&up), modifiers);
    }
    CGEvent::post_to_pid(pid, Some(&up));

    Ok(())
}

fn type_text(pid: i32, text: &str) -> Result<(), String> {
    for ch in text.chars() {
        let down =
            CGEvent::new_keyboard_event(None, 0, true).ok_or("failed to create keyboard event")?;
        let mut buf = [0u16; 2];
        let encoded = ch.encode_utf16(&mut buf);
        // SAFETY: keyboard_set_unicode_string sets the unicode string
        // for a keyboard event. encoded.as_ptr() points to valid u16
        // data of encoded.len() length. The event was just created.
        unsafe {
            CGEvent::keyboard_set_unicode_string(
                Some(&down),
                encoded.len() as u64,
                encoded.as_ptr(),
            );
        }
        CGEvent::post_to_pid(pid, Some(&down));
        std::thread::sleep(std::time::Duration::from_millis(10));

        let up =
            CGEvent::new_keyboard_event(None, 0, false).ok_or("failed to create keyboard event")?;
        // SAFETY: Same as above — encoded data is still valid.
        unsafe {
            CGEvent::keyboard_set_unicode_string(Some(&up), encoded.len() as u64, encoded.as_ptr());
        }
        CGEvent::post_to_pid(pid, Some(&up));
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    Ok(())
}

fn scroll(pid: i32, direction: &str, amount: u32) -> Result<(), String> {
    let amt = amount as i32;
    let (wheel1, wheel2) = match direction {
        "up" => (0, amt),
        "down" => (0, -(amt)),
        "left" => (amt, 0),
        "right" => (-(amt), 0),
        _ => (0, -(amt)),
    };

    let event =
        CGEvent::new_scroll_wheel_event2(None, CGScrollEventUnit::Line, 2, wheel1, wheel2, 0)
            .ok_or("failed to create scroll event")?;

    CGEvent::post_to_pid(pid, Some(&event));
    Ok(())
}

// ── App listing ──

fn list_apps() -> Result<Vec<AppInfo>, String> {
    let ws = NSWorkspace::sharedWorkspace();
    let apps = ws.runningApplications();
    let frontmost_pid = ws.frontmostApplication().map(|a| a.processIdentifier());

    let mut result = Vec::new();
    for app in apps.iter() {
        let name = app
            .localizedName()
            .map(|n| n.to_string())
            .unwrap_or_default();
        if name.is_empty() {
            continue;
        }
        let pid = app.processIdentifier();
        result.push(AppInfo {
            name,
            bundle_id: app.bundleIdentifier().map(|b| b.to_string()),
            pid,
            is_frontmost: frontmost_pid == Some(pid),
        });
    }
    Ok(result)
}

// ── SOM overlay ──

fn render_som_overlay(
    cg_image: &CGImage,
    elements: &[SomElement],
) -> Result<image::DynamicImage, String> {
    let mut img_buf = cgimage_to_image_buffer(cg_image)?;

    use image::Rgba;
    use imageproc::drawing::draw_hollow_rect_mut;
    use imageproc::rect::Rect;

    let red = Rgba([255u8, 40, 40, 255]);

    for elem in elements {
        let (x, y, w, h) = elem.bounds;
        if w <= 0.0 || h <= 0.0 || x < 0.0 || y < 0.0 {
            continue;
        }
        let rect = Rect::at(x as i32, y as i32).of_size(w as u32, h as u32);
        draw_hollow_rect_mut(&mut img_buf, rect, red);
    }

    Ok(image::DynamicImage::ImageRgba8(img_buf))
}

// ── Public backend ──

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
