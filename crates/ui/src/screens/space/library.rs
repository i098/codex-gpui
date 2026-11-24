//! Space ▸ `Files`: the original's `LibraryPage` (`/library/*`, which the desktop app shows in
//! Space). Files and folders from `/files/library/nodes`, the `Favorites` tab, and `Trash`.

use super::{SpacePage, button};
use crate::{
    confirmation::{Confirmation, Outcome},
    icon::icon,
    text,
};
use gpui::{
    AppContext as _, Context, Div, InteractiveElement, IntoElement, ParentElement, SharedString,
    StatefulInteractiveElement as _, Styled, Window, div, px,
};
use theme::ActiveTheme as _;

/// `libraryNext.tabs.*` and `library.tabs.favorites`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Tab {
    #[default]
    All,
    Folders,
    Favorites,
}

impl Tab {
    pub const ALL: [Tab; 3] = [Tab::All, Tab::Folders, Tab::Favorites];

    pub fn label(self) -> &'static str {
        match self {
            Tab::All => "All",
            Tab::Folders => "Folders",
            Tab::Favorites => "Favorites",
        }
    }
}

/// One Library file or folder.
#[derive(Clone, Debug, PartialEq)]
pub struct Node {
    pub id: String,
    pub name: String,
    pub folder: bool,
    pub parent: Option<String>,
    pub file_id: Option<String>,
    /// `favorite_id` of the item's Favorite; `Some` when it is in Favorites.
    pub favorite_id: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub enum Items {
    #[default]
    Loading,
    Ready(Vec<Node>),
    Failed,
}

/// What the Library shows; the app reads it to build the request.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Library {
    pub tab: Tab,
    /// `Trash` (`trashed_only`) instead of the tab.
    pub trash: bool,
    /// The open folders, outermost first: `(id, name)`.
    pub path: Vec<(String, String)>,
    pub items: Items,
    /// An opened `/library/share/*` link; replaces the list while it is `Some`.
    pub share: Option<Share>,
    pub target: Option<ShareTarget>,
    /// The invitation the person just accepted; the next resolve skips it.
    pub accepted: Option<String>,
    /// The invitation dialog was answered and the request is in flight.
    pub answering: bool,
    /// The last answer failed; the dialog opens again with the error.
    pub answer_failed: bool,
}

/// What a `/library/share/*` link names (`share-link` `o(path, search)`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ShareTarget {
    pub file_id: Option<String>,
    pub folder_id: Option<String>,
    pub invitation_id: Option<String>,
}

/// A Library invitation (`/sharing/invitations/{id}`).
#[derive(Clone, Debug, PartialEq)]
pub struct Invitation {
    pub id: String,
    pub status: String,
    pub folder: bool,
    pub name: Option<String>,
    pub editor: bool,
    /// Sender name and email.
    pub inviter: Option<(String, String)>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Share {
    Loading,
    Invitation(Invitation),
    Unavailable,
}

/// How a share link resolved.
#[derive(Clone, Debug, PartialEq)]
pub enum Resolved {
    Invitation(Invitation),
    /// A folder the person can open.
    Folder(String, String),
    /// A file; the file preview is not built, so Library shows its root.
    File,
    Unavailable,
}

impl Library {
    /// The folder whose children the list shows.
    pub fn folder(&self) -> Option<&str> {
        self.path.last().map(|(id, _)| id.as_str())
    }
}

/// A Library request for the app to send.
#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    /// Read the list that [`SpacePage::library`] describes.
    Load,
    /// `library.favorites.add` or `library.favorites.remove`.
    Favorite(Node),
    /// `libraryNext.actions.delete`: move a file to Trash.
    Delete(Node),
    /// `codex.space.trash.restore`.
    Restore(Node),
    /// `library.emptyTrash.confirm`.
    EmptyTrash,
    /// Resolve [`Library::target`] (a `/library/share/*` link).
    ResolveShare,
    /// `library.share.invitation.accept` or `.decline`.
    Answer {
        invitation: String,
        accept: bool,
        /// `always_accept_from_inviter`.
        always: bool,
    },
}

impl SpacePage {
    fn library_mut(&mut self, cx: &mut Context<Self>, change: impl FnOnce(&mut Library)) {
        if self.library.as_ref().is_some_and(|l| l.share.is_some()) {
            self.confirmation = None;
        }
        let library = self.library.get_or_insert_default();
        change(library);
        library.share = None;
        library.target = None;
        library.items = Items::Loading;
        cx.emit(Action::Load);
        cx.notify();
    }

    /// Reads the open Library list again.
    pub fn reload_library(&mut self, cx: &mut Context<Self>) {
        if self.library.is_some() {
            self.library_mut(cx, |_| {});
        }
    }

    /// `Files`: opens the Library on its `All` tab.
    pub fn open_library(&mut self, cx: &mut Context<Self>) {
        self.library_mut(cx, |library| *library = Library::default());
    }

    /// Opens a `/library/share/*` link in the Library.
    pub fn open_share(&mut self, target: ShareTarget, cx: &mut Context<Self>) {
        self.library = Some(Library {
            share: Some(Share::Loading),
            target: Some(target),
            ..Library::default()
        });
        cx.emit(Action::ResolveShare);
        cx.notify();
    }

    /// The invitation was accepted: resolve the link again, as the original does.
    pub fn invitation_accepted(&mut self, id: String, cx: &mut Context<Self>) {
        if let Some(library) = &mut self.library {
            library.accepted = Some(id);
            library.answering = false;
            library.share = Some(Share::Loading);
            self.confirmation = None;
            cx.emit(Action::ResolveShare);
            cx.notify();
        }
    }

    /// The answer request failed: show the invitation dialog again with the error.
    pub fn answer_failed(&mut self, cx: &mut Context<Self>) {
        if let Some(library) = &mut self.library {
            library.answering = false;
            library.answer_failed = true;
            cx.notify();
        }
    }

    /// `library.share.invitation.*`: the original's compact dialog (`Confirmation` here), shown
    /// while a pending invitation is open and unanswered.
    pub(super) fn share_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(library) = &mut self.library else {
            return;
        };
        let failed = std::mem::take(&mut library.answer_failed);
        let pending = match &library.share {
            Some(Share::Invitation(i)) if i.status == "pending" && !library.answering => i.clone(),
            _ => return,
        };
        if self.confirmation.is_some() {
            return;
        }
        let role = if pending.editor { "edit" } else { "view" };
        let what = match &pending.name {
            Some(name) if !name.trim().is_empty() => name.clone(),
            _ if pending.folder => "this folder".into(),
            _ => "this file".into(),
        };
        let mut note = String::from("Only accept files from people you know");
        if let Some((name, email)) = &pending.inviter {
            note.push_str(&format!("\n{name} ({email})"));
        }
        let trust = pending
            .inviter
            .as_ref()
            .map(|(name, _)| format!("Always accept files and folders from {name}"));
        let id = pending.id;
        let dialog = cx.new(|cx| {
            let mut dialog = Confirmation::new(
                format!("You\u{2019}ve been invited to {role} {what}"),
                note,
                "Accept invite",
                window,
                cx,
            )
            .alternate("Decline", window, cx);
            if let Some(trust) = trust {
                dialog = dialog.with_opt_out(trust, cx);
            }
            if failed {
                dialog.failed(
                    "We couldn\u{2019}t update your invitation. Please try again",
                    window,
                    cx,
                );
            }
            dialog
        });
        cx.subscribe(&dialog, move |this, dialog, outcome, cx| {
            this.confirmation = None;
            match outcome {
                Outcome::Cancel => this.open_library(cx),
                Outcome::Confirm | Outcome::Alternate => {
                    let always = *outcome == Outcome::Confirm && dialog.read(cx).opt_out_checked();
                    if let Some(library) = &mut this.library {
                        library.answering = true;
                    }
                    cx.emit(Action::Answer {
                        invitation: id.clone(),
                        accept: *outcome == Outcome::Confirm,
                        always,
                    });
                }
            }
            cx.notify();
        })
        .detach();
        self.confirmation = Some(dialog);
    }

    /// Shows what the open share link resolved to.
    pub fn set_share(&mut self, resolved: Resolved, cx: &mut Context<Self>) {
        let Some(library) = &mut self.library else {
            return;
        };
        match resolved {
            Resolved::Invitation(invitation) => library.share = Some(Share::Invitation(invitation)),
            Resolved::Unavailable => library.share = Some(Share::Unavailable),
            Resolved::Folder(id, name) => {
                self.library_mut(cx, |library| library.path = vec![(id, name)]);
                return;
            }
            Resolved::File => {
                self.open_library(cx);
                return;
            }
        }
        cx.notify();
    }

    /// Back to the Spaces list.
    pub fn close_library(&mut self, cx: &mut Context<Self>) {
        self.library = None;
        self.confirmation = None;
        cx.notify();
    }

    pub fn set_tab(&mut self, tab: Tab, cx: &mut Context<Self>) {
        self.library_mut(cx, |library| {
            *library = Library {
                tab,
                ..Library::default()
            }
        });
    }

    pub fn open_folder(&mut self, node: &Node, cx: &mut Context<Self>) {
        let entry = (node.id.clone(), node.name.clone());
        self.library_mut(cx, |library| library.path.push(entry));
    }

    /// A breadcrumb: keeps the first `depth` open folders (0 is the Library root).
    pub fn open_crumb(&mut self, depth: usize, cx: &mut Context<Self>) {
        self.library_mut(cx, |library| {
            library.trash = false;
            library.path.truncate(depth);
        });
    }

    pub fn open_trash(&mut self, cx: &mut Context<Self>) {
        self.library_mut(cx, |library| {
            *library = Library {
                trash: true,
                ..Library::default()
            }
        });
    }

    pub fn set_items(&mut self, items: Items, cx: &mut Context<Self>) {
        if let Some(library) = &mut self.library {
            library.items = items;
            cx.notify();
        }
    }

    /// `library.emptyTrash.action`: asks before it deletes every file in Trash.
    pub fn confirm_empty_trash(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let confirmation = cx.new(|cx| {
            Confirmation::new(
                "Delete forever?",
                "Files in Trash will be deleted forever. This can't be undone.",
                "Delete forever",
                window,
                cx,
            )
            .destructive()
        });
        cx.subscribe(&confirmation, |this, _, outcome, cx| {
            this.confirmation = None;
            if *outcome == Outcome::Confirm {
                cx.emit(Action::EmptyTrash);
            }
            cx.notify();
        })
        .detach();
        self.confirmation = Some(confirmation);
        cx.notify();
    }

    pub(super) fn library_header(
        &self,
        library: &Library,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let t = cx.theme().clone();
        let mut crumbs = vec![
            (SharedString::from("Space"), None),
            ("Library".into(), Some(0)),
        ];
        crumbs.extend(
            library
                .path
                .iter()
                .enumerate()
                .map(|(i, (_, name))| (SharedString::from(name.clone()), Some(i + 1))),
        );
        if library.trash {
            crumbs.push(("Trash".into(), Some(usize::MAX)));
        }
        let last = crumbs.len() - 1;
        let mut trail = div().flex().items_center().gap(px(4.)).min_w_0();
        for (i, (label, depth)) in crumbs.into_iter().enumerate() {
            if i > 0 {
                trail = trail.child(icon("chevron-right", 16., t.text_tertiary));
            }
            trail = trail.child(if i == last {
                text(label, 24., 28.8, 500., t.text)
                    .truncate()
                    .into_any_element()
            } else {
                button(
                    format!("library-crumb-{i}"),
                    label,
                    window,
                    cx,
                    move |page, _, cx| match depth {
                        None => page.close_library(cx),
                        Some(depth) => page.open_crumb(depth, cx),
                    },
                )
                .into_any_element()
            });
        }
        let tabs: Vec<_> = Tab::ALL
            .into_iter()
            .map(|tab| {
                let current = !library.trash && library.path.is_empty() && library.tab == tab;
                let tab_button = button(
                    format!("library-tab-{}", tab.label()),
                    tab.label(),
                    window,
                    cx,
                    move |page, _, cx| page.set_tab(tab, cx),
                )
                .role(gpui::Role::Tab)
                .aria_selected(current);
                if current {
                    tab_button.bg(t.ghost_hover)
                } else {
                    tab_button
                }
            })
            .collect();
        let side = if library.trash {
            button(
                "library-empty-trash",
                "Empty file trash",
                window,
                cx,
                |page, window, cx| page.confirm_empty_trash(window, cx),
            )
        } else {
            button("library-trash", "Trash", window, cx, |page, _, cx| {
                page.open_trash(cx)
            })
        };
        div()
            .flex()
            .flex_col()
            .gap(px(16.))
            .mb(px(24.))
            .child(
                div()
                    .h(px(32.))
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap(px(16.))
                    .child(trail)
                    .child(side),
            )
            .child(if library.trash {
                text(
                    "Restore files you've recently deleted. Files are deleted forever after 30 days.",
                    14.,
                    20.,
                    400.,
                    t.text_secondary,
                )
                .into_any_element()
            } else {
                div()
                    .id("library-tabs")
                    .flex()
                    .gap(px(4.))
                    .role(gpui::Role::TabList)
                    .aria_label("Library sections")
                    .children(tabs)
                    .into_any_element()
            })
    }

    pub(super) fn library_body(
        &self,
        library: &Library,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let t = cx.theme().clone();
        let message = |s: &'static str| text(s, 14., 20., 400., t.text_secondary);
        if let Some(share) = &library.share {
            return self.share_body(share, window, cx);
        }
        let nodes = match &library.items {
            Items::Loading => return div(),
            Items::Failed => {
                return div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(message(
                        if library.tab == Tab::Favorites && !library.trash {
                            "Failed to load favorites"
                        } else {
                            "Failed to load files."
                        },
                    ))
                    .child(button(
                        "library-retry",
                        "Try again",
                        window,
                        cx,
                        |_, _, cx| cx.emit(Action::Load),
                    ));
            }
            Items::Ready(nodes) if nodes.is_empty() => {
                return message(if library.trash {
                    "No files found"
                } else if !library.path.is_empty() {
                    "This folder is empty."
                } else {
                    match library.tab {
                        Tab::All => "Create something new",
                        Tab::Folders => "Create your first folder",
                        Tab::Favorites => "No favorites yet",
                    }
                });
            }
            Items::Ready(nodes) => nodes,
        };
        let rows: Vec<_> = nodes
            .iter()
            .enumerate()
            .map(|(i, node)| {
                let actions: Vec<(&'static str, Action)> = if library.trash {
                    vec![("Restore", Action::Restore(node.clone()))]
                } else if node.folder {
                    Vec::new()
                } else {
                    vec![
                        (
                            if node.favorite_id.is_some() {
                                "Remove from Favorites"
                            } else {
                                "Add to Favorites"
                            },
                            Action::Favorite(node.clone()),
                        ),
                        ("Delete", Action::Delete(node.clone())),
                    ]
                };
                let name = text(node.name.clone(), 14., 20., 400., t.text).truncate();
                let name = if node.folder && !library.trash {
                    let node = node.clone();
                    button(
                        format!("library-open-{i}"),
                        node.name.clone(),
                        window,
                        cx,
                        move |page, _, cx| page.open_folder(&node, cx),
                    )
                    .into_any_element()
                } else {
                    name.into_any_element()
                };
                div()
                    .id(SharedString::from(format!("library-row-{}", node.id)))
                    .h(px(44.))
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .px(px(12.))
                    .border_b_1()
                    .border_color(t.border_subtle)
                    .child(icon(
                        if node.folder {
                            "folder"
                        } else {
                            "diff-file-default"
                        },
                        16.,
                        t.text_secondary,
                    ))
                    .child(div().flex_1().min_w_0().child(name))
                    .children(actions.into_iter().enumerate().map(|(j, (label, action))| {
                        button(
                            format!("library-action-{i}-{j}"),
                            label,
                            window,
                            cx,
                            move |_, _, cx| cx.emit(action.clone()),
                        )
                    }))
            })
            .collect();
        div().child(
            div()
                .id("library-list")
                .flex()
                .flex_col()
                .role(gpui::Role::List)
                .aria_label("Library files")
                .children(rows),
        )
    }

    /// The original's `/library/share/*` page: an invitation card or the unavailable state.
    fn share_body(&self, share: &Share, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let t = cx.theme().clone();
        let heading = |s: String| text(s, 20., 28., 500., t.text);
        let note = |s: String| text(s, 14., 20., 400., t.text_secondary);
        let back = |window: &mut Window, cx: &mut Context<Self>| {
            button(
                "library-share-back",
                "Back to Library",
                window,
                cx,
                |page, _, cx| page.open_library(cx),
            )
        };
        let card = div()
            .flex()
            .flex_col()
            .items_start()
            .gap(px(12.))
            .py(px(24.));
        match share {
            Share::Loading => div(),
            Share::Unavailable => card
                .child(heading("Unable to open this shared item".into()))
                .child(note(
                    "Ask the owner to share it with you, or switch to an account with access"
                        .into(),
                ))
                .child(
                    div()
                        .flex()
                        .gap(px(8.))
                        .child(button(
                            "library-share-retry",
                            "Try again",
                            window,
                            cx,
                            |_, _, cx| cx.emit(Action::ResolveShare),
                        ))
                        .child(back(window, cx)),
                ),
            Share::Invitation(i) if i.status == "accepted" => {
                let id = i.id.clone();
                card.child(heading("This invitation was already accepted".into()))
                    .child(button(
                        "library-share-open",
                        "Open in ChatGPT",
                        window,
                        cx,
                        move |page, _, cx| page.invitation_accepted(id.clone(), cx),
                    ))
            }
            Share::Invitation(i) if i.status != "pending" => card
                .child(heading("This invitation is no longer available".into()))
                .child(back(window, cx)),
            // The invitation itself is the dialog [`Self::share_dialog`] opens.
            Share::Invitation(_) => div(),
        }
    }
}
