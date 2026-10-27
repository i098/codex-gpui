#[cfg(not(target_os = "macos"))]
fn main() -> anyhow::Result<()> {
    anyhow::bail!("The native substitution check requires macOS")
}

#[cfg(target_os = "macos")]
fn main() -> anyhow::Result<()> {
    std::panic::set_hook(Box::new(|info| {
        eprintln!("{info}");
        std::process::exit(101);
    }));
    use anyhow::{Context as _, ensure};
    use gpui::{AnyWindowHandle, AppContext as _, Focusable as _, WindowOptions};
    use objc2::{
        class, msg_send,
        runtime::{AnyObject, Imp, Sel},
        sel,
    };
    use objc2_foundation::{NSRange, NSString};
    use raw_window_handle::{HasWindowHandle as _, RawWindowHandle};
    use std::{
        cell::RefCell,
        rc::Rc,
        time::{Duration, Instant},
    };

    unsafe extern "C" {
        static NSTextCheckingReplacementsKey: *mut AnyObject;
    }

    static ORIGINAL_CHECK: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

    unsafe extern "C-unwind" fn fixture_replacements(
        checker: *mut AnyObject,
        selector: Sel,
        text: *mut AnyObject,
        range: NSRange,
        kinds: u64,
        mut options: *mut AnyObject,
        tag: isize,
        orthography: *mut *mut AnyObject,
        words: *mut isize,
    ) -> *mut AnyObject {
        unsafe {
            type Original = unsafe extern "C-unwind" fn(
                *mut AnyObject,
                Sel,
                *mut AnyObject,
                NSRange,
                u64,
                *mut AnyObject,
                isize,
                *mut *mut AnyObject,
                *mut isize,
            ) -> *mut AnyObject;
            let original: Original =
                std::mem::transmute(ORIGINAL_CHECK.load(std::sync::atomic::Ordering::Acquire));
            if options.is_null()
                && kinds & (1 << 8) != 0
                && (&*text.cast::<NSString>())
                    .to_string()
                    .starts_with("cgxproof")
            {
                let replacements: *mut AnyObject = msg_send![class!(NSDictionary), dictionaryWithObject: &*NSString::from_str("expanded value"), forKey: &*NSString::from_str("cgxproof")];
                options = msg_send![class!(NSDictionary), dictionaryWithObject: replacements, forKey: NSTextCheckingReplacementsKey];
            }
            original(
                checker,
                selector,
                text,
                range,
                kinds,
                options,
                tag,
                orthography,
                words,
            )
        }
    }

    fn prepare_field(
        field: gpui::WindowHandle<ui::composer::Composer>,
        cx: &mut gpui::AsyncApp,
    ) -> anyhow::Result<std::ptr::NonNull<AnyObject>> {
        field.update(cx, |field, window, cx| {
            window.focus(&field.focus_handle(cx), cx)
        })?;
        let window: AnyWindowHandle = field.into();
        window.update(cx, |_, window, cx| window.draw(cx).clear(cx))?;
        window.update(cx, |_, window, _| -> anyhow::Result<_> {
            let RawWindowHandle::AppKit(handle) = window.window_handle()?.as_raw() else {
                anyhow::bail!("missing native AppKit view");
            };
            Ok(handle.ns_view.cast::<AnyObject>())
        })?
    }

    async fn type_native(
        field: gpui::WindowHandle<ui::composer::Composer>,
        view: std::ptr::NonNull<AnyObject>,
        scenario: &str,
        text: &str,
        events: &mut Vec<serde_json::Value>,
        cx: &mut gpui::AsyncApp,
    ) -> anyhow::Result<String> {
        for character in text.chars() {
            let input = character.to_string();
            let before = field.update(cx, |field, _, _| field.field_text().to_owned())?;
            unsafe {
                let _: () = msg_send![view.as_ref(), insertText: &*NSString::from_str(&input), replacementRange: NSRange::new(isize::MAX as usize, 0)];
            }
            cx.background_executor()
                .timer(Duration::from_millis(30))
                .await;
            let after = field.update(cx, |field, _, _| field.field_text().to_owned())?;
            events.push(serde_json::json!({
                "scenario": scenario, "selector": "insertText:replacementRange:",
                "text": input, "replacementRange": {"location": isize::MAX as usize, "length": 0},
                "before": before, "after": after
            }));
        }
        field.update(cx, |field, _, _| field.field_text().to_owned())
    }
    let outcome = Rc::new(RefCell::new(None));
    let received = outcome.clone();
    gpui_platform::application().with_assets(theme::Assets).run(move |cx| {
        theme::load_fonts(cx);
        cx.set_global(theme::Theme::dark());
        ui::composer::bind_keys(cx);
        let field = cx.open_window(WindowOptions { show: false, focus: false, ..Default::default() }, |_, cx| {
            cx.new(|cx| ui::composer::Composer::settings_field("Text", "", true, cx))
        }).expect("hidden native window");
        let single_line = cx.open_window(WindowOptions { show: false, focus: false, ..Default::default() }, |_, cx| {
            cx.new(|cx| ui::composer::Composer::settings_field("Single line", "", false, cx))
        }).expect("hidden single-line window");
        cx.spawn(async move |cx| {
            let mut events = Vec::new();
            let mut observed = serde_json::Map::new();
            let result = async {
                ensure!(!objc2_app_kit::NSRunningApplication::currentApplication().isActive(), "the check activated the app");
                let view = prepare_field(field, cx)?;
                const QUOTED_INPUT: &str = "He said \"hello\". ";
                unsafe {
                    let _: () = msg_send![view.as_ref(), setSmartQuotesType: 1isize];
                    let _: () = msg_send![view.as_ref(), setSmartDashesType: 1isize];
                    let _: () = msg_send![view.as_ref(), setTextReplacementType: 1isize];
                }
                let disabled = type_native(field, view, "disabled", QUOTED_INPUT, &mut events, cx).await?;
                observed.insert("disabled".into(), disabled.clone().into());
                ensure!(disabled == QUOTED_INPUT, "disabled substitutions changed text: {disabled:?}");
                unsafe {
                    let _: () = msg_send![view.as_ref(), insertText: &*NSString::from_str(""), replacementRange: NSRange::new(0, disabled.encode_utf16().count())];
                    let _: () = msg_send![view.as_ref(), setSmartQuotesType: 2isize];
                }
                events.push(serde_json::json!({
                    "scenario": "clear-control", "selector": "insertText:replacementRange:",
                    "text": "", "replacementRange": {"location": 0, "length": disabled.encode_utf16().count()},
                    "before": disabled, "after": field.update(cx, |field, _, _| field.field_text().to_owned())?
                }));
                let quote_trait: isize = unsafe { msg_send![view.as_ref(), smartQuotesType] };
                observed.insert("enabledQuotesTrait".into(), quote_trait.into());
                let typed = type_native(field, view, "quotes", QUOTED_INPUT, &mut events, cx).await?;
                observed.insert("quotes".into(), typed.into());
                let started = Instant::now();
                let quotes = loop {
                    let text = field.update(cx, |field, _, _| field.field_text().to_owned())?;
                    if text.contains("hello") && !text.contains('"') { break text; }
                    ensure!(started.elapsed() < Duration::from_secs(5), "native quote substitution did not finish: {text:?}");
                    cx.background_executor().timer(Duration::from_millis(20)).await;
                };
                observed.insert("quotes".into(), quotes.clone().into());
                field.update(cx, |field, _, cx| field.set_text("", cx))?;
                unsafe {
                    let _: () = msg_send![view.as_ref(), setSmartQuotesType: 1isize];
                    let _: () = msg_send![view.as_ref(), setSmartDashesType: 2isize];
                }
                let typed = type_native(field, view, "dashes", "one -- two ", &mut events, cx).await?;
                observed.insert("dashes".into(), typed.into());
                let started = Instant::now();
                let dashes = loop {
                    let text = field.update(cx, |field, _, _| field.field_text().to_owned())?;
                    if text.contains('—') || text.contains('–') { break text; }
                    ensure!(started.elapsed() < Duration::from_secs(5), "native dash substitution did not finish: {text:?}");
                    cx.background_executor().timer(Duration::from_millis(20)).await;
                };
                observed.insert("dashes".into(), dashes.clone().into());
                field.update(cx, |field, _, cx| field.set_text("", cx))?;
                let method = class!(NSSpellChecker)
                    .instance_method(sel!(checkString:range:types:options:inSpellDocumentWithTag:orthography:wordCount:))
                    .context("native replacement checker")?;
                let original = method.implementation();
                ORIGINAL_CHECK.store(original as usize, std::sync::atomic::Ordering::Release);
                unsafe {
                    method.set_implementation(std::mem::transmute::<*const (), Imp>(fixture_replacements as *const ()));
                    let _: () = msg_send![view.as_ref(), setSmartDashesType: 1isize];
                    let _: () = msg_send![view.as_ref(), setTextReplacementType: 2isize];
                }
                let replacement = type_native(field, view, "replacement", "cgxproof ", &mut events, cx).await?;
                unsafe { method.set_implementation(original); }
                ensure!(replacement == "expanded value ", "native text replacement failed: {replacement:?}");
                ensure!(events.iter().any(|event| event["scenario"] == "replacement" && event["text"] == "f" && event["after"] == "cgxproof"), "replacement happened before the word boundary");
                let selection: NSRange = unsafe { msg_send![view.as_ref(), selectedRange] };
                ensure!(selection.location == replacement.encode_utf16().count() && selection.length == 0, "replacement moved the caret incorrectly: {selection:?}");
                observed.insert("replacementFixture".into(), "Process-local native checker options; no shared OS settings changed".into());
                ensure!(!objc2_app_kit::NSRunningApplication::currentApplication().isActive(), "the check activated the app");
                observed.insert("replacement".into(), replacement.into());
                field.update(cx, |field, _, cx| field.set_text("", cx))?;
                unsafe {
                    let _: () = msg_send![view.as_ref(), setTextReplacementType: 1isize];
                    let _: () = msg_send![view.as_ref(), setSmartQuotesType: 2isize];
                    let _: () = msg_send![view.as_ref(), setMarkedText: &*NSString::from_str("abcdef"), selectedRange: NSRange::new(6, 0), replacementRange: NSRange::new(isize::MAX as usize, 0)];
                }
                let marked = field.update(cx, |field, _, _| field.field_text().to_owned())?;
                events.push(serde_json::json!({"scenario":"composition","selector":"setMarkedText:selectedRange:replacementRange:","text":"abcdef","after":marked}));
                unsafe {
                    let _: () = msg_send![view.as_ref(), insertText: &*NSString::from_str("中"), replacementRange: NSRange::new(isize::MAX as usize, 0)];
                }
                let composed = field.update(cx, |field, _, _| field.field_text().to_owned())?;
                events.push(serde_json::json!({"scenario":"composition","selector":"insertText:replacementRange:","text":"中","after":composed}));
                ensure!(composed == "中", "composition commit failed: {composed:?}");
                observed.insert("composition".into(), composed.into());

                let single_view = prepare_field(single_line, cx)?;
                unsafe {
                    let _: () = msg_send![single_view.as_ref(), setSmartQuotesType: 2isize];
                    let _: () = msg_send![single_view.as_ref(), setSmartDashesType: 1isize];
                    let _: () = msg_send![single_view.as_ref(), setTextReplacementType: 1isize];
                    let _: () = msg_send![single_view.as_ref(), insertText: &*NSString::from_str("a\nb"), replacementRange: NSRange::new(isize::MAX as usize, 0)];
                }
                let filtered = single_line.update(cx, |field, _, _| field.field_text().to_owned())?;
                events.push(serde_json::json!({"scenario":"single-line","selector":"insertText:replacementRange:","text":"a\nb","after":filtered}));
                ensure!(filtered == "ab", "single-line filtering failed: {filtered:?}");
                observed.insert("singleLine".into(), filtered.into());

                let paragraph = "a".repeat(1024 * 1024);
                let mut timings = serde_json::Map::new();
                for (label, setting) in [("disabled", 1isize), ("enabled", 2isize)] {
                    single_line.update(cx, |field, _, cx| field.set_text(&paragraph, cx))?;
                    unsafe { let _: () = msg_send![single_view.as_ref(), setSmartQuotesType: setting]; }
                    let started = Instant::now();
                    unsafe {
                        let _: () = msg_send![single_view.as_ref(), insertText: &*NSString::from_str("z"), replacementRange: NSRange::new(isize::MAX as usize, 0)];
                    }
                    let elapsed = started.elapsed().as_micros() as u64;
                    let length = single_line.update(cx, |field, _, _| field.field_text().len())?;
                    ensure!(length == paragraph.len() + 1, "long paragraph lost text");
                    timings.insert(label.into(), elapsed.into());
                    single_line.update(cx, |field, _, cx| field.set_text("", cx))?;
                }
                observed.insert("longParagraphMicros".into(), timings.into());
                ensure!(!objc2_app_kit::NSRunningApplication::currentApplication().isActive(), "the check activated the app");
                anyhow::Ok(())
            }.await;
            if let Some(path) = std::env::var_os("SUBSTITUTIONS_OUT") {
                let active = objc2_app_kit::NSRunningApplication::currentApplication().isActive();
                let error = result.as_ref().err().map(|error| format!("{error:#}"));
                if let Err(error) = std::fs::write(path, serde_json::to_vec_pretty(
                    &serde_json::json!({"events": events, "observed": observed, "error": error, "active": active})
                ).expect("serialize native input evidence")) {
                    eprintln!("Cannot retain native input evidence: {error}");
                    std::process::exit(1);
                }
            }
            if let Err(error) = &result {
                eprintln!("Native substitution check failed: {error:#}");
                std::process::exit(1);
            }
            println!("PASS: native quotes, dashes, and text replacement without activation");
            *received.borrow_mut() = Some(result);
            cx.update(|cx| cx.quit());
        }).detach();
    });
    outcome
        .borrow_mut()
        .take()
        .context("native substitution check did not finish")?
}
