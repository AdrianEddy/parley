# Include in-flow inline boxes in bidi analysis as U+FFFC

## The defect

An in-flow `InlineBox` is inline content: it occupies a position in the text's logical order and
must be reordered along with the text around it. Parley never gives it a bidi level of its own.
Instead, `shape_text` hands it the level of whichever shaped run happened to precede it:

```rust
// parley/src/shape/mod.rs
//
// TODO: this lets the inline box take the bidi level of the previous run, but in principle
// inline boxes should be included in bidi analysis as an object replacement character
// (U+FFFC). The box should then take the bidi level of that character.
let prev_bidi_level = if shaped_run_idx > 0 {
    layout.data.shaped_text.runs()[&shaped_run_idx - 1].bidi_level
} else {
    BidiLevel::new(0)
};
```

This is wrong wherever the preceding run's level is not the box's level, and it has no answer at
all at the start of a paragraph, where it falls back to a hardcoded level 0.

The clearest symptom is a box at the logical **start** of a right-to-left paragraph. There is no
preceding run, so the box takes level 0 and `reorder_line_items` leaves it where it is — at the
visual **left** of the line, when it belongs at the right:

```
text  = "\u{0627}\u{0628}\u{0629}"   (three Arabic letters, so the paragraph is RTL)
box   = InlineBox { kind: InFlow, index: 0, width: 20.0, .. }

before:  layout.width() == 39.02, box spans 0.00..20.00   <- wrong end of the line
after:   layout.width() == 39.02, box spans 19.02..39.02
```

The same bug is invisible at the logical *end* of that paragraph, where the preceding run's level
happens to be the right answer, which is why it has gone unnoticed.

## What the specifications say

[UAX #9 § 3.2 *Bidirectional Character Types*][uax9-types] is explicit:

> For the purpose of the Bidirectional Algorithm, inline objects (such as graphics) are treated as
> if they are an U+FFFC OBJECT REPLACEMENT CHARACTER.

U+FFFC has `Bidi_Class=ON` (Other Neutral), so an inline object is a *neutral*: rules N0–N2 resolve
it against the surrounding strong context, and rules I1/I2 then give it a level. CSS agrees — [CSS
Writing Modes 3 § 2.4.3 *Bidi Treatment of Atomic Inlines*][css-atomic] says "all other atomic
inline-level boxes are treated as neutral characters always".

Crucially, a neutral's resolved level is **not** in general the level of either of its neighbours.
In a right-to-left paragraph, `1<object>2` resolves to levels `[2, 1, 2]`:

- the digits are `EN`, and I2 raises them from the odd embedding level 1 to level 2;
- N1 resolves the object to `R` (European and Arabic numbers "act as if they were R in terms of
  their influence on adjacent NIs"), and I2 leaves an `R` at an odd level alone, so it stays at 1.

So the object's level matches neither neighbour, and no amount of copying a neighbouring run's
level can produce it. It has to come out of the bidi resolver.

## The change

`parley_engine`'s analysis pass now accepts the byte offsets at which inline objects sit, splices a
U+FFFC into the character stream it feeds to the bidi resolver at each of them, and reports the
levels the resolver assigns them:

```rust
pub struct AnalysisOptions<'a> {
    ...
    /// The byte offsets of inline objects positioned within the text.
    pub inline_objects: &'a [usize],
}

impl Analysis {
    /// The bidi level resolved for each inline object, parallel to
    /// `AnalysisOptions::inline_objects`.
    pub fn inline_object_levels(&self) -> &[BidiLevel];
}
```

`parley` passes the offsets of its in-flow boxes and gives each box the level that comes back.

Three properties of the design are worth calling out:

- **Only bidi is affected.** The object replacement characters are spliced into the iterator that
  feeds `BidiResolver::resolve` and nowhere else. The line, word and grapheme segmenters see the
  text exactly as given, so no line-breaking, word-selection or cursor behaviour changes. This is
  deliberate: UAX #14 gives U+FFFC the class `CB`, and turning a box into a `CB` would change break
  opportunities around it — a separate question from this defect, and one where parley's current
  "we can always line break after an inline box" already matches [CSS Text 3 § 5.1][css-wrap]
  ("there is a soft wrap opportunity before and after each replaced element or other atomic
  inline"). Line breaking is left exactly as it was.
- **`Analysis` stays parallel to the text.** `char_info()` and `bidi_levels()` still have one entry
  per character of the text; the objects' levels are reported separately. The resolver's output is
  split back apart after resolution rather than leaking the augmented indexing to consumers.
- **Out-of-flow boxes are excluded.** `OutOfFlow` and `CustomOutOfFlow` boxes are not part of the
  inline flow — they are positioned "as if they were a zero-sized inline box" — so they contribute
  no neutral to the resolution and would perturb the surrounding text's levels if they did. They
  keep taking the surrounding text's level, as before.

There is no cost for text without inline objects: `inline_objects` is empty, and the resolver is
driven by the same iterator, and its level vector moved into `Analysis` by the same swap, as
before.

## Observable difference

| case | before | after |
| --- | --- | --- |
| in-flow box at the logical start of an RTL paragraph | placed at the line's left edge | placed at the line's right edge |
| in-flow box at the logical end of an RTL paragraph | left edge (right, by accident) | left edge (right, by construction) |
| in-flow box in an LTR paragraph | level 0 | level 0 — unchanged |
| `1<box>2` in an RTL paragraph | box takes level 2 from the preceding run, so the digits lay out left to right | box resolves to level 1, so the digits are reordered right to left around it |
| caret at the logical start of that RTL paragraph | at the far right, past the misplaced box | at the text's leading edge, next to the box |
| box among out-of-flow boxes | — | unchanged; out-of-flow boxes take no part |
| line breaking, word segmentation, content widths | — | unchanged |

`parley_tests::inline_box_bidi` covers each of these. No existing snapshot changes: the suite has
no test that combines right-to-left text with an inline box, and left-to-right text does not reach
the bidi resolver at all.

## The second half: a line of only boxes was never reordered

Resolving a box's level is not enough on its own. `break_next` raised its `needs_reorder` flag
only from the `LayoutItemKind::TextRun` arm, so a line made up entirely of inline boxes kept its
logical order however its boxes' levels resolved — the correct level was computed and then nothing
acted on it.

`reorder_line_items` already reads `bidi_level` from *every* line item, boxes included, and
`LineItemData::push_inline_box` already carries one. Only the flag was missing, so the in-flow box
arm now raises it exactly as the text arm does:

```rust
// parley/src/layout/line_break.rs
LayoutItemKind::InlineBox => {
    let item = &self.layout.data.inline_boxes[line_item.index];
    if item.kind == InlineBoxKind::InFlow {
        have_metrics = true;

        // An in-flow box carries the level resolved for the U+FFFC that stands in for it
        // (UAX #9 3.2), so it raises the reorder flag exactly as a text run does.
        if line_item.bidi_level != BidiLevel::new(0) {
            needs_reorder = true;
        }
    }
}
```

Out-of-flow boxes are deliberately excluded, for the same reason they are excluded from the object
stream: they take no part in resolution and keep the surrounding text's level, so nothing about
their placement should change.

This is not a regression fix in isolation — before the analysis change such boxes were all level 0
and the output was identical either way. The two halves are only useful together, which is why they
are one change. `inline_box_bidi_boxes_without_text` covers it.

## API surface

`parley`'s public API is untouched: an `InlineBox` is laid out correctly with no change to how it
is constructed or pushed. The two additions are on `parley_engine`, which is the crate that owns
analysis and has no notion of inline boxes, so it has to be told where the objects are and has to
report what it resolved for them:

- `AnalysisOptions::inline_objects` (field)
- `Analysis::inline_object_levels` (method)

Both are additive; existing callers using `..AnalysisOptions::default()` are unaffected.

A neighbouring run's level cannot substitute for either of them — see the `1<object>2` case above,
where the object's level matches neither neighbour — so the information genuinely has to cross the
`parley` / `parley_engine` boundary.

## Changelog

This needs an entry under `Unreleased`; it is omitted here because it has to cite the PR number:

- **Added / Parley Engine**: `AnalysisOptions::inline_objects` and `Analysis::inline_object_levels`,
  to include inline objects in bidi resolution as U+FFFC OBJECT REPLACEMENT CHARACTERs.
- **Fixed / Parley**: in-flow inline boxes now take their own resolved bidi level instead of
  inheriting the preceding run's, so a box at the logical start of a right-to-left paragraph is
  placed at the correct end of the line.

[uax9-types]: https://www.unicode.org/reports/tr9/#Bidirectional_Character_Types
[css-atomic]: https://www.w3.org/TR/css-writing-modes-3/#bidi-atomic-inline
[css-wrap]: https://www.w3.org/TR/css-text-3/#line-break-details
