use crate::{Root, Route};
use gpui::{Context, Window};
use serde::{Deserialize, Serialize, Serializer};
use url::form_urlencoded;

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub enum SettingsDestination {
    Browser,
    ComputerUse,
}

impl SettingsDestination {
    pub fn section(&self) -> &'static str {
        match self {
            Self::Browser => "Browser",
            Self::ComputerUse => "Computer use",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct HandoffTarget {
    pub thread_id: String,
    pub tab_id: String,
    pub connection_thread_id: String,
}

impl HandoffTarget {
    fn parse(query: &str) -> Option<Self> {
        let (mut thread, mut tab, mut connection) = (None, None, None);
        for (name, value) in form_urlencoded::parse(query.as_bytes()) {
            let slot = match name.as_ref() {
                "thread_id" => &mut thread,
                "tab_id" => &mut tab,
                "connection_thread_id" => &mut connection,
                _ => continue,
            };
            if slot.replace(value).is_some() {
                return None;
            }
        }
        let thread_id = thread?.into_owned();
        let tab_id = tab?.into_owned();
        let connection_thread_id =
            connection.map_or_else(|| thread_id.clone(), |id| id.into_owned());
        let valid_thread = |id: &str| {
            id.len() <= 128
                && id.chars().any(|c| {
                    !matches!(c, '\u{0009}'..='\u{000d}' | ' ' | '\u{00a0}' | '\u{1680}'
                        | '\u{2000}'..='\u{200a}' | '\u{2028}' | '\u{2029}'
                        | '\u{202f}' | '\u{205f}' | '\u{3000}' | '\u{feff}')
                })
        };
        if !valid_thread(&thread_id)
            || !valid_thread(&connection_thread_id)
            || !matches!(tab_id.as_bytes().first(), Some(b'1'..=b'9'))
            || !tab_id.bytes().all(|c| c.is_ascii_digit())
            || tab_id.parse::<u64>().ok()? > 9_007_199_254_740_991
        {
            return None;
        }
        Some(Self {
            thread_id,
            tab_id,
            connection_thread_id,
        })
    }
}

pub fn handoff_route(path: &str) -> Option<Route> {
    let (path, fragment) = path
        .split_once('#')
        .map_or((path, None), |(path, fragment)| (path, Some(fragment)));
    let (pathname, query) = path.split_once('?').unwrap_or((path, ""));
    if pathname != "/browser/handoff" {
        return None;
    }
    let target = fragment
        .is_none()
        .then(|| HandoffTarget::parse(query))
        .flatten();
    Some(Route::BrowserHandoff(target))
}

pub fn settings_route(path: &str) -> Option<Route> {
    let path = path.split(['?', '#']).next()?;
    let destination = match path {
        "/settings/browser-use"
        | "/settings/browser-use/extensions"
        | "/settings/browser-use/downloads"
        | "/settings/browser-use/history"
        | "/settings/browser-use/contact-info"
        | "/settings/browser-use/passwords"
        | "/settings/browser-use/site-settings" => SettingsDestination::Browser,
        path if path.starts_with("/settings/browser-use/site-settings/") => {
            SettingsDestination::Browser
        }
        path if path
            .strip_prefix("/settings/computer-use/")
            .is_some_and(|family| !family.is_empty() && !family.contains('/')) =>
        {
            SettingsDestination::ComputerUse
        }
        _ => return None,
    };
    // The native host has no Chromium services. These routes retain the original unavailable-provider redirect.
    Some(Route::BrowserSettings(destination))
}

pub(crate) fn persisted_route<S: Serializer>(
    route: &Route,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    match route {
        Route::BrowserHandoff(_) => Route::Home.serialize(serializer),
        Route::BrowserSettings(_) => Route::Settings.serialize(serializer),
        _ => route.serialize(serializer),
    }
}

impl Root {
    pub fn open_browser_path(&mut self, path: &str, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(route) = handoff_route(path).or_else(|| settings_route(path)) {
            self.command_menu = None;
            self.dialog = None;
            self.navigate(route, window, cx);
        }
    }

    pub(super) fn leave_handoff(
        &mut self,
        _: &ui::screens::browser_use::ReturnHome,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.state.route = Route::Home;
        self.navigation_epoch += 1;
        self.forward.clear();
        self.sync(window, cx);
        self.select_route(cx);
    }
}
