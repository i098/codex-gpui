use anyhow::{Result, ensure};
use app::host::{
    Host, LocalHost,
    local_environments::{DEFAULT_FILE, DIRECTORY, ParsedEnvironment, SaveResult, Selection},
};

fn main() -> Result<()> {
    std::panic::set_hook(Box::new(|info| {
        eprintln!("{info}");
        std::process::exit(101);
    }));
    let temporary = tempfile::tempdir()?;
    let workspace = temporary.path().join("project");
    std::fs::create_dir_all(workspace.join(".git"))?;
    let host = LocalHost::new(temporary.path().join("home"));
    let config = workspace.join(DIRECTORY).join(DEFAULT_FILE);
    let text = "name='Smoke environment'\n[setup]\nscript='echo setup'\n";
    ensure!(host.save_local_environment_config(&config, None, text)? == SaveResult::Success);
    host.select_local_environment(&workspace, Some(&config))?;
    let read = host.local_environment_config(&config)?;
    let Some(ParsedEnvironment::Success { environment, .. }) = read.environment else {
        anyhow::bail!("environment did not load")
    };
    ensure!(environment.name == "Smoke environment" && environment.setup.script == "echo setup");
    ensure!(host.local_environment_selection(&workspace)? == Selection::Config(config.clone()));
    let edited = "name='External edit'\n[setup]\nscript='echo external'\n";
    std::fs::write(&config, edited)?;
    ensure!(
        host.save_local_environment_config(&config, read.revision.as_deref(), text)?
            == SaveResult::Conflict
    );
    ensure!(std::fs::read_to_string(&config)? == edited);
    ensure!(host.local_environments(&workspace)?.len() == 1);
    println!(
        "Created, listed, selected and read a real environment; rejected a stale save without changing the external edit."
    );
    Ok(())
}
