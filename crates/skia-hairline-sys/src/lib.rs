#[cfg(target_os = "macos")]
use std::ffi::c_void;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Kind {
    Horizontal,
    Rectangle,
    Vertical,
    AntiHorizontal,
    AntiHorizontalPairOpaque,
    AntiHorizontalPairAlpha,
    AntiVerticalPairOpaque,
    AntiVerticalPairAlpha,
    Mask,
}

#[cfg(target_os = "macos")]
impl Kind {
    fn from_code(code: u32) -> Option<Self> {
        Some(match code {
            0 => Self::Horizontal,
            1 => Self::Rectangle,
            2 => Self::Vertical,
            3 => Self::AntiHorizontal,
            4 => Self::AntiHorizontalPairOpaque,
            5 => Self::AntiHorizontalPairAlpha,
            6 => Self::AntiVerticalPairOpaque,
            7 => Self::AntiVerticalPairAlpha,
            8 => Self::Mask,
            _ => return None,
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Operation {
    pub kind: Kind,
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    pub first: u8,
    pub second: u8,
}

#[cfg(target_os = "macos")]
#[derive(Default)]
struct Collector {
    operations: Vec<Operation>,
    valid: bool,
}

#[cfg(target_os = "macos")]
type Record = extern "C" fn(
    context: *mut c_void,
    kind: u32,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    first: u8,
    second: u8,
);

#[cfg(target_os = "macos")]
unsafe extern "C" {
    fn skia_hairline_scan(
        path: *const c_void,
        matrix: *const f32,
        clip: *const i32,
        alpha: u8,
        record: Record,
        context: *mut c_void,
    ) -> bool;
}

/// Collect Skia's ordered legacy hairline operations without reading destination pixels.
///
/// # Safety
/// `path` must point to a live Skia `SkPath` built by the linked Skia revision.
#[cfg(target_os = "macos")]
pub unsafe fn scan(
    path: *const c_void,
    matrix: &[f32; 9],
    clip: [i32; 4],
    alpha: u8,
) -> Option<Vec<Operation>> {
    extern "C" fn record(
        context: *mut c_void,
        code: u32,
        x: i32,
        y: i32,
        width: i32,
        height: i32,
        first: u8,
        second: u8,
    ) {
        let collector = unsafe { &mut *context.cast::<Collector>() };
        let Some(kind) = Kind::from_code(code) else {
            collector.valid = false;
            return;
        };
        collector.operations.push(Operation {
            kind,
            x,
            y,
            width,
            height,
            first,
            second,
        });
    }

    let mut collector = Collector {
        valid: true,
        ..Default::default()
    };
    let ok = unsafe {
        skia_hairline_scan(
            path,
            matrix.as_ptr(),
            clip.as_ptr(),
            alpha,
            record,
            (&mut collector as *mut Collector).cast(),
        )
    };
    (ok && collector.valid).then_some(collector.operations)
}
