//! `composer` parity screen: Home with `Refactor src/math.py` typed and the model picker
//! open.

use crate::{
    composer::{Composer, EnabledEfforts},
    fixture,
};
use bridge::types::{ConfigReadResponse, ModelListResponse};
use gpui::{AnyView, App, AppContext as _, Context, Entity, Window};

/// A composer with the fixture's `model/list` and `config/read` (GPT-6 Astra, Light) and its
/// signed-in ChatGPT account.
pub fn fixture_composer(
    window: &mut Window,
    cx: &mut App,
    new: fn(&mut Window, &mut Context<Composer>) -> Composer,
) -> Entity<Composer> {
    let models: ModelListResponse = fixture::response("readback.jsonl", "model/list", 0);
    let config: ConfigReadResponse = fixture::response("readback.jsonl", "config/read", 0);
    cx.set_global(EnabledEfforts {
        setting: None,
        chatgpt: true,
    });
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
    let composer = main.read(cx).composer.clone();
    composer.update(cx, |c, cx| c.set_text("Refactor src/math.py", cx));
    // Open after the first (closed) frame: the open trigger keeps its closed width.
    window.defer(cx, move |window, cx| {
        composer.update(cx, |c, cx| c.open_model_picker(window, cx))
    });
    super::home::shell(main.into(), cx)
}
