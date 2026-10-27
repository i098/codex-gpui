use anyhow::{Context as _, Result, ensure};
use gpui::{Bounds, Pixels, RenderImage, Size, point, px, size};
use image::{Delay, Frame, imageops};
use parking_lot::Mutex;
use std::{
    collections::HashMap,
    sync::{Arc, LazyLock},
};

pub const PET_SIZE: Size<Pixels> = size(px(112.), px(121.));
const IDLE_DELAYS: [u32; 6] = [1680, 660, 660, 840, 840, 1920];
type SpriteCache = HashMap<(&'static str, u32, bool), Arc<RenderImage>>;
static SPRITES: LazyLock<Mutex<SpriteCache>> = LazyLock::new(Mutex::default);

pub fn pet_bounds(viewport: Size<Pixels>) -> Bounds<Pixels> {
    Bounds::new(
        point(
            (viewport.width - PET_SIZE.width - px(24.)).max(px(0.)),
            (viewport.height - PET_SIZE.height - px(24.)).max(px(0.)),
        ),
        PET_SIZE,
    )
}

pub fn composer_bounds(viewport: Size<Pixels>) -> Bounds<Pixels> {
    let pet = pet_bounds(viewport);
    let width = px(330.).min((viewport.width - px(32.)).max(px(0.)));
    let left = (pet.center().x - width / 2.)
        .max(px(16.))
        .min(viewport.width - width - px(16.));
    Bounds::new(point(left, pet.center().y - px(20.)), size(width, px(40.)))
}

pub fn chat_bounds(viewport: Size<Pixels>) -> Bounds<Pixels> {
    let composer = composer_bounds(viewport);
    Bounds::new(
        composer.origin - point(px(0.), px(48.)),
        size(composer.size.width, px(36.)),
    )
}

pub fn hit_bounds(
    viewport: Size<Pixels>,
    expanded: bool,
    has_chat: bool,
) -> impl Iterator<Item = Bounds<Pixels>> {
    [
        Some(if expanded {
            composer_bounds(viewport)
        } else {
            pet_bounds(viewport)
        }),
        has_chat.then(|| chat_bounds(viewport)),
    ]
    .into_iter()
    .flatten()
}

pub fn pet(id: &str, scale: f32, reduced_motion: bool) -> Result<Arc<RenderImage>> {
    let id = match id {
        "codex" => "codex",
        "dewey" => "dewey",
        "fireball" => "fireball",
        "hoots" => "hoots",
        "null-signal" => "null-signal",
        "rocky" => "rocky",
        "seedy" => "seedy",
        "stacky" => "stacky",
        "bsod" => "bsod",
        _ => anyhow::bail!("The selected Mini pet is unavailable: {id}"),
    };
    ensure!(
        scale.is_finite() && scale > 0.,
        "Invalid Mini display scale"
    );
    let width = (112. * scale).round().max(1.) as u32;
    let key = (id, width, reduced_motion);
    let mut cache = SPRITES.lock();
    if let Some(image) = cache.get(&key) {
        return Ok(image.clone());
    }
    let asset =
        theme::Assets::get(&format!("mini/{id}.webp")).context("Missing Mini pet artwork")?;
    let sheet = image::load_from_memory(&asset.data)?.to_rgba8();
    ensure!(
        sheet.width() == 1536 && sheet.height() == 2288,
        "Invalid Mini pet spritesheet dimensions"
    );
    let height = (121. * scale).round().max(1.) as u32;
    let count = if reduced_motion { 1 } else { IDLE_DELAYS.len() };
    let frames = IDLE_DELAYS
        .iter()
        .take(count)
        .enumerate()
        .map(|(column, delay)| {
            let frame = imageops::crop_imm(&sheet, column as u32 * 192, 0, 192, 208).to_image();
            let mut frame = imageops::resize(&frame, width, height, imageops::FilterType::Nearest);
            for pixel in frame.pixels_mut() {
                pixel.0.swap(0, 2);
            }
            Frame::from_parts(frame, 0, 0, Delay::from_numer_denom_ms(*delay, 1))
        });
    let image = Arc::new(RenderImage::new(frames.collect::<Vec<_>>()));
    cache.insert(key, image.clone());
    Ok(image)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mini_stays_inside_the_display_and_rejects_unknown_pets() {
        let viewport = size(px(356.), px(320.));
        let pet = pet_bounds(viewport);
        assert_eq!(pet.bottom_right(), point(px(332.), px(296.)));
        let composer = composer_bounds(viewport);
        assert!(composer.left() >= px(16.));
        assert!(composer.right() <= viewport.width - px(16.));
        assert_eq!(composer.center().y, pet.center().y);
        assert!(super::pet("custom:missing", 2., false).is_err());
        assert!(super::pet("codex", f32::NAN, false).is_err());
        let link = chat_bounds(viewport).center();
        assert!(hit_bounds(viewport, true, true).any(|bounds| bounds.contains(&link)));
        assert!(hit_bounds(viewport, false, true).any(|bounds| bounds.contains(&link)));
        assert!(!hit_bounds(viewport, true, false).any(|bounds| bounds.contains(&link)));
        let gap = point(link.x, chat_bounds(viewport).bottom() + px(6.));
        assert!(!hit_bounds(viewport, true, true).any(|bounds| bounds.contains(&gap)));
    }
}
