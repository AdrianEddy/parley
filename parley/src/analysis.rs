// Copyright 2026 the Parley Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use crate::{Brush, InlineBoxKind, LayoutContext};

use parley_engine::break_overrides::LineBreakOverrideFn;

use parley_engine::AnalysisOptions;

use parlance::{BaseDirection, WordBreak};

pub(crate) fn analyze_text<B: Brush>(
    lcx: &mut LayoutContext<B>,
    text: &str,
    base_direction: BaseDirection,
    line_break_override: Option<&LineBreakOverrideFn>,
) {
    let text = if text.is_empty() { " " } else { text };

    // Collect the style runs' word breaks. Gaps are `WordBreak::Normal`, so only non-`Normal`s need
    // an entry.
    lcx.word_break.clear();
    lcx.word_break
        .extend(lcx.style_runs.iter().filter_map(|sr| {
            let word_break = lcx.style_table[sr.style_index as usize].word_break;
            (word_break != WordBreak::Normal).then(|| (sr.range.clone(), word_break))
        }));

    // In-flow inline boxes take part in bidi resolution as U+FFFC OBJECT REPLACEMENT CHARACTERs
    // (UAX #9 § 3.2). Out-of-flow boxes are not part of the inline flow, so they take no part in
    // it. The boxes are already sorted by index, so the offsets come out sorted as required.
    lcx.inline_object_offsets.clear();
    lcx.inline_object_offsets.extend(
        lcx.inline_boxes
            .iter()
            .filter(|inline_box| inline_box.kind == InlineBoxKind::InFlow)
            .map(|inline_box| inline_box.index.min(text.len())),
    );

    let options = AnalysisOptions {
        base_direction,
        word_break: &lcx.word_break,
        line_break_override,
        inline_objects: &lcx.inline_object_offsets,
    };
    lcx.analyzer.analyze(text, &options, &mut lcx.analysis);
}
