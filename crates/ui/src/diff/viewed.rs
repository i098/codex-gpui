use super::*;

impl DiffPanel {
    pub(super) fn viewed_identity(&self, index: usize) -> Option<(String, String)> {
        if self.source != DiffSource::Branch || self.pending || self.error.is_some() {
            return None;
        }
        let repository = self.repository.as_ref()?;
        repository.before_ref.as_ref()?;
        repository.after_ref.as_ref()?;
        let file = self.files.get(index)?;
        let revision = file
            .raw
            .lines()
            .take_while(|line| !line.starts_with("@@"))
            .filter(|line| {
                line.starts_with("index ")
                    || line.starts_with("old mode ")
                    || line.starts_with("new mode ")
                    || line.starts_with("new file mode ")
                    || line.starts_with("deleted file mode ")
            })
            .collect::<Vec<_>>()
            .join("\n");
        if revision.is_empty() {
            return None;
        }
        let key = serde_json::json!([
            repository.root,
            repository.head_branch,
            repository.base_branch,
            file.path_key
        ])
        .to_string();
        Some((key, revision))
    }

    pub(super) fn apply_viewed_files(&mut self) {
        for index in 0..self.files.len() {
            if let Some((key, revision)) = self.viewed_identity(index)
                && let Some(saved) = self.viewed_files.get(&key)
            {
                self.set_file_open(index, saved.as_str() != Some(&revision));
            }
        }
    }

    pub fn viewed_file_saved(
        &mut self,
        key: String,
        revision: Option<String>,
        cx: &mut Context<Self>,
    ) {
        match revision {
            Some(revision) => {
                self.viewed_files.insert(key, revision.into());
            }
            None => {
                self.viewed_files.remove(&key);
                for index in 0..self.files.len() {
                    if self
                        .viewed_identity(index)
                        .is_some_and(|(identity, _)| identity == key)
                    {
                        self.set_file_open(index, true);
                    }
                }
            }
        }
        self.apply_viewed_files();
        cx.notify();
    }

    fn toggle_viewed(&self, index: usize, cx: &mut Context<Self>) {
        if let Some((key, revision)) = self.viewed_identity(index) {
            let viewed = self.viewed_files.get(&key).and_then(|value| value.as_str())
                == Some(revision.as_str());
            cx.emit(DiffRequest::ViewedFile {
                key,
                revision: (!viewed).then_some(revision),
            });
        }
    }

    pub(super) fn viewed_button(
        &self,
        index: usize,
        t: &Theme,
        cx: &Context<Self>,
    ) -> Stateful<Div> {
        let viewed = self.viewed_identity(index).is_some_and(|(key, revision)| {
            self.viewed_files.get(&key).and_then(|value| value.as_str()) == Some(&revision)
        });
        round_icon(
            if viewed {
                "diff-unviewed"
            } else {
                "diff-viewed"
            },
            20.,
            t,
        )
        .id(("mark-viewed", index))
        .role(gpui::Role::Button)
        .aria_label(if viewed {
            "Mark as unviewed"
        } else {
            "Mark as viewed"
        })
        .aria_toggled(if viewed {
            gpui::Toggled::True
        } else {
            gpui::Toggled::False
        })
        .focusable()
        .tab_index(0)
        .cursor_pointer()
        .focus_visible(|s| s.border_2().border_color(rgb(0x3a83f7)))
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_click(cx.listener(move |this, _, _, cx| {
            this.toggle_viewed(index, cx);
            cx.stop_propagation();
        }))
        .on_key_down(cx.listener(move |this, event: &KeyDownEvent, _, cx| {
            if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                this.toggle_viewed(index, cx);
                cx.stop_propagation();
            }
        }))
    }
}
