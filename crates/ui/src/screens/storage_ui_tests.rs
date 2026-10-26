use super::*;
use gpui::{HeadlessAppContext, KeyUpEvent, Keystroke, PlatformInput, size};
use serde_json::json;
use std::sync::Arc;

fn capture(cx: &mut HeadlessAppContext, window: gpui::AnyWindowHandle, name: &str) {
    if let Some(directory) = std::env::var_os("CGX_STORAGE_TEST_EVIDENCE") {
        let directory = std::path::PathBuf::from(directory);
        std::fs::create_dir_all(&directory).unwrap();
        cx.test_window(window)
            .simulate_frame_request(gpui::RequestFrameOptions {
                require_presentation: true,
                ..Default::default()
            });
        cx.run_until_parked();
        cx.capture_screenshot(window)
            .unwrap()
            .save(directory.join(format!("{name}.png")))
            .unwrap();
    }
}

#[test]
fn review_keyboard_selection_and_two_cancel_levels() {
    #[cfg(target_os = "macos")]
    std::panic::set_hook(Box::new(|info| {
        eprintln!("{info}");
        std::process::exit(101);
    }));
    let capture_enabled = std::env::var_os("CGX_STORAGE_TEST_EVIDENCE").is_some();
    let mut cx = HeadlessAppContext::with_platform(
        textsys::text_system(),
        Arc::new(theme::Assets),
        move || {
            if capture_enabled {
                gpui_platform::current_headless_renderer()
            } else {
                Ok(None)
            }
        },
    );
    cx.update(|cx| {
        cx.set_global(theme::Theme::dark());
        theme::load_fonts(cx);
    });
    let trigger = cx.update(|cx| cx.focus_handle());
    let saved = trigger.clone();
    let window = cx
        .open_window(size(px(900.), px(700.)), |window, cx| {
            saved.focus(window, cx);
            let integrations =
                cx.new(|_| crate::screens::settings::integrations::State::default());
            let settings = cx.new(|cx| SettingsPage::new(&json!({}), &json!({}), integrations, cx));
            cx.new(|cx| {
                let mut page = StoragePage::new(settings, cx);
                page.started = true;
                page.cleanup_supported = true;
                page.completed = 6;
                page.return_focus = Some(saved);
                page.usage = Some(Usage {
                    bytes: [1024, 0, 0, 0, 0, 0],
                    thread_bytes: Arc::new(HashMap::from([("test".to_owned(), 1024)])),
                    ..Default::default()
                });
                page.review = Some(Review {
                    entries: vec![Entry {
                        archived: true,
                        thread: json!({"id":"test","name":"Old chat","source":"cli","threadSource":"user","updatedAt":1,"status":{"type":"idle"},"ephemeral":false}),
                    }],
                    rows: Vec::new(),
                    selected: HashSet::new(),
                    days: 90,
                    archived_only: true,
                    cutoff: 0,
                    limit: 50,
                });
                page.filter_review();
                page
            })
        })
        .unwrap();
    cx.run_until_parked();
    capture(&mut cx, window.into(), "synthetic-review");
    cx.update_window(window.into(), |_, window, cx| {
        window.dispatch_keystroke(Keystroke::parse("tab").unwrap(), cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update(|cx| {
        window
            .update(cx, |page, window, _| {
                assert!(page.controls["review-delete"].is_focused(window));
            })
            .unwrap()
    });
    cx.update(|cx| {
        window
            .update(cx, |page, window, cx| {
                page.controls["review-chat-test"].focus(window, cx)
            })
            .unwrap()
    });
    for expected in [0, 1] {
        cx.update_window(window.into(), |_, window, cx| {
            let stroke = Keystroke::parse("space").unwrap();
            window.dispatch_keystroke(stroke.clone(), cx);
            window.dispatch_event(PlatformInput::KeyUp(KeyUpEvent { keystroke: stroke }), cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update(|cx| {
            window
                .update(cx, |page, _, _| {
                    assert_eq!(page.review.as_ref().unwrap().selected.len(), expected)
                })
                .unwrap()
        });
    }
    cx.update(|cx| {
        window
            .update(cx, |page, window, cx| {
                page.controls["review-delete"].focus(window, cx)
            })
            .unwrap()
    });
    cx.update_window(window.into(), |_, window, cx| {
        let stroke = Keystroke::parse("enter").unwrap();
        window.dispatch_keystroke(stroke.clone(), cx);
        window.dispatch_event(PlatformInput::KeyUp(KeyUpEvent { keystroke: stroke }), cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update(|cx| {
        window
            .update(cx, |page, _, _| assert!(page.confirmation.is_some()))
            .unwrap()
    });
    capture(&mut cx, window.into(), "synthetic-confirmation");
    for confirmation_only in [true, false] {
        cx.update_window(window.into(), |_, window, cx| {
            window.dispatch_keystroke(Keystroke::parse("escape").unwrap(), cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update(|cx| {
            window
                .update(cx, |page, window, cx| {
                    assert!(page.confirmation.is_none());
                    assert!(!cx.global::<CleanupState>().deleting);
                    assert_eq!(page.review.is_some(), confirmation_only);
                    if !confirmation_only {
                        assert!(trigger.is_focused(window));
                    }
                })
                .unwrap()
        });
    }
    capture(&mut cx, window.into(), "synthetic-page");
}

#[test]
fn persistent_cleanup_toast_waits_for_the_operation() {
    #[cfg(target_os = "macos")]
    std::panic::set_hook(Box::new(|info| {
        eprintln!("{info}");
        std::process::exit(101);
    }));
    let mut cx =
        HeadlessAppContext::with_asset_source(textsys::text_system(), Arc::new(theme::Assets));
    cx.update(|cx| cx.set_global(theme::Theme::dark()));
    let window = cx
        .open_window(size(px(600.), px(400.)), |_, cx| cx.new(|_| Toaster::new()))
        .unwrap();
    let id = cx.update(|cx| {
        window
            .update(cx, |toaster, _, cx| {
                toaster.push_persistent(Toast::info("Deleting chats"), cx)
            })
            .unwrap()
    });
    cx.advance_clock(std::time::Duration::from_secs(10));
    cx.run_until_parked();
    cx.update(|cx| {
        window
            .update(cx, |toaster, _, cx| {
                assert_eq!(toaster.titles(), vec![SharedString::from("Deleting chats")]);
                toaster.dismiss(id, cx);
                assert!(toaster.titles().is_empty());
            })
            .unwrap()
    });
}

#[test]
fn byte_labels_match_original_rounding_and_grouping() {
    #[cfg(target_os = "macos")]
    std::panic::set_hook(Box::new(|info| {
        eprintln!("{info}");
        std::process::exit(101);
    }));
    assert_eq!(format_bytes(1000), "1,000 bytes");
    assert_eq!(format_bytes(1280), "1.3 kB");
    assert_eq!(format_bytes(1048320), "1,023.8 kB");
    assert_eq!(format_bytes(1024 * 1024), "1 MB");
}

#[test]
fn usage_segments_truncate_percentages_like_blink() {
    // The original's measured 728 CSS px bar at scale 2, from x 444.25: Blink ends the last
    // segment at 2344.484375 device px, so pixel 2344 keeps the bar background.
    let edges = segment_edges(&[363_416, 0, 0, 0, 0, 438_272], 888.5, 1456.);
    let units: Vec<_> = edges.iter().map(|&(l, r)| (l * 64., r * 64.)).collect();
    assert_eq!(units[0], (56_864., 99_105.));
    assert_eq!(units[5], (99_105., 150_047.));
    assert!(edges[1..5].iter().all(|&(left, right)| left == right));
    assert_eq!(segment_edges(&[0; 6], 10., 100.)[5], (10., 10.));
}

#[test]
fn refreshed_categories_keep_cached_values_until_complete() {
    #[cfg(target_os = "macos")]
    std::panic::set_hook(Box::new(|info| {
        eprintln!("{info}");
        std::process::exit(101);
    }));
    let mut cx =
        HeadlessAppContext::with_asset_source(textsys::text_system(), Arc::new(theme::Assets));
    cx.update(|cx| {
        let integrations = cx.new(|_| crate::screens::settings::integrations::State::default());
        let settings = cx.new(|cx| SettingsPage::new(&json!({}), &json!({}), integrations, cx));
        let page = cx.new(|cx| StoragePage::new(settings, cx));
        page.update(cx, |page, cx| {
            page.measured = true;
            page.scanning = true;
            page.usage = Some(Usage {
                bytes: [10, 0, 0, 0, 50, 100],
                ..Default::default()
            });
            page.apply_measurement(Measurement::Supported(true), cx);
            page.apply_measurement(
                Measurement::Progress(
                    Usage {
                        bytes: [20, 0, 0, 0, 0, 0],
                        ..Default::default()
                    },
                    4,
                ),
                cx,
            );
            assert_eq!(page.usage.as_ref().unwrap().bytes, [20, 0, 0, 0, 50, 100]);
            assert!(page.can_review(cx));
            assert!(page.scanning);
            page.apply_measurement(
                Measurement::Progress(
                    Usage {
                        bytes: [20, 0, 0, 0, 0, 200],
                        ..Default::default()
                    },
                    5,
                ),
                cx,
            );
            assert_eq!(page.usage.as_ref().unwrap().bytes, [20, 0, 0, 0, 50, 200]);
            page.apply_measurement(
                Measurement::Progress(
                    Usage {
                        bytes: [20, 0, 0, 0, 70, 200],
                        ..Default::default()
                    },
                    6,
                ),
                cx,
            );
            page.apply_measurement(Measurement::Done(Ok(()), true), cx);
            assert_eq!(page.usage.as_ref().unwrap().bytes, [20, 0, 0, 0, 70, 200]);
            assert!(!page.scanning);
        });
    });
}
