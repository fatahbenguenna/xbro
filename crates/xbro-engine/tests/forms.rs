//! Forms: typing, checkboxes, radios, selects, GET/POST submission, reset,
//! keyboard focus and the focus indication.

mod common;

use common::*;
use xbro_engine::{ClickAction, NavigationRequest, Page, Point};
use xbro_paint::DisplayItem;

fn click_id(page: &mut Page, id: &str) -> ClickAction {
    let p = center_of(page, id);
    page.click(p, Point::ZERO)
}

fn checked(page: &Page, id: &str) -> bool {
    page.document().attr(by_id(page, id), "checked").is_some()
}

fn query_pairs(request: &NavigationRequest) -> Vec<(String, String)> {
    request
        .url()
        .query_pairs()
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect()
}

fn has(pairs: &[(String, String)], k: &str, v: &str) -> bool {
    pairs.iter().any(|(a, b)| a == k && b == v)
}

/// Clicks the first `<input type=submit>` whose value is `label`.
fn submit_button(page: &Page, label: &str) -> Point {
    let doc = page.document();
    let node = doc
        .descendants(doc.root())
        .find(|&n| doc.is_element_named(n, "input") && doc.attr(n, "value") == Some(label))
        .unwrap_or_else(|| panic!("no button {label}"));
    let r = page.layout_tree().node_rect(node).unwrap();
    Point::new(r.x + r.width / 2.0, r.y + r.height / 2.0)
}

#[test]
fn typing_and_implicit_get_submission() {
    let mut page = load_fixture("forms.html");
    assert!(!page.has_text_focus());
    assert_eq!(click_id(&mut page, "q"), ClickAction::Repaint);
    assert!(page.has_text_focus());
    page.insert_text("rust browser");
    page.backspace();
    page.insert_text("r!");
    assert_eq!(
        page.document().attr(by_id(&page, "q"), "value"),
        Some("rust browser!")
    );
    // The typed text is laid out.
    assert!(laid_out_text(&page).contains("rust browser!"));
    let request = page.submit_focused().expect("implicit submission");
    let NavigationRequest::Get(url) = &request else {
        panic!("expected GET, got {request:?}");
    };
    assert!(url.path().ends_with("/results.html"), "{url}");
    let pairs = query_pairs(&request);
    assert!(has(&pairs, "q", "rust browser!"), "{pairs:?}");
    assert!(has(&pairs, "lang", "fr"));
    assert!(has(&pairs, "exact", "1"));
    assert!(has(&pairs, "sort", "date"));
    assert!(has(&pairs, "src", "xbro"));
    assert!(!pairs.iter().any(|(k, _)| k == "images"));
    assert!(url.as_str().contains("q=rust+browser%21"), "{url}");
}

#[test]
fn checkboxes_radios_and_selects() {
    let mut page = load_fixture("forms.html");
    assert!(checked(&page, "exact") && !checked(&page, "images"));
    assert_eq!(click_id(&mut page, "images"), ClickAction::Repaint);
    assert!(checked(&page, "images"));
    click_id(&mut page, "exact");
    assert!(!checked(&page, "exact"));
    // Radio exclusivity.
    assert!(checked(&page, "r1") && !checked(&page, "r2"));
    assert_eq!(click_id(&mut page, "r2"), ClickAction::Repaint);
    assert!(checked(&page, "r2") && !checked(&page, "r1"));
    click_id(&mut page, "r2");
    assert!(checked(&page, "r2") && !checked(&page, "r1"));
    // Select cycling: fr → en → de → fr.
    let lang = by_id(&page, "lang");
    let selected = |page: &Page| {
        let doc = page.document();
        doc.descendants(lang)
            .find(|&o| doc.is_element_named(o, "option") && doc.attr(o, "selected").is_some())
            .and_then(|o| doc.attr(o, "value").map(str::to_string))
    };
    assert_eq!(selected(&page).as_deref(), Some("fr"));
    assert_eq!(click_id(&mut page, "lang"), ClickAction::Repaint);
    assert_eq!(selected(&page).as_deref(), Some("en"));
    assert!(laid_out_text(&page).contains("English"));
    click_id(&mut page, "lang");
    assert_eq!(selected(&page).as_deref(), Some("de"));
    click_id(&mut page, "lang");
    assert_eq!(selected(&page).as_deref(), Some("fr"));
    // Labels activate their control.
    let doc = page.document();
    let label = doc
        .descendants(doc.root())
        .find(|&n| doc.is_element_named(n, "label") && doc.attr(n, "for") == Some("q"))
        .unwrap();
    let r = page.layout_tree().node_rect(label).unwrap();
    page.click(Point::new(r.x + 3.0, r.y + r.height / 2.0), Point::ZERO);
    assert_eq!(page.focused(), Some(by_id(&page, "q")));
    // The submit button sends the new state.
    page.insert_text("x");
    let p = submit_button(&page, "Chercher");
    let ClickAction::Navigate(request) = page.click(p, Point::ZERO) else {
        panic!("submit expected");
    };
    let pairs = query_pairs(&request);
    assert!(
        has(&pairs, "images", "1") && has(&pairs, "sort", "score"),
        "{pairs:?}"
    );
    assert!(!pairs.iter().any(|(k, _)| k == "exact"));
    assert!(has(&pairs, "q", "x"));
}

#[test]
fn reset_restores_initial_values() {
    let mut page = load_fixture("forms.html");
    click_id(&mut page, "q");
    page.insert_text("abc");
    click_id(&mut page, "images");
    click_id(&mut page, "r2");
    click_id(&mut page, "lang");
    let p = submit_button(&page, "Effacer");
    assert_eq!(page.click(p, Point::ZERO), ClickAction::Repaint);
    assert_eq!(page.document().attr(by_id(&page, "q"), "value"), None);
    assert!(!checked(&page, "images") && checked(&page, "r1") && !checked(&page, "r2"));
    assert!(laid_out_text(&page).contains("Français"));
    // `<button type=button>` does nothing.
    let doc = page.document();
    let button = doc
        .descendants(doc.root())
        .find(|&n| doc.is_element_named(n, "button") && doc.attr(n, "type") == Some("button"))
        .unwrap();
    let r = page.layout_tree().node_rect(button).unwrap();
    assert_eq!(
        page.click(Point::new(r.x + 4.0, r.y + 4.0), Point::ZERO),
        ClickAction::None
    );
}

#[test]
fn post_form_fields() {
    let mut page = load_fixture("forms.html");
    // The disabled submit button does nothing; the <button> submits.
    assert_eq!(
        page.click(submit_button(&page, "Envoyer"), Point::ZERO),
        ClickAction::None
    );
    click_id(&mut page, "msg");
    page.insert_text("\nMerci");
    let doc = page.document();
    let button = doc
        .descendants(doc.root())
        .filter(|&n| doc.is_element_named(n, "button"))
        .last()
        .unwrap();
    let r = page.layout_tree().node_rect(button).unwrap();
    let action = page.click(Point::new(r.x + 5.0, r.y + r.height / 2.0), Point::ZERO);
    let ClickAction::Navigate(NavigationRequest::Post { url, fields }) = action else {
        panic!("POST expected, got {action:?}");
    };
    assert!(url.path().ends_with("/post-target"));
    let get = |k: &str| fields.iter().find(|(a, _)| a == k).map(|(_, v)| v.as_str());
    assert_eq!(get("email"), Some("ferris@example.com"));
    assert_eq!(get("pw"), Some("secret"));
    assert_eq!(
        get("msg"),
        Some("Bonjour,\r\nceci est un textarea\r\nsur plusieurs lignes.\r\nMerci")
    );
    assert_eq!(fields.len(), 3, "{fields:?}");
}

#[test]
fn keyboard_focus_ring_and_caret() {
    let mut page = load_fixture("forms.html");
    let base_items = page.display_list().len();
    let rect = page.focus_next(false).expect("first focusable");
    assert_eq!(page.focused(), Some(by_id(&page, "q")));
    assert!(rect.width > 0.0);
    // Ring (Border) + caret (Rect) appended.
    let items = &page.display_list().items;
    assert!(items.len() >= base_items + 2);
    let ring = items.iter().rev().find_map(|i| match i {
        DisplayItem::Border { sides, .. } => Some(sides[0]),
        _ => None,
    });
    let ring = ring.expect("focus ring");
    assert_eq!(
        (ring.color.r, ring.color.g, ring.color.b),
        (0x1a, 0x73, 0xe8)
    );
    assert_eq!(ring.width, 2.0);
    assert!(matches!(items.last(), Some(DisplayItem::Rect { rect, .. }) if rect.width == 1.0));
    // Typing moves the caret right.
    let caret_x = |page: &Page| match page.display_list().items.last() {
        Some(DisplayItem::Rect { rect, .. }) => rect.x,
        _ => f32::NAN,
    };
    let before = caret_x(&page);
    page.insert_text("hello");
    assert!(caret_x(&page) > before + 10.0);
    // Tab order: q → lang → exact → images → r1 …
    page.focus_next(false);
    assert_eq!(page.focused(), Some(by_id(&page, "lang")));
    assert!(!page.has_text_focus());
    assert_eq!(page.activate_focused(), ClickAction::Repaint);
    page.focus_next(false);
    assert_eq!(page.focused(), Some(by_id(&page, "exact")));
    assert_eq!(page.activate_focused(), ClickAction::Repaint);
    assert!(!checked(&page, "exact"));
    page.focus_next(true);
    page.focus_next(true);
    assert_eq!(page.focused(), Some(by_id(&page, "q")));
    // Backwards from the first wraps to the last focusable.
    page.focus_next(true);
    assert_ne!(page.focused(), Some(by_id(&page, "q")));
    assert!(page.blur());
    assert!(!page.blur());
    assert_eq!(page.display_list().len(), base_items);
    // Enter on a focused submit button submits.
    let doc = page.document();
    let chercher = doc
        .descendants(doc.root())
        .find(|&n| doc.attr(n, "value") == Some("Chercher"))
        .unwrap();
    while page.focused() != Some(chercher) {
        page.focus_next(false).unwrap();
    }
    assert!(matches!(page.activate_focused(), ClickAction::Navigate(_)));
    dump(&page, "forms-focus", 1024, 400);
}

#[test]
fn textarea_enter_inserts_a_line_break() {
    let mut page = load_fixture("forms.html");
    click_id(&mut page, "msg");
    assert!(page.has_text_focus());
    assert_eq!(page.submit_focused(), None);
    let msg = by_id(&page, "msg");
    assert!(page.document().text_content(msg).ends_with("lignes.\n"));
    // Clicking outside clears the focus.
    assert_eq!(
        page.click(Point::new(900.0, 700.0), Point::ZERO),
        ClickAction::Repaint
    );
    assert!(!page.has_text_focus());
    // Typing without focus is a no-op.
    page.insert_text("ignored");
    assert!(!page.document().text_content(msg).contains("ignored"));
}

#[test]
fn generic_forms_edge_cases() {
    let mut page = load_html(
        "<form action='/s' id=f><input id=a name=a value=1 readonly>\
         <input id=b name=b maxlength=3><input id=img type=image name=pos src='data:,x'>\
         <details id=d><summary id=s>More</summary><p id=hidden>Hidden text</p></details>\
         <label id=l><input type=checkbox id=c name=c> Check me</label>\
         <input type=password id=p name=p></form>\
         <a id=js href='javascript:alert(1)'>js link</a>",
    );
    // Readonly fields keep their value.
    click_id(&mut page, "a");
    assert!(page.has_text_focus());
    page.insert_text("zz");
    assert_eq!(page.document().attr(by_id(&page, "a"), "value"), Some("1"));
    // maxlength.
    click_id(&mut page, "b");
    page.insert_text("abcdef");
    assert_eq!(
        page.document().attr(by_id(&page, "b"), "value"),
        Some("abc")
    );
    // Password fields store the text but show bullets.
    click_id(&mut page, "p");
    page.insert_text("pw");
    assert_eq!(page.document().attr(by_id(&page, "p"), "value"), Some("pw"));
    assert!(laid_out_text(&page).contains("••"));
    assert!(!laid_out_text(&page).contains("pw"));
    // details/summary.
    assert!(page
        .layout_tree()
        .node_rect(by_id(&page, "hidden"))
        .is_none());
    assert_eq!(click_id(&mut page, "s"), ClickAction::Repaint);
    assert!(page.document().attr(by_id(&page, "d"), "open").is_some());
    assert!(page
        .layout_tree()
        .node_rect(by_id(&page, "hidden"))
        .is_some());
    click_id(&mut page, "s");
    assert!(page
        .layout_tree()
        .node_rect(by_id(&page, "hidden"))
        .is_none());
    // A label wrapping a checkbox toggles it.
    let l = page.layout_tree().node_rect(by_id(&page, "l")).unwrap();
    page.click(
        Point::new(l.right() - 3.0, l.y + l.height / 2.0),
        Point::ZERO,
    );
    assert!(checked(&page, "c"));
    // javascript: links are ignored.
    assert_eq!(click_id(&mut page, "js"), ClickAction::None);
    assert_eq!(page.link_at(center_of(&page, "js"), Point::ZERO), None);
    // Image buttons submit their click position.
    if page.layout_tree().node_rect(by_id(&page, "img")).is_some() {
        if let ClickAction::Navigate(request) = click_id(&mut page, "img") {
            let pairs = query_pairs(&request);
            assert!(pairs.iter().any(|(k, _)| k == "pos.x"), "{pairs:?}");
        }
    }
}
