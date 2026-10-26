use crate::screens::thread::view::menu::{ContextMenu, Item};
use anyhow::Context as _;
use gpui::{App, AsyncApp, ClipboardItem, Image, Pixels, Point, PromptLevel, Window};
use std::{
    io::Write as _,
    path::{Path, PathBuf},
    sync::Arc,
};

pub(crate) const COPY: &str = "copyImage";
pub(crate) const SAVE: &str = "saveImageAs";
pub(crate) const REVEAL: &str = "revealImage";

#[derive(Clone)]
pub(crate) struct ImageContext {
    pub image: Arc<Image>,
    pub path: Option<PathBuf>,
}

impl ImageContext {
    fn items(&self) -> Vec<Item> {
        let mut items = Vec::with_capacity(3);
        if self.path.is_some() {
            items.push(Item::new(REVEAL, "Reveal in Finder", true));
        }
        items.push(Item::new(COPY, "Copy Image", true));
        items.push(Item::new(SAVE, "Save Image As…", true));
        items
    }

    pub fn menu(&self, position: Point<Pixels>, window: &mut Window, cx: &mut App) -> ContextMenu {
        ContextMenu::open(self.items(), String::new(), position, window, cx)
    }

    pub fn activate(self, action: &str, window: &mut Window, cx: &mut App) {
        match action {
            COPY => cx.write_to_clipboard(ClipboardItem::new_image(&self.image)),
            REVEAL => {
                if let Some(path) = self.path {
                    cx.reveal_path(&path);
                }
            }
            SAVE => self.save(window, cx),
            _ => {}
        }
    }

    fn save(self, window: &mut Window, cx: &mut App) {
        let suggested = self
            .path
            .as_ref()
            .and_then(|path| path.file_name())
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| format!("image.{}", self.image.format.extension()));
        let directory = self
            .path
            .as_ref()
            .and_then(|path| path.parent())
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                std::env::var_os("HOME")
                    .map(PathBuf::from)
                    .unwrap_or_else(std::env::temp_dir)
            });
        let chosen = cx.prompt_for_new_path(&directory, Some(&suggested));
        let handle = window.window_handle();
        cx.spawn(async move |cx| {
            let path = match chosen.await {
                Ok(Ok(Some(path))) => path,
                Ok(Ok(None)) | Err(_) => return,
                Ok(Err(error)) => {
                    show_error(handle, error, cx);
                    return;
                }
            };
            let image = self.image;
            let written = cx
                .background_executor()
                .spawn(async move { write_image(&path, &image.bytes) })
                .await;
            if let Err(error) = written {
                show_error(handle, error, cx);
            }
        })
        .detach();
    }
}

fn write_image(path: &Path, bytes: &[u8]) -> anyhow::Result<()> {
    write_image_with(path, |file| file.write_all(bytes))
}

fn write_image_with(
    path: &Path,
    write: impl FnOnce(&mut std::fs::File) -> std::io::Result<()>,
) -> anyhow::Result<()> {
    let directory = path.parent().context("image save path has no parent")?;
    let mut file = tempfile::NamedTempFile::new_in(directory)?;
    write(file.as_file_mut())?;
    file.as_file().sync_all()?;
    file.persist(path).map_err(|error| error.error)?;
    Ok(())
}

fn show_error(handle: gpui::AnyWindowHandle, error: anyhow::Error, cx: &mut AsyncApp) {
    handle
        .update(cx, |_, window, cx| {
            drop(window.prompt(
                PromptLevel::Critical,
                "Unable to save image",
                Some(&format!("{error:#}")),
                &["OK"],
                cx,
            ));
        })
        .ok();
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{Render, TestAppContext};

    struct Empty;
    impl Render for Empty {
        fn render(
            &mut self,
            _: &mut Window,
            _: &mut gpui::Context<Self>,
        ) -> impl gpui::IntoElement {
            gpui::div()
        }
    }

    #[test]
    fn image_actions_preserve_bytes_and_only_reveal_local_files() {
        #[cfg(target_os = "macos")]
        std::panic::set_hook(Box::new(|info| {
            eprintln!("{info}");
            std::process::exit(101);
        }));
        let mut encoded = std::io::Cursor::new(Vec::new());
        image::RgbaImage::from_pixel(2, 2, image::Rgba([30, 60, 90, 255]))
            .write_to(&mut encoded, image::ImageFormat::Png)
            .unwrap();
        let bytes = encoded.into_inner();
        let context = ImageContext {
            image: Arc::new(Image::from_bytes(gpui::ImageFormat::Png, bytes.clone())),
            path: None,
        };
        assert_eq!(
            context
                .items()
                .iter()
                .map(|item| item.id)
                .collect::<Vec<_>>(),
            [COPY, SAVE]
        );
        let mut local = context.clone();
        local.path = Some(std::env::temp_dir().join("image.png"));
        assert_eq!(
            local.items().iter().map(|item| item.id).collect::<Vec<_>>(),
            [REVEAL, COPY, SAVE]
        );
        let mut cx = TestAppContext::single();
        let window = cx.add_window(|_, _| Empty);
        window
            .update(&mut cx, |_, window, cx| {
                context.clone().activate(COPY, window, cx)
            })
            .unwrap();
        cx.update(|cx| {
            let item = cx.read_from_clipboard().unwrap();
            let gpui::ClipboardEntry::Image(image) = &item.entries()[0] else {
                panic!("image clipboard entry");
            };
            assert_eq!(image.bytes, bytes);
        });
        let directory = tempfile::tempdir().unwrap();
        let output = directory.path().join("image.png");
        std::fs::write(&output, b"old image").unwrap();
        window
            .update(&mut cx, |_, window, cx| context.activate(SAVE, window, cx))
            .unwrap();
        cx.simulate_new_path_selection(|_| Some(output.clone()));
        cx.run_until_parked();
        assert_eq!(std::fs::read(&output).unwrap(), bytes);
        let failed = write_image_with(&output, |file| {
            file.write_all(b"incomplete")?;
            Err(std::io::Error::other("write failed"))
        });
        assert!(failed.is_err());
        assert_eq!(std::fs::read(&output).unwrap(), bytes);
        let remaining: Vec<_> = std::fs::read_dir(directory.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(remaining, [std::ffi::OsString::from("image.png")]);
    }
}
