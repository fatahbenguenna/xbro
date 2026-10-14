//! Visual inspection aid: with `XBRO_DUMP=<dir>` set, renders border
//! styles, rounded borders, gradients, text with decorations, form
//! controls and list markers at 1x and 2x into PNGs in `<dir>` (does
//! nothing otherwise).
mod common;

use common::*;
use xbro_layout::{BoxKind, ControlKind, Fragment, ListMarker, MarkerContent, Point, Rect, Size};
use xbro_paint::{
    build_display_list_full, BorderSideItem, DisplayItem, DisplayList, RasterOptions,
};
use xbro_style::{BorderStyle, Color, LengthPercentage, TextDecorationLine};

#[test]
fn dump() {
    let Ok(dir) = std::env::var("XBRO_DUMP") else {
        return;
    };
    let Some(fonts) = fonts() else { return };
    let r = Rect::new;
    let side = |w, s, c| BorderSideItem {
        width: w,
        style: s,
        color: c,
    };
    let mut items = Vec::new();
    let styles = [
        BorderStyle::Solid,
        BorderStyle::Dashed,
        BorderStyle::Dotted,
        BorderStyle::Double,
        BorderStyle::Groove,
        BorderStyle::Ridge,
        BorderStyle::Inset,
        BorderStyle::Outset,
    ];
    for (i, s) in styles.iter().enumerate() {
        let x = 10.0 + i as f32 * 70.0;
        items.push(DisplayItem::Border {
            rect: r(x, 10.0, 60.0, 40.0),
            sides: [side(6.0, *s, Color::rgb(120, 140, 200)); 4],
            radii: [0.0; 4],
        });
        items.push(DisplayItem::Border {
            rect: r(x, 60.0, 60.0, 40.0),
            sides: [side(4.0, *s, Color::rgb(200, 80, 60)); 4],
            radii: [10.0; 4],
        });
    }
    items.push(DisplayItem::Border {
        rect: r(10.0, 110.0, 80.0, 50.0),
        sides: [
            side(8.0, BorderStyle::Solid, RED),
            side(8.0, BorderStyle::Solid, LIME),
            side(8.0, BorderStyle::Solid, BLUE),
            side(8.0, BorderStyle::Solid, BLACK),
        ],
        radii: [0.0; 4],
    });
    items.push(DisplayItem::Border {
        rect: r(100.0, 110.0, 80.0, 50.0),
        sides: [
            side(8.0, BorderStyle::Solid, RED),
            side(8.0, BorderStyle::Solid, LIME),
            side(8.0, BorderStyle::Solid, BLUE),
            side(8.0, BorderStyle::Solid, BLACK),
        ],
        radii: [20.0; 4],
    });
    items.push(DisplayItem::LinearGradient {
        rect: r(190.0, 110.0, 120.0, 50.0),
        angle_deg: 135.0,
        stops: vec![(RED, 0.0), (Color::rgb(255, 255, 0), 0.5), (BLUE, 1.0)],
        radii: [8.0; 4],
    });
    items.push(DisplayItem::LinearGradient {
        rect: r(320.0, 110.0, 120.0, 50.0),
        angle_deg: 90.0,
        stops: vec![(RED, 0.0), (Color::TRANSPARENT, 1.0)],
        radii: [0.0; 4],
    });
    let list = DisplayList {
        items,
        background: Color::rgb(250, 250, 245),
    };

    // Tree with text, controls, markers.
    let mut children = Vec::new();
    let deco = style(|s| {
        s.color = Color::rgb(0, 0, 238);
        s.decorations_in_effect = TextDecorationLine {
            underline: true,
            ..TextDecorationLine::NONE
        };
        s.decoration_color = Color::rgb(0, 0, 238);
    });
    let strike = style(|s| {
        s.decorations_in_effect = TextDecorationLine {
            line_through: true,
            overline: true,
            ..TextDecorationLine::NONE
        };
        s.decoration_color = RED;
    });
    let mut para = block(3, r(10.0, 180.0, 600.0, 30.0), style(|_| {}));
    let t1 = text(
        fonts,
        4,
        "Hello, xbro! The quick brown fox ",
        30.0,
        200.0,
        16.0,
        style(|_| {}),
    );
    let x1 = 30.0 + t1.rect.width;
    let t2 = text(fonts, 5, "jumps over", x1, 200.0, 16.0, deco);
    let x2 = x1 + t2.rect.width;
    let t3 = text(fonts, 6, " the lazy dog.", x2, 200.0, 16.0, strike);
    para.lines = vec![line(
        r(10.0, 180.0, 600.0, 30.0),
        200.0,
        vec![Fragment::Text(t1), Fragment::Text(t2), Fragment::Text(t3)],
    )];
    para.marker = Some(ListMarker {
        rect: r(14.0, 192.0, 6.0, 6.0),
        color: BLACK,
        content: MarkerContent::Disc,
    });
    children.push(para);
    let control = |node, x: f32, w: f32, h: f32, kind| {
        let mut b = bordered(
            node,
            r(x, 230.0, w, h),
            style(|s| {
                if matches!(kind, ControlKind::TextField | ControlKind::Select) {
                    s.border = solid(1.0, Color::rgb(0x76, 0x76, 0x76));
                    s.background_color = WHITE;
                }
            }),
        );
        b.kind = BoxKind::Control(kind);
        b
    };
    children.push(control(
        10,
        10.0,
        13.0,
        13.0,
        ControlKind::Checkbox { checked: true },
    ));
    children.push(control(
        11,
        30.0,
        13.0,
        13.0,
        ControlKind::Checkbox { checked: false },
    ));
    children.push(control(
        12,
        50.0,
        13.0,
        13.0,
        ControlKind::Radio { checked: true },
    ));
    children.push(control(
        13,
        70.0,
        13.0,
        13.0,
        ControlKind::Radio { checked: false },
    ));
    children.push(control(
        14,
        90.0,
        120.0,
        16.0,
        ControlKind::Range { value: 0.3 },
    ));
    children.push(control(
        15,
        220.0,
        44.0,
        23.0,
        ControlKind::Color(Color::rgb(200, 50, 120)),
    ));
    children.push(control(
        16,
        280.0,
        160.0,
        16.0,
        ControlKind::Progress { value: 0.6 },
    ));
    children.push(control(17, 450.0, 100.0, 22.0, ControlKind::Select));
    let mut rounded = block(
        20,
        r(10.0, 270.0, 100.0, 60.0),
        style(|s| {
            s.background_color = Color::rgb(30, 160, 90);
            s.border_radius = [LengthPercentage::px(16.0); 4];
            s.opacity = 0.5;
        }),
    );
    rounded.children = vec![];
    children.push(rounded);
    let _ = Point::ZERO;
    let tree = page(Size::new(640.0, 360.0), children);
    let list2 = build_display_list_full(&tree, fonts, &xbro_paint::NoImageProvider);

    for (name, list, scale) in [
        ("shapes", &list, 1.0f32),
        ("shapes2x", &list, 2.0),
        ("tree", &list2, 1.0),
        ("tree2x", &list2, 2.0),
    ] {
        let w = (640.0 * scale) as u32;
        let h = (360.0 * scale) as u32;
        let options = RasterOptions {
            scale,
            ..RasterOptions::default()
        };
        let p = render_with(list, w, h, &options, &xbro_paint::NoImageProvider);
        p.save_png(format!("{dir}/{name}.png")).unwrap();
    }
}
