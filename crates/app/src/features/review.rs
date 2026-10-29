//! `Code review` (`/review-mode`, the CLI's `/review`): opening the submenu reads the open
//! chat's branches and commits (host `git-review-targets`); a pick runs `review/start` inline on
//! the open chat, resumed first when this app-server has not loaded it. The review turn then
//! streams like any turn, so the composer's stop control interrupts it (`turn/interrupt`).
//! A refused request shows the original's `Failed to start code review`.

use crate::*;
use ui::composer::ReviewEvent;

impl Root {
    pub(crate) fn review_event(
        &mut self,
        _: &Entity<Composer>,
        event: &ReviewEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(view) = &self.thread else { return };
        let view = view.read(cx);
        let id = view.id().to_owned();
        match event {
            ReviewEvent::LoadTargets => {
                let (host, cwd) = (self.host.clone(), view.cwd().to_owned());
                let read = cx.background_spawn(async move {
                    git_call(host.as_ref(), "git-review-targets", json!({"cwd": cwd}))
                });
                cx.spawn(async move |this, cx| {
                    let targets = read.await;
                    this.update(cx, |this, cx| {
                        let open = this.thread.as_ref().map(|view| view.read(cx).id() == id);
                        if open == Some(true) {
                            this.thread_composer.update(cx, |composer, cx| {
                                composer.set_review_targets(targets.map_err(drop), cx)
                            });
                        }
                    })
                    .ok();
                })
                .detach();
            }
            ReviewEvent::Start(target) => {
                let resume = !self.loaded.contains(&id);
                let bridge = self.bridge.clone();
                let params = json!({"threadId": id, "target": target});
                let task = cx.spawn_in(window, async move |this, cx| -> Result<()> {
                    let started = async {
                        resume_if(resume, &bridge, &this, &id, cx).await?;
                        bridge.request::<_, Value>("review/start", params).await
                    }
                    .await;
                    // Only the reply's turn carries the review hint (`current changes`) as its
                    // user message; the notifications that follow carry the internal prompt.
                    this.update(cx, |this, cx| match &started {
                        Ok(reply) => this.on_event(
                            Event::Notification {
                                method: "turn/started".into(),
                                params: json!({"threadId": id, "turn": reply["turn"]}),
                            },
                            cx,
                        ),
                        Err(_) => this.toast(Toast::danger("Failed to start code review"), cx),
                    })?;
                    started.map(drop)
                });
                log_err(task, cx);
            }
        }
    }
}
