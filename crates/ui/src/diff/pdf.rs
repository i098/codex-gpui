#[cfg(target_os = "macos")]
pub(super) fn render(
    bytes: &[u8],
    page: usize,
    width: u32,
    scale: f32,
) -> anyhow::Result<(image::RgbaImage, usize)> {
    anyhow::ensure!(width > 0 && !bytes.is_empty(), "Empty PDF preview");
    match pdfjs::render(bytes, page, width, scale) {
        Some(rendered) => Ok(rendered),
        None => quartz(bytes, page, width),
    }
}

/// The page as Quartz draws it: content PDF.js paints differently keeps its edges apart.
#[cfg(target_os = "macos")]
fn quartz(bytes: &[u8], page: usize, width: u32) -> anyhow::Result<(image::RgbaImage, usize)> {
    use anyhow::{Context as _, ensure};
    use objc2::AnyThread;
    use objc2_app_kit::{
        NSBitmapFormat, NSBitmapImageRep, NSDeviceRGBColorSpace, NSGraphicsContext, NSPDFImageRep,
    };
    use objc2_foundation::{NSData, NSPoint, NSRect, NSSize};
    use std::{ffi::c_void, ptr::NonNull};

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
    _scale: f32,
) -> anyhow::Result<(image::RgbaImage, usize)> {
    anyhow::bail!("PDF previews require macOS")
}

/// The page as react-pdf's canvas in the Codex renderer paints it with PDF.js 5.4.296, for
/// pages that only place JPEG images (`q`, `Q`, `cm`, `Do`). Chromium's `ImageDecoder`
/// returns such a JPEG as a YUV frame (libjpeg raw data); the canvas converts it with
/// libyuv's JPEG matrix (nearest chroma) and `drawImageAtIntegerCoords` draws it on the
/// white page canvas.
/// ponytail: other operators and image kinds, rotation, more than 2x downscaling and a canvas
/// the browser would resample take Quartz; port each when a recorded page needs it.
#[cfg(target_os = "macos")]
mod pdfjs {
    use objc2_core_foundation::{CFData, CGRect};
    use objc2_core_graphics::{
        CGDataProvider, CGPDFArray, CGPDFArrayRef, CGPDFBoolean, CGPDFBox, CGPDFDataFormat,
        CGPDFDictionary, CGPDFDictionaryRef, CGPDFDocument, CGPDFInteger, CGPDFObjectRef,
        CGPDFPage, CGPDFStream, CGPDFStreamRef,
    };
    use std::{
        ffi::{CStr, CString, c_char},
        ptr::{NonNull, null_mut},
    };

    type Matrix = [f64; 6];

    pub(super) fn render(
        bytes: &[u8],
        page: usize,
        width: u32,
        scale: f32,
    ) -> Option<(image::RgbaImage, usize)> {
        let data = CFData::from_bytes(bytes);
        let provider = CGDataProvider::with_cf_data(Some(&data))?;
        let document = CGPDFDocument::with_provider(Some(&provider))?;
        let pages = CGPDFDocument::number_of_pages(Some(&document));
        if page == 0 || page > pages || !CGPDFDocument::is_unlocked(Some(&document)) {
            return None;
        }
        let pdf_page = CGPDFDocument::page(Some(&document), page)?;
        if CGPDFPage::rotation_angle(Some(&pdf_page)) % 360 != 0 {
            return None;
        }
        // PDF.js's `view`: the crop box inside the media box, as Quartz returns it.
        let view = CGPDFPage::box_rect(Some(&pdf_page), CGPDFBox::CropBox);
        let ratio = f64::from(scale);
        let (viewport, height) = viewport(view, width, ratio)?;
        let dict = CGPDFPage::dictionary(Some(&pdf_page));
        let content = unsafe { contents(dict) }?;
        let draws = unsafe { paint_ops(&content, resources(dict), viewport, ratio) }?;
        let pixels = shapes::canvas_images(width, height, &draws);
        Some((pixels, pages))
    }

    /// The canvas viewport transform and canvas height for a page canvas `width` device px
    /// wide. react-pdf: `width` is floor(contentRect.width) CSS px, the viewport scale is
    /// width / view width * devicePixelRatio and the canvas is sized by truncation. `None`
    /// when that backing size differs from the floor(1x viewport) CSS px canvas element,
    /// which the compositor would resample.
    fn viewport(view: CGRect, width: u32, ratio: f64) -> Option<(Matrix, u32)> {
        let (x0, y0) = (view.origin.x, view.origin.y);
        let (x1, y1) = (x0 + view.size.width, y0 + view.size.height);
        let css = f64::from(width) / ratio;
        let s = css / (x1 - x0) * ratio;
        let canvas = [(x1 - x0) * s, (y1 - y0) * s].map(f64::trunc);
        let shown = [(x1 - x0), (y1 - y0)].map(|side| (side * (css / (x1 - x0))).floor() * ratio);
        if canvas != shown || canvas[0] != f64::from(width) || !(1. ..=16_384.).contains(&canvas[1])
        {
            return None;
        }
        let (cx, cy) = ((x1 + x0) / 2., (y1 + y0) / 2.);
        let viewport = [
            s,
            0.,
            0.,
            -s,
            (cx - x0).abs() * s - s * cx,
            (cy - y0).abs() * s + s * cy,
        ];
        Some((viewport, canvas[1] as u32))
    }

    /// The page's image paints in content order; `None` for any other operator.
    unsafe fn paint_ops(
        content: &[u8],
        resources: CGPDFDictionaryRef,
        viewport: Matrix,
        ratio: f64,
    ) -> Option<Vec<shapes::CanvasDraw>> {
        let mut draws = Vec::new();
        let (mut ctm, mut saved) = (viewport, Vec::new());
        let mut operands = Vec::new();
        for token in Tokens(content) {
            let op = match token? {
                Token::Operator(op) => op,
                operand => {
                    operands.push(operand);
                    continue;
                }
            };
            match (op, operands.as_slice()) {
                (b"q", []) => saved.push(ctm),
                (b"Q", []) => ctm = saved.pop().unwrap_or(ctm),
                (b"cm", m) if m.len() == 6 => {
                    let mut v = [0.; 6];
                    for (v, token) in v.iter_mut().zip(m) {
                        let Token::Number(n) = token else { return None };
                        *v = *n;
                    }
                    ctm = concat(ctm, v);
                }
                (b"Do", [Token::Name(name)]) => {
                    draws.push(unsafe { paint_image(resources, name, ctm, ratio) }?);
                }
                _ => return None,
            }
            operands.clear();
        }
        Some(draws)
    }

    /// Blink's `AffineTransform` product `m * t` (canvas `transform()`).
    fn concat([a, b, c, d, e, f]: Matrix, [ta, tb, tc, td, te, tf]: Matrix) -> Matrix {
        [
            ta * a + tb * c,
            ta * b + tb * d,
            tc * a + td * c,
            tc * b + td * d,
            te * a + tf * c + e,
            te * b + tf * d + f,
        ]
    }

    /// PDF.js `paintImageXObject` of the image `name` under `ctm`: its `drawImage` call on the
    /// page canvas.
    unsafe fn paint_image(
        resources: CGPDFDictionaryRef,
        name: &CStr,
        ctm: Matrix,
        ratio: f64,
    ) -> Option<shapes::CanvasDraw> {
        let xobjects = unsafe { dictionary(resources, c"XObject") }?;
        let mut stream: CGPDFStreamRef = null_mut();
        if !unsafe { CGPDFDictionary::stream(xobjects, key(name), &mut stream) } {
            return None;
        }
        let dict = unsafe { CGPDFStream::dictionary(stream) };
        // Only PDF.js's `ImageDecoder` route: 8-bit DeviceRGB without masks or decode maps.
        let plain = unsafe {
            name_is(dict, c"Subtype", c"Image")
                && name_is(dict, c"ColorSpace", c"DeviceRGB")
                && integer(dict, c"BitsPerComponent") == Some(8)
                && [c"SMask", c"Mask", c"Decode", c"DecodeParms", c"ImageMask"]
                    .iter()
                    .all(|k| !has(dict, k))
        };
        let size = unsafe { integer(dict, c"Width").zip(integer(dict, c"Height")) };
        let (w, h) = size.filter(|&(w, h)| plain && w > 0 && h > 0)?;
        let interpolate = unsafe { boolean(dict, c"Interpolate") };
        let mut format = CGPDFDataFormat::Raw;
        let jpeg = unsafe { CGPDFStream::data(stream, &mut format) }?;
        if format != CGPDFDataFormat::JPEGEncoded {
            return None;
        }
        let (w, h) = (w as f64, h as f64);
        let image = super::jpeg::yuv_frame(&jpeg.to_vec(), w as usize, h as usize)?;
        let [a, b, c, d, e, f] = concat(ctm, [1. / w, 0., 0., -1. / h, 0., 0.]);
        // `_scaleImage` halves images first below 1/2 scale; flips and skews take other paths.
        if b != 0. || c != 0. || a < 0.5 || d < 0.5 {
            return None;
        }
        // `getImageSmoothingEnabled`: Float32Array singular values vs fround(dpr * 96 / 72).
        let limit = (ratio * (96. / 72.)) as f32;
        let smooth = interpolate || (a as f32 <= limit && d as f32 <= limit);
        // `drawImageAtIntegerCoords(ctx, img, 0, 0, w, h, 0, -h, w, h)`, `Math.round` corners.
        let round = |v: f64| (v + 0.5).floor();
        let (left, top) = (round(0. * a + e), round(-h * d + f));
        let size = |far: f64, near: f64| ((round(far) - near).abs() as i32).max(1);
        let rect = [
            left as i32,
            top as i32,
            size(w * a + e, left),
            size((-h + h) * d + f, top),
        ];
        Some(shapes::CanvasDraw {
            image,
            rect,
            smooth,
        })
    }

    fn key(k: &CStr) -> NonNull<c_char> {
        NonNull::from(k).cast()
    }

    unsafe fn has(dict: CGPDFDictionaryRef, k: &CStr) -> bool {
        let mut object: CGPDFObjectRef = null_mut();
        unsafe { CGPDFDictionary::object(dict, key(k), &mut object) }
    }

    unsafe fn integer(dict: CGPDFDictionaryRef, k: &CStr) -> Option<CGPDFInteger> {
        let mut v = 0;
        unsafe { CGPDFDictionary::integer(dict, key(k), &mut v) }.then_some(v)
    }

    unsafe fn boolean(dict: CGPDFDictionaryRef, k: &CStr) -> bool {
        let mut v: CGPDFBoolean = 0;
        let found = unsafe { CGPDFDictionary::boolean(dict, key(k), &mut v) };
        found && v != 0
    }

    unsafe fn name_is(dict: CGPDFDictionaryRef, k: &CStr, expected: &CStr) -> bool {
        let mut v: *const c_char = std::ptr::null();
        unsafe { CGPDFDictionary::name(dict, key(k), &mut v) && CStr::from_ptr(v) == expected }
    }

    unsafe fn dictionary(dict: CGPDFDictionaryRef, k: &CStr) -> Option<CGPDFDictionaryRef> {
        let mut v: CGPDFDictionaryRef = null_mut();
        unsafe { CGPDFDictionary::dictionary(dict, key(k), &mut v) }.then_some(v)
    }

    /// The page's `Resources`, inherited from its page tree parents when absent.
    unsafe fn resources(mut dict: CGPDFDictionaryRef) -> CGPDFDictionaryRef {
        for _ in 0..64 {
            if let Some(resources) = unsafe { dictionary(dict, c"Resources") } {
                return resources;
            }
            match unsafe { dictionary(dict, c"Parent") } {
                Some(parent) => dict = parent,
                None => break,
            }
        }
        null_mut()
    }

    /// The decoded `Contents` stream, or its array's streams joined by line breaks.
    unsafe fn contents(dict: CGPDFDictionaryRef) -> Option<Vec<u8>> {
        let read = |stream: CGPDFStreamRef| {
            let mut format = CGPDFDataFormat::Raw;
            let data = unsafe { CGPDFStream::data(stream, &mut format) }?;
            (format == CGPDFDataFormat::Raw).then(|| data.to_vec())
        };
        let mut stream: CGPDFStreamRef = null_mut();
        if unsafe { CGPDFDictionary::stream(dict, key(c"Contents"), &mut stream) } {
            return read(stream);
        }
        let mut array: CGPDFArrayRef = null_mut();
        if !unsafe { CGPDFDictionary::array(dict, key(c"Contents"), &mut array) } {
            return (!unsafe { has(dict, c"Contents") }).then(Vec::new);
        }
        let mut content = Vec::new();
        for index in 0..unsafe { CGPDFArray::count(array) } {
            let mut stream: CGPDFStreamRef = null_mut();
            if !unsafe { CGPDFArray::stream(array, index, &mut stream) } {
                return None;
            }
            content.extend(read(stream)?);
            content.push(b'\n');
        }
        Some(content)
    }

    enum Token<'a> {
        Number(f64),
        Name(CString),
        Operator(&'a [u8]),
    }

    /// Content stream tokens; `None` for strings, arrays, dictionaries and inline images.
    struct Tokens<'a>(&'a [u8]);

    impl<'a> Iterator for Tokens<'a> {
        type Item = Option<Token<'a>>;

        fn next(&mut self) -> Option<Self::Item> {
            let white = |b: &u8| b"\0\t\n\x0c\r ".contains(b);
            let delimiter = |b: &u8| white(b) || b"()<>[]{}/%".contains(b);
            loop {
                let start = self.0.iter().position(|b| !white(b))?;
                self.0 = &self.0[start..];
                if self.0[0] != b'%' {
                    break;
                }
                let end = self.0.iter().position(|b| b"\r\n".contains(b));
                self.0 = &self.0[end.unwrap_or(self.0.len())..];
            }
            let name = self.0[0] == b'/';
            let body = &self.0[usize::from(name)..];
            let end = body.iter().position(delimiter).unwrap_or(body.len());
            let (word, rest) = body.split_at(end);
            self.0 = rest;
            if name {
                return Some(decode_name(word).map(Token::Name));
            }
            if word.is_empty() {
                return Some(None);
            }
            if word[0].is_ascii_digit() || b"+-.".contains(&word[0]) {
                let number = std::str::from_utf8(word).ok().and_then(|n| n.parse().ok());
                return Some(number.map(Token::Number));
            }
            Some(Some(Token::Operator(word)))
        }
    }

    /// A name token's bytes with `#xx` escapes resolved.
    fn decode_name(word: &[u8]) -> Option<CString> {
        let mut out = Vec::with_capacity(word.len());
        let mut bytes = word.iter();
        while let Some(&b) = bytes.next() {
            if b != b'#' {
                out.push(b);
                continue;
            }
            let hex = [*bytes.next()?, *bytes.next()?];
            out.push(u8::from_str_radix(std::str::from_utf8(&hex).ok()?, 16).ok()?);
        }
        CString::new(out).ok()
    }
}

/// Chromium's `ImageDecoder` YUV frame of a JPEG as its 2D canvas draws it.
#[cfg(target_os = "macos")]
mod jpeg {
    use mozjpeg::{ColorSpace, CompInfoExt as _, Decompress};

    /// libjpeg's raw planes (ISLOW IDCT, no upsampling) of an 8-bit YCbCr JPEG in 4:4:4,
    /// 4:2:2 or 4:2:0 (the subsamplings Blink decodes to I444/I422/I420), converted like
    /// `PaintCanvasVideoRenderer` with libyuv's JPEG matrix and nearest chroma. The pixels
    /// are opaque RGBA. `None` for other JPEGs, which Blink decodes to RGB, and for errors.
    pub(super) fn yuv_frame(jpeg: &[u8], width: usize, height: usize) -> Option<image::RgbaImage> {
        // mozjpeg reports libjpeg errors by unwinding without the panic hook.
        std::panic::catch_unwind(|| decode(jpeg, width, height))
            .ok()
            .flatten()
    }

    fn decode(jpeg: &[u8], width: usize, height: usize) -> Option<image::RgbaImage> {
        let decoder = Decompress::new_mem(jpeg).ok()?;
        let sampling: Vec<_> = decoder.components().iter().map(|c| c.sampling()).collect();
        let shift = match sampling.as_slice() {
            [(1, 1), (1, 1), (1, 1)] => (0, 0),
            [(2, 1), (1, 1), (1, 1)] => (1, 0),
            [(2, 2), (1, 1), (1, 1)] => (1, 1),
            _ => return None,
        };
        if decoder.color_space() != ColorSpace::JCS_YCbCr || decoder.size() != (width, height) {
            return None;
        }
        let mut started = decoder.raw().ok()?;
        let strides: Vec<_> = started
            .components()
            .iter()
            .map(|c| c.row_stride())
            .collect();
        let mut planes = [Vec::new(), Vec::new(), Vec::new()];
        started.read_raw_data(&mut planes.each_mut());
        started.finish().ok()?;
        let [y, u, v] = &planes;
        let mut out = image::RgbaImage::new(width as u32, height as u32);
        for (row, line) in out.rows_mut().enumerate() {
            let (luma, chroma) = (row * strides[0], (row >> shift.1) * strides[1]);
            for (col, pixel) in line.enumerate() {
                let c = chroma + (col >> shift.0);
                pixel.0 = rgb(y[luma + col], u[c], v[c]);
            }
        }
        Some(out)
    }

    /// libyuv's `YuvPixel` with `kYuvJPEGConstants`.
    fn rgb(y: u8, u: u8, v: u8) -> [u8; 4] {
        const YG: u32 = 16320;
        const YB: i32 = 32;
        const UB: i32 = 113;
        const UG: i32 = 22;
        const VG: i32 = 46;
        const VR: i32 = 90;
        let y1 = ((u32::from(y) * 0x0101 * YG) >> 16) as i32;
        let (u, v) = (i32::from(u), i32::from(v));
        let channel = |c: i32| (c >> 6).clamp(0, 255) as u8;
        [
            channel(y1 + v * VR - (VR * 128 - YB)),
            channel(y1 + UG * 128 + VG * 128 + YB - (u * UG + v * VG)),
            channel(y1 + u * UB - (UB * 128 - YB)),
            255,
        ]
    }
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    /// The recorded multipage PDF's first page against the original app's PDF.js canvas,
    /// outside the corner squares its rounded page clip covers.
    #[test]
    fn jpeg_pages_match_the_original_pdfjs_canvas() {
        let pdf = include_bytes!("../../../../fixtures/media/guide.pdf");
        let original = include_bytes!("../../../../fixtures/media/guide-page-1-pdfjs.png");
        let original = image::load_from_memory(original).unwrap().to_rgb8();
        let (page, pages) = super::render(pdf, 1, 314, 2.).unwrap();
        assert_eq!(pages, 3);
        assert_eq!(page.dimensions(), original.dimensions());
        let (w, h) = page.dimensions();
        let corner = |v: u32, side: u32| v < 16 || v >= side - 16;
        let differing = page
            .enumerate_pixels()
            .filter(|(x, y, _)| !(corner(*x, w) && corner(*y, h)))
            .filter(|(x, y, p)| p.0[..3] != original.get_pixel(*x, *y).0)
            .count();
        assert_eq!(differing, 0);
    }
}
