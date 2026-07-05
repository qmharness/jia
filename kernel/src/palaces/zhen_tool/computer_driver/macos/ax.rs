//! AX accessibility tree traversal.
use super::*;

// ── AX safe wrappers ──
// All AX attribute access uses raw pointers; these helpers encapsulate the unsafety.

pub(crate) fn ax_attr_cftype(
    element: &AXUIElement,
    attr: &'static str,
) -> Option<CFRetained<CFType>> {
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

pub(crate) fn ax_attr_str(element: &AXUIElement, attr: &'static str) -> Option<String> {
    ax_attr_cftype(element, attr)
        .and_then(|v| v.downcast::<CFString>().ok())
        .map(|s| s.to_string())
}

pub(crate) fn ax_attr_bool(element: &AXUIElement, attr: &'static str) -> Option<bool> {
    ax_attr_cftype(element, attr)
        .and_then(|v| v.downcast::<CFBoolean>().ok())
        .map(|b| b.as_bool())
}

pub(crate) fn ax_attr_point(element: &AXUIElement, attr: &'static str) -> Option<(f64, f64)> {
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

pub(crate) fn ax_attr_children(element: &AXUIElement) -> Vec<CFRetained<AXUIElement>> {
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

pub(crate) fn ax_app_for_pid(pid: i32) -> CFRetained<AXUIElement> {
    // SAFETY: AXUIElement::new_application(pid) creates an accessibility
    // object for the process. pid is a valid process ID obtained from the
    // system. Returns a valid retained AXUIElement or null (handled by caller).
    unsafe { AXUIElement::new_application(pid) }
}

pub(crate) fn ax_tree_for_pid(
    pid: i32,
    max_depth: u32,
) -> Result<(String, Vec<SomElement>), String> {
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

pub(crate) fn render_ax_children(
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

pub(crate) fn render_ax_node(
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
