use super::*;
use async_channel::{Receiver, Sender};

#[derive(Default)]
pub(super) struct ContextFiles {
    sender: Mutex<Option<Sender<PathBuf>>>,
}

impl ContextFiles {
    pub(super) fn bind(&self) -> Receiver<PathBuf> {
        let (sender, receiver) = async_channel::unbounded();
        *self.sender.lock().unwrap_or_else(PoisonError::into_inner) = Some(sender);
        receiver
    }

    pub(super) fn add(&self, params: &Value) -> Result<Value> {
        let path = params["path"].as_str().context("path must be a string")?;
        ensure!(
            !path.is_empty() && !path.contains('\0'),
            "path must be nonempty and contain no NUL"
        );
        self.sender
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()
            .context("the host has no bound view")?
            .try_send(path.into())
            .context("the bound view is closed")?;
        Ok(json!({ "success": true }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rebinding_disconnects_the_previous_view() -> Result<()> {
        #[cfg(target_os = "macos")]
        std::panic::set_hook(Box::new(|info| {
            eprintln!("{info}");
            std::process::exit(101);
        }));
        let home = tempfile::tempdir()?;
        let host = LocalHost::new(home.path());
        let request = Request::post(
            "add-context-file",
            Some(json!({ "path": "/work/source.rs" })),
        );
        assert!(host.fetch(&request).is_err());
        let previous = host.bind_context_files().unwrap();
        let current = host.bind_context_files().unwrap();
        for path in ["", "invalid\0path"] {
            assert!(
                host.fetch(&Request::post(
                    "add-context-file",
                    Some(json!({ "path": path }))
                ))
                .is_err()
            );
        }
        host.fetch(&request)?;
        assert!(previous.try_recv().is_err());
        assert_eq!(current.try_recv()?, PathBuf::from("/work/source.rs"));
        drop(current);
        assert!(host.fetch(&request).is_err());
        Ok(())
    }
}
