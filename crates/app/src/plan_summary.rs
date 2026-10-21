use crate::{Root, Route, SidePanel};
use gpui::{AppContext as _, Context, Entity, Window};
use ui::screens::plan_summary::{DownloadPlan, OpenPlan, PlanSummary, download};

#[derive(Default)]
pub(crate) struct State {
    pub page: Option<Entity<PlanSummary>>,
    pub panel: Option<Entity<PlanSummary>>,
}

impl Root {
    pub(crate) fn sync_plan_route(&mut self, cx: &mut Context<Self>) {
        if let (Route::PlanSummary(plan), Some(page)) = (&self.state.route, &self.plans.page)
            && page.read(cx).plan() == plan
        {
            return;
        }
        self.plans.page = match &self.state.route {
            Route::PlanSummary(plan) => Some(cx.new(|_| PlanSummary::page(plan.clone()))),
            _ => None,
        };
    }

    pub(crate) fn sync_plan_preview(&self, cx: &mut Context<Self>) {
        if let Some(thread) = &self.thread {
            let id = self
                .plans
                .panel
                .as_ref()
                .filter(|_| self.side == Some(SidePanel::Plan))
                .map(|panel| panel.read(cx).plan().item_id.clone());
            thread.update(cx, |thread, cx| thread.set_open_plan(id, cx));
        }
    }

    pub(crate) fn open_plan(
        &mut self,
        action: &OpenPlan,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(thread) = &self.thread else { return };
        if thread.read(cx).id() != action.0.conversation_id
            || !thread.read(cx).plan_completed(&action.0.item_id)
        {
            return;
        }
        let Some(content) = thread
            .read(cx)
            .turns()
            .iter()
            .flat_map(|turn| &turn.items)
            .find_map(|item| match item {
                bridge::types::ThreadItem::Plan { id, text } if id == &action.0.item_id => {
                    Some(text.clone())
                }
                _ => None,
            })
        else {
            return;
        };
        let plan = ui::screens::plan_summary::Plan {
            content,
            ..action.0.clone()
        };
        let same = self.plans.panel.as_ref().is_some_and(|panel| {
            let current = panel.read(cx).plan();
            current.conversation_id == plan.conversation_id && current.item_id == plan.item_id
        });
        if same && self.side == Some(SidePanel::Plan) {
            self.close_side(SidePanel::Plan, window, cx);
            return;
        }
        self.plans.panel = Some(cx.new(|_| PlanSummary::panel(plan)));
        if self.side == Some(SidePanel::Plan) {
            self.sync(window, cx);
        } else {
            self.set_side(SidePanel::Plan, window, cx);
        }
    }

    pub(crate) fn download_plan(
        &mut self,
        action: &DownloadPlan,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let task = download(action.0.clone(), cx);
        cx.spawn(async move |this, cx| {
            if let Err(error) = task.await {
                this.update(cx, |this, cx| {
                    this.toaster.update(cx, |toaster, cx| {
                        toaster.push(
                            ui::toast::Toast::danger(format!("Could not download plan: {error}")),
                            cx,
                        );
                    });
                })
                .ok();
            }
        })
        .detach();
    }
}
