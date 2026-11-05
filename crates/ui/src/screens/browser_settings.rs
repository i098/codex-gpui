use super::*;

/// `browser_use.default_origin_policy.full_cdp_access`: `allow` or `deny`.
pub(super) const FULL_CDP: &str = "browser_use.default_origin_policy.full_cdp_access";

/// The original's `Allow`/`Block` choices for a default site permission.
const POLICY_OPTIONS: &[(&str, &str)] = &[("allow", "Allow"), ("deny", "Block")];

impl SettingsPane {
    /// The in-app browser's `browser_use` permissions in `config.toml`, as the original's
    /// Browser settings write them (`config/batchWrite` on `browser_use.*`).
    pub(super) fn browser(
        &mut self,
        t: &Theme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<(&'static str, Vec<Div>)> {
        let row = |title, description, control| settings_row(title, description, control, t);
        let permissions = vec![
            row(
                "Browse",
                Some("Default for websites without custom permissions"),
                self.select_menu(
                    "browser_use.default_origin_policy.access",
                    POLICY_OPTIONS,
                    None,
                    |select| select,
                    cx,
                ),
            ),
            row(
                "Download",
                None,
                self.select_menu(
                    "browser_use.default_origin_policy.downloads",
                    POLICY_OPTIONS,
                    None,
                    |select| select,
                    cx,
                ),
            ),
            row(
                "Upload",
                None,
                self.select_menu(
                    "browser_use.default_origin_policy.uploads",
                    POLICY_OPTIONS,
                    None,
                    |select| select,
                    cx,
                ),
            ),
            row(
                "History",
                Some("Access history without asking"),
                self.toggle("browser_use.allow_history_access", false, t, window, cx),
            ),
            row(
                "Enable full CDP access",
                Some(
                    "Allow ChatGPT to use full Chrome DevTools Protocol (CDP) access in connected Browser Use sessions. Full CDP access lets ChatGPT inspect and control sensitive browser internals that may put your data at risk.",
                ),
                self.toggle(FULL_CDP, false, t, window, cx),
            ),
        ];
        let sites = site_rows(&self.page.read(cx).values["browser_use"]["origins"]);
        let sites = if sites.is_empty() {
            vec![row(
                "No custom site permissions",
                Some("Only sites with custom permissions appear here"),
                div().into_any_element(),
            )]
        } else {
            sites
                .into_iter()
                .map(|(origin, summary)| {
                    settings_row_content(
                        origin,
                        Some(text(summary, 12., 16., 430., t.text_secondary).into_any_element()),
                        div().into_any_element(),
                        t,
                    )
                })
                .collect()
        };
        vec![("Permissions", permissions), ("Site permissions", sites)]
    }
}

/// `browser_use.origins` as `(origin, "Browse: Allow · Download: Block")` rows, sorted.
fn site_rows(origins: &Value) -> Vec<(String, String)> {
    let mut rows: Vec<_> = origins
        .as_object()
        .into_iter()
        .flatten()
        .map(|(origin, policy)| {
            let summary = [
                ("access", "Browse"),
                ("downloads", "Download"),
                ("uploads", "Upload"),
                ("full_cdp_access", "Debug / CDP"),
            ]
            .iter()
            .filter_map(|&(key, label)| {
                let value = match policy[key].as_str()? {
                    "allow" => "Allow",
                    "deny" => "Block",
                    _ => return None,
                };
                Some(format!("{label}: {value}"))
            })
            .collect::<Vec<_>>()
            .join(" · ");
            let summary = if summary.is_empty() {
                "Uses the default permission".to_owned()
            } else {
                summary
            };
            (origin.clone(), summary)
        })
        .collect();
    rows.sort();
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn site_rows_summarize_custom_permissions() {
        let rows = site_rows(&json!({
            "https://b.example": {},
            "https://a.example": {"access": "allow", "downloads": "deny", "webmcp": "x"},
        }));
        assert_eq!(
            rows,
            [
                (
                    "https://a.example".to_owned(),
                    "Browse: Allow · Download: Block".to_owned()
                ),
                (
                    "https://b.example".to_owned(),
                    "Uses the default permission".to_owned()
                ),
            ]
        );
    }
}
