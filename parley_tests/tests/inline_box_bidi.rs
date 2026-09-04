// Copyright 2026 the Parley Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! In-flow inline boxes take part in bidi resolution.
//!
//! [UAX #9 § 3.2][uax9-types] specifies that "for the purpose of the Bidirectional Algorithm,
//! inline objects (such as graphics) are treated as if they are an U+FFFC OBJECT REPLACEMENT
//! CHARACTER". An in-flow [`InlineBox`] is such an object, so it takes the bidi level resolved for
//! that character rather than inheriting one from neighbouring text.
//!
//! [uax9-types]: https://www.unicode.org/reports/tr9/#Bidirectional_Character_Types

use crate::{
    test_name,
    util::{ColorBrush, TestEnv},
};
use core::ops::Range;
use parley::{
    Alignment, AlignmentOptions, BaseDirection, InlineBox, InlineBoxKind, Layout,
    PositionedLayoutItem,
};

const BOX_WIDTH: f32 = 20.0;
const BOX_HEIGHT: f32 = 20.0;

/// Three Arabic letters: strongly right-to-left text.
const ARABIC: &str = "\u{0627}\u{0628}\u{0629}";

fn build(
    env: &mut TestEnv,
    text: &str,
    base_direction: BaseDirection,
    boxes: &[(u64, InlineBoxKind, usize)],
) -> Layout<ColorBrush> {
    let mut builder = env.ranged_builder(text);
    builder.set_base_direction(base_direction);
    for &(id, kind, index) in boxes {
        builder.push_inline_box(InlineBox {
            id,
            kind,
            index,
            width: BOX_WIDTH,
            height: BOX_HEIGHT,
            baseline: None,
        });
    }
    let mut layout = builder.build(text);
    layout.break_all_lines(None);
    layout.align(Alignment::Start, AlignmentOptions::default());
    layout
}

/// The leading edge of the box with the given id.
#[track_caller]
fn box_x(layout: &Layout<ColorBrush>, id: u64) -> f32 {
    layout
        .lines()
        .flat_map(|line| line.items())
        .find_map(|item| match item {
            PositionedLayoutItem::InlineBox(inline_box) if inline_box.id == id => {
                Some(inline_box.x)
            }
            _ => None,
        })
        .unwrap_or_else(|| panic!("inline box {id} was not positioned"))
}

/// The leading edge of the glyph run covering `text_range`.
#[track_caller]
fn glyph_run_x(layout: &Layout<ColorBrush>, text_range: Range<usize>) -> f32 {
    layout
        .lines()
        .flat_map(|line| line.items())
        .find_map(|item| match item {
            PositionedLayoutItem::GlyphRun(run) if run.run().text_range() == text_range => {
                Some(run.offset())
            }
            _ => None,
        })
        .unwrap_or_else(|| panic!("no glyph run covers {text_range:?}"))
}

/// The number of glyph runs in the layout.
fn glyph_run_count(layout: &Layout<ColorBrush>) -> usize {
    layout
        .lines()
        .flat_map(|line| line.items())
        .filter(|item| matches!(item, PositionedLayoutItem::GlyphRun(_)))
        .count()
}

/// A box at the logical start of a right-to-left paragraph belongs at the *visual* end of the
/// line, i.e. its right edge is the line's right edge.
///
/// Previously the box inherited the bidi level of the preceding run, and there is no preceding run
/// at the start of the paragraph, so it fell back to level 0 and was placed at the left.
#[test]
fn inline_box_bidi_rtl_paragraph_start() {
    let mut env = TestEnv::new(test_name!(), None);

    let layout = build(
        &mut env,
        ARABIC,
        BaseDirection::Auto,
        &[(0, InlineBoxKind::InFlow, 0)],
    );

    let x = box_x(&layout, 0);
    assert!(
        (x + BOX_WIDTH - layout.width()).abs() < 0.01,
        "box at the logical start of RTL text should sit at the line's right edge: \
         box spans {x}..{} of a {} wide line",
        x + BOX_WIDTH,
        layout.width()
    );
}

/// The mirror image: a box at the logical end of a right-to-left paragraph belongs at the visual
/// start of the line.
#[test]
fn inline_box_bidi_rtl_paragraph_end() {
    let mut env = TestEnv::new(test_name!(), None);

    let layout = build(
        &mut env,
        ARABIC,
        BaseDirection::Auto,
        &[(0, InlineBoxKind::InFlow, ARABIC.len())],
    );

    assert!(
        box_x(&layout, 0).abs() < 0.01,
        "box at the logical end of RTL text should sit at the line's left edge, not at {}",
        box_x(&layout, 0)
    );
}

/// A box in a left-to-right paragraph resolves to the paragraph level and stays put.
#[test]
fn inline_box_bidi_ltr_paragraph() {
    let mut env = TestEnv::new(test_name!(), None);

    let text = "abcd";
    let with_box = build(
        &mut env,
        text,
        BaseDirection::Auto,
        &[(0, InlineBoxKind::InFlow, 2)],
    );
    let without_box = build(&mut env, text, BaseDirection::Auto, &[]);

    // The box splits the text into two runs, the second of which is displaced by the box's width.
    assert_eq!(glyph_run_count(&with_box), 2);
    let ab = glyph_run_x(&with_box, 0..2);
    let cd = glyph_run_x(&with_box, 2..4);
    let x = box_x(&with_box, 0);
    assert!(
        ab < x && x < cd,
        "an LTR layout should read \"ab\", box, \"cd\" left to right, \
         but they are at {ab}, {x} and {cd}"
    );
    assert!(
        (with_box.width() - without_box.width() - BOX_WIDTH).abs() < 0.01,
        "the box should add exactly its own width"
    );
}

/// The decisive case: the box's level is *resolved*, not inherited from either neighbour.
///
/// In a right-to-left paragraph, `1<box>2` resolves to levels `[2, 1, 2]`: the digits are
/// `EN`, which UAX #9 I2 raises from the odd embedding level 1 to level 2, while the object
/// replacement character is a neutral that N1 resolves to `R` — the digits on both sides "act as
/// if they were R in terms of their influence on adjacent NIs" — leaving it at level 1.
///
/// So the box's level matches *neither* neighbour, and reordering puts the digits in RTL order
/// around it. Inheriting the preceding run's level 2 would instead leave all three items at the
/// same level and lay the digits out left to right.
#[test]
fn inline_box_bidi_level_is_resolved_not_inherited() {
    let mut env = TestEnv::new(test_name!(), None);

    let text = "12";
    let layout = build(
        &mut env,
        text,
        BaseDirection::Rtl,
        &[(0, InlineBoxKind::InFlow, 1)],
    );

    assert_eq!(
        glyph_run_count(&layout),
        2,
        "the box splits the digits into two runs"
    );
    let one = glyph_run_x(&layout, 0..1);
    let two = glyph_run_x(&layout, 1..2);
    let x = box_x(&layout, 0);
    assert!(
        two < x && x < one,
        "the digits should be reordered right-to-left around the box, so that \"2\" is \
         leftmost, but \"1\" is at {one}, the box at {x} and \"2\" at {two}"
    );
}

/// Out-of-flow boxes are not part of the inline flow, so they take no part in bidi resolution and
/// must not consume the level resolved for an in-flow box.
#[test]
fn inline_box_bidi_out_of_flow_box_does_not_shift_levels() {
    let mut env = TestEnv::new(test_name!(), None);

    let reference = build(
        &mut env,
        ARABIC,
        BaseDirection::Auto,
        &[(0, InlineBoxKind::InFlow, 0)],
    );
    let with_out_of_flow = build(
        &mut env,
        ARABIC,
        BaseDirection::Auto,
        &[
            (1, InlineBoxKind::OutOfFlow, 0),
            (0, InlineBoxKind::InFlow, 0),
        ],
    );

    assert_eq!(
        reference.width(),
        with_out_of_flow.width(),
        "an out-of-flow box should not affect the layout's width"
    );
    assert_eq!(
        box_x(&reference, 0),
        box_x(&with_out_of_flow, 0),
        "an out-of-flow box should not change the in-flow box's resolved level"
    );
}

/// A layout holding nothing but in-flow boxes resolves them against the paragraph direction, so
/// they are laid out right to left in a right-to-left paragraph.
///
/// This was the second half of the same defect. Giving a box its resolved level is not enough on
/// its own: `break_next` used to raise its `needs_reorder` flag only from the
/// `LayoutItemKind::TextRun` arm, so a line made up entirely of boxes kept its logical order
/// whatever levels those boxes carried. The in-flow box arm now raises the flag too.
#[test]
fn inline_box_bidi_boxes_without_text() {
    let mut env = TestEnv::new(test_name!(), None);

    let layout = build(
        &mut env,
        "",
        BaseDirection::Rtl,
        &[(0, InlineBoxKind::InFlow, 0), (1, InlineBoxKind::InFlow, 0)],
    );

    assert!(
        box_x(&layout, 0) > box_x(&layout, 1),
        "in a right-to-left paragraph the first box belongs to the right of the second, \
         but they are at {} and {}",
        box_x(&layout, 0),
        box_x(&layout, 1)
    );
}
