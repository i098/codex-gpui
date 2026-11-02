//! `/memories` (`composer.memoriesSlashCommand`): the `Chat memories` dialog. On an open chat,
//! `Generate memories` sends `thread/memoryMode/set`; on Home, both switches apply to the next
//! chat's `thread/start` as `memories.*` config overrides.
//! ponytail: no `Import from ChatGPT` (needs a ChatGPT `/wham/memories/import` session) and no
//! `memory/status` pipeline probe (it only picks a Statsig-gated memory version).

use crate::Root;
use crate::routes::Route;
use gpui::{AppContext as _, Context, Entity, Window};
use serde_json::{Value, json};
use ui::dialog::Dialog;

/// The two `Chat memories` switches.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct MemoryPrefs {
    pub use_memories: bool,
    pub generate: bool,
}

impl MemoryPrefs {
    /// The `[memories]` config defaults, `true` when unset (as in Settings > Personalization).
    pub(crate) fn from_config(config: &Value) -> Self {
        let memories = &config["memories"];
        Self {
            use_memories: memories["use_memories"].as_bool().unwrap_or(true),
            generate: memories["generate_memories"].as_bool().unwrap_or(true),
        }
    }

    /// The `thread/start` config overrides of a chat started with these switches.
    pub(crate) fn apply(self, params: &mut Value) {
        params["config"]["memories.generate_memories"] = self.generate.into();
        params["config"]["memories.use_memories"] = self.use_memories.into();
    }
}

impl Root {
    /// `/memories` offered only while the `memories` feature is on.
    pub(crate) fn memories_available(&self) -> bool {
        self.config["features"]["memories"].as_bool() == Some(true)
    }

    pub(crate) fn open_memories(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.has_dialog() {
            return;
        }
        let defaults = MemoryPrefs::from_config(&self.config);
        let (started, prefs) = match &self.state.route {
            Route::Thread(id) => (
                true,
                self.thread_memory.get(id).copied().unwrap_or(defaults),
            ),
            _ => (false, self.new_chat_memory.unwrap_or(defaults)),
        };
        let dialog =
            cx.new(|cx| Dialog::chat_memories(started, prefs.use_memories, prefs.generate, cx));
        self.show_dialog(dialog, window, cx);
        self.dialog_to_composer = true;
    }

    /// A switch of `dialog` changed. A refused `thread/memoryMode/set` restores the switch and
    /// is only logged, as in the original.
    pub(crate) fn change_chat_memory(
        &mut self,
        dialog: Entity<Dialog>,
        prefs: MemoryPrefs,
        cx: &mut Context<Self>,
    ) {
        let Route::Thread(id) = self.state.route.clone() else {
            self.new_chat_memory = Some(prefs);
            return;
        };
        let previous = self
            .thread_memory
            .insert(id.clone(), prefs)
            .unwrap_or_else(|| MemoryPrefs::from_config(&self.config));
        if previous.generate == prefs.generate {
            return;
        }
        let bridge = self.bridge.clone();
        let mode = if prefs.generate {
            "enabled"
        } else {
            "disabled"
        };
        cx.spawn(async move |this, cx| {
            let result = bridge
                .request::<_, Value>(
                    "thread/memoryMode/set",
                    json!({"threadId": id, "mode": mode}),
                )
                .await;
            if let Err(error) = result {
                eprintln!("codex-gpui: thread/memoryMode/set: {error:#}");
                this.update(cx, |this, cx| {
                    this.thread_memory.insert(id, previous);
                    dialog.update(cx, |dialog, cx| {
                        dialog.set_memory_generate(previous.generate, cx)
                    });
                })
                .ok();
            }
        })
        .detach();
    }
}
