use super::*;

impl Composer {
    pub(super) fn add_files(&mut self, cx: &mut Context<Self>) {
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: true,
            prompt: None,
        });
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(Some(paths))) = paths.await {
                this.update(cx, |this, cx| {
                    this.attachments.extend(paths);
                    cx.notify();
                })
                .ok();
            }
        })
        .detach();
    }

    pub(super) fn attachments_row(&self, t: &Theme, cx: &mut Context<Self>) -> Div {
        div()
            .pt(px(8.))
            .px(px(8.))
            .pb(px(6.))
            .flex()
            .flex_wrap()
            .gap(px(6.))
            .children(self.attachments.iter().enumerate().map(|(i, path)| {
                let name = path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                div()
                    .h(px(28.))
                    .pl(px(8.))
                    .pr(px(2.))
                    .flex()
                    .items_center()
                    .gap(px(4.))
                    .relative()
                    .child(shapes::decoration(deco(10., t.text_alpha_5, vec![])))
                    .child(label(name, t.text))
                    .child(
                        pill(("remove-attachment", i), t)
                            .size(px(22.))
                            .justify_center()
                            .child(icon("close", 12., t.text_tertiary))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.attachments.remove(i);
                                cx.notify();
                            })),
                    )
            }))
    }
}
