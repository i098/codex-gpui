mod common;

use anyhow::{Result, ensure};
use common::Fx;

fn titles(fx: &Fx) -> Vec<&'static str> {
    fx.read(|root, cx| root.dialog().unwrap().read(cx).visible_titles())
}

#[test]
fn dialog_search_matches_command_ids_descriptions_and_fuzzy_titles() -> Result<()> {
    let mut fx = Fx::new()?;
    fx.keys("cmd-/")?;
    for (query, expected) in [
        ("  ARCHIVETHREAD  ", vec!["Archive chat"]),
        ("outside of any project", vec!["New standalone chat"]),
        ("arch*ve chat", vec!["Archive chat"]),
        ("zzzz-no-shortcut", vec![]),
    ] {
        fx.keys("cmd-a")?;
        fx.type_text(query)?;
        ensure!(
            titles(&fx) == expected,
            "query {query:?}: {:?}",
            titles(&fx)
        );
    }
    Ok(())
}
