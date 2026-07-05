//! Screen capture and image encoding.
use super::*;

// ── Screen capture ──

pub(crate) fn screen_info() -> ScreenInfo {
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
pub(crate) fn capture_screenshot() -> Result<CFRetained<CGImage>, String> {
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

pub(crate) fn cgimage_to_image_buffer(
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

pub(crate) fn cgimage_to_png_bytes(image: &CGImage) -> Result<Vec<u8>, String> {
    let img_buf = cgimage_to_image_buffer(image)?;
    let mut png = Vec::new();
    img_buf
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .map_err(|e| format!("png encode: {e}"))?;
    Ok(png)
}

// ── Base64 ──

pub(crate) fn base64_encode(data: &[u8]) -> String {
    base64::Engine::encode(&base64::engine::general_purpose::STANDARD, data)
}
