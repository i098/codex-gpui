#[cfg(target_os = "macos")]
pub(super) fn render(
    bytes: &[u8],
    page: usize,
    width: u32,
) -> anyhow::Result<(image::RgbaImage, usize)> {
    use anyhow::{Context as _, ensure};
    use objc2::AnyThread;
    use objc2_app_kit::{
        NSBitmapFormat, NSBitmapImageRep, NSDeviceRGBColorSpace, NSGraphicsContext, NSPDFImageRep,
    };
    use objc2_foundation::{NSData, NSPoint, NSRect, NSSize};
    use std::{ffi::c_void, ptr::NonNull};

    ensure!(width > 0 && !bytes.is_empty(), "Empty PDF preview");
    // NSData borrows the bytes only while the page and its representation exist below.
    let data = unsafe {
        NSData::initWithBytesNoCopy_length_freeWhenDone(
            NSData::alloc(),
            NonNull::new(bytes.as_ptr() as *mut c_void).context("Empty PDF")?,
            bytes.len(),
            false,
        )
    };
    let pdf = NSPDFImageRep::imageRepWithData(&data).context("Invalid PDF")?;
    let pages = usize::try_from(pdf.pageCount()).context("Invalid PDF page count")?;
    ensure!(page > 0 && page <= pages, "PDF page is out of range");
    pdf.setCurrentPage((page - 1) as isize);
    let bounds = pdf.bounds();
    ensure!(
        bounds.size.width.is_finite()
            && bounds.size.width > 0.
            && bounds.size.height.is_finite()
            && bounds.size.height > 0.,
        "Invalid PDF page size"
    );
    let height = (width as f64 * bounds.size.height / bounds.size.width).ceil();
    ensure!(
        height >= 1. && height <= u32::MAX as f64 && u64::from(width) * height as u64 <= 16_777_216,
        "PDF page is too large to render"
    );
    let height = height as u32;
    let mut pixels = image::RgbaImage::from_pixel(width, height, image::Rgba([255, 255, 255, 255]));
    let mut planes = [
        pixels.as_mut_ptr(),
        std::ptr::null_mut(),
        std::ptr::null_mut(),
        std::ptr::null_mut(),
        std::ptr::null_mut(),
    ];
    // The bitmap borrows pixels; drop it before moving the buffer out of this function.
    let bitmap = unsafe {
        NSBitmapImageRep::initWithBitmapDataPlanes_pixelsWide_pixelsHigh_bitsPerSample_samplesPerPixel_hasAlpha_isPlanar_colorSpaceName_bitmapFormat_bytesPerRow_bitsPerPixel(
            NSBitmapImageRep::alloc(), planes.as_mut_ptr(), width as isize, height as isize,
            8, 4, true, false, NSDeviceRGBColorSpace, NSBitmapFormat::empty(),
            width as isize * 4, 32,
        )
    }.context("Cannot allocate PDF bitmap")?;
    let context = NSGraphicsContext::graphicsContextWithBitmapImageRep(&bitmap)
        .context("Cannot create PDF graphics context")?;
    let previous = NSGraphicsContext::currentContext();
    NSGraphicsContext::setCurrentContext(Some(&context));
    let drawn = pdf.drawInRect(NSRect::new(
        NSPoint::new(0., 0.),
        NSSize::new(width as f64, height as f64),
    ));
    NSGraphicsContext::setCurrentContext(previous.as_deref());
    drop(context);
    drop(bitmap);
    ensure!(drawn, "Cannot render PDF page");
    Ok((pixels, pages))
}

#[cfg(not(target_os = "macos"))]
pub(super) fn render(
    _bytes: &[u8],
    _page: usize,
    _width: u32,
) -> anyhow::Result<(image::RgbaImage, usize)> {
    anyhow::bail!("PDF previews require macOS")
}
