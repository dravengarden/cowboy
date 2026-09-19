// SPDX-License-Identifier: GPL-3.0-or-later
//! Validate native FullOffsets, including tombstones, without copying text.
use super::*;

impl BufferSnapshot {
    pub fn cowboy_check_full_ranges(
        &self,
        version: &clock::Global,
        ranges: &[Range<FullOffset>],
    ) -> bool {
        let mut cursor = self.fragments.cursor::<FragmentTextSummary>(&None);
        cursor.next();
        let mut full = 0usize;
        let mut previous = 0usize;
        for range in ranges {
            for offset in [range.start.0, range.end.0] {
                if offset < previous {
                    return false;
                }
                previous = offset;
                loop {
                    let Some(fragment) = cursor.item() else {
                        if offset != full {
                            return false;
                        }
                        break;
                    };
                    if !version.observed(fragment.timestamp) {
                        cursor.next();
                        continue;
                    }
                    let end = full + fragment.len as usize;
                    if offset <= end {
                        let (rope, start) = if fragment.visible {
                            (&self.visible_text, cursor.start().visible)
                        } else {
                            (&self.deleted_text, cursor.start().deleted)
                        };
                        let position = start + offset - full;
                        if rope.clip_offset(position, Bias::Left) != position {
                            return false;
                        }
                        break;
                    }
                    full = end;
                    cursor.next();
                }
            }
        }
        true
    }
}
