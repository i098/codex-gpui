//! Slash completion keeps keyboard focus in the editor and preserves text outside the command token.
//! Fast, Archive, New chat, and Usage use existing app handlers.
//! S058/S059 own the other command bodies and submenus; these entries remain visible but inactive.
//! The same list state serves the `@` mention trigger (`super::mention`).
//! `cargo test -p app --test s021` replays the original menu, keys, colors, focus, and requests.
use super::*;

#[derive(Clone, Copy)]
pub enum SlashCommand {
    Fast,
    Archive,
    NewChat,
    Usage,
}

impl gpui::EventEmitter<SlashCommand> for Composer {}

struct Command {
    id: &'static str,
    title: &'static str,
    description: &'static str,
    icon: &'static str,
    requires_empty: bool,
}

const COMMANDS: &[Command] = &[
    Command {
        id: "archive",
        title: "Archive",
        description: "Archive the current chat",
        icon: "sidebar-archive",
        requires_empty: true,
    },
    Command {
        id: "review-mode",
        title: "Code review",
        description: "Review uncommitted changes or compare against a branch",
        icon: "code-review",
        requires_empty: true,
    },
    Command {
        id: "compact",
        title: "Compact",
        description: "Compact this chat's context",
        icon: "composer-reset",
        requires_empty: true,
    },
    Command {
        id: "fast",
        title: "Fast",
        description: "1.5x speed, increased usage",
        icon: "composer-bolt",
        requires_empty: false,
    },
    Command {
        id: "feedback",
        title: "Feedback",
        description: "Send feedback about this chat",
        icon: "sidebar-cmd-feedback",
        requires_empty: false,
    },
    Command {
        id: "fork",
        title: "Fork chat",
        description: "Fork this chat in the current workspace or a new worktree",
        icon: "thread-fork",
        requires_empty: true,
    },
    Command {
        id: "goal",
        title: "Goal",
        description: "Set a goal to keep pursuing",
        icon: "thread-summary",
        requires_empty: false,
    },
    Command {
        id: "init",
        title: "Init",
        description: "Create an AGENTS.md file with instructions for Codex",
        icon: "sidebar-cmd-create-page",
        requires_empty: true,
    },
    Command {
        id: "mcp",
        title: "MCP",
        description: "Show MCP server status",
        icon: "sidebar-cmd-mcp-servers",
        requires_empty: false,
    },
    Command {
        id: "model",
        title: "Model",
        description: "",
        icon: "composer-model",
        requires_empty: false,
    },
    Command {
        id: "new",
        title: "New chat",
        description: "Start a blank chat in the same workspace",
        icon: "sidebar-cmd-new-chat",
        requires_empty: true,
    },
    Command {
        id: "pet",
        title: "Pet",
        description: "Wake or tuck away the desktop pet",
        icon: "settings-mini-pets",
        requires_empty: false,
    },
    Command {
        id: "pin",
        title: "Pin chat",
        description: "Keep this chat in the sidebar",
        icon: "sidebar-pin",
        requires_empty: true,
    },
    Command {
        id: "plan-mode",
        title: "Plan mode",
        description: "Turn plan mode on",
        icon: "settings-personalization",
        requires_empty: false,
    },
    Command {
        id: "reasoning",
        title: "Reasoning",
        description: "",
        icon: "settings-personalization",
        requires_empty: false,
    },
    Command {
        id: "rename",
        title: "Rename",
        description: "Rename the current chat",
        icon: "pencil",
        requires_empty: true,
    },
    Command {
        id: "share",
        title: "Share chat",
        description: "Create a link to a snapshot of this chat",
        icon: "arrow-up-right",
        requires_empty: true,
    },
    Command {
        id: "side",
        title: "Side",
        description: "Start a temporary side chat",
        icon: "new-chat",
        requires_empty: false,
    },
    Command {
        id: "sketch",
        title: "Sketch",
        description: "Draw a sketch",
        icon: "pencil",
        requires_empty: false,
    },
    Command {
        id: "status",
        title: "Status",
        description: "Show chat ID, context usage, and rate limits",
        icon: "settings-usage",
        requires_empty: false,
    },
    Command {
        id: "ultrafast",
        title: "Ultrafast",
        description: "The fastest available responses for latency-sensitive work.",
        icon: "composer-bolt",
        requires_empty: false,
    },
    Command {
        id: "usage",
        title: "Usage & billing",
        description: "Open usage and billing settings",
        icon: "settings-usage",
        requires_empty: false,
    },
    Command {
        id: "project",
        title: "Work in a project",
        description: "Choose project for new chats",
        icon: "folder",
        requires_empty: false,
    },
];

pub(super) struct Skill {
    pub(super) name: String,
    pub(super) title: String,
    pub(super) description: String,
    /// `SKILL.md` path: the `$name` chip's link target.
    pub(super) path: String,
}

#[derive(Default)]
pub(super) struct Slash {
    pub open: bool,
    /// `@` opened the list (else `/`).
    pub(super) mention: bool,
    pub(super) range: Range<usize>,
    pub(super) dismissed: Option<usize>,
    pub(super) selected: usize,
    pub(super) matches: Vec<usize>,
    pub(super) skills: Vec<Skill>,
    scroll: gpui::ScrollHandle,
    pub(super) search: mention::Search,
}

impl Slash {
    pub(super) fn has_selection(&self) -> bool {
        self.open && !self.matches.is_empty()
    }

    pub(super) fn reset(&mut self) {
        self.open = false;
        self.dismissed = None;
        self.matches.clear();
    }
}

fn matches_query(text: &str, query: &str) -> bool {
    query.is_empty()
        || text
            .as_bytes()
            .windows(query.len())
            .any(|part| part.eq_ignore_ascii_case(query.as_bytes()))
}

impl Composer {
    pub fn set_slash_skills(&mut self, response: &Value, cx: &mut Context<Self>) {
        self.slash.skills = response["data"]
            .as_array()
            .into_iter()
            .flatten()
            .flat_map(|entry| entry["skills"].as_array().into_iter().flatten())
            .filter(|skill| skill["enabled"] != false)
            .filter_map(|skill| {
                let name = skill["name"].as_str()?;
                let title = skill["interface"]["displayName"]
                    .as_str()
                    .map(str::to_owned)
                    .unwrap_or_else(|| {
                        name.split('-')
                            .map(|word| {
                                let mut chars = word.chars();
                                chars.next().map_or_else(String::new, |first| {
                                    first.to_uppercase().chain(chars).collect()
                                })
                            })
                            .collect::<Vec<_>>()
                            .join(" ")
                    });
                let description = skill["interface"]["shortDescription"]
                    .as_str()
                    .or_else(|| skill["description"].as_str())
                    .unwrap_or_default()
                    .into();
                Some(Skill {
                    name: name.into(),
                    title,
                    description,
                    path: skill["path"].as_str().unwrap_or_default().into(),
                })
            })
            .collect();
        self.slash.skills.sort_by(|a, b| a.title.cmp(&b.title));
        self.refresh_slash(cx);
    }

    /// The token before the cursor: `@` at the start or after whitespace (spaces allowed in
    /// its query, newlines not), else `/` without whitespace in its query.
    pub(super) fn refresh_slash(&mut self, cx: &mut Context<Self>) {
        let cursor = self.cursor();
        let before = &self.content[..cursor];
        let mention = before.rfind('@').filter(|&at| {
            (at == 0 || before[..at].ends_with(char::is_whitespace))
                && !before[at + 1..].contains('\n')
        });
        let slash = before
            .rfind('/')
            .filter(|&at| !before[at + 1..].contains(char::is_whitespace));
        let (mention, start) = match (mention, slash) {
            (Some(at), _) => (true, at),
            (None, Some(at)) => (false, at),
            (None, None) => {
                self.slash.open = false;
                self.slash.dismissed = None;
                self.end_search(cx);
                return;
            }
        };
        let range = start..cursor;
        if self.slash.dismissed == Some(range.start) {
            return;
        }
        if !mention {
            self.end_search(cx);
        }
        self.slash.mention = mention;
        let query = self.content[range.start + 1..range.end].to_lowercase();
        let empty = self.content[..range.start].trim().is_empty()
            && self.content[range.end..].trim().is_empty()
            && self.attachments.is_empty();
        self.slash.matches.clear();
        if mention {
            let query = self.content[range.start + 1..range.end].to_owned();
            self.refresh_mention(&query, cx);
        } else {
            for (index, command) in COMMANDS.iter().enumerate() {
                let available = match command.id {
                    "archive" | "review-mode" | "compact" | "fork" | "init" | "new" | "pin"
                    | "rename" | "share" | "side" => self.docked,
                    "project" => !self.docked,
                    "fast" | "model" | "plan-mode" | "ultrafast" => self.current_model().is_some(),
                    _ => true,
                };
                if available
                    && (!command.requires_empty || empty)
                    && (matches_query(command.title, &query) || matches_query(command.id, &query))
                {
                    self.slash.matches.push(index);
                }
            }
            for (index, skill) in self.slash.skills.iter().enumerate() {
                if matches_query(&skill.title, &query) || matches_query(&skill.name, &query) {
                    self.slash.matches.push(COMMANDS.len() + index);
                }
            }
        }
        self.slash.range = range;
        self.slash.open = true;
        self.slash.selected = 0;
        self.slash.scroll.set_offset(point(px(0.), px(0.)));
        self.menu = None;
        cx.notify();
    }

    pub(super) fn close_slash(&mut self, cx: &mut Context<Self>) {
        self.slash.open = false;
        self.slash.dismissed = Some(self.slash.range.start);
        self.end_search(cx);
        cx.notify();
    }

    pub(super) fn move_slash(&mut self, forward: bool, cx: &mut Context<Self>) -> bool {
        if !self.slash.open {
            return false;
        }
        let count = self.slash.matches.len();
        if count > 0 {
            self.slash.selected = if forward {
                (self.slash.selected + 1) % count
            } else {
                (self.slash.selected + count - 1) % count
            };
            let heading = usize::from(self.slash.matches[self.slash.selected] >= COMMANDS.len());
            self.slash
                .scroll
                .scroll_to_item(self.slash.selected + heading);
            cx.notify();
        }
        true
    }

    pub(super) fn next_command(&mut self, _: &NextCommand, _: &mut Window, cx: &mut Context<Self>) {
        if !self.move_slash(true, cx) {
            cx.propagate();
        }
    }

    pub(super) fn previous_command(
        &mut self,
        _: &PreviousCommand,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.move_slash(false, cx) {
            cx.propagate();
        }
    }

    pub(super) fn complete_command(
        &mut self,
        _: &CompleteCommand,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.slash.open {
            self.activate_slash(window, cx);
        } else {
            cx.propagate();
        }
    }

    fn slash_label(&self, index: usize) -> (&str, &str, &str) {
        if let Some(command) = COMMANDS.get(index) {
            let description = if command.id == "reasoning" {
                effort_label(&self.effort)
            } else {
                command.description
            };
            (command.title, description, command.icon)
        } else {
            let skill = &self.slash.skills[index - COMMANDS.len()];
            (&skill.title, &skill.description, "sidebar-cmd-create-page")
        }
    }

    pub fn slash_state(&self) -> Option<(Vec<String>, Option<usize>)> {
        (self.slash.open && !self.slash.mention).then(|| {
            (
                self.slash
                    .matches
                    .iter()
                    .map(|&index| {
                        let (title, description, _) = self.slash_label(index);
                        if description.is_empty() {
                            title.into()
                        } else {
                            format!("{title}\n{description}")
                        }
                    })
                    .collect(),
                (!self.slash.matches.is_empty()).then_some(self.slash.selected),
            )
        })
    }

    pub(super) fn activate_slash(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(&index) = self.slash.matches.get(self.slash.selected) else {
            return;
        };
        if self.slash.mention {
            self.activate_mention(index, cx);
            return;
        }
        let Some(command) = COMMANDS.get(index) else {
            return;
        };
        let action = match command.id {
            "fast" => SlashCommand::Fast,
            "archive" => SlashCommand::Archive,
            "new" => SlashCommand::NewChat,
            "usage" => SlashCommand::Usage,
            _ => return,
        };
        self.content.replace_range(self.slash.range.clone(), "");
        self.selected_range = self.slash.range.start..self.slash.range.start;
        self.marked_range = None;
        self.slash.open = false;
        self.slash.dismissed = None;
        if matches!(action, SlashCommand::Fast) {
            self.fast = true;
        }
        self.focus_handle.focus(window, cx);
        cx.emit(action);
        cx.notify();
    }

    /// One suggestion row: 28.5625 px, `rounded-xl`, highlighted at 8% text, else 75% opacity.
    pub(super) fn suggestion_row(
        &self,
        t: &Theme,
        selected: usize,
        index: usize,
        (title, description, icon_name): (&str, &str, &str),
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let active = selected == self.slash.selected;
        div()
            .id(("suggestion", index))
            .flex_none()
            .h(px(28.5625))
            .px(px(8.))
            .flex()
            .items_center()
            .gap(px(8.))
            .rounded(px(15.))
            .when(active, |d| d.bg(t.text.opacity(0.08)))
            .when(!active, |d| d.opacity(0.75))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_hover(cx.listener(move |this, hovered, _, cx| {
                if *hovered {
                    this.slash.selected = selected;
                    cx.notify();
                }
            }))
            .on_click(cx.listener(move |this, _, window, cx| {
                this.slash.selected = selected;
                this.activate_slash(window, cx);
            }))
            .child(icon(icon_name, 16., t.text))
            .child(text(title.to_owned(), 13., 18.5714, 400., t.text).flex_none())
            .child(
                text(
                    description.to_owned(),
                    13.,
                    18.5714,
                    400.,
                    t.text.opacity(0.5),
                )
                .truncate(),
            )
    }

    pub(super) fn slash_menu(&self, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let has_skills = self
            .slash
            .matches
            .iter()
            .any(|&index| index >= COMMANDS.len());
        let height = self.slash.matches.len() as f32 * 28.5625 + if has_skills { 30. } else { 0. };
        let mut list = div()
            .id("slash-list")
            .overflow_y_scroll()
            .max_h(px(310.))
            .track_scroll(&self.slash.scroll)
            .when(height > 310., |d| d.pr(px(10.)))
            .flex()
            .flex_col();
        if self.slash.mention {
            list = list.child(self.mention_list(t, cx));
        } else {
            let mut skills_heading = false;
            for (selected, &index) in self.slash.matches.iter().enumerate() {
                if index >= COMMANDS.len() && !skills_heading {
                    list = list.child(
                        text("Skills", 12., 20., 500., t.text.opacity(0.5))
                            .px(px(8.))
                            .py(px(5.)),
                    );
                    skills_heading = true;
                }
                list = list.child(self.suggestion_row(
                    t,
                    selected,
                    index,
                    self.slash_label(index),
                    cx,
                ));
            }
            if self.slash.matches.is_empty() {
                list = list.child(
                    text("No commands", 13., 18.5714, 400., t.text.opacity(0.5))
                        .px(px(8.))
                        .py(px(5.)),
                );
            }
        }
        // Gap to the card: 8 px for `/` (rec 07), 4 px for `@` (s019.json).
        let gap = if self.slash.mention { 4. } else { 8. };
        deferred(
            div()
                .absolute()
                .bottom(relative(1.))
                .mb(px(gap))
                .w_full()
                .child(
                    div()
                        .id("slash-menu")
                        .occlude()
                        .p(px(4.))
                        .border_1()
                        .border_color(t.text.opacity(0.082))
                        .rounded(px(20.))
                        .bg(t.surface_elevated_secondary)
                        .on_mouse_down_out(cx.listener(|this, _, _, cx| this.close_slash(cx)))
                        .child(list),
                ),
        )
        .with_priority(2)
        .into_any_element()
    }
}
