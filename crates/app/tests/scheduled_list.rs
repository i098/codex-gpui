//! Scheduled task list (issue 281): a cold `?automationId=` link selects its task, the
//! selected cron task lists its previous runs, and search, `Filter by` and the load states
//! act on the list, through the real `Root` on a local host.
#[path = "common/mod.rs"]
mod common;

use anyhow::{Result, ensure};
use app::host::LocalHost;
use app::{Root, Route};
use common::Fx;
use rusqlite::Connection;
use std::{fs, path::Path, sync::Arc};
use ui::screens::sidebar::ListState;

/// The header's `Search scheduled tasks` and `Filter tasks` buttons.
const SEARCH: (f32, f32) = (306., 68.);
const FILTER: (f32, f32) = (272., 68.);
/// The first run row under the selected first task.
const FIRST_RUN: (f32, f32) = (150., 196.);

fn definition(id: &str, name: &str, status: &str, updated: i64) -> String {
    format!(
        "version = 1\nid = \"{id}\"\nkind = \"cron\"\nname = \"{name}\"\nprompt = \"p\"\n\
         status = \"{status}\"\nrrule = \"FREQ=DAILY;BYHOUR=9\"\ncwds = [\"/tmp/project\"]\n\
         created_at = 10\nupdated_at = {updated}\n"
    )
}

/// `Alpha` (active, four runs) above `Beta` (paused).
fn host(home: &Path) -> Result<LocalHost> {
    fs::create_dir_all(home.join("sqlite"))?;
    let db = Connection::open(home.join("sqlite/codex.db"))?;
    db.execute_batch(
        "CREATE TABLE automations (id TEXT PRIMARY KEY, name TEXT, prompt TEXT,
            next_run_at INTEGER, next_run_nominal_at INTEGER, last_run_at INTEGER, rrule TEXT,
            status TEXT, account_id TEXT, user_id TEXT, installation_id TEXT);
         CREATE TABLE automation_runs (thread_id TEXT PRIMARY KEY, automation_id TEXT,
            status TEXT, read_at INTEGER, thread_title TEXT, source_cwd TEXT, inbox_title TEXT,
            inbox_summary TEXT, archived_user_message TEXT, archived_assistant_message TEXT,
            archived_reason TEXT, created_at INTEGER, updated_at INTEGER);
         INSERT INTO automations VALUES
            ('alpha', 'Alpha', 'p', 1, 1, 1, 'FREQ=DAILY;BYHOUR=9', 'ACTIVE', NULL, NULL, NULL),
            ('beta', 'Beta', 'p', 1, 1, 1, 'FREQ=DAILY;BYHOUR=9', 'PAUSED', NULL, NULL, NULL);
         INSERT INTO automation_runs (thread_id, automation_id, status, created_at) VALUES
            ('run-1', 'alpha', 'ACCEPTED', 1000), ('run-2', 'alpha', 'ACCEPTED', 2000),
            ('run-3', 'alpha', 'ACCEPTED', 3000), ('run-4', 'alpha', 'PENDING_REVIEW', 4000),
            ('run-old', 'alpha', 'ARCHIVED', 5000);",
    )?;
    let host = LocalHost::new(home);
    host.write_automation("alpha", &definition("alpha", "Alpha", "ACTIVE", 30))?;
    host.write_automation("beta", &definition("beta", "Beta", "PAUSED", 20))?;
    Ok(host)
}

fn visible(fx: &mut Fx) -> Vec<String> {
    fx.read(|r, cx| {
        r.scheduled_sidebar()
            .map(|s| s.read(cx).visible_ids(cx))
            .unwrap_or_default()
    })
}

fn set_host(fx: &mut Fx, host: LocalHost) {
    fx.cx.update(|cx| {
        fx.root
            .update(cx, |root: &mut Root, cx| root.set_host(Arc::new(host), cx))
    });
}

/// Saves the window as `<name>.png` under `CGX_SCHED_LIST_SHOTS` when it is set.
fn shot(fx: &mut Fx, name: &str) -> Result<()> {
    if let Ok(output) = std::env::var("CGX_SCHED_LIST_SHOTS") {
        fs::create_dir_all(&output)?;
        fx.shot()?.save(format!("{output}/{name}.png"))?;
    }
    Ok(())
}

#[test]
fn cold_link_selects_its_task_and_its_runs_open_their_chat() -> Result<()> {
    let home = tempfile::tempdir()?;
    let mut fx = Fx::on_route(Route::Automation("alpha".into()))?;
    set_host(&mut fx, host(home.path())?);
    fx.wait("cold link selects Alpha", |fx| {
        fx.read(|r, cx| {
            r.scheduled_sidebar()
                .is_some_and(|s| s.read(cx).selected() == Some("alpha"))
        })
    })?;
    ensure!(fx.route() == Route::Automation("alpha".into()));
    ensure!(visible(&mut fx) == ["alpha", "beta"]);
    shot(&mut fx, "1-runs")?;

    // The newest unarchived run opens its chat with the task kept in the list.
    fx.click(FIRST_RUN)?;
    ensure!(
        fx.route() == Route::Thread("run-4".into()),
        "{:?}",
        fx.route()
    );
    let selected = |fx: &mut Fx| {
        fx.read(|r, cx| {
            r.scheduled_sidebar()
                .and_then(|s| s.read(cx).selected().map(str::to_owned))
        })
    };
    ensure!(selected(&mut fx).as_deref() == Some("alpha"));

    // A selected paused task stays listed while `Paused` is off.
    fx.action(Box::new(ui::screens::automations::OpenAutomation {
        id: "beta".into(),
    }))?;
    ensure!(selected(&mut fx).as_deref() == Some("beta"));
    fx.click(FILTER)?;
    fx.keys("down down enter")?;
    ensure!(
        visible(&mut fx) == ["alpha", "beta"],
        "{:?}",
        visible(&mut fx)
    );
    Ok(())
}

#[test]
fn search_and_filter_narrow_the_list() -> Result<()> {
    let home = tempfile::tempdir()?;
    let mut fx = Fx::on_route(Route::Scheduled)?;
    set_host(&mut fx, host(home.path())?);
    fx.wait("tasks listed", |fx| visible(fx).len() == 2)?;
    fx.click(SEARCH)?;
    fx.type_text("BET")?;
    shot(&mut fx, "2-search-match")?;
    ensure!(visible(&mut fx) == ["beta"], "{:?}", visible(&mut fx));
    fx.type_text("x")?;
    shot(&mut fx, "3-no-matching")?;
    ensure!(visible(&mut fx).is_empty());
    fx.keys("escape")?;
    ensure!(visible(&mut fx) == ["alpha", "beta"]);

    // `Active` off keeps the paused task; `Paused` off too leaves nothing listed.
    fx.click(FILTER)?;
    shot(&mut fx, "4-filter-menu")?;
    fx.keys("escape")?;
    fx.click(FILTER)?;
    fx.keys("home enter")?;
    ensure!(visible(&mut fx) == ["beta"], "{:?}", visible(&mut fx));
    // Opening a task keeps the choices: the active task stays hidden.
    fx.action(Box::new(ui::screens::automations::OpenAutomation {
        id: "beta".into(),
    }))?;
    ensure!(fx.route() == Route::Automation("beta".into()));
    ensure!(visible(&mut fx) == ["beta"], "{:?}", visible(&mut fx));
    fx.action(Box::new(ui::OpenScheduled))?;
    fx.click(FILTER)?;
    fx.keys("down down enter")?;
    ensure!(visible(&mut fx).is_empty());
    // Escape closes the menu without a change.
    fx.click(FILTER)?;
    fx.keys("down escape")?;
    ensure!(visible(&mut fx).is_empty());
    fx.click(FILTER)?;
    fx.keys("home enter")?;
    fx.click(FILTER)?;
    fx.keys("down down enter")?;
    ensure!(visible(&mut fx) == ["alpha", "beta"]);
    Ok(())
}

#[test]
fn unreadable_list_fails_and_try_again_loads_it() -> Result<()> {
    let home = tempfile::tempdir()?;
    // A file where the automations folder belongs makes `list-automations` fail.
    fs::write(home.path().join("automations"), "")?;
    let mut fx = Fx::on_route(Route::Scheduled)?;
    set_host(&mut fx, LocalHost::new(home.path()));
    let state =
        |fx: &mut Fx| fx.read(|r, cx| r.scheduled_sidebar().map(|s| s.read(cx).load_state()));
    fx.wait("list fails", |fx| state(fx) == Some(ListState::Failed))?;
    fs::remove_file(home.path().join("automations"))?;
    host(home.path())?;
    fx.action(Box::new(ui::screens::automations::RetryAutomations))?;
    fx.wait("retry lists the tasks", |fx| {
        state(fx) == Some(ListState::Ready)
    })?;
    ensure!(visible(&mut fx) == ["alpha", "beta"]);
    Ok(())
}
