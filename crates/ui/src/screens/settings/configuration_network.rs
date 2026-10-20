use super::*;

const WEB_SEARCH_OPTIONS: &[(&str, &str)] = &[
    ("disabled", "Disabled"),
    ("cached", "Cached"),
    ("indexed", "Indexed"),
    ("live", "Live"),
];

impl SettingsPane {
    pub(super) fn configuration_network(
        &self,
        t: &Theme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<Div> {
        const NETWORK: &str = "config.sandbox_workspace_write.network_access";
        const SEARCH: &str = "config.web_search";
        let mut rows = Vec::new();
        if self
            .page
            .read(cx)
            .choice("config.sandbox_mode", "read-only")
            == "workspace-write"
        {
            let page = self.page.read(cx);
            let lock = page.config_lock(NETWORK);
            let disabled = lock.is_some() || page.pending_config.contains(&NETWORK);
            let control = if disabled {
                switch(NETWORK, page.flag(NETWORK, false), true, t).into_any_element()
            } else {
                self.toggle(NETWORK, false, t, window, cx)
            };
            rows.push(settings_row(
                "Allow network access",
                Some(
                    lock.unwrap_or(
                        "Allow network access when the sandbox is set to workspace write",
                    ),
                ),
                control,
                t,
            ));
        }
        let lock = self.page.read(cx).config_lock(SEARCH);
        rows.push(settings_row(
            "Web search",
            Some(lock.unwrap_or("Choose how ChatGPT accesses the web")),
            self.configuration_select(SEARCH, WEB_SEARCH_OPTIONS, t, window, cx),
            t,
        ));
        rows
    }
}
