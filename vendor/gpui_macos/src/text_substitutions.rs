use crate::{BoolExt, NSRange, NSStringExt, ns_string, window::with_input_handler};
use cocoa::{
    base::{id, nil},
    foundation::{NSPoint, NSRect, NSSize, NSUInteger},
};
use gpui::FocusId;
use objc::{
    class,
    declare::ClassDecl,
    msg_send,
    rc::StrongPtr,
    runtime::{BOOL, Object, Protocol, Sel},
    sel, sel_impl,
};
use std::{ffi::c_void, ops::Range, ptr};

const STATE: &str = "gpuiTextSubstitutions";
const QUOTES: u64 = 1 << 6;
const DASHES: u64 = 1 << 7;
const REPLACEMENTS: u64 = 1 << 8;
// ponytail: check 512 UTF-16 units around an edit; widen only for a proven long-range substitution case.
const CONTEXT_UTF16: usize = 512;

struct State {
    enabled: bool,
    focus: Option<FocusId>,
    traits: [isize; 3],
    controller: Option<StrongPtr>,
    annotations: Option<StrongPtr>,
}

pub(super) fn register(decl: &mut ClassDecl) {
    decl.add_ivar::<*mut c_void>(STATE);
    decl.add_protocol(
        Protocol::get("NSTextCheckingClient").expect("AppKit text checking protocol"),
    );
    unsafe {
        decl.add_method(
            sel!(annotatedSubstringForProposedRange:actualRange:),
            annotated_substring as extern "C" fn(&Object, Sel, NSRange, *mut c_void) -> id,
        );
        decl.add_method(
            sel!(setAnnotations:range:),
            set_annotations as extern "C" fn(&Object, Sel, id, NSRange),
        );
        decl.add_method(
            sel!(addAnnotations:range:),
            add_annotations as extern "C" fn(&Object, Sel, id, NSRange),
        );
        decl.add_method(
            sel!(removeAnnotation:range:),
            remove_annotation as extern "C" fn(&Object, Sel, id, NSRange),
        );
        decl.add_method(
            sel!(replaceCharactersInRange:withAnnotatedString:),
            replace_characters as extern "C" fn(&Object, Sel, NSRange, id),
        );
        decl.add_method(
            sel!(selectAndShowRange:),
            select_range as extern "C" fn(&Object, Sel, NSRange),
        );
        decl.add_method(
            sel!(viewForRange:firstRect:actualRange:),
            view_for_range as extern "C" fn(&Object, Sel, NSRange, *mut c_void, *mut c_void) -> id,
        );
        decl.add_method(
            sel!(candidateListTouchBarItem),
            no_candidates as extern "C" fn(&Object, Sel) -> id,
        );
        decl.add_method(
            sel!(orderFrontSubstitutionsPanel:),
            show_panel as extern "C" fn(&Object, Sel, id),
        );
        decl.add_method(
            sel!(validateMenuItem:),
            validate_menu as extern "C" fn(&Object, Sel, id) -> BOOL,
        );
        for selector in [
            sel!(smartQuotesType),
            sel!(smartDashesType),
            sel!(textReplacementType),
        ] {
            decl.add_method(
                selector,
                trait_value as extern "C" fn(&Object, Sel) -> isize,
            );
        }
        for selector in [
            sel!(setSmartQuotesType:),
            sel!(setSmartDashesType:),
            sel!(setTextReplacementType:),
        ] {
            decl.add_method(selector, set_trait as extern "C" fn(&Object, Sel, isize));
        }
        for selector in [
            sel!(toggleAutomaticQuoteSubstitution:),
            sel!(toggleAutomaticDashSubstitution:),
            sel!(toggleAutomaticTextReplacement:),
        ] {
            decl.add_method(selector, toggle_trait as extern "C" fn(&Object, Sel, id));
        }
        for selector in [
            sel!(autocorrectionType),
            sel!(spellCheckingType),
            sel!(grammarCheckingType),
            sel!(smartInsertDeleteType),
            sel!(dataDetectionType),
            sel!(linkDetectionType),
            sel!(textCompletionType),
            sel!(inlinePredictionType),
        ] {
            decl.add_method(
                selector,
                disabled_trait as extern "C" fn(&Object, Sel) -> isize,
            );
        }
    }
}

pub(super) unsafe fn initialize(view: id) {
    unsafe {
        let state = Box::new(State {
            enabled: false,
            focus: None,
            traits: [0; 3],
            controller: None,
            annotations: None,
        });
        (*view).set_ivar(STATE, Box::into_raw(state).cast::<c_void>());
    }
}

unsafe fn state(view: &Object) -> Option<*mut State> {
    unsafe {
        let value = (*view.get_ivar::<*mut c_void>(STATE)).cast::<State>();
        (!value.is_null()).then_some(value)
    }
}

unsafe fn reset(state: *mut State) {
    unsafe {
        let controller = (*state).controller.take();
        (*state).annotations = None;
        if let Some(controller) = controller {
            let _: () = msg_send![*controller, invalidate];
        }
    }
}

pub(super) unsafe fn destroy(view: &Object) {
    unsafe {
        if let Some(state) = state(view) {
            reset(state);
            drop(Box::from_raw(state));
        }
    }
}

pub(super) fn focus_changed(view: &Object, focus: Option<FocusId>) {
    unsafe {
        if let Some(state) = state(view) {
            if (*state).focus != focus {
                (*state).focus = focus;
                reset(state);
            }
        }
    }
}

pub(super) fn configure(view: &Object, enabled: bool) {
    unsafe {
        if let Some(state) = state(view) {
            (*state).enabled = enabled;
            if !enabled {
                reset(state);
            }
        }
    }
}

fn enabled(view: &Object) -> bool {
    unsafe { state(view).is_some_and(|state| (*state).enabled && (*state).focus.is_some()) }
}

unsafe fn controller(view: &Object) -> id {
    unsafe {
        if !enabled(view) {
            return nil;
        }
        let Some(state) = state(view) else {
            return nil;
        };
        if (*state).controller.is_none() {
            let value: id = msg_send![class!(NSTextCheckingController), alloc];
            let value: id = msg_send![value, initWithClient: view];

            if value.is_null() {
                return nil;
            }
            (*state).controller = Some(StrongPtr::new(value));
        }
        (*state).controller.as_ref().map_or(nil, |value| **value)
    }
}

pub(super) fn accepted_range(
    replaced: Range<usize>,
    before: usize,
    after: usize,
) -> Option<Range<usize>> {
    if replaced.start > replaced.end || replaced.start > before {
        return None;
    }
    let removed = replaced.end.min(before) - replaced.start;
    let accepted = after.checked_sub(before - removed)?;
    let end = replaced.start.checked_add(accepted)?;
    (end <= after).then_some(replaced.start..end)
}

pub(super) fn inserted(view: &Object, range: Range<usize>) {
    if !enabled(view)
        || with_input_handler(view, |handler| handler.marked_text_range())
            .flatten()
            .is_some()
    {
        return;
    }
    unsafe {
        let controller = controller(view);
        if !controller.is_null()
            && let Some(range) = apply_substitutions(view, range, controller)
        {
            let _: () = msg_send![controller, didChangeTextInRange: NSRange::from(range)];
        }
    }
}

unsafe fn apply_substitutions(
    view: &Object,
    inserted: Range<usize>,
    controller: id,
) -> Option<Range<usize>> {
    unsafe {
        let length = with_input_handler(view, |handler| handler.text_length_utf16()).flatten()?;
        if inserted.start > inserted.end || inserted.end > length {
            return None;
        }
        let kinds = [QUOTES, DASHES, REPLACEMENTS]
            .into_iter()
            .enumerate()
            .filter(|(index, _)| is_checked(view, *index))
            .fold(0, |mask, (_, kind)| mask | kind);
        if kinds == 0 || inserted.is_empty() {
            return Some(inserted);
        }
        let (mut selection, text, context) = with_input_handler(view, |handler| {
            let requested = inserted.start.saturating_sub(CONTEXT_UTF16)
                ..inserted.end.saturating_add(CONTEXT_UTF16).min(length);
            let mut actual = None;
            let text = handler.text_for_range(requested.clone(), &mut actual)?;
            Some((
                handler.selected_text_range(false)?.range,
                text,
                actual.unwrap_or(requested),
            ))
        })
        .flatten()?;
        let string = ns_string(&text);
        let context_length: NSUInteger = msg_send![string, length];
        let checking_range = NSRange {
            location: 0,
            length: context_length,
        };
        let checker: id = msg_send![class!(NSSpellChecker), sharedSpellChecker];
        let tag: isize = msg_send![controller, spellCheckerDocumentTag];
        let results: id = msg_send![checker, checkString: string range: checking_range types: kinds options: nil inSpellDocumentWithTag: tag orthography: ptr::null_mut::<id>() wordCount: ptr::null_mut::<isize>()];
        let count: usize = msg_send![results, count];
        let mut changed = inserted.clone();
        let mut next_start = length;
        let mut applied = false;
        for index in (0..count).rev() {
            let result: id = msg_send![results, objectAtIndex: index];
            let kind: u64 = msg_send![result, resultType];
            let range: NSRange = msg_send![result, range];
            let local_start = range.location as usize;
            let local_end = local_start.checked_add(range.length as usize)?;
            if local_end > context_length as usize {
                continue;
            }
            let start = context.start.checked_add(local_start)?;
            let end = context.start.checked_add(local_end)?;
            if kind & kinds == 0
                || start >= end
                || end > next_start
                || end < inserted.start
                || start >= inserted.end
                || (kind == REPLACEMENTS && end > inserted.start)
            {
                continue;
            }
            let replacement: id = msg_send![result, replacementString];
            if replacement.is_null() {
                continue;
            }
            let original: id = msg_send![string, substringWithRange: range];
            let same: BOOL = msg_send![original, isEqualToString: replacement];
            if same != objc::runtime::NO {
                continue;
            }
            let replacement_length: usize = msg_send![replacement, length];
            let value: id = msg_send![class!(NSAttributedString), alloc];
            let value: id = msg_send![value, initWithString: replacement];
            if value.is_null() {
                continue;
            }
            let value = StrongPtr::new(value);
            replace_characters(
                view,
                sel!(replaceCharactersInRange:withAnnotatedString:),
                NSRange {
                    location: start as NSUInteger,
                    length: (end - start) as NSUInteger,
                },
                *value,
            );
            for position in [&mut selection.start, &mut selection.end] {
                if *position >= end {
                    *position = *position - (end - start) + replacement_length;
                } else if *position > start {
                    *position = start + replacement_length;
                }
            }
            changed.start = changed.start.min(start);
            changed.end = changed.end.max(end) - (end - start) + replacement_length;
            next_start = start;
            applied = true;
        }
        if applied {
            with_input_handler(view, |handler| handler.set_selected_text_range(selection));
        }
        Some(changed)
    }
}

fn index(selector: Sel) -> Option<usize> {
    match selector.name() {
        "smartQuotesType" | "setSmartQuotesType:" | "toggleAutomaticQuoteSubstitution:" => Some(0),
        "smartDashesType" | "setSmartDashesType:" | "toggleAutomaticDashSubstitution:" => Some(1),
        "textReplacementType" | "setTextReplacementType:" | "toggleAutomaticTextReplacement:" => {
            Some(2)
        }
        _ => None,
    }
}

extern "C" fn disabled_trait(_: &Object, _: Sel) -> isize {
    1
}

extern "C" fn trait_value(view: &Object, selector: Sel) -> isize {
    unsafe {
        if !enabled(view) {
            return 1;
        }
        let value = state(view)
            .zip(index(selector))
            .map_or(1, |(state, index)| (*state).traits[index]);

        value
    }
}

extern "C" fn set_trait(view: &Object, selector: Sel, value: isize) {
    unsafe {
        if enabled(view)
            && let Some((state, index)) = state(view).zip(index(selector))
        {
            (*state).traits[index] = value.clamp(0, 2);
            let checker: id = msg_send![class!(NSSpellChecker), sharedSpellChecker];
            let _: () = msg_send![checker, updatePanels];
        }
    }
}

fn is_checked(view: &Object, index: usize) -> bool {
    unsafe {
        let Some(state) = state(view) else {
            return false;
        };
        match (*state).traits[index] {
            1 => false,
            2 => true,
            _ => {
                let value: BOOL = match index {
                    0 => msg_send![class!(NSSpellChecker), isAutomaticQuoteSubstitutionEnabled],
                    1 => msg_send![class!(NSSpellChecker), isAutomaticDashSubstitutionEnabled],
                    _ => msg_send![class!(NSSpellChecker), isAutomaticTextReplacementEnabled],
                };
                value != objc::runtime::NO
            }
        }
    }
}

extern "C" fn toggle_trait(view: &Object, selector: Sel, sender: id) {
    unsafe {
        if !enabled(view) {
            return;
        }
        if let Some((state, index)) = state(view).zip(index(selector)) {
            let checked = !is_checked(view, index);
            (*state).traits[index] = if checked { 2 } else { 1 };
            if !sender.is_null() {
                let _: () = msg_send![sender, setState: isize::from(checked)];
            }
            let checker: id = msg_send![class!(NSSpellChecker), sharedSpellChecker];
            let _: () = msg_send![checker, updatePanels];
        }
    }
}

extern "C" fn validate_menu(view: &Object, _: Sel, item: id) -> BOOL {
    unsafe {
        let selector: Sel = msg_send![item, action];
        if let Some(index) = index(selector) {
            let _: () = msg_send![item, setState: isize::from(is_checked(view, index))];
            return enabled(view).to_objc();
        }
        if selector == sel!(orderFrontSubstitutionsPanel:) {
            return enabled(view).to_objc();
        }
        let app: id = msg_send![class!(NSApplication), sharedApplication];
        let delegate: id = msg_send![app, delegate];
        msg_send![delegate, validateMenuItem: item]
    }
}

extern "C" fn show_panel(view: &Object, _: Sel, sender: id) {
    unsafe {
        let controller = controller(view);
        if !controller.is_null() {
            let _: () = msg_send![controller, orderFrontSubstitutionsPanel: sender];
        }
    }
}

unsafe fn snapshot(view: &Object) -> Option<(id, usize)> {
    unsafe {
        let text = with_input_handler(view, |handler| {
            let length = handler.text_length_utf16()?;
            handler.text_for_range(0..length, &mut None)
        })
        .flatten()?;
        let state = state(view)?;

        let unchanged = (*state).annotations.as_ref().is_some_and(|annotations| {
            let value: id = msg_send![**annotations, string];
            value.to_str() == text
        });
        if !unchanged {
            let value: id = msg_send![class!(NSMutableAttributedString), alloc];
            let value: id = msg_send![value, initWithString: ns_string(&text)];
            if value.is_null() {
                return None;
            }
            (*state).annotations = Some(StrongPtr::new(value));
        }
        let value = (*state).annotations.as_ref().map(|value| **value)?;
        let length: NSUInteger = msg_send![value, length];
        Some((value, length as usize))
    }
}

fn bounded(range: NSRange, length: usize) -> Option<NSRange> {
    if !range.is_valid() {
        return Some(NSRange {
            location: 0,
            length: length as NSUInteger,
        });
    }
    let start = range.location as usize;
    if start > length {
        return None;
    }
    Some(NSRange {
        location: range.location,
        length: range.length.min((length - start) as NSUInteger),
    })
}

extern "C" fn annotated_substring(
    view: &Object,
    _: Sel,
    range: NSRange,
    actual: *mut c_void,
) -> id {
    unsafe {
        let actual = actual.cast::<NSRange>();
        let Some((text, length)) = snapshot(view) else {
            return nil;
        };
        let Some(range) = bounded(range, length) else {
            return nil;
        };
        let string: id = msg_send![text, string];
        let mut start: NSUInteger = 0;
        let mut end: NSUInteger = 0;
        let _: () = msg_send![string, getParagraphStart: &mut start end: &mut end contentsEnd: ptr::null_mut::<NSUInteger>() forRange: range];
        let range = NSRange {
            location: start,
            length: end - start,
        };
        if !actual.is_null() {
            *actual = range;
        }
        msg_send![text, attributedSubstringFromRange: range]
    }
}

extern "C" fn set_annotations(view: &Object, _: Sel, annotations: id, range: NSRange) {
    unsafe {
        if let Some((text, length)) = snapshot(view)
            && let Some(range) = bounded(range, length)
        {
            let _: () = msg_send![text, setAttributes: annotations range: range];
        }
    }
}

extern "C" fn add_annotations(view: &Object, _: Sel, annotations: id, range: NSRange) {
    unsafe {
        if let Some((text, length)) = snapshot(view)
            && let Some(range) = bounded(range, length)
        {
            let _: () = msg_send![text, addAttributes: annotations range: range];
        }
    }
}

extern "C" fn remove_annotation(view: &Object, _: Sel, annotation: id, range: NSRange) {
    unsafe {
        if let Some((text, length)) = snapshot(view)
            && let Some(range) = bounded(range, length)
        {
            let _: () = msg_send![text, removeAttribute: annotation range: range];
        }
    }
}

extern "C" fn replace_characters(view: &Object, _: Sel, range: NSRange, replacement: id) {
    if !enabled(view) || replacement.is_null() {
        return;
    }
    unsafe {
        let Some((text, length)) = snapshot(view) else {
            return;
        };
        let Some(range) = bounded(range, length) else {
            return;
        };
        let value: id = msg_send![replacement, string];

        let _: () =
            msg_send![text, replaceCharactersInRange: range withAttributedString: replacement];
        with_input_handler(view, |handler| {
            handler.replace_text_in_range(range.to_range(), value.to_str());
        });
    }
}

extern "C" fn select_range(view: &Object, _: Sel, range: NSRange) {
    unsafe {
        if let Some((_, length)) = snapshot(view)
            && let Some(range) = bounded(range, length)
        {
            with_input_handler(view, |handler| {
                handler.set_selected_text_range(range.to_range().unwrap())
            });
        }
    }
}

extern "C" fn view_for_range(
    view: &Object,
    _: Sel,
    range: NSRange,
    rect: *mut c_void,
    actual: *mut c_void,
) -> id {
    unsafe {
        let rect = rect.cast::<NSRect>();
        let actual = actual.cast::<NSRange>();
        let Some((_, length)) = snapshot(view) else {
            return nil;
        };
        let Some(range) = bounded(range, length) else {
            return nil;
        };
        let bounds = with_input_handler(view, |handler| {
            handler.bounds_for_range(range.to_range().unwrap())
        })
        .flatten();
        let Some(bounds) = bounds else {
            return nil;
        };

        let frame: NSRect = msg_send![view, bounds];
        if !rect.is_null() {
            *rect = NSRect::new(
                NSPoint::new(
                    bounds.origin.x.as_f32() as f64,
                    frame.size.height
                        - bounds.origin.y.as_f32() as f64
                        - bounds.size.height.as_f32() as f64,
                ),
                NSSize::new(
                    bounds.size.width.as_f32() as f64,
                    bounds.size.height.as_f32() as f64,
                ),
            );
        }
        if !actual.is_null() {
            *actual = range;
        }
        view as *const Object as id
    }
}

extern "C" fn no_candidates(_: &Object, _: Sel) -> id {
    nil
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_ranges_stay_inside_the_document() {
        std::panic::set_hook(Box::new(|info| {
            eprintln!("{info}");
            std::process::exit(101);
        }));
        let entire = bounded(NSRange::invalid(), 9).unwrap();
        assert_eq!((entire.location, entire.length), (0, 9));
        assert!(
            bounded(
                NSRange {
                    location: 10,
                    length: 1
                },
                9
            )
            .is_none()
        );
        let clipped = bounded(
            NSRange {
                location: 7,
                length: NSUInteger::MAX,
            },
            9,
        )
        .unwrap();
        assert_eq!((clipped.location, clipped.length), (7, 2));
        assert_eq!(accepted_range(0..6, 6, 1), Some(0..1));
        assert_eq!(accepted_range(0..0, 0, 2), Some(0..2));
        assert_eq!(accepted_range(6..6, 6, 1), None);
    }
}
