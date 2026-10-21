use gpui::App;
use std::ops::Range;

pub(crate) struct Spelling {
    pub range_utf16: Range<usize>,
    pub guesses: Vec<String>,
}

pub(crate) fn misspelling(text: &str, offset_utf16: usize) -> Option<Spelling> {
    #[cfg(target_os = "macos")]
    return native::misspelling(text, offset_utf16);
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (text, offset_utf16);
        None
    }
}

pub(crate) fn learn(word: &str) {
    #[cfg(target_os = "macos")]
    native::learn(word);
    #[cfg(not(target_os = "macos"))]
    let _ = word;
}

/// Reports whether this app's native speech session is active.
pub fn is_speaking(cx: &App) -> bool {
    #[cfg(target_os = "macos")]
    return native::is_speaking(cx);
    #[cfg(not(target_os = "macos"))]
    {
        let _ = cx;
        false
    }
}

pub(crate) fn speak(text: &str, cx: &mut App) -> anyhow::Result<()> {
    #[cfg(target_os = "macos")]
    return native::speak(text, cx);
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (text, cx);
        anyhow::bail!("Speech requires macOS")
    }
}

/// Stops speech started from a composer or transcript selection.
pub fn stop_speaking(cx: &mut App) {
    #[cfg(target_os = "macos")]
    native::stop_speaking(cx);
    #[cfg(not(target_os = "macos"))]
    let _ = cx;
}

#[cfg(target_os = "macos")]
mod native {
    use super::*;
    use objc2::{class, msg_send, rc::autoreleasepool, runtime::AnyObject};
    use objc2_foundation::{NSRange, NSString};

    #[derive(Default)]
    struct Speech {
        synthesizer: Option<objc2::rc::Retained<AnyObject>>,
        monitor: Option<gpui::Task<()>>,
    }
    impl gpui::Global for Speech {}

    pub(super) fn is_speaking(cx: &App) -> bool {
        cx.try_global::<Speech>()
            .and_then(|state| state.synthesizer.as_ref())
            .is_some_and(|synthesizer| unsafe { msg_send![&**synthesizer, isSpeaking] })
    }

    fn synthesizer() -> anyhow::Result<objc2::rc::Retained<AnyObject>> {
        use anyhow::Context as _;
        let object: *mut AnyObject = unsafe { msg_send![class!(NSSpeechSynthesizer), new] };
        unsafe { objc2::rc::Retained::from_raw(object) }
            .context("The native speech synthesizer is unavailable")
    }

    pub(super) fn speak(text: &str, cx: &mut App) -> anyhow::Result<()> {
        if text.is_empty() {
            return Ok(());
        }
        let state = cx.default_global::<Speech>();
        state.monitor = None;
        if state.synthesizer.is_none() {
            state.synthesizer = Some(synthesizer()?);
        }
        let synthesizer = state.synthesizer.as_ref().expect("speech synthesizer");
        let started: bool = unsafe {
            let _: () = msg_send![&**synthesizer, stopSpeaking];
            msg_send![&**synthesizer, startSpeakingString: &*NSString::from_str(text)]
        };
        anyhow::ensure!(started, "The native speech synthesizer could not start");
        cx.refresh_windows();
        // NSSpeechSynthesizer has no GPUI completion event; poll only while speaking.
        let monitor = cx.spawn(async |cx| {
            loop {
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(100))
                    .await;
                if !cx.update(|cx| is_speaking(cx)) {
                    cx.update(|cx| cx.refresh_windows());
                    break;
                }
            }
        });
        cx.global_mut::<Speech>().monitor = Some(monitor);
        Ok(())
    }

    pub(super) fn stop_speaking(cx: &mut App) {
        if cx.has_global::<Speech>() {
            let state = cx.global_mut::<Speech>();
            state.monitor = None;
            if let Some(synthesizer) = &state.synthesizer {
                unsafe {
                    let _: () = msg_send![&**synthesizer, stopSpeaking];
                }
            }
            cx.refresh_windows();
        }
    }

    pub(super) fn misspelling(text: &str, offset: usize) -> Option<Spelling> {
        autoreleasepool(|_| unsafe {
            let checker: *mut AnyObject = msg_send![class!(NSSpellChecker), sharedSpellChecker];
            let text = NSString::from_str(text);
            let length = text.length();
            if offset > length {
                return None;
            }
            let mut start = 0usize;
            while start < length {
                let range: NSRange = msg_send![checker,
                    checkSpellingOfString: &*text,
                    startingAt: start as isize,
                    language: std::ptr::null::<AnyObject>(),
                    wrap: false,
                    inSpellDocumentWithTag: 0isize,
                    wordCount: std::ptr::null_mut::<isize>()
                ];
                let end = range.location.checked_add(range.length)?;
                if range.length == 0 || end > length || end <= start || offset < range.location {
                    return None;
                }
                if offset <= end {
                    let guesses: *mut AnyObject = msg_send![checker,
                        guessesForWordRange: range,
                        inString: &*text,
                        language: std::ptr::null::<AnyObject>(),
                        inSpellDocumentWithTag: 0isize
                    ];
                    let count: usize = if guesses.is_null() {
                        0
                    } else {
                        msg_send![guesses, count]
                    };
                    let mut values = Vec::with_capacity(count);
                    for index in 0..count {
                        let guess: &NSString = msg_send![guesses, objectAtIndex: index];
                        values.push(guess.to_string());
                    }
                    return Some(Spelling {
                        range_utf16: range.location..end,
                        guesses: values,
                    });
                }
                start = end;
            }
            None
        })
    }

    pub(super) fn learn(word: &str) {
        if word.is_empty() {
            return;
        }
        autoreleasepool(|_| unsafe {
            let checker: *mut AnyObject = msg_send![class!(NSSpellChecker), sharedSpellChecker];
            let _: () = msg_send![checker, learnWord: &*NSString::from_str(word)];
        });
    }

    #[cfg(test)]
    #[test]
    fn speech_stops_without_audible_output() {
        std::panic::set_hook(Box::new(|info| {
            eprintln!("{info}");
            std::process::exit(101);
        }));
        let mut cx = gpui::TestAppContext::single();
        cx.update(|cx| {
            let synthesizer = synthesizer().expect("native speech service");
            unsafe {
                let _: () = msg_send![&*synthesizer, setVolume: 0.0f32];
            }
            cx.set_global(Speech {
                synthesizer: Some(synthesizer),
                monitor: None,
            });
            speak("This is a silent native speech test.", cx).expect("start native speech");
            assert!(is_speaking(cx));
            stop_speaking(cx);
            assert!(!is_speaking(cx));
        });
    }
}
