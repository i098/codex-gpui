use anyhow::{Context as _, Result, ensure};
use gpui::{Bounds, Global, Pixels, RenderImage, Size, point, px, size};
use image::{Delay, Frame, imageops};
use parking_lot::Mutex;
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{Arc, LazyLock},
};

/// `avatar-overlay-mascot-width-px`: default, minimum and maximum pet width.
pub const DEFAULT_WIDTH: f32 = 112.;
pub const MIN_WIDTH: f32 = 80.;
pub const MAX_WIDTH: f32 = 224.;
/// The Mini bar (`sn`) that stands in for the pet when `None` is chosen.
pub const BAR_SIZE: Size<Pixels> = size(px(120.), px(40.));
const IDLE_DELAYS: [u32; 6] = [1680, 660, 660, 840, 840, 1920];
type SpriteCache = HashMap<(String, u32, bool), Arc<RenderImage>>;
static SPRITES: LazyLock<Mutex<SpriteCache>> = LazyLock::new(Mutex::default);

/// The bundled pets (`HQ`), in the original's order: id, name, description.
pub const BUNDLED: [(&str, &str, &str); 9] = [
    ("codex", "Codex", "The original Codex companion."),
    (
        "dewey",
        "Dewey",
        "A calm companion for focused workspace days",
    ),
    (
        "fireball",
        "Fireball",
        "Hot path energy for fast iteration.",
    ),
    (
        "hoots",
        "Hoots",
        "A sharp-eyed owl for polished work in a blink.",
    ),
    ("rocky", "Rocky", "A steady rock when the diff gets large."),
    ("seedy", "Seedy", "Small green shoots for new ideas."),
    ("stacky", "Stacky", "A balanced stack for deep work."),
    ("bsod", "BSOD", "A tiny blue-screen gremlin."),
    ("null-signal", "Null Signal", "Quiet signal from the void."),
];

/// A pet from `$CODEX_HOME/pets/<dir>/pet.json` (or legacy `avatars/<dir>/avatar.json`).
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct CustomPet {
    /// `custom:<dir>`.
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub sheet: PathBuf,
    pub directory: PathBuf,
}

/// The host's custom pets (`customAvatars.load`).
#[derive(Default)]
pub struct CustomPets {
    pub pets: Vec<CustomPet>,
    pub loading: bool,
    pub failed: bool,
    /// `Create pet` is installing Hatch Pet.
    pub creating: bool,
}

impl Global for CustomPets {}

/// Whether the Mini window is open (`avatar-overlay-open-state-changed`).
#[derive(Default)]
pub struct MiniOpen(pub bool);

impl Global for MiniOpen {}

/// The pet box for a width: the sprite cell is 192×208.
pub fn pet_size(width: f32) -> Size<Pixels> {
    size(px(width), px((width * 208. / 192.).round()))
}

/// The stored width, clamped to the slider's range.
pub fn width(stored: Option<f64>) -> f32 {
    stored.map_or(DEFAULT_WIDTH, |w| (w as f32).clamp(MIN_WIDTH, MAX_WIDTH))
}

pub fn pet_bounds(viewport: Size<Pixels>, figure: Size<Pixels>) -> Bounds<Pixels> {
    Bounds::new(
        point(
            (viewport.width - figure.width - px(24.)).max(px(0.)),
            (viewport.height - figure.height - px(24.)).max(px(0.)),
        ),
        figure,
    )
}

pub fn composer_bounds(viewport: Size<Pixels>, figure: Size<Pixels>) -> Bounds<Pixels> {
    let pet = pet_bounds(viewport, figure);
    let width = px(330.).min((viewport.width - px(32.)).max(px(0.)));
    let left = (pet.center().x - width / 2.)
        .max(px(16.))
        .min(viewport.width - width - px(16.));
    Bounds::new(point(left, pet.center().y - px(20.)), size(width, px(40.)))
}

pub fn chat_bounds(viewport: Size<Pixels>, figure: Size<Pixels>) -> Bounds<Pixels> {
    let composer = composer_bounds(viewport, figure);
    Bounds::new(
        composer.origin - point(px(0.), px(48.)),
        size(composer.size.width, px(36.)),
    )
}

pub fn hit_bounds(
    viewport: Size<Pixels>,
    figure: Size<Pixels>,
    expanded: bool,
    has_chat: bool,
) -> impl Iterator<Item = Bounds<Pixels>> {
    [
        Some(if expanded {
            composer_bounds(viewport, figure)
        } else {
            pet_bounds(viewport, figure)
        }),
        has_chat.then(|| chat_bounds(viewport, figure)),
    ]
    .into_iter()
    .flatten()
}

/// The idle animation of pet `id` at `width` logical pixels. Custom ids resolve through
/// `custom`; a custom sheet is either version 1 (9 rows) or 2 (11 rows) of 192×208 cells.
pub fn pet(
    id: &str,
    custom: &[CustomPet],
    width: f32,
    scale: f32,
    reduced_motion: bool,
) -> Result<Arc<RenderImage>> {
    ensure!(
        scale.is_finite() && scale > 0. && width.is_finite() && width > 0.,
        "Invalid Mini display scale"
    );
    let pixels = (width * scale).round().max(1.) as u32;
    let key = (id.to_owned(), pixels, reduced_motion);
    if let Some(image) = SPRITES.lock().get(&key) {
        return Ok(image.clone());
    }
    let data: std::borrow::Cow<'static, [u8]> =
        if let Some((bundled, _, _)) = BUNDLED.iter().find(|(bundled, ..)| *bundled == id) {
            theme::Assets::get(&format!("mini/{bundled}.webp"))
                .context("Missing Mini pet artwork")?
                .data
        } else if let Some(pet) = custom.iter().find(|pet| pet.id == id) {
            std::fs::read(&pet.sheet)
                .with_context(|| format!("Cannot read {}", pet.sheet.display()))?
                .into()
        } else {
            anyhow::bail!("The selected Mini pet is unavailable: {id}")
        };
    let sheet = image::load_from_memory(&data)?.to_rgba8();
    ensure!(
        sheet.width() == 1536 && matches!(sheet.height(), 1872 | 2288),
        "Invalid Mini pet spritesheet dimensions"
    );
    let height = (pixels as f32 * 208. / 192.).round().max(1.) as u32;
    let count = if reduced_motion { 1 } else { IDLE_DELAYS.len() };
    let frames = IDLE_DELAYS
        .iter()
        .take(count)
        .enumerate()
        .map(|(column, delay)| {
            let frame = imageops::crop_imm(&sheet, column as u32 * 192, 0, 192, 208).to_image();
            let mut frame = imageops::resize(&frame, pixels, height, imageops::FilterType::Nearest);
            for pixel in frame.pixels_mut() {
                pixel.0.swap(0, 2);
            }
            Frame::from_parts(frame, 0, 0, Delay::from_numer_denom_ms(*delay, 1))
        });
    let image = Arc::new(RenderImage::new(frames.collect::<Vec<_>>()));
    SPRITES.lock().insert(key, image.clone());
    Ok(image)
}

/// Drops cached custom sprites so a reloaded sheet is read again.
pub fn forget_custom() {
    SPRITES
        .lock()
        .retain(|(id, ..), _| !id.starts_with("custom:"));
}

/// The collapsed Mini bar (`sn`): compose, voice and expand cells split by 1 px rules, on a
/// 40 px pill with the original's material fill, border and drop shadow.
pub fn bar(t: &theme::Theme) -> gpui::Div {
    use gpui::{ParentElement as _, Styled as _, div};
    let rule = || div().w(px(1.)).h(px(20.)).bg(t.border);
    let cell = |name: &'static str| {
        div()
            .size(px(18.))
            .flex()
            .items_center()
            .justify_center()
            .child(crate::icon::icon(name, 18., t.text))
    };
    div()
        .w(BAR_SIZE.width)
        .h(BAR_SIZE.height)
        .flex()
        .items_center()
        .justify_around()
        .px(px(8.))
        .rounded(px(20.))
        .bg(gpui::rgba(0x202020e6))
        .border_1()
        .border_color(gpui::rgba(0xffffff1f))
        .shadow(vec![gpui::BoxShadow {
            color: gpui::rgba(0x00000059).into(),
            offset: point(px(0.), px(3.)),
            blur_radius: px(16.),
            spread_radius: px(0.),
            inset: false,
        }])
        .child(cell("new-chat"))
        .child(rule())
        .child(cell("voice"))
        .child(rule())
        .child(cell("chevron-down"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mini_stays_inside_the_display_and_rejects_unknown_pets() {
        let viewport = size(px(356.), px(320.));
        let figure = pet_size(DEFAULT_WIDTH);
        let pet = pet_bounds(viewport, figure);
        assert_eq!(pet.bottom_right(), point(px(332.), px(296.)));
        let composer = composer_bounds(viewport, figure);
        assert!(composer.left() >= px(16.));
        assert!(composer.right() <= viewport.width - px(16.));
        assert_eq!(composer.center().y, pet.center().y);
        assert!(super::pet("custom:missing", &[], 112., 2., false).is_err());
        assert!(super::pet("codex", &[], 112., f32::NAN, false).is_err());
        let link = chat_bounds(viewport, figure).center();
        assert!(hit_bounds(viewport, figure, true, true).any(|b| b.contains(&link)));
        assert!(hit_bounds(viewport, figure, false, true).any(|b| b.contains(&link)));
        assert!(!hit_bounds(viewport, figure, true, false).any(|b| b.contains(&link)));
        let gap = point(link.x, chat_bounds(viewport, figure).bottom() + px(6.));
        assert!(!hit_bounds(viewport, figure, true, true).any(|b| b.contains(&gap)));
    }

    #[test]
    fn width_clamps_to_the_slider_range() {
        assert_eq!(width(None), 112.);
        assert_eq!(width(Some(40.)), 80.);
        assert_eq!(width(Some(500.)), 224.);
        assert_eq!(pet_size(112.), size(px(112.), px(121.)));
    }
}
