use std::sync::{
    Arc, OnceLock,
    atomic::{AtomicBool, Ordering},
};

use anyhow::{Context as _, Result};
use async_channel::Receiver;
use async_lock::Mutex;
use serde_json::{Value, json};

use crate::Bridge;

type Restored = Box<dyn Fn() -> Result<()> + Send + Sync>;

#[derive(Default)]
struct Archive {
    archived: bool,
    restored: Option<Restored>,
}

pub struct AutomationTurn {
    pub muted: bool,
    pub auto_archive: bool,
    id: OnceLock<String>,
    started: Receiver<()>,
    completed: AtomicBool,
    cancelled: AtomicBool,
    archive: Mutex<Archive>,
}

pub struct AutomationCompletion<'a> {
    turn: &'a AutomationTurn,
    committed: bool,
}

impl AutomationCompletion<'_> {
    pub fn commit(mut self) {
        self.committed = true;
    }
}

impl Drop for AutomationCompletion<'_> {
    fn drop(&mut self) {
        if !self.committed {
            self.turn.completed.store(false, Ordering::SeqCst);
        }
    }
}

impl AutomationTurn {
    pub async fn matches_turn(&self, turn_id: &str) -> bool {
        let _ = self.started.recv().await;
        self.id.get().is_some_and(|id| id == turn_id)
    }

    pub fn claim(&self) -> Option<AutomationCompletion<'_>> {
        self.completed
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .ok()
            .map(|_| AutomationCompletion {
                turn: self,
                committed: false,
            })
    }

    pub async fn archived(&self) -> bool {
        self.archive.lock().await.archived
    }
}

impl Bridge {
    pub async fn start_automation_turn(
        &self,
        params: Value,
        muted: bool,
        auto_archive: bool,
    ) -> Result<Value> {
        let thread = params["threadId"].as_str().unwrap_or_default().to_owned();
        let (started, ready) = async_channel::bounded(1);
        let turn = Arc::new(AutomationTurn {
            muted,
            auto_archive,
            id: OnceLock::new(),
            started: ready,
            completed: AtomicBool::new(false),
            cancelled: AtomicBool::new(false),
            archive: Mutex::new(Archive::default()),
        });
        self.automation_turns
            .lock()
            .insert(thread.clone(), turn.clone());
        let result = self
            .request_inner::<_, Value>("turn/start", params)
            .await
            .and_then(|response| {
                let id = response["turn"]["id"]
                    .as_str()
                    .context("turn/start returned no turn")?;
                let _ = turn.id.set(id.to_owned());
                Ok(response)
            });
        if result.is_err() {
            self.release_automation_turn(&thread, &turn);
        }
        drop(started);
        result
    }

    pub fn automation_turn(&self, thread: &str) -> Option<Arc<AutomationTurn>> {
        self.automation_turns.lock().get(thread).cloned()
    }

    pub fn release_automation_turn(&self, thread: &str, turn: &Arc<AutomationTurn>) {
        let mut turns = self.automation_turns.lock();
        if turns
            .get(thread)
            .is_some_and(|current| Arc::ptr_eq(current, turn))
        {
            turns.remove(thread);
        }
    }

    pub async fn archive_automation_turn(
        &self,
        thread: &str,
        turn: &AutomationTurn,
        archived: impl FnOnce() -> Result<()>,
        restored: impl Fn() -> Result<()> + Send + Sync + 'static,
    ) -> Result<bool> {
        let mut archive = turn.archive.lock().await;
        if turn.cancelled.load(Ordering::SeqCst) {
            return Ok(false);
        }
        self.request_inner::<_, Value>("thread/archive", json!({"threadId": thread}))
            .await?;
        archive.archived = true;
        archive.restored = Some(Box::new(restored));
        if let Err(error) = archived() {
            self.restore_automation_archive(thread, &mut archive)
                .await?;
            return Err(error);
        }
        if turn.cancelled.load(Ordering::SeqCst) {
            self.restore_automation_archive(thread, &mut archive)
                .await?;
        }
        Ok(archive.archived)
    }

    pub(crate) async fn observe_automation_request(
        &self,
        method: &str,
        params: &Value,
    ) -> Result<()> {
        if !matches!(
            method,
            "turn/start"
                | "turn/steer"
                | "turn/addUserMessage"
                | "thread/queue/add"
                | "thread/queue/start"
        ) {
            return Ok(());
        }
        let Some(thread) = params["threadId"].as_str() else {
            return Ok(());
        };
        let Some(turn) = self.automation_turn(thread) else {
            return Ok(());
        };
        turn.cancelled.store(true, Ordering::SeqCst);
        let mut archive = turn.archive.lock().await;
        self.restore_automation_archive(thread, &mut archive)
            .await?;
        drop(archive);
        if turn.completed.load(Ordering::SeqCst) {
            self.release_automation_turn(thread, &turn);
        }
        Ok(())
    }

    async fn restore_automation_archive(&self, thread: &str, archive: &mut Archive) -> Result<()> {
        if archive.archived {
            self.request_inner::<_, Value>("thread/unarchive", json!({"threadId": thread}))
                .await?;
            if let Some(restored) = &archive.restored {
                restored()?;
            }
            archive.archived = false;
        }
        Ok(())
    }
}
