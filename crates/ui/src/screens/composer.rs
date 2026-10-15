//! `composer` parity screen: Home with `Refactor src/math.py` typed and the model picker
//! open.

use crate::{composer::Composer, fixture};
use bridge::types::{ConfigReadResponse, ModelListResponse};
use gpui::{AnyView, App, AppContext as _, Context, Entity, Window};

/// A composer with the fixture's `model/list` and `config/read` (GPT-6 Astra, Light).
pub fn fixture_composer(
    window: &mut Window,
    cx: &mut App,
    new: fn(&mut Window, &mut Context<Composer>) -> Composer,
) -> Entity<Composer> {
    let models: ModelListResponse = fixture::response("readback.jsonl", "model/list", 0);
    let config: ConfigReadResponse = fixture::response("readback.jsonl", "config/read", 0);
    cx.new(|cx| {
        let mut composer = new(window, cx);
        composer.set_models(&models.data, &config.config, cx);
        composer
    })
}

/// The thread view's bottom-docked composer (`Work with Codex`) with the fixture models.
pub fn thread_composer(window: &mut Window, cx: &mut App) -> Entity<Composer> {
    fixture_composer(window, cx, Composer::thread)
}

pub fn build(window: &mut Window, cx: &mut App) -> AnyView {
    let main = cx.new(|cx| super::home::HomeMain::new(window, cx));
    main.read(cx).composer.clone().update(cx, |c, cx| {
        c.set_text("Refactor src/math.py", cx);
        c.open_model_picker(cx);
    });
    super::home::shell(main.into(), cx)
}
