pub const SECTIONS: &[(&str, &str)] = &[
    ("general-settings", "General"),
    ("notifications", "Notifications"),
    ("import", "Import"),
    ("profile", "Profile"),
    ("security", "Security"),
    ("account", "Account"),
    ("age", "Age"),
    ("agreements", "Agreements"),
    ("parental-controls", "Parental controls"),
    ("trusted-contact", "Trusted contact"),
    ("safety", "Safety"),
    ("time-management", "Time management"),
    ("appearance", "Appearance"),
    ("voice", "Voice"),
    ("storage", "Storage"),
    ("pets", "Mini & Pets"),
    ("chronicle", "Computer history"),
    ("appshots", "Appshots"),
    ("git-settings", "Git"),
    ("connections", "Connections"),
    ("cloud-settings", "Codex Cloud"),
    ("codex-cloud", "Legacy Codex Cloud"),
    ("cloud-environments", "Cloud environments"),
    ("code-review", "Code Review"),
    ("local-environments", "Environments"),
    ("worktrees", "Worktrees"),
    ("agent", "Configuration"),
    ("work-settings", "Configuration"),
    ("personalization", "Personalization"),
    ("keyboard-shortcuts", "Keyboard shortcuts"),
    ("codex-micro", "Codex Micro"),
    ("usage", "Usage & billing"),
    ("billing", "Usage & billing"),
    ("analytics", "Usage & billing"),
    ("consumer-view", "Usage & billing"),
    ("usage-code-review", "Usage & billing"),
    ("debug", "Debug"),
    ("browser-use", "Browser"),
    ("extension", "Browser extension"),
    ("computer-use", "Computer use"),
    ("mcp-settings", "MCP servers"),
    ("hooks-settings", "Hooks"),
    ("plugins-settings", "Plugins"),
    ("file-type-handlers", "File type handlers"),
    ("skills-settings", "Skills"),
    ("data-controls", "Data controls"),
    ("archived-chats", "Archived chats"),
    ("ads-controls", "Ads controls"),
    ("teams", "Teams"),
    ("cloud-computer", "Cloud computer"),
    ("passwords", "Passwords"),
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SettingsRoute {
    pub path: String,
    pub section: &'static str,
}

impl SettingsRoute {
    pub fn parse(path: &str) -> Option<Self> {
        let path = path
            .strip_prefix("codex://")
            .map(|path| format!("/{path}"))
            .unwrap_or_else(|| path.to_owned());
        if path.split(['?', '#']).next() == Some("/profile") {
            return Some(Self {
                path,
                section: "Profile",
            });
        }
        let rest = path.strip_prefix("/settings")?;
        if !rest.is_empty() && !rest.starts_with('/') && !rest.starts_with('?') {
            return None;
        }
        let slug = rest
            .trim_start_matches('/')
            .split(['/', '?', '#'])
            .next()
            .unwrap_or_default();
        if matches!(slug, "open-source-licenses" | "third-party-notices") {
            return Some(Self {
                path: "/settings/open-source-licenses".into(),
                section: "Open source licenses",
            });
        }
        if slug == "work-settings" {
            return Some(Self {
                path: redirect_query(&path, "/settings/agent", "mode", "work")?,
                section: "Configuration",
            });
        }
        let tab = match slug {
            "analytics" => Some("analytics"),
            "consumer-view" => Some("consumer-view"),
            "usage-code-review" => Some("code-review"),
            _ => None,
        };
        if let Some(tab) = tab {
            return Some(Self {
                path: redirect_query(&path, "/settings/usage", "tab", tab)?,
                section: "Usage & billing",
            });
        }
        let section = SECTIONS
            .iter()
            .find(|(name, _)| *name == slug)
            .map(|(_, label)| *label);
        Some(Self {
            path: if section.is_some() {
                path
            } else {
                "/settings/general-settings".into()
            },
            section: section.unwrap_or("General"),
        })
    }

    pub fn for_section(label: &str) -> Option<Self> {
        if label == "Profile" {
            return Some(Self {
                path: "/profile".into(),
                section: "Profile",
            });
        }
        let (slug, section) = SECTIONS.iter().find(|(_, title)| *title == label)?;
        Some(Self {
            path: format!("/settings/{slug}"),
            section,
        })
    }
}

fn redirect_query(path: &str, destination: &str, key: &str, value: &str) -> Option<String> {
    let parsed = url::Url::parse(&format!("https://settings.invalid{path}")).ok()?;
    let mut pairs = Vec::new();
    let mut replaced = false;
    for (name, old_value) in parsed.query_pairs() {
        if name == key {
            if replaced {
                continue;
            }
            replaced = true;
            pairs.push((name.into_owned(), value.to_owned()));
        } else {
            pairs.push((name.into_owned(), old_value.into_owned()));
        }
    }
    if !replaced {
        pairs.push((key.to_owned(), value.to_owned()));
    }
    let query = url::form_urlencoded::Serializer::new(String::new())
        .extend_pairs(pairs)
        .finish();
    let fragment = parsed
        .fragment()
        .map_or(String::new(), |fragment| format!("#{fragment}"));
    Some(format!("{destination}?{query}{fragment}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_routes_keep_nested_destinations_and_redirect_only_known_aliases() {
        assert_eq!(
            SettingsRoute::parse("/settings/work-settings")
                .unwrap()
                .path,
            "/settings/agent?mode=work"
        );
        assert_eq!(
            SettingsRoute::parse("/settings/usage-code-review?range=month")
                .unwrap()
                .path,
            "/settings/usage?range=month&tab=code-review"
        );
        assert_eq!(
            SettingsRoute::parse("/settings/connections/wsl")
                .unwrap()
                .path,
            "/settings/connections/wsl"
        );
        assert_eq!(
            SettingsRoute::parse("/settings/work-settings?mode=other&scope=local#model")
                .unwrap()
                .path,
            "/settings/agent?mode=work&scope=local#model"
        );
        assert_eq!(
            SettingsRoute::parse("/settings/missing").unwrap().section,
            "General"
        );
        assert_eq!(
            SettingsRoute::parse("/settings").unwrap().section,
            "General"
        );
        assert!(SettingsRoute::parse("/settings-elsewhere").is_none());
        assert!(SettingsRoute::parse("https://example.com/settings/agent").is_none());
    }
}
