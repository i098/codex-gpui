//! Slash completion keeps keyboard focus in the editor and preserves text outside the command token.
//! Chat commands and MCP use existing app handlers; skill rows insert linked mention chips.
//! Model, Reasoning, Fork chat, and Work in a project open a submenu. It replaces the token with
//! `/id ` and filters on the text typed after it; Backspace over the space returns to the list,
//! Escape closes it, and only a pick removes the token and its query and acts.
//! Goal uses the query as an objective; its submenu also shows the stored goal and lifecycle actions.
//! Thread memory offers enable, disable, and status actions without changing global settings.
//! The other commands stay visible but inactive.
//! The same list state serves the `@` mention trigger (`super::mention`).
//! `cargo test -p app --test s021` replays the original menu, keys, colors, focus, and requests.
use super::*;
use crate::screens::sidebar::SelectProject;

#[derive(Clone)]
pub enum SlashCommand {
    Fast,
    Archive,
    NewChat,
    Usage,
    Compact,
    Approve,
    Rename,
    Pin,
    Fork,
    Mcp,
    ForkWorktree,
    Feedback,
    Export,
    Recap,
    Goal {
        objective: Option<String>,
        status: &'static str,
    },
    ReadGoal,
    RefreshGoal,
    ClearGoal,
    MemoryMode(&'static str),
    MemoryStatus,
    Memories,
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
        id: "approve",
        title: "Approve",
        description: "Allow one retry of the last denied action",
        icon: "check",
        requires_empty: true,
    },
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
        id: "export",
        title: "Export",
        description: "Export this conversation as Markdown",
        icon: "sidebar-cmd-create-page",
        requires_empty: false,
    },
    Command {
        id: "recap",
        title: "Recap",
        description: "Summarize this conversation",
        icon: "thread-summary",
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
        icon: "slash-goal",
        requires_empty: false,
    },
    Command {
        id: "memory",
        title: "Thread memory",
        description: "Choose memory behavior for this chat",
        icon: "settings-personalization",
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
        icon: "slash-mcp",
        requires_empty: false,
    },
    Command {
        id: "memories",
        title: "Memories",
        description: "",
        icon: "project-marker-brain",
        requires_empty: false,
    },
    Command {
        id: "model",
        title: "Model",
        description: "",
        icon: "diff-hide-imports",
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
        icon: "slash-pet",
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
        icon: "slash-plan-mode",
        requires_empty: false,
    },
    Command {
        id: "reasoning",
        title: "Reasoning",
        description: "",
        icon: "project-marker-brain",
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
        icon: "slash-sketch",
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
        icon: "slash-ultrafast",
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
    /// `@` or `+` opened the list (else `/`).
    pub(super) mention: bool,
    /// The `+` query starts at this cursor position without a literal `@`.
    pub(super) add: Option<usize>,
    /// Preserve the trigger state across the popup's capture-phase outside dismissal.
    pub(super) add_mouse_open: bool,
    pub(super) range: Range<usize>,
    pub(super) dismissed: Option<usize>,
    pub(super) selected: usize,
    pub(super) matches: Vec<usize>,
    pub(super) skills: Vec<Skill>,
    pub(super) sub: Option<Sub>,
    /// Home's projects for `/project`: label and root path.
    pub(super) projects: Vec<(SharedString, String)>,
    scroll: gpui::ScrollHandle,
    goal: Option<Value>,
    pub(super) search: mention::Search,
    /// The `memories` feature is on, so `/memories` is offered.
    pub(super) memories: bool,
}

impl Slash {
    pub(super) fn reset(&mut self) {
        self.open = false;
        self.add = None;
        self.add_mouse_open = false;
        self.dismissed = None;
        self.matches.clear();
        self.sub = None;
    }
}

fn matches_query(text: &str, query: &str) -> bool {
    query.is_empty()
        || text
            .as_bytes()
            .windows(query.len())
            .any(|part| part.eq_ignore_ascii_case(query.as_bytes()))
}

/// One submenu row. `action` runs when it is picked.
pub(super) struct SubItem {
    title: String,
    description: String,
    icon: &'static str,
    action: SubAction,
}

#[derive(Clone)]
enum SubAction {
    Model(usize),
    Effort(String),
    Fork,
    ForkWorktree,
    Project(String),
    Command(SlashCommand),
}

/// The open submenu of command `id`. `items` are all its rows; `Slash::matches` indexes the
/// rows that the typed query keeps.
pub(super) struct Sub {
    id: &'static str,
    items: Vec<SubItem>,
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
        let selected = self.slash.selected;
        self.refresh_slash(cx);
        self.slash.selected = selected.min(self.slash.matches.len().saturating_sub(1));
    }

    /// The token before the cursor: `@` or `/` at the start or after whitespace (an `@` query
    /// allows spaces, newlines not; a `/` query allows no whitespace).
    pub(super) fn refresh_slash(&mut self, cx: &mut Context<Self>) {
        if self.plain_field.is_some() {
            self.field_autoscroll = self.plain_field == Some(true);
            return;
        }
        self.field_autoscroll = self.field_scroll.is_some();
        let cursor = self.cursor();
        if let Some(id) = self.slash.sub.as_ref().map(|sub| sub.id) {
            let start = self.slash.range.start;
            let open = self.content[..cursor].get(start..).is_some_and(|text| {
                text.strip_prefix('/')
                    .and_then(|text| text.strip_prefix(id))
                    .is_some_and(|text| text.starts_with(' '))
                    && !text.contains('\n')
            });
            if open {
                self.slash.range.end = cursor;
                self.refresh_sub(cx);
                return;
            }
            self.slash.sub = None;
        }
        let before = &self.content[..cursor];
        self.slash.add = self.slash.add.filter(|&start| {
            self.slash.open
                && before
                    .get(start..)
                    .is_some_and(|query| !query.contains('\n'))
        });
        let mention = before.rfind('@').filter(|&at| {
            (at == 0 || before[..at].ends_with(char::is_whitespace))
                && !before[at + 1..].contains('\n')
        });
        let slash = before
            .rfind('/')
            .filter(|&at| at == 0 || before[..at].ends_with(char::is_whitespace))
            .filter(|&at| !before[at + 1..].contains(char::is_whitespace));
        let (mention, start) = match (self.slash.add, mention, slash) {
            (Some(start), _, _) => (true, start),
            (None, Some(at), _) => (true, at),
            (None, None, Some(at)) => (false, at),
            (None, None, None) => {
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
        let empty = self.content[..range.start].trim().is_empty()
            && self.content[range.end..].trim().is_empty()
            && self.attachments.is_empty();
        self.slash.matches.clear();
        if mention {
            let start = range.start + usize::from(self.slash.add.is_none());
            let query = self.content[start..range.end].to_owned();
            self.refresh_mention(&query, cx);
        } else {
            let query = self.content[range.start + 1..range.end].to_lowercase();
            for (index, command) in COMMANDS.iter().enumerate() {
                let available = match command.id {
                    "archive" | "review-mode" | "compact" | "fork" | "init" | "new" | "pin"
                    | "rename" | "share" | "side" | "goal" | "memory" => self.docked,
                    "project" => !self.docked,
                    "plan-mode" => self.current_model().is_some() && self.plan_supported(),
                    "fast" | "model" | "reasoning" | "ultrafast" => self.current_model().is_some(),
                    "memories" => self.slash.memories,
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
        self.slash.dismissed = self.slash.add.is_none().then_some(self.slash.range.start);
        self.slash.add = None;
        self.slash.sub = None;
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
            let heading = usize::from(self.is_skill(self.slash.matches[self.slash.selected]));
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
        if let Some(sub) = &self.slash.sub {
            let item = &sub.items[index];
            return (&item.title, &item.description, item.icon);
        }
        if let Some(command) = COMMANDS.get(index) {
            let description = match command.id {
                "reasoning" => effort_label(self.displayed_effort()),
                "plan-mode" if self.plan => "Turn plan mode off",
                _ => command.description,
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
        if self.slash.sub.is_some() {
            self.pick_sub(index, window, cx);
            return;
        }
        let Some(command) = COMMANDS.get(index) else {
            self.activate_slash_skill(index - COMMANDS.len(), cx);
            return;
        };
        let action = match command.id {
            "fast" => SlashCommand::Fast,
            "archive" => SlashCommand::Archive,
            "new" => SlashCommand::NewChat,
            "usage" => SlashCommand::Usage,
            "compact" => SlashCommand::Compact,
            "approve" => SlashCommand::Approve,
            "rename" => SlashCommand::Rename,
            "pin" => SlashCommand::Pin,
            "mcp" => SlashCommand::Mcp,
            "feedback" => SlashCommand::Feedback,
            "export" => SlashCommand::Export,
            "recap" => SlashCommand::Recap,
            "memories" => SlashCommand::Memories,
            "model" | "reasoning" | "fork" | "project" | "goal" | "memory" => {
                self.open_sub(command.id, cx);
                return;
            }
            "plan-mode" => {
                self.finish_command(window, cx);
                self.toggle_plan(cx);
                return;
            }
            "ultrafast" => {
                self.finish_command(window, cx);
                self.speed = if self.speed == 2 { 0 } else { 2 };
                cx.notify();
                return;
            }
            _ => return,
        };
        self.finish_command(window, cx);
        if matches!(action, SlashCommand::Fast) {
            self.speed = usize::from(self.speed == 0);
        }
        cx.emit(action);
        cx.notify();
    }

    /// Removes the command token (and a submenu's query) and returns focus to the editor.
    fn finish_command(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.cancel_dictation(cx);
        self.splice(self.slash.range.clone(), "");
        self.selected_range = self.slash.range.start..self.slash.range.start;
        self.marked_range = None;
        self.slash.open = false;
        self.slash.dismissed = None;
        self.slash.sub = None;
        self.focus_handle.focus(window, cx);
    }

    fn is_skill(&self, index: usize) -> bool {
        self.slash.sub.is_none() && index >= COMMANDS.len()
    }

    /// The projects `/project` offers on Home: label and root path.
    pub fn set_slash_projects(&mut self, projects: Vec<(SharedString, String)>) {
        self.slash.projects = projects;
    }

    /// Offers `/memories` while the app-server's `memories` feature is on.
    pub fn set_memories_available(&mut self, available: bool) {
        self.slash.memories = available;
    }

    fn sub_items(&self, id: &str) -> Vec<SubItem> {
        let current = |is_current: bool| if is_current { "Current" } else { "" };
        match id {
            "model" => self
                .models
                .iter()
                .enumerate()
                .map(|(index, model)| SubItem {
                    title: model.label.to_string(),
                    description: current(index == self.model).into(),
                    icon: "composer-model",
                    action: SubAction::Model(index),
                })
                .collect(),
            "reasoning" => self
                .current_model()
                .into_iter()
                .flat_map(|model| &model.efforts)
                .map(|effort| SubItem {
                    title: effort_label(effort).into(),
                    description: current(effort == self.displayed_effort()).into(),
                    icon: "settings-personalization",
                    action: SubAction::Effort(effort.clone()),
                })
                .collect(),
            "goal" => {
                let goal = self.slash.goal.as_ref();
                let objective = goal.and_then(|goal| goal["objective"].as_str());
                let status = goal
                    .and_then(|goal| goal["status"].as_str())
                    .unwrap_or("No goal");
                [
                    (
                        "Set goal",
                        "Type the objective after /goal",
                        SlashCommand::Goal {
                            objective: None,
                            status: "active",
                        },
                    ),
                    (
                        "Show goal",
                        objective.unwrap_or("No goal"),
                        SlashCommand::ReadGoal,
                    ),
                    (
                        "Pause goal",
                        status,
                        SlashCommand::Goal {
                            objective: None,
                            status: "paused",
                        },
                    ),
                    (
                        "Resume goal",
                        status,
                        SlashCommand::Goal {
                            objective: None,
                            status: "active",
                        },
                    ),
                    (
                        "Clear goal",
                        "Remove this chat's goal",
                        SlashCommand::ClearGoal,
                    ),
                ]
                .into_iter()
                .map(|(title, description, action)| SubItem {
                    title: title.into(),
                    description: description.into(),
                    icon: "thread-summary",
                    action: SubAction::Command(action),
                })
                .collect()
            }
            "memory" => [
                ("Enable thread memory", SlashCommand::MemoryMode("enabled")),
                (
                    "Disable thread memory",
                    SlashCommand::MemoryMode("disabled"),
                ),
                ("Memory status", SlashCommand::MemoryStatus),
            ]
            .into_iter()
            .map(|(title, action)| SubItem {
                title: title.into(),
                description: String::new(),
                icon: "settings-personalization",
                action: SubAction::Command(action),
            })
            .collect(),
            "fork" => vec![
                SubItem {
                    title: "Fork chat".into(),
                    description: "Fork this chat in the current workspace".into(),
                    icon: "thread-fork",
                    action: SubAction::Fork,
                },
                SubItem {
                    title: "Fork into new worktree".into(),
                    description: "Fork this chat in an isolated checkout".into(),
                    icon: "thread-fork",
                    action: SubAction::ForkWorktree,
                },
            ],
            _ => self
                .slash
                .projects
                .iter()
                .map(|(label, root)| SubItem {
                    title: label.to_string(),
                    description: root.clone(),
                    icon: "folder",
                    action: SubAction::Project(root.clone()),
                })
                .collect(),
        }
    }

    /// Enter replaces the command token with `/id ` and keeps the remaining draft.
    /// Text filters the rows; Goal uses it as the objective.
    fn open_sub(&mut self, id: &'static str, cx: &mut Context<Self>) {
        self.cancel_dictation(cx);
        if id == "goal" {
            self.slash.goal = None;
        }
        let start = self.slash.range.start;
        let token = format!("/{id} ");
        self.splice(self.slash.range.clone(), &token);
        let end = start + token.len();
        self.selected_range = end..end;
        self.selection_reversed = false;
        self.marked_range = None;
        self.slash.range = start..end;
        self.slash.sub = Some(Sub {
            id,
            items: self.sub_items(id),
        });
        self.refresh_sub(cx);
        if id == "goal" {
            cx.emit(SlashCommand::RefreshGoal);
        }
    }

    fn refresh_sub(&mut self, cx: &mut Context<Self>) {
        let Some(sub) = &self.slash.sub else {
            return;
        };
        let query = self.content[self.slash.range.start + 2 + sub.id.len()..self.slash.range.end]
            .to_lowercase();
        self.slash.matches = sub
            .items
            .iter()
            .enumerate()
            .filter(|(_, item)| sub.id == "goal" || matches_query(&item.title, &query))
            .map(|(index, _)| index)
            .collect();
        self.slash.open = true;
        self.slash.selected = 0;
        self.slash.scroll.set_offset(point(px(0.), px(0.)));
        self.menu = None;
        cx.notify();
    }

    pub fn set_goal(&mut self, goal: Option<Value>, cx: &mut Context<Self>) {
        self.slash.goal = goal;
        if self.slash.sub.as_ref().is_some_and(|sub| sub.id == "goal") {
            let items = self.sub_items("goal");
            self.slash.sub.as_mut().unwrap().items = items;
        }
        cx.notify();
    }

    fn pick_sub(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(mut action) = self
            .slash
            .sub
            .as_ref()
            .and_then(|sub| sub.items.get(index))
            .map(|item| item.action.clone())
        else {
            return;
        };
        if index == 0
            && let Some(sub) = &self.slash.sub
            && sub.id == "goal"
        {
            let objective = self.content
                [self.slash.range.start + 2 + sub.id.len()..self.slash.range.end]
                .trim()
                .to_owned();
            if objective.is_empty() {
                return;
            }
            action = SubAction::Command(SlashCommand::Goal {
                objective: Some(objective),
                status: "active",
            });
        }
        if index != 0
            && let Some(sub) = &self.slash.sub
            && sub.id == "goal"
        {
            self.slash.range.end = self.slash.range.start + 2 + sub.id.len();
        }
        self.finish_command(window, cx);
        match action {
            SubAction::Model(model) => {
                self.select_model(model);
                self.picked(cx);
            }
            SubAction::Effort(effort) => {
                self.effort = Some(effort);
                self.picked(cx);
            }
            SubAction::Fork => cx.emit(SlashCommand::Fork),
            SubAction::ForkWorktree => cx.emit(SlashCommand::ForkWorktree),
            SubAction::Command(command) => cx.emit(command),
            SubAction::Project(root) => {
                window.dispatch_action(Box::new(SelectProject(root)), cx);
            }
        }
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
        let command = COMMANDS
            .get(index)
            .filter(|_| !self.slash.mention && self.slash.sub.is_none());
        // The original authors these two glyphs' colors; the others follow the text color.
        let icon_color = match command.map(|command| command.id) {
            Some("mcp") => gpui::rgb(0x222326).into(),
            Some("sketch") => gpui::rgb(0x04bb9f).into(),
            _ => t.text,
        };
        let submenu = command.is_some_and(|command| {
            matches!(
                command.id,
                "mcp" | "model" | "reasoning" | "fork" | "project"
            )
        });
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
            .child(icon(icon_name, 16., icon_color))
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
            .when(submenu, |row| {
                row.child(
                    div()
                        .ml_auto()
                        .child(icon("chevron-right", 16., t.text_tertiary)),
                )
            })
    }

    pub(super) fn slash_menu(&self, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let has_skills = self.slash.matches.iter().any(|&index| self.is_skill(index));
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
                if self.is_skill(index) && !skills_heading {
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
                    text(
                        if self.slash.sub.is_some() {
                            "No results"
                        } else {
                            "No commands"
                        },
                        13.,
                        18.5714,
                        400.,
                        t.text.opacity(0.5),
                    )
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
