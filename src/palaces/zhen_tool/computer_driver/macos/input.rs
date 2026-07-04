//! Input event dispatch (click, key, type, scroll) + app discovery.
use super::*;

// ── Event helpers ──

pub(crate) fn modifier_mask(name: &str) -> CGEventFlags {
    match name {
        "cmd" | "command" => CGEventFlags::MaskCommand,
        "shift" => CGEventFlags::MaskShift,
        "option" | "opt" | "alt" => CGEventFlags::MaskAlternate,
        "ctrl" | "control" => CGEventFlags::MaskControl,
        _ => CGEventFlags::empty(),
    }
}

pub(crate) fn virtual_keycode(name: &str) -> Option<u16> {
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

pub(crate) fn frontmost_pid() -> Result<i32, String> {
    let ws = NSWorkspace::sharedWorkspace();
    ws.frontmostApplication()
        .ok_or("no frontmost app".into())
        .map(|a| a.processIdentifier())
}

pub(crate) fn find_app_pid(name_or_bundle: &str) -> Result<i32, String> {
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

pub(crate) fn app_info_for_pid(pid: i32) -> AppInfo {
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

pub(crate) fn click(pid: i32, x: f64, y: f64, button: &str) -> Result<(), String> {
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

pub(crate) fn key_combo(pid: i32, keys: &str) -> Result<(), String> {
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

pub(crate) fn type_text(pid: i32, text: &str) -> Result<(), String> {
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

pub(crate) fn scroll(pid: i32, direction: &str, amount: u32) -> Result<(), String> {
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

pub(crate) fn list_apps() -> Result<Vec<AppInfo>, String> {
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

pub(crate) fn render_som_overlay(
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
