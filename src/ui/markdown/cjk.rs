//! Making CommonMark's emphasis rules workable for Chinese text.

use std::borrow::Cow;
use std::ops::RangeInclusive;

/// Punctuation that CJK text sets flush against the words around it. Ranges
/// hold a few non-punctuation members (fullwidth digits, 〇, 々); escaping
/// those is a no-op for parsing and keeps the table readable.
const CJK_PUNCTUATION: &[RangeInclusive<u32>] = &[
    0x2013..=0x2015, // – — ―  Chinese dashes, written doubled as ——
    0x2018..=0x201F, // ‘ ’ “ ”  Chinese quotation marks
    0x2025..=0x2027, // ‥ … ‧  Chinese ellipsis, written doubled as ……
    0x3001..=0x3020, // 、。〈〉《》「」『』【】〔〕
    0x3030..=0x303F, // 〰 〜
    0xFE10..=0xFE19, // ︐ ︑ ︒  vertical forms
    0xFE30..=0xFE6F, // ﹁ ﹂ ﹃ ﹄ ﹏  compatibility and small forms
    0xFF01..=0xFF0F, // ！ ＂ ＃ ， － ．／
    0xFF1A..=0xFF20, // ： ； ＜ ＝ ＞ ？ ＠
    0xFF3B..=0xFF40, // ［ ＼ ］ ＾ ＿ ｀
    0xFF5B..=0xFF65, // ｛ ｜ ｝ ～ ｟ ｠ ｡ ｢ ｣ ､ ･
];

/// Where the stand-in codepoints start inside the Basic Multilingual Plane's
/// private use area. Each escaped character maps to its own placeholder, so
/// the substitution is reversible without tracking positions.
const PLACEHOLDER_BASE: u32 = 0xE000;

/// The characters that can form an emphasis delimiter run.
const DELIMITERS: [char; 3] = ['*', '_', '~'];

/// CommonMark decides whether a `**` run may open or close from the two
/// characters around it. A run preceded by punctuation only closes when
/// whitespace or punctuation follows; a run followed by punctuation only opens
/// when whitespace or punctuation precedes. CJK punctuation is Unicode
/// punctuation and Chinese leaves no space around it, so `**要点：**内容` never
/// closes and `这是**「引用」**的` never opens. The asterisks reach the screen.
/// pulldown-cmark has no knob for this, and markdown-it needs its
/// "cjk-friendly" plugin for the same reason.
///
/// The fix swaps such a character for a private-use placeholder that reads as
/// a letter, then puts it back in every event carrying text. Only the flanking
/// decision changes.
///
/// It has to be surgical rather than wholesale. CJK punctuation is also what
/// *rescues* a run in the mirror-image case: `**50%**，` closes only because
/// the `，` after it is punctuation, and `“_重要_”` opens only because the `“`
/// before it is. Escaping every CJK punctuation character fixes the first pair
/// of cases and breaks the second, which is a bad trade — those shapes are
/// common in Chinese summaries. So a character is escaped only where it
/// unblocks an adjacent run and cannot be load-bearing for one:
///
/// * it sits directly before a `*`/`~` run that a letter follows, or directly
///   after a `*`/`~` run that a letter precedes;
/// * never when runs sit on both sides, where the two roles conflict;
/// * never around `_`, whose stricter intraword rules the swap can only break.
///
/// ASCII punctuation stays strict throughout: `**Note:**text` reads as literal
/// asterisks in English too.
pub(super) fn escape_cjk_punctuation(input: &str) -> (Cow<'_, str>, bool) {
    if !input.chars().any(is_escapable) {
        return (Cow::Borrowed(input), false);
    }
    // A document already carrying characters from this private-use range
    // (Nerd Font glyphs, say) cannot be unescaped without corrupting it. A
    // numeric character reference is the one way to conjure such a character
    // after the swap has run, so those bar it too.
    if input.chars().any(is_placeholder) || input.contains("&#") {
        return (Cow::Borrowed(input), false);
    }

    let characters: Vec<char> = input.chars().collect();
    let mut escaped = false;
    let output: String = characters
        .iter()
        .enumerate()
        .map(|(index, &character)| {
            if is_escapable(character) && unblocks_a_run(&characters, index) {
                escaped = true;
                placeholder_for(character).expect("an escapable character has a placeholder")
            } else {
                character
            }
        })
        .collect();
    if !escaped {
        return (Cow::Borrowed(input), false);
    }
    (Cow::Owned(output), true)
}

/// Undoes [`escape_cjk_punctuation`] for one event's text. `escaped` says
/// whether this document was rewritten at all; without it a private-use
/// character the author wrote would be mistaken for a placeholder.
pub(super) fn restore_cjk_punctuation(text: &str, escaped: bool) -> String {
    if !escaped {
        return text.to_string();
    }
    text.chars()
        .map(|character| original_for(character).unwrap_or(character))
        .collect()
}

/// Whether swapping the character at `index` lets a neighbouring run open or
/// close that CommonMark would otherwise block. See
/// [`escape_cjk_punctuation`] for why each arm is shaped the way it is.
fn unblocks_a_run(characters: &[char], index: usize) -> bool {
    let before = run_ending_at(characters, index);
    let after = run_starting_at(characters, index + 1);
    match (before, after) {
        // Both sides want the character to be a different thing.
        (Some(_), Some(_)) => false,
        // A run this character would let close, if a letter follows it.
        (None, Some(run)) => {
            relaxable(&run, characters)
                && characters
                    .get(run.end)
                    .is_some_and(|character| !is_punctuation(*character))
        }
        // A run this character would let open, if a letter precedes it.
        (Some(run), None) => {
            relaxable(&run, characters)
                && run
                    .start
                    .checked_sub(1)
                    .and_then(|previous| characters.get(previous))
                    .is_some_and(|character| !is_punctuation(*character))
        }
        (None, None) => false,
    }
}

struct Run {
    start: usize,
    end: usize,
}

fn run_starting_at(characters: &[char], start: usize) -> Option<Run> {
    let marker = *characters.get(start)?;
    if !DELIMITERS.contains(&marker) {
        return None;
    }
    let mut end = start;
    while characters.get(end) == Some(&marker) {
        end += 1;
    }
    Some(Run { start, end })
}

fn run_ending_at(characters: &[char], end: usize) -> Option<Run> {
    let marker = *characters.get(end.checked_sub(1)?)?;
    if !DELIMITERS.contains(&marker) {
        return None;
    }
    let mut start = end;
    while start > 0 && characters[start - 1] == marker {
        start -= 1;
    }
    Some(Run { start, end })
}

/// Whether a run is one the swap may safely touch.
///
/// `_` is left alone: its intraword rules mean the swap can only cost it an
/// opener or a closer, never win one. A run of four or more is two markers
/// that happen to abut, as in `**a、****b**`, and moving the boundary between
/// them loses the emphasis on one side.
fn relaxable(run: &Run, characters: &[char]) -> bool {
    matches!(characters[run.start], '*' | '~') && run.end - run.start <= MAX_RELAXABLE_RUN
}

/// The longest delimiter run that still reads as a single marker.
const MAX_RELAXABLE_RUN: usize = 3;

/// CommonMark's notion of punctuation, which also swallows whitespace here —
/// both block a run the same way, and neither is a letter.
fn is_punctuation(character: char) -> bool {
    character.is_ascii_punctuation() || !(character.is_alphanumeric() || character.is_control())
}

fn is_escapable(character: char) -> bool {
    placeholder_for(character).is_some()
}

fn placeholder_for(character: char) -> Option<char> {
    let code = u32::from(character);
    let mut base = PLACEHOLDER_BASE;
    for range in CJK_PUNCTUATION {
        if range.contains(&code) {
            return Some(
                char::from_u32(base + (code - range.start()))
                    .expect("placeholder stays inside the private use area"),
            );
        }
        base += range.end() - range.start() + 1;
    }
    None
}

fn original_for(placeholder: char) -> Option<char> {
    let code = u32::from(placeholder);
    let mut base = PLACEHOLDER_BASE;
    for range in CJK_PUNCTUATION {
        let width = range.end() - range.start() + 1;
        if (base..base + width).contains(&code) {
            return Some(
                char::from_u32(range.start() + (code - base))
                    .expect("range holds valid scalar values"),
            );
        }
        base += width;
    }
    None
}

fn is_placeholder(character: char) -> bool {
    original_for(character).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_escaped_character_round_trips_through_its_placeholder() {
        for range in CJK_PUNCTUATION {
            for code in range.clone() {
                let character = char::from_u32(code).expect("range holds valid scalar values");
                let placeholder = placeholder_for(character).expect("range member is escaped");

                assert!(is_placeholder(placeholder));
                assert_eq!(original_for(placeholder), Some(character));
            }
        }
    }
}
