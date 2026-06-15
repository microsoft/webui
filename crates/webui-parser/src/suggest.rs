// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! "Did you mean …?" suggestions for authoring typos.
//!
//! When the parser encounters an unknown name (e.g. a component tag that is not
//! registered), it offers the closest registered name as a fix hint. The
//! matcher is a deterministic, iterative Levenshtein edit distance — no
//! recursion and no regular expressions — run only on the cold error path.

/// Return the candidate closest to `target` by Levenshtein edit distance, if
/// one is within a small typo tolerance.
///
/// The tolerance scales with the longer of the two names (`max_len / 3`, at
/// least 2 so a single transposition on a short word like `each`→`eahc` still
/// matches), so a one- or two-character slip matches while grossly different
/// names are rejected. On ties the first-seen candidate wins; callers that need
/// determinism across runs should pass a sorted iterator.
///
/// Marked `#[cold]`/`#[inline(never)]`: only invoked while building a build
/// error (or for the rare hyphenated-but-unregistered tag), so keeping it
/// out-of-line preserves hot parse-path code layout.
#[cold]
#[inline(never)]
#[must_use]
pub(crate) fn closest_match<'a>(
    target: &str,
    candidates: impl Iterator<Item = &'a str>,
) -> Option<&'a str> {
    let target_len = target.chars().count();
    let mut best: Option<(&str, usize)> = None;
    for candidate in candidates {
        let tolerance = (target_len.max(candidate.chars().count()) / 3).max(2);
        let distance = levenshtein(target, candidate);
        if distance <= tolerance && best.is_none_or(|(_, best_distance)| distance < best_distance) {
            best = Some((candidate, distance));
        }
    }
    best.map(|(name, _)| name)
}

/// Compute the Levenshtein edit distance between `a` and `b`.
///
/// Iterative two-row dynamic programming: O(len(a) × len(b)) time and
/// O(len(b)) space, counting characters (not bytes) so multi-byte UTF-8 is
/// handled correctly. No recursion.
#[must_use]
fn levenshtein(a: &str, b: &str) -> usize {
    let b_chars: Vec<char> = b.chars().collect();
    if b_chars.is_empty() {
        return a.chars().count();
    }
    let mut prev: Vec<usize> = (0..=b_chars.len()).collect();
    let mut curr: Vec<usize> = vec![0; b_chars.len() + 1];
    for (i, a_char) in a.chars().enumerate() {
        curr[0] = i + 1;
        for (j, &b_char) in b_chars.iter().enumerate() {
            let substitution_cost = usize::from(a_char != b_char);
            curr[j + 1] = (prev[j + 1] + 1)
                .min(curr[j] + 1)
                .min(prev[j] + substitution_cost);
        }
        std::mem::swap(&mut prev, &mut curr);
    }
    prev[b_chars.len()]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn levenshtein_basic_distances() {
        assert_eq!(levenshtein("", ""), 0);
        assert_eq!(levenshtein("abc", "abc"), 0);
        assert_eq!(levenshtein("abc", ""), 3);
        assert_eq!(levenshtein("", "abc"), 3);
        assert_eq!(levenshtein("kitten", "sitting"), 3);
        assert_eq!(levenshtein("mp-buton", "mp-button"), 1);
    }

    #[test]
    fn levenshtein_counts_chars_not_bytes() {
        // Multi-byte characters count as one edit each.
        assert_eq!(levenshtein("café", "cafe"), 1);
        assert_eq!(levenshtein("naïve", "naive"), 1);
    }

    #[test]
    fn closest_match_finds_near_typo() {
        let names = ["mp-button", "mp-card", "mp-list"];
        assert_eq!(
            closest_match("mp-buton", names.into_iter()),
            Some("mp-button")
        );
    }

    #[test]
    fn closest_match_rejects_distant_names() {
        let names = ["mp-button", "mp-card"];
        assert_eq!(closest_match("sidebar-nav", names.into_iter()), None);
    }

    #[test]
    fn closest_match_picks_nearest_on_multiple_candidates() {
        let names = ["mp-button", "mp-buttonx", "mp-bton"];
        // Exact-ish "mp-button" (distance 1) beats the others.
        assert_eq!(
            closest_match("mp-buttn", names.into_iter()),
            Some("mp-button")
        );
    }

    #[test]
    fn closest_match_empty_candidates_is_none() {
        assert_eq!(closest_match("anything", std::iter::empty()), None);
    }

    #[test]
    fn closest_match_tolerates_short_word_transposition() {
        // `eahc` is a transposition of `each` (Levenshtein distance 2); the
        // minimum tolerance of 2 lets short directive attributes still match.
        let attrs = ["eahc", "template"];
        assert_eq!(closest_match("each", attrs.into_iter()), Some("eahc"));
    }
}
