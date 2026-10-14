//! Display list construction: canvas propagation, paint order, stacking
//! contexts, clips, opacity, fixed boxes and per-item content.

mod common;

use common::*;
use xbro_dom::NodeId;
use xbro_layout::{
    BoxKind, ControlKind, Edges, Fragment, LayoutTree, ListMarker, MarkerContent, Point, Rect, Size,
};
use xbro_paint::{
    build_display_list, build_display_list_full, build_display_list_with_images, DisplayItem,
    DisplayList,
};
use xbro_style::{
    BackgroundImage, BackgroundRepeat, BackgroundSize, Color, Float, LengthPercentage, Overflow,
    Position, TextDecorationLine, Visibility,
};

fn c(r: u8, g: u8, b: u8) -> Color {
    Color::rgb(r, g, b)
}

/// Index of the first `Rect` item with `color`.
fn rect_index(list: &DisplayList, color: Color) -> usize {
    list.items
        .iter()
        .position(|i| matches!(i, DisplayItem::Rect { color: c, .. } if *c == color))
        .unwrap_or_else(|| panic!("no rect with {color:?} in {:#?}", list.items))
}

fn has_rect(list: &DisplayList, color: Color) -> bool {
    list.items
        .iter()
        .any(|i| matches!(i, DisplayItem::Rect { color: c, .. } if *c == color))
}

fn count(list: &DisplayList, f: impl Fn(&DisplayItem) -> bool) -> usize {
    list.items.iter().filter(|i| f(i)).count()
}

fn bg(color: Color) -> std::sync::Arc<xbro_style::ComputedStyle> {
    style(|s| s.background_color = color)
}

fn r(x: f32, y: f32, w: f32, h: f32) -> Rect {
    Rect::new(x, y, w, h)
}

const VIEW: Size = Size::new(200.0, 200.0);

#[test]
fn empty_tree_is_white() {
    let tree = LayoutTree {
        root: None,
        viewport: VIEW,
        document_size: VIEW,
    };
    let list = build_display_list(&tree);
    assert_eq!(list.background, Color::WHITE);
    assert!(list.items.is_empty());
}

#[test]
fn canvas_defaults_to_white() {
    let list = build_display_list(&page(VIEW, vec![]));
    assert_eq!(list.background, Color::WHITE);
    assert!(list.is_empty());
}

#[test]
fn body_background_propagates_to_canvas() {
    let mut tree = page(VIEW, vec![]);
    let root = tree.root.as_mut().unwrap();
    root.children[0].style = bg(RED);
    let list = build_display_list(&tree);
    assert_eq!(list.background, RED);
    assert!(
        !has_rect(&list, RED),
        "body background must not be painted twice"
    );
}

#[test]
fn root_background_wins_and_body_paints_normally() {
    let mut tree = page(VIEW, vec![]);
    let root = tree.root.as_mut().unwrap();
    root.style = bg(BLUE);
    root.children[0].style = bg(RED);
    let list = build_display_list(&tree);
    assert_eq!(list.background, BLUE);
    assert!(!has_rect(&list, BLUE));
    assert!(has_rect(&list, RED));
}

#[test]
fn translucent_canvas_is_composited_over_white() {
    let mut tree = page(VIEW, vec![]);
    tree.root.as_mut().unwrap().style = bg(Color::rgba(0, 0, 0, 128));
    assert_eq!(build_display_list(&tree).background, c(127, 127, 127));
}

#[test]
fn canvas_background_image_covers_document() {
    let mut tree = page(VIEW, vec![]);
    tree.document_size = Size::new(200.0, 1000.0);
    tree.root.as_mut().unwrap().style = style(|s| {
        s.background_image = Some(BackgroundImage::Url("bg.png".into()));
    });
    let list = build_display_list(&tree);
    match &list.items[0] {
        DisplayItem::DeferredBackgroundImage { url, clip, .. } => {
            assert_eq!(url, "bg.png");
            assert_eq!(*clip, r(0.0, 0.0, 200.0, 1000.0));
        }
        other => panic!("unexpected {other:?}"),
    }
}

#[test]
fn children_paint_above_parent_backgrounds() {
    let parent = with_children(
        block(3, r(0.0, 0.0, 100.0, 100.0), bg(RED)),
        vec![block(4, r(10.0, 10.0, 50.0, 50.0), bg(BLUE))],
    );
    let list = build_display_list(&page(VIEW, vec![parent]));
    assert!(rect_index(&list, RED) < rect_index(&list, BLUE));
}

#[test]
fn positioned_boxes_paint_above_later_in_flow_blocks() {
    let abs = block(
        3,
        r(0.0, 0.0, 50.0, 50.0),
        style(|s| {
            s.position = Position::Absolute;
            s.background_color = RED;
        }),
    );
    let later = block(4, r(0.0, 0.0, 100.0, 100.0), bg(BLUE));
    let list = build_display_list(&page(VIEW, vec![abs, later]));
    assert!(rect_index(&list, BLUE) < rect_index(&list, RED));
}

#[test]
fn z_index_orders_siblings() {
    let pos = |node, z: Option<i32>, color| {
        block(
            node,
            r(0.0, 0.0, 50.0, 50.0),
            style(|s| {
                s.position = Position::Absolute;
                s.z_index = z;
                s.background_color = color;
            }),
        )
    };
    let flow = block(9, r(0.0, 0.0, 100.0, 100.0), bg(c(9, 9, 9)));
    let list = build_display_list(&page(
        VIEW,
        vec![
            pos(3, Some(2), RED),
            pos(4, Some(1), BLUE),
            pos(5, Some(-1), GREEN),
            pos(6, None, LIME),
            flow,
        ],
    ));
    let (red, blue, green, lime, flow) = (
        rect_index(&list, RED),
        rect_index(&list, BLUE),
        rect_index(&list, GREEN),
        rect_index(&list, LIME),
        rect_index(&list, c(9, 9, 9)),
    );
    assert!(green < flow, "negative z below in-flow content");
    assert!(flow < lime, "z auto above in-flow");
    assert!(lime < blue && blue < red, "positive z by value");
}

#[test]
fn equal_z_index_keeps_tree_order() {
    let pos = |node, color| {
        block(
            node,
            r(0.0, 0.0, 50.0, 50.0),
            style(|s| {
                s.position = Position::Relative;
                s.z_index = Some(3);
                s.background_color = color;
            }),
        )
    };
    let list = build_display_list(&page(VIEW, vec![pos(3, RED), pos(4, BLUE)]));
    assert!(rect_index(&list, RED) < rect_index(&list, BLUE));
}

#[test]
fn z_index_escapes_auto_positioned_ancestors() {
    // A dropdown (z-index 10) inside a relative wrapper must paint above a
    // later relative sibling.
    let dropdown = block(
        4,
        r(0.0, 20.0, 50.0, 100.0),
        style(|s| {
            s.position = Position::Absolute;
            s.z_index = Some(10);
            s.background_color = RED;
        }),
    );
    let wrapper = with_children(
        block(
            3,
            r(0.0, 0.0, 100.0, 20.0),
            style(|s| s.position = Position::Relative),
        ),
        vec![dropdown],
    );
    let later = block(
        5,
        r(0.0, 20.0, 100.0, 100.0),
        style(|s| {
            s.position = Position::Relative;
            s.background_color = BLUE;
        }),
    );
    let list = build_display_list(&page(VIEW, vec![wrapper, later]));
    assert!(rect_index(&list, BLUE) < rect_index(&list, RED));
}

#[test]
fn stacking_context_contains_its_descendants() {
    // z-index inside an opacity stacking context does not escape it.
    let inner = block(
        4,
        r(0.0, 0.0, 10.0, 10.0),
        style(|s| {
            s.position = Position::Absolute;
            s.z_index = Some(100);
            s.background_color = RED;
        }),
    );
    let ctx = with_children(
        block(
            3,
            r(0.0, 0.0, 50.0, 50.0),
            style(|s| {
                s.position = Position::Relative;
                s.z_index = Some(1);
            }),
        ),
        vec![inner],
    );
    let above = block(
        5,
        r(0.0, 0.0, 50.0, 50.0),
        style(|s| {
            s.position = Position::Relative;
            s.z_index = Some(2);
            s.background_color = BLUE;
        }),
    );
    let list = build_display_list(&page(VIEW, vec![ctx, above]));
    assert!(rect_index(&list, RED) < rect_index(&list, BLUE));
}

#[test]
fn floats_paint_between_blocks_and_inline_content() {
    let Some(fonts) = fonts() else { return };
    let float = block(
        3,
        r(0.0, 0.0, 50.0, 50.0),
        style(|s| {
            s.float = Float::Left;
            s.background_color = RED;
        }),
    );
    let mut para = block(4, r(0.0, 0.0, 200.0, 20.0), style(|_| {}));
    para.lines = vec![line(
        r(50.0, 0.0, 150.0, 20.0),
        15.0,
        vec![Fragment::Text(text(
            fonts,
            5,
            "hi",
            50.0,
            15.0,
            16.0,
            style(|_| {}),
        ))],
    )];
    let later = block(6, r(0.0, 20.0, 200.0, 20.0), bg(BLUE));
    let list = build_display_list(&page(VIEW, vec![float, para, later]));
    let text_index = list
        .items
        .iter()
        .position(|i| matches!(i, DisplayItem::Text(_)))
        .unwrap();
    assert!(
        rect_index(&list, BLUE) < rect_index(&list, RED),
        "block backgrounds before floats"
    );
    assert!(
        rect_index(&list, RED) < text_index,
        "floats before inline content"
    );
}

#[test]
fn text_paints_above_later_block_backgrounds() {
    let Some(fonts) = fonts() else { return };
    let mut para = block(3, r(0.0, 0.0, 200.0, 20.0), style(|_| {}));
    para.lines = vec![line(
        r(0.0, 0.0, 200.0, 20.0),
        15.0,
        vec![Fragment::Text(text(
            fonts,
            4,
            "Text",
            0.0,
            15.0,
            16.0,
            style(|_| {}),
        ))],
    )];
    let overlapping = block(5, r(0.0, 10.0, 200.0, 20.0), bg(BLUE));
    let list = build_display_list(&page(VIEW, vec![para, overlapping]));
    let text_index = list
        .items
        .iter()
        .position(|i| matches!(i, DisplayItem::Text(_)))
        .unwrap();
    assert!(rect_index(&list, BLUE) < text_index);
    match &list.items[text_index] {
        DisplayItem::Text(run) => {
            assert_eq!(run.origin, Point::new(0.0, 15.0));
            assert_eq!(run.color, Color::BLACK);
            assert!(!run.glyphs.is_empty());
        }
        _ => unreachable!(),
    }
}

#[test]
fn whitespace_and_transparent_text_emit_no_glyph_runs() {
    let Some(fonts) = fonts() else { return };
    let mut para = block(3, r(0.0, 0.0, 200.0, 20.0), style(|_| {}));
    para.lines = vec![line(
        r(0.0, 0.0, 200.0, 20.0),
        15.0,
        vec![
            Fragment::Text(text(fonts, 4, "   ", 0.0, 15.0, 16.0, style(|_| {}))),
            Fragment::Text(text(
                fonts,
                5,
                "x",
                20.0,
                15.0,
                16.0,
                style(|s| s.color = Color::TRANSPARENT),
            )),
            Fragment::Text(text(
                fonts,
                6,
                "y",
                40.0,
                15.0,
                16.0,
                style(|s| s.visibility = Visibility::Hidden),
            )),
        ],
    )];
    let list = build_display_list(&page(VIEW, vec![para]));
    assert_eq!(count(&list, |i| matches!(i, DisplayItem::Text(_))), 0);
}

#[test]
fn visibility_hidden_hides_own_painting_only() {
    let child = block(
        4,
        r(10.0, 10.0, 20.0, 20.0),
        style(|s| {
            s.background_color = BLUE;
            s.visibility = Visibility::Visible;
        }),
    );
    let hidden = with_children(
        block(
            3,
            r(0.0, 0.0, 100.0, 100.0),
            style(|s| {
                s.background_color = RED;
                s.border = solid(2.0, RED);
                s.outline_width = 2.0;
                s.outline_color = RED;
                s.visibility = Visibility::Hidden;
            }),
        ),
        vec![child],
    );
    let list = build_display_list(&page(VIEW, vec![hidden]));
    assert!(!has_rect(&list, RED));
    assert_eq!(count(&list, |i| matches!(i, DisplayItem::Border { .. })), 0);
    assert!(has_rect(&list, BLUE));
}

#[test]
fn opacity_wraps_everything_the_box_paints() {
    let child = block(4, r(10.0, 10.0, 20.0, 20.0), bg(BLUE));
    let translucent = with_children(
        block(
            3,
            r(0.0, 0.0, 100.0, 100.0),
            style(|s| {
                s.background_color = RED;
                s.opacity = 0.5;
            }),
        ),
        vec![child],
    );
    let list = build_display_list(&page(VIEW, vec![translucent]));
    let push = list
        .items
        .iter()
        .position(|i| *i == DisplayItem::PushOpacity(0.5))
        .unwrap();
    let pop = list
        .items
        .iter()
        .position(|i| *i == DisplayItem::PopOpacity)
        .unwrap();
    assert!(push < rect_index(&list, RED));
    assert!(rect_index(&list, BLUE) < pop);
}

#[test]
fn opacity_zero_paints_nothing() {
    let invisible = with_children(
        block(
            3,
            r(0.0, 0.0, 100.0, 100.0),
            style(|s| {
                s.background_color = RED;
                s.opacity = 0.0;
            }),
        ),
        vec![block(4, r(0.0, 0.0, 10.0, 10.0), bg(BLUE))],
    );
    let list = build_display_list(&page(VIEW, vec![invisible]));
    assert!(list.items.is_empty(), "{:?}", list.items);
}

#[test]
fn overflow_clips_descendants_not_own_background() {
    let child = block(4, r(50.0, 50.0, 100.0, 100.0), bg(BLUE));
    let mut clipper = with_children(
        bordered(
            3,
            r(0.0, 0.0, 100.0, 100.0),
            style(|s| {
                s.background_color = RED;
                s.border = solid(5.0, BLACK);
                s.overflow_x = Overflow::Hidden;
                s.overflow_y = Overflow::Hidden;
            }),
        ),
        vec![child],
    );
    clipper.padding = Edges::all(1.0);
    clipper.content = clipper.content.inset(Edges::all(1.0));
    let list = build_display_list(&page(VIEW, vec![clipper]));
    let push = list
        .items
        .iter()
        .position(|i| matches!(i, DisplayItem::PushClip { .. }))
        .expect("clip");
    match &list.items[push] {
        DisplayItem::PushClip { rect, radii } => {
            assert_eq!(*rect, r(5.0, 5.0, 90.0, 90.0), "padding box");
            assert_eq!(*radii, [0.0; 4]);
        }
        _ => unreachable!(),
    }
    assert!(
        rect_index(&list, RED) < push,
        "own background outside the clip"
    );
    let pop = list
        .items
        .iter()
        .rposition(|i| *i == DisplayItem::PopClip)
        .unwrap();
    assert!(push < rect_index(&list, BLUE) && rect_index(&list, BLUE) < pop);
}

#[test]
fn rounded_overflow_clip_uses_inner_radii() {
    let clipper = with_children(
        bordered(
            3,
            r(0.0, 0.0, 100.0, 100.0),
            style(|s| {
                s.border = solid(4.0, BLACK);
                s.border_radius = [LengthPercentage::px(10.0); 4];
                s.overflow_x = Overflow::Hidden;
                s.overflow_y = Overflow::Hidden;
            }),
        ),
        vec![block(4, r(0.0, 0.0, 100.0, 100.0), bg(BLUE))],
    );
    let list = build_display_list(&page(VIEW, vec![clipper]));
    assert!(list
        .items
        .iter()
        .any(|i| matches!(i, DisplayItem::PushClip { radii, .. } if *radii == [6.0; 4])));
}

#[test]
fn empty_clips_are_elided() {
    let clipper = block(
        3,
        r(0.0, 0.0, 100.0, 100.0),
        style(|s| {
            s.overflow_x = Overflow::Hidden;
            s.overflow_y = Overflow::Hidden;
        }),
    );
    let list = build_display_list(&page(VIEW, vec![clipper]));
    assert!(list.items.is_empty(), "{:?}", list.items);
}

#[test]
fn root_and_body_overflow_do_not_clip() {
    let mut tree = page(VIEW, vec![block(3, r(0.0, 0.0, 400.0, 400.0), bg(BLUE))]);
    let root = tree.root.as_mut().unwrap();
    root.children[0].style = style(|s| s.overflow_x = Overflow::Hidden);
    let list = build_display_list(&tree);
    assert_eq!(
        count(&list, |i| matches!(i, DisplayItem::PushClip { .. })),
        0
    );
}

#[test]
fn absolute_descendants_follow_their_containing_block_clip() {
    let abs = || {
        block(
            5,
            r(150.0, 150.0, 20.0, 20.0),
            style(|s| {
                s.position = Position::Absolute;
                s.background_color = RED;
            }),
        )
    };
    let clipper = |position| {
        with_children(
            block(
                3,
                r(0.0, 0.0, 100.0, 100.0),
                style(|s| {
                    s.position = position;
                    s.overflow_x = Overflow::Hidden;
                    s.overflow_y = Overflow::Hidden;
                }),
            ),
            vec![abs()],
        )
    };
    // Static clipper: the abspos box's containing block is outside it.
    let list = build_display_list(&page(VIEW, vec![clipper(Position::Static)]));
    assert_eq!(
        count(&list, |i| matches!(i, DisplayItem::PushClip { .. })),
        0,
        "{:?}",
        list.items
    );
    // Relative clipper: it is the containing block, so it clips.
    let list = build_display_list(&page(VIEW, vec![clipper(Position::Relative)]));
    let red = rect_index(&list, RED);
    assert!(matches!(list.items[red - 1], DisplayItem::PushClip { .. }));
    assert_eq!(list.items[red + 1], DisplayItem::PopClip);
}

#[test]
fn fixed_boxes_are_wrapped() {
    let fixed = with_children(
        block(
            3,
            r(0.0, 0.0, 200.0, 30.0),
            style(|s| {
                s.position = Position::Fixed;
                s.background_color = RED;
            }),
        ),
        vec![block(4, r(0.0, 0.0, 20.0, 20.0), bg(BLUE))],
    );
    let list = build_display_list(&page(
        VIEW,
        vec![fixed, block(5, r(0.0, 0.0, 100.0, 100.0), bg(GREEN))],
    ));
    let push = list
        .items
        .iter()
        .position(|i| *i == DisplayItem::PushFixed)
        .unwrap();
    let pop = list
        .items
        .iter()
        .position(|i| *i == DisplayItem::PopFixed)
        .unwrap();
    assert!(rect_index(&list, GREEN) < push);
    assert!(push < rect_index(&list, RED) && rect_index(&list, BLUE) < pop);
}

#[test]
fn borders_and_outlines() {
    let b = bordered(
        3,
        r(10.0, 10.0, 50.0, 40.0),
        style(|s| {
            s.border.top = xbro_style::BorderSide {
                width: 1.0,
                style: xbro_style::BorderStyle::Solid,
                color: RED,
            };
            s.border.left = xbro_style::BorderSide {
                width: 3.0,
                style: xbro_style::BorderStyle::Dashed,
                color: BLUE,
            };
            s.outline_width = 2.0;
            s.outline_color = GREEN;
        }),
    );
    let list = build_display_list(&page(VIEW, vec![b]));
    let borders: Vec<_> = list
        .items
        .iter()
        .filter_map(|i| match i {
            DisplayItem::Border { rect, sides, .. } => Some((*rect, *sides)),
            _ => None,
        })
        .collect();
    assert_eq!(borders.len(), 2);
    let (rect, sides) = borders[0];
    assert_eq!(rect, r(10.0, 10.0, 50.0, 40.0));
    assert_eq!(sides[0].color, RED);
    assert_eq!(sides[0].width, 1.0);
    assert_eq!(sides[3].width, 3.0);
    assert_eq!(sides[3].style, xbro_style::BorderStyle::Dashed);
    assert_eq!(sides[1].width, 0.0);
    let (outline, sides) = borders[1];
    assert_eq!(outline, r(8.0, 8.0, 54.0, 44.0));
    assert!(sides.iter().all(|s| s.color == GREEN && s.width == 2.0));
}

#[test]
fn radii_are_resolved_and_clamped() {
    let b = block(
        3,
        r(0.0, 0.0, 100.0, 40.0),
        style(|s| {
            s.background_color = RED;
            s.border_radius = [LengthPercentage::percent(50.0); 4];
        }),
    );
    let list = build_display_list(&page(VIEW, vec![b]));
    match &list.items[rect_index(&list, RED)] {
        DisplayItem::Rect { radii, .. } => assert_eq!(*radii, [20.0; 4]),
        _ => unreachable!(),
    }
}

#[test]
fn gradient_and_background_images() {
    let stops = vec![(RED, 0.0), (BLUE, 1.0)];
    let grad = block(
        3,
        r(0.0, 0.0, 50.0, 50.0),
        style(|s| {
            s.background_image = Some(BackgroundImage::LinearGradient {
                angle_deg: 90.0,
                stops: stops.clone(),
            });
        }),
    );
    let mut img = block(
        4,
        r(0.0, 50.0, 100.0, 60.0),
        style(|s| {
            s.background_image = Some(BackgroundImage::Url("tile.png".into()));
            s.background_repeat = BackgroundRepeat::RepeatX;
            s.background_position = (LengthPercentage::percent(100.0), LengthPercentage::px(5.0));
        }),
    );
    img.border = Edges::all(2.0);
    img.content = img.content.inset(Edges::all(2.0));
    let tree = page(VIEW, vec![grad, img]);

    let list = build_display_list(&tree);
    assert!(list.items.iter().any(|i| matches!(i,
        DisplayItem::LinearGradient { rect, angle_deg, stops: s, .. }
            if *rect == r(0.0, 0.0, 50.0, 50.0) && *angle_deg == 90.0 && *s == stops)));
    assert!(list.items.iter().any(|i| matches!(i,
        DisplayItem::DeferredBackgroundImage { area, clip, repeat: BackgroundRepeat::RepeatX, .. }
            if *area == r(2.0, 52.0, 96.0, 56.0) && *clip == r(0.0, 50.0, 100.0, 60.0))));

    let mut images = MapImages::default();
    images
        .urls
        .insert("tile.png".into(), solid_image(10, 20, RED));
    let list = build_display_list_with_images(&tree, &images);
    let bg = list.items.iter().find_map(|i| match i {
        DisplayItem::BackgroundImage {
            origin, tile, clip, ..
        } => Some((*origin, *tile, *clip)),
        _ => None,
    });
    assert_eq!(
        bg,
        Some((
            Point::new(88.0, 57.0),
            Size::new(10.0, 20.0),
            r(0.0, 50.0, 100.0, 60.0)
        ))
    );
}

#[test]
fn explicit_background_size_resolves_without_images() {
    let b = block(
        3,
        r(0.0, 0.0, 100.0, 100.0),
        style(|s| {
            s.background_image = Some(BackgroundImage::Url("a.svg".into()));
            s.background_size = BackgroundSize::Explicit(
                Some(LengthPercentage::px(16.0)),
                Some(LengthPercentage::px(8.0)),
            );
        }),
    );
    let list = build_display_list(&page(VIEW, vec![b]));
    assert!(list.items.iter().any(
        |i| matches!(i, DisplayItem::BackgroundImage { tile, .. } if *tile == Size::new(16.0, 8.0))
    ));
}

#[test]
fn rounded_background_images_are_clipped() {
    let b = block(
        3,
        r(0.0, 0.0, 100.0, 100.0),
        style(|s| {
            s.background_image = Some(BackgroundImage::Url("a.png".into()));
            s.border_radius = [LengthPercentage::px(8.0); 4];
        }),
    );
    let list = build_display_list(&page(VIEW, vec![b]));
    assert!(matches!(list.items[0], DisplayItem::PushClip { radii, .. } if radii == [8.0; 4]));
    assert!(matches!(
        list.items[1],
        DisplayItem::DeferredBackgroundImage { .. }
    ));
    assert_eq!(list.items[2], DisplayItem::PopClip);
}

#[test]
fn decorations_wrap_text() {
    let Some(fonts) = fonts() else { return };
    let deco = style(|s| {
        s.decorations_in_effect = TextDecorationLine {
            underline: true,
            overline: true,
            line_through: true,
        };
        s.decoration_color = RED;
    });
    let frag = text(fonts, 4, "Deco", 10.0, 30.0, 20.0, deco);
    let width = frag.rect.width;
    let mut para = block(3, r(0.0, 0.0, 200.0, 40.0), style(|_| {}));
    para.lines = vec![line(
        r(0.0, 0.0, 200.0, 40.0),
        30.0,
        vec![Fragment::Text(frag)],
    )];
    let tree = page(VIEW, vec![para]);

    for list in [
        build_display_list(&tree),
        build_display_list_full(&tree, fonts, &xbro_paint::NoImageProvider),
    ] {
        let kinds: Vec<_> = list
            .items
            .iter()
            .map(|i| match i {
                DisplayItem::TextDecoration { rect, color } => {
                    assert_eq!(*color, RED);
                    assert_eq!(rect.x, 10.0);
                    assert!((rect.width - width).abs() < 1e-3);
                    assert!(rect.height >= 1.0);
                    format!("line@{:.0}", rect.y)
                }
                DisplayItem::Text(_) => "text".to_string(),
                other => panic!("unexpected {other:?}"),
            })
            .collect();
        assert_eq!(kinds.len(), 4, "{kinds:?}");
        assert_eq!(
            kinds[2], "text",
            "underline and overline below, line-through above: {kinds:?}"
        );
        let ys: Vec<f32> = list
            .items
            .iter()
            .filter_map(|i| match i {
                DisplayItem::TextDecoration { rect, .. } => Some(rect.y),
                _ => None,
            })
            .collect();
        let (under, over, through) = (ys[0], ys[1], ys[2]);
        assert!(
            under > 30.0 && under < 34.0,
            "underline below baseline: {under}"
        );
        assert!(over < 18.0, "overline near the ascent: {over}");
        assert!(
            through > over && through < 30.0,
            "line-through in the x-height: {through}"
        );
    }
}

#[test]
fn inline_box_backgrounds_precede_their_text() {
    let Some(fonts) = fonts() else { return };
    let span = style(|s| {
        s.display = xbro_style::Display::Inline;
        s.background_color = LIME;
        s.border = solid(1.0, BLUE);
    });
    let mut para = block(3, r(0.0, 0.0, 200.0, 20.0), style(|_| {}));
    para.lines = vec![line(
        r(0.0, 0.0, 200.0, 20.0),
        15.0,
        vec![
            Fragment::InlineBox(inline_box(4, r(0.0, 0.0, 40.0, 20.0), span)),
            Fragment::Text(text(fonts, 5, "ab", 1.0, 15.0, 16.0, style(|_| {}))),
        ],
    )];
    let list = build_display_list(&page(VIEW, vec![para]));
    assert!(matches!(&list.items[0], DisplayItem::Rect { color, .. } if *color == LIME));
    assert!(matches!(&list.items[1], DisplayItem::Border { sides, .. } if sides[0].width == 1.0));
    assert!(matches!(&list.items[2], DisplayItem::Text(_)));
}

#[test]
fn atomic_inlines_paint_with_their_line_unless_positioned() {
    let mut para = block(3, r(0.0, 0.0, 200.0, 40.0), style(|_| {}));
    let ib = block(4, r(0.0, 0.0, 30.0, 30.0), bg(RED));
    let positioned = block(
        5,
        r(40.0, 0.0, 30.0, 30.0),
        style(|s| {
            s.position = Position::Relative;
            s.background_color = BLUE;
        }),
    );
    para.lines = vec![line(
        r(0.0, 0.0, 200.0, 40.0),
        30.0,
        vec![Fragment::Atomic(positioned), Fragment::Atomic(ib)],
    )];
    let later = block(6, r(0.0, 40.0, 200.0, 40.0), bg(GREEN));
    let list = build_display_list(&page(VIEW, vec![para, later]));
    assert!(
        rect_index(&list, GREEN) < rect_index(&list, RED),
        "atomic inline in the inline phase"
    );
    assert!(
        rect_index(&list, RED) < rect_index(&list, BLUE),
        "positioned atomic inline in the positioned layer"
    );
    assert_eq!(
        count(
            &list,
            |i| matches!(i, DisplayItem::Rect { color, .. } if *color == BLUE)
        ),
        1
    );
}

#[test]
fn flex_items_paint_atomically_and_z_index_applies() {
    let mut flex = block(
        3,
        r(0.0, 0.0, 200.0, 50.0),
        style(|s| s.display = xbro_style::Display::Flex),
    );
    flex.kind = BoxKind::Flex;
    let item_a = with_children(
        block(4, r(0.0, 0.0, 100.0, 50.0), style(|s| s.z_index = Some(5))),
        vec![block(7, r(0.0, 0.0, 10.0, 10.0), bg(RED))],
    );
    let item_b = block(5, r(50.0, 0.0, 100.0, 50.0), bg(BLUE));
    flex.children = vec![item_a, item_b];
    let list = build_display_list(&page(VIEW, vec![flex]));
    assert!(
        rect_index(&list, BLUE) < rect_index(&list, RED),
        "z-index on a static flex item"
    );
}

#[test]
fn list_markers() {
    let Some(fonts) = fonts() else { return };
    let marker = |content| ListMarker {
        rect: r(0.0, 0.0, 10.0, 10.0),
        color: RED,
        content,
    };
    let li = |node, content| {
        let mut b = block(node, r(20.0, 0.0, 100.0, 20.0), style(|_| {}));
        b.marker = Some(marker(content));
        b
    };
    let list = build_display_list(&page(
        VIEW,
        vec![
            li(3, MarkerContent::Disc),
            li(4, MarkerContent::Circle),
            li(5, MarkerContent::Square),
            li(
                6,
                MarkerContent::Text(text(fonts, 6, "1.", 0.0, 15.0, 16.0, style(|_| {}))),
            ),
            li(7, MarkerContent::Image("bullet.png".into())),
        ],
    ));
    let kinds: Vec<_> = list.items.iter().collect();
    assert_eq!(kinds.len(), 5, "{kinds:#?}");
    assert!(matches!(kinds[0], DisplayItem::Ellipse { fill: true, color, .. } if *color == RED));
    assert!(
        matches!(kinds[1], DisplayItem::Ellipse { fill: false, stroke_width, .. } if *stroke_width >= 1.0)
    );
    assert!(matches!(kinds[2], DisplayItem::Rect { color, .. } if *color == RED));
    assert!(matches!(kinds[3], DisplayItem::Text(run) if run.color == RED));
    assert!(
        matches!(kinds[4], DisplayItem::UrlImage { url, rect } if url == "bullet.png" && *rect == r(0.0, 0.0, 10.0, 10.0))
    );
}

#[test]
fn replaced_images_and_controls() {
    let Some(fonts) = fonts() else { return };
    let mut img = block(3, r(0.0, 0.0, 40.0, 30.0), style(|_| {}));
    img.kind = BoxKind::Image;
    let mut checkbox = block(4, r(50.0, 0.0, 13.0, 13.0), style(|_| {}));
    checkbox.kind = BoxKind::Control(ControlKind::Checkbox { checked: true });
    let mut field = bordered(
        5,
        r(0.0, 40.0, 100.0, 20.0),
        style(|s| s.border = solid(1.0, c(0x76, 0x76, 0x76))),
    );
    field.kind = BoxKind::Control(ControlKind::TextField);
    field.lines = vec![line(
        r(1.0, 41.0, 98.0, 18.0),
        55.0,
        vec![Fragment::Text(text(
            fonts,
            5,
            "a very long value that overflows",
            1.0,
            55.0,
            13.0,
            style(|_| {}),
        ))],
    )];
    let mut select = block(6, r(0.0, 70.0, 80.0, 20.0), style(|_| {}));
    select.kind = BoxKind::Control(ControlKind::Select);
    let list = build_display_list(&page(VIEW, vec![img, checkbox, field, select]));

    assert!(list.items.iter().any(|i| matches!(i, DisplayItem::NodeImage { rect, node } if *rect == r(0.0, 0.0, 40.0, 30.0) && *node == NodeId(3))));
    assert!(list.items.iter().any(|i| matches!(i, DisplayItem::Rect { color, rect, .. } if *color == c(0, 0x75, 0xff) && *rect == r(50.0, 0.0, 13.0, 13.0))));
    assert!(list
        .items
        .iter()
        .any(|i| matches!(i, DisplayItem::Polyline { color, .. } if *color == Color::WHITE)));
    let text_index = list
        .items
        .iter()
        .position(|i| matches!(i, DisplayItem::Text(_)))
        .unwrap();
    assert!(
        matches!(&list.items[text_index - 1], DisplayItem::PushClip { rect, .. } if *rect == r(1.0, 41.0, 98.0, 18.0))
    );
    assert_eq!(list.items[text_index + 1], DisplayItem::PopClip);
    assert!(list
        .items
        .iter()
        .any(|i| matches!(i, DisplayItem::Polygon { points, .. } if points.len() == 3)));
}

#[test]
fn other_controls_emit_shapes() {
    let control = |node, x: f32, kind| {
        let mut b = block(node, r(x, 0.0, 40.0, 16.0), style(|_| {}));
        b.kind = BoxKind::Control(kind);
        b
    };
    let list = build_display_list(&page(
        VIEW,
        vec![
            control(3, 0.0, ControlKind::Radio { checked: true }),
            control(4, 50.0, ControlKind::Range { value: 0.5 }),
            control(5, 100.0, ControlKind::Color(RED)),
            control(6, 150.0, ControlKind::Progress { value: f32::NAN }),
            control(7, 0.0, ControlKind::Checkbox { checked: false }),
        ],
    ));
    assert!(count(&list, |i| matches!(i, DisplayItem::Ellipse { .. })) >= 4);
    assert!(has_rect(&list, RED));
    assert!(has_rect(&list, c(0xef, 0xef, 0xef)));
}

#[test]
fn deep_trees_do_not_overflow_the_stack() {
    let handle = std::thread::Builder::new()
        .stack_size(2 << 20)
        .spawn(|| {
            let mut b = block(0, r(0.0, 0.0, 10.0, 10.0), bg(RED));
            for i in 0..5000 {
                let pos = if i % 3 == 0 {
                    Position::Relative
                } else {
                    Position::Static
                };
                b = with_children(
                    block(i + 10, r(0.0, 0.0, 10.0, 10.0), style(|s| s.position = pos)),
                    vec![b],
                );
            }
            let list = build_display_list(&page(VIEW, vec![b]));
            let _ = list.len();
        })
        .unwrap();
    handle.join().expect("no stack overflow");
}

#[test]
fn hostile_values_do_not_panic() {
    let b = block(
        3,
        r(f32::NAN, f32::INFINITY, -5.0, f32::MAX),
        style(|s| {
            s.background_color = RED;
            s.border = solid(f32::NAN, BLUE);
            s.border_radius = [
                LengthPercentage::px(f32::INFINITY),
                LengthPercentage::percent(f32::NAN),
                LengthPercentage::px(-1.0),
                LengthPercentage::px(1e30),
            ];
            s.opacity = f32::NAN;
            s.outline_width = f32::INFINITY;
            s.background_image = Some(BackgroundImage::LinearGradient {
                angle_deg: f32::NAN,
                stops: vec![(RED, f32::NAN)],
            });
        }),
    );
    let list = build_display_list(&page(VIEW, vec![b]));
    for item in &list.items {
        if let DisplayItem::Rect { rect, radii, .. } = item {
            assert!(
                rect.x.is_finite() && rect.y.is_finite() && rect.width >= 0.0 && rect.height >= 0.0
            );
            assert!(radii.iter().all(|r| r.is_finite() && *r >= 0.0));
        }
    }
    let _ = render(&list, 50, 50);
}

#[test]
fn border_top_gap_leaves_the_legend_area_unpainted() {
    let mut b = bordered(
        3,
        r(10.0, 10.0, 100.0, 50.0),
        style(|s| s.border = solid(4.0, RED)),
    );
    b.border_top_gap = Some((30.0, 70.0));
    let list = build_display_list(&page(VIEW, vec![b.clone()]));
    // The border is painted in three clipped parts.
    let clips: Vec<Rect> = list
        .items
        .iter()
        .filter_map(|i| match i {
            DisplayItem::PushClip { rect, .. } => Some(*rect),
            _ => None,
        })
        .collect();
    assert_eq!(
        clips,
        vec![
            r(10.0, 10.0, 20.0, 50.0),
            r(70.0, 10.0, 40.0, 50.0),
            r(29.0, 15.0, 42.0, 45.0)
        ]
    );
    assert_eq!(count(&list, |i| matches!(i, DisplayItem::Border { .. })), 3);
    let p = render(&list, 200, 200);
    // Top border left and right of the gap, not inside it.
    assert_eq!(px(&p, 20, 11), (255, 0, 0, 255));
    assert_eq!(px(&p, 90, 11), (255, 0, 0, 255));
    assert_eq!(px(&p, 50, 11), (255, 255, 255, 255));
    // The other sides are complete.
    assert_eq!(px(&p, 50, 58), (255, 0, 0, 255));
    assert_eq!(px(&p, 11, 30), (255, 0, 0, 255));
    assert_eq!(px(&p, 108, 30), (255, 0, 0, 255));
    // A gap outside the box or without a top border changes nothing.
    b.border_top_gap = Some((500.0, 600.0));
    let list = build_display_list(&page(VIEW, vec![b]));
    assert_eq!(
        count(&list, |i| matches!(i, DisplayItem::PushClip { .. })),
        0
    );
    assert_eq!(count(&list, |i| matches!(i, DisplayItem::Border { .. })), 1);
}

#[test]
fn border_top_gap_has_no_hairline_at_fractional_positions() {
    // At a fractional device position the clip of the part below the top
    // side used to share an anti-aliased pixel row with the top border's
    // inner edge, leaving a faint line across the gap.
    for (y, scale_top) in [(10.4, 3.0), (10.7, 2.0), (10.25, 1.5)] {
        let mut b = bordered(
            3,
            r(10.3, y, 100.0, 50.0),
            style(|s| s.border = solid(scale_top, RED)),
        );
        b.border_top_gap = Some((30.0, 70.0));
        let list = build_display_list(&page(VIEW, vec![b]));
        let p = render(&list, 200, 200);
        for py in 8..16 {
            for px_x in [40, 50, 60] {
                assert_eq!(
                    px(&p, px_x, py),
                    (255, 255, 255, 255),
                    "gap pixel ({px_x}, {py}) for y {y}"
                );
            }
        }
        // The border still runs left and right of the gap, and below.
        assert_ne!(px(&p, 20, y as u32 + 1), (255, 255, 255, 255));
        assert_ne!(px(&p, 90, y as u32 + 1), (255, 255, 255, 255));
        assert_ne!(px(&p, 50, (y + 49.0) as u32), (255, 255, 255, 255));
    }
}
