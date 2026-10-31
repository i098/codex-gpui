//! Settings ▸ Appshots (`appshots-settings-*.js`): the double-modifier hotkey, where an
//! appshot goes, Mini preference and sound. The app registers the hotkey from the
//! stored host settings.

use super::{SettingsPane, settings_row};
use gpui::{Context, Div, Window};
use theme::Theme;

const HOTKEYS: &[(&str, &str)] = &[
    ("DoubleCommand", "⌘ + ⌘"),
    ("DoubleOption", "⌥ + ⌥"),
    ("DoubleShift", "⇧ + ⇧"),
    ("none", "None"),
];
const HOTKEY_DETAILS: &[(&str, &str)] = &[
    ("DoubleCommand", "Press both ⌘ keys simultaneously"),
    ("DoubleOption", "Press both ⌥ keys simultaneously"),
    ("DoubleShift", "Press both ⇧ keys simultaneously"),
];
const DESTINATIONS: &[(&str, &str)] = &[
    ("automatic", "Automatic"),
    ("last-chat", "Current chat"),
    ("new-chat", "New chat"),
];
const DESTINATION_DETAILS: &[(&str, &str)] = &[
    (
        "automatic",
        "Uses the current chat if used recently, otherwise starts a new chat. In Mini, always starts a new chat",
    ),
    ("last-chat", "Always use the current chat"),
    ("new-chat", "Always start a new chat"),
];

impl SettingsPane {
    pub(super) fn appshots_page(
        &mut self,
        t: &Theme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<(&'static str, Vec<Div>)> {
        let hotkey = self.select_menu(
            "host.appshotHotkey",
            HOTKEYS,
            None,
            |s| s.with_details(HOTKEY_DETAILS),
            cx,
        );
        let destination = self.select_menu(
            "host.appshotDestination",
            DESTINATIONS,
            None,
            |s| s.with_details(DESTINATION_DETAILS),
            cx,
        );
        let rows = vec![
            settings_row(
                "Hotkey",
                Some("Take an appshot to show ChatGPT your frontmost window"),
                hotkey,
                t,
            ),
            settings_row(
                "Appshot destination",
                Some("Choose where appshots go when you use the hotkey"),
                destination,
                t,
            ),
            settings_row(
                "Use Mini when visible",
                Some("When Mini isn't visible, Appshots use your chosen destination"),
                self.toggle("host.appshotAlwaysSendToMini", false, t, window, cx),
                t,
            ),
            settings_row(
                "Play sound effect",
                None,
                self.toggle("host.appshotSoundEnabled", true, t, window, cx),
                t,
            ),
        ];
        vec![("", rows)]
    }
}
