//! Turns a stream of repeated whole-window hypotheses into stable, growing
//! captions (YouTube style): words that stay the same across two consecutive
//! passes are *committed* and never change again; only the last few words (the
//! tentative tail) may be revised. This is the "local agreement" strategy from
//! streaming Whisper implementations.
//!
//! Pure logic, no Whisper dependency, so it is unit-testable.

/// One recognised word with its time span inside the current audio window.
#[derive(Debug, Clone, PartialEq)]
pub struct Word {
    pub text: String,
    /// Seconds from the start of the current audio window.
    pub t0: f32,
    pub t1: f32,
}

impl Word {
    pub fn new(text: &str, t0: f32, t1: f32) -> Self {
        Self {
            text: text.to_string(),
            t0,
            t1,
        }
    }
}

/// Most committed words kept for display (older ones scroll off anyway).
const MAX_COMMITTED_WORDS: usize = 80;
/// If the window grows past this without any agreement, commit most of it.
const FORCE_COMMIT_SECS: f32 = 12.0;
/// Words left tentative when force-committing.
const FORCE_KEEP_WORDS: usize = 3;
/// If no silent gap to cut at ever turns up (continuous speech), accept a hard
/// cut at the word boundary once the window is this long.
const HARD_CUT_SECS: f32 = 16.0;

/// CJK scripts have no spaces between words.
pub fn is_cjk(c: char) -> bool {
    matches!(c as u32,
        0x2E80..=0x2FDF | 0x3000..=0x303F | 0x3040..=0x30FF | 0x3100..=0x312F
        | 0x31F0..=0x31FF | 0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xAC00..=0xD7AF
        | 0xF900..=0xFAFF | 0xFF00..=0xFFEF | 0x20000..=0x2FA1F)
}

/// Comparison key: letters/digits only, lower-cased, so "Hello," == "hello".
fn norm(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(|c| c.to_lowercase())
        .collect()
}

fn common_prefix_len(a: &[Word], b: &[Word]) -> usize {
    a.iter()
        .zip(b)
        .take_while(|(x, y)| {
            let (nx, ny) = (norm(&x.text), norm(&y.text));
            !nx.is_empty() && nx == ny
        })
        .count()
}

/// Whisper's token timestamps are heuristic and can be missing (all zero) or
/// non-monotonic. If they look unusable, spread the words over the window in
/// proportion to their length instead.
pub fn sanitize_times(words: &mut [Word], audio_secs: f32) {
    let valid = !words.is_empty()
        && words.iter().all(|w| {
            w.t0.is_finite()
                && w.t1.is_finite()
                && w.t0 >= 0.0
                && w.t1 >= w.t0
                && w.t1 <= audio_secs + 0.75
        })
        && words.windows(2).all(|p| p[1].t0 + 0.05 >= p[0].t0)
        && words.last().is_some_and(|w| w.t1 > 0.0);
    if valid {
        return;
    }
    let total: f32 = words
        .iter()
        .map(|w| w.text.chars().count().max(1) as f32)
        .sum();
    if total <= 0.0 {
        return;
    }
    let mut at = 0.0f32;
    for w in words.iter_mut() {
        let span = audio_secs * (w.text.chars().count().max(1) as f32 / total);
        w.t0 = at;
        at += span;
        w.t1 = at;
    }
}

/// Result of feeding one hypothesis.
#[derive(Debug, Clone, PartialEq)]
pub struct Update {
    /// Everything to show: committed words followed by the tentative tail.
    pub display: String,
    /// Drop this many seconds from the start of the audio window (it has been
    /// committed and will not be transcribed again).
    pub cut_secs: Option<f32>,
}

#[derive(Default)]
pub struct StreamState {
    committed: Vec<String>,
    /// Previous hypothesis, re-based to the current window.
    prev: Vec<Word>,
    spaceless: bool,
    /// The last committed word did not end a sentence, so the next window's
    /// first word must not keep the capital letter Whisper gives every window start.
    lower_first: bool,
}

/// A word starting within this many seconds of the window start can be the
/// remainder of the word that was just committed.
const REMNANT_WINDOW_SECS: f32 = 0.3;

fn ends_sentence(word: &str) -> bool {
    word.trim_end().chars().next_back().is_some_and(|c| {
        matches!(
            c,
            '.' | '?' | '!' | '\u{2026}' | '\u{3002}' | '\u{FF1F}' | '\u{FF01}'
        )
    })
}

/// A word that really ends a sentence. Short tokens such as "p.", "1." or
/// "Dr." (abbreviations, list numbers) do not count.
fn is_sentence_break(word: &str) -> bool {
    if !ends_sentence(word) {
        return false;
    }
    let letters = word.chars().filter(|c| c.is_alphabetic()).count();
    // CJK sentences are short in characters; one is enough there.
    let min = if word.chars().any(is_cjk) { 1 } else { 3 };
    letters >= min
}

fn lowercase_first(word: &str) -> String {
    let mut chars = word.chars();
    match chars.next() {
        // Keep ALL-CAPS words (acronyms) and "I"-like single capitals untouched.
        Some(c) if c.is_uppercase() && chars.clone().any(|x| x.is_lowercase()) => {
            c.to_lowercase().chain(chars).collect()
        }
        _ => word.to_string(),
    }
}

impl StreamState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Forget everything (a new utterance starts).
    pub fn reset(&mut self) {
        self.committed.clear();
        self.prev.clear();
        self.spaceless = false;
        self.lower_first = false;
    }

    pub fn is_empty(&self) -> bool {
        self.committed.is_empty() && self.prev.is_empty()
    }

    /// Joins words into caption text. A finished sentence is followed by a line
    /// break, so the next sentence starts on a new line (live-caption style).
    fn join(&self, words: impl Iterator<Item = String>) -> String {
        let mut out = String::new();
        let mut prev: Option<String> = None;
        for word in words {
            if let Some(p) = &prev {
                if is_sentence_break(p) {
                    out.push('\n');
                } else if !self.spaceless {
                    out.push(' ');
                }
            }
            out.push_str(&word);
            prev = Some(word);
        }
        out
    }

    /// Feed the latest hypothesis for the current audio window (`audio_secs` long).
    ///
    /// Committing drops the committed audio, and Whisper's word timestamps are
    /// only approximate, so a cut must not land inside a word. `snap(lo, hi)`
    /// must return a time within `lo..=hi` that lies in a silent gap of the real
    /// audio, or `None` if there is none; with `None` the words simply stay
    /// tentative for now.
    pub fn update(
        &mut self,
        mut hyp: Vec<Word>,
        audio_secs: f32,
        snap: &dyn Fn(f32, f32) -> Option<f32>,
    ) -> Update {
        hyp.retain(|w| !w.text.trim().is_empty());
        self.spaceless = hyp.iter().any(|w| w.text.chars().any(is_cjk));
        sanitize_times(&mut hyp, audio_secs);

        // The audio left after a cut can still contain the tail of the word that
        // was just committed; drop it instead of showing it twice.
        if let (Some(last), Some(first)) = (self.committed.last(), hyp.first()) {
            if first.t0 < REMNANT_WINDOW_SECS
                && !norm(&first.text).is_empty()
                && norm(&first.text) == norm(last)
            {
                hyp.remove(0);
            }
        }
        if self.lower_first && !self.spaceless {
            if let Some(first) = hyp.first_mut() {
                first.text = lowercase_first(&first.text);
            }
        }

        let mut agreed = common_prefix_len(&self.prev, &hyp);
        if agreed == 0 && audio_secs > FORCE_COMMIT_SECS && hyp.len() > FORCE_KEEP_WORDS {
            agreed = hyp.len() - FORCE_KEEP_WORDS;
        }

        let mut cut_secs = None;
        let mut committed_now = false;
        if agreed > 0 {
            let end = hyp[agreed - 1].t1;
            // The cut belongs in the gap between the last agreed word and the next one.
            let (lo, hi) = match hyp.get(agreed) {
                Some(next) => (end - 0.1, (next.t0 + 0.25).max(end)),
                None => (end - 0.1, end + 0.6),
            };
            let (lo, hi) = (lo.max(0.0), hi.min((audio_secs - 0.2).max(0.0)));
            let mut cut = if hi >= lo { snap(lo, hi) } else { None };
            if cut.is_none() && audio_secs > HARD_CUT_SECS {
                cut = Some(end.clamp(0.0, (audio_secs - 0.2).max(0.0)));
            }
            if let Some(cut) = cut {
                self.committed
                    .extend(hyp[..agreed].iter().map(|w| w.text.clone()));
                self.lower_first = self.committed.last().is_some_and(|w| !ends_sentence(w));
                let excess = self.committed.len().saturating_sub(MAX_COMMITTED_WORDS);
                self.committed.drain(..excess);
                if cut > 0.0 {
                    cut_secs = Some(cut);
                }
                self.prev = hyp[agreed..]
                    .iter()
                    .map(|w| Word {
                        text: w.text.clone(),
                        t0: (w.t0 - cut).max(0.0),
                        t1: (w.t1 - cut).max(0.0),
                    })
                    .collect();
                committed_now = true;
            }
        }
        if !committed_now {
            self.prev = hyp;
        }

        let display = self.join(
            self.committed
                .iter()
                .cloned()
                .chain(self.prev.iter().map(|w| w.text.clone())),
        );
        Update { display, cut_secs }
    }

    /// The last `n` words of `text`, for feeding the translator a short snippet.
    pub fn tail_words(text: &str, n: usize) -> String {
        if text.chars().any(is_cjk) {
            let chars: Vec<char> = text.chars().collect();
            return chars[chars.len().saturating_sub(n * 3)..].iter().collect();
        }
        let words: Vec<&str> = text.split_whitespace().collect();
        words[words.len().saturating_sub(n)..].join(" ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Snap that always finds a gap in the middle of the allowed range.
    fn mid(lo: f32, hi: f32) -> Option<f32> {
        Some((lo + hi) / 2.0)
    }

    fn words(text: &str) -> Vec<Word> {
        text.split_whitespace()
            .enumerate()
            .map(|(i, w)| Word::new(w, i as f32 * 0.4, i as f32 * 0.4 + 0.35))
            .collect()
    }

    #[test]
    fn first_hypothesis_is_shown_but_nothing_is_committed_yet() {
        let mut s = StreamState::new();
        let u = s.update(words("hello this is"), 1.4, &mid);
        assert_eq!(u.display, "hello this is");
        assert_eq!(u.cut_secs, None);
        assert!(s.committed.is_empty());
    }

    #[test]
    fn agreeing_prefix_is_committed_and_audio_is_cut() {
        let mut s = StreamState::new();
        s.update(words("hello this is"), 1.4, &mid);
        let u = s.update(words("hello this is a test"), 2.0, &mid);
        assert_eq!(u.display, "hello this is a test");
        // "hello this is" agreed: cut between "is" and "a"
        let cut = u.cut_secs.expect("cut");
        assert!((1.05..=1.45).contains(&cut), "cut = {cut}");
        assert_eq!(s.committed, ["hello", "this", "is"]);
        // remaining tentative words were re-based to the cut point
        assert!(s.prev[0].t0 < 0.2, "{:?}", s.prev);
    }

    #[test]
    fn committed_words_never_change_even_if_later_passes_disagree() {
        let mut s = StreamState::new();
        s.update(words("hello this is"), 1.4, &mid);
        s.update(words("hello this is a test"), 2.0, &mid);
        // After the cut the window starts at "a": a wild later guess changes only the tail.
        let u = s.update(words("a text of live"), 1.5, &mid);
        assert!(u.display.starts_with("hello this is a"), "{}", u.display);
        let u2 = s.update(words("a test of live"), 1.9, &mid);
        assert!(u2.display.starts_with("hello this is a"), "{}", u2.display);
    }

    #[test]
    fn display_only_ever_appends_in_the_committed_part() {
        // Simulate a flickery recogniser: the tail wobbles, the head must be stable.
        let hyps = [
            "to jest",
            "to jest test",
            "to jest tekst napisów",
            "to jest test napisów na",
            "to jest test napisów na żywo",
            "to jest test napisów na żywo dzisiaj",
        ];
        let mut s = StreamState::new();
        let mut shown: Vec<String> = vec![];
        let mut window_words: Vec<String> = vec![];
        for (i, h) in hyps.iter().enumerate() {
            let mut hyp = words(h);
            // emulate the caller cutting committed audio: later hypotheses only contain new words
            if !window_words.is_empty() {
                hyp = words(
                    &h.split_whitespace()
                        .skip(window_words.len())
                        .collect::<Vec<_>>()
                        .join(" "),
                );
            }
            let u = s.update(hyp, 0.4 * (i as f32 + 2.0), &mid);
            if u.cut_secs.is_some() {
                window_words = s.committed.clone();
            }
            shown.push(u.display);
        }
        let last = shown.last().unwrap();
        assert!(last.starts_with("to jest"), "{last}");
        // once "to jest" was committed it is a prefix of everything after
        let first_committed = shown
            .iter()
            .position(|d| s_committed_prefix(d, "to jest"))
            .unwrap();
        assert!(shown[first_committed..]
            .iter()
            .all(|d| d.starts_with("to jest")));
    }

    fn s_committed_prefix(d: &str, p: &str) -> bool {
        d.starts_with(p)
    }

    #[test]
    fn punctuation_and_case_do_not_break_agreement() {
        let mut s = StreamState::new();
        s.update(
            vec![Word::new("Hello,", 0.0, 0.4), Word::new("world", 0.5, 0.9)],
            1.0,
            &mid,
        );
        let u = s.update(
            vec![
                Word::new("hello", 0.0, 0.4),
                Word::new("world.", 0.5, 0.9),
                Word::new("again", 1.0, 1.4),
            ],
            1.5,
            &mid,
        );
        assert!(u.cut_secs.is_some());
        assert_eq!(s.committed.len(), 2);
    }

    #[test]
    fn forces_a_commit_when_the_window_gets_too_long() {
        let mut s = StreamState::new();
        let long = words("one two three four five six seven eight");
        s.update(words("zzz"), 1.0, &mid); // prev that never agrees
        let u = s.update(long, 13.0, &mid);
        assert!(u.cut_secs.is_some());
        assert_eq!(s.committed.len(), 5); // all but the last 3
        assert_eq!(u.display, "one two three four five six seven eight");
    }

    #[test]
    fn without_a_silent_gap_nothing_is_committed_or_cut() {
        let mut s = StreamState::new();
        let no_gap = |_: f32, _: f32| None;
        s.update(words("hello this is"), 1.4, &no_gap);
        let u = s.update(words("hello this is a test"), 2.0, &no_gap);
        assert_eq!(u.display, "hello this is a test");
        assert_eq!(u.cut_secs, None, "must not cut inside speech");
        assert!(s.committed.is_empty());
        // ...and everything is still shown, so the display keeps growing
        let u = s.update(words("hello this is a test of it"), 2.8, &no_gap);
        assert_eq!(u.display, "hello this is a test of it");
    }

    #[test]
    fn the_cut_is_requested_between_the_agreed_word_and_the_next() {
        let mut s = StreamState::new();
        let asked = std::cell::Cell::new((0.0f32, 0.0f32));
        let probe = |lo: f32, hi: f32| {
            asked.set((lo, hi));
            Some((lo + hi) / 2.0)
        };
        s.update(words("hello this is"), 1.4, &probe);
        s.update(words("hello this is a test"), 2.0, &probe);
        let (lo, hi) = asked.get();
        // "is" ends at 1.15, "a" starts at 1.2
        assert!(lo < 1.15 && lo > 1.0, "lo = {lo}");
        assert!((1.2..1.6).contains(&hi), "hi = {hi}");
    }

    #[test]
    fn a_hard_cut_is_taken_only_for_very_long_windows() {
        let mut s = StreamState::new();
        let no_gap = |_: f32, _: f32| None;
        s.update(words("a b c d"), 10.0, &no_gap);
        assert_eq!(s.update(words("a b c d e"), 10.5, &no_gap).cut_secs, None);
        let mut s = StreamState::new();
        s.update(words("a b c d"), 16.5, &no_gap);
        assert!(s
            .update(words("a b c d e"), 17.0, &no_gap)
            .cut_secs
            .is_some());
    }

    #[test]
    fn a_remnant_of_the_committed_word_is_not_shown_twice() {
        let mut s = StreamState::new();
        s.update(words("mówię powoli żeby"), 1.6, &mid);
        let u = s.update(words("mówię powoli żeby sprawdzić"), 2.0, &mid);
        assert!(s.committed.len() >= 3, "{:?}", s.committed);
        // next window starts with a sliver of the last committed word again
        let again = vec![
            Word::new("żeby", 0.0, 0.1),
            Word::new("sprawdzić", 0.25, 0.7),
        ];
        let u2 = s.update(again, 0.9, &mid);
        assert_eq!(u2.display.matches("żeby").count(), 1, "{}", u2.display);
        assert!(u2.display.contains("sprawdzić"));
        let _ = u;
    }

    #[test]
    fn a_genuinely_repeated_word_later_in_the_window_is_kept() {
        let mut s = StreamState::new();
        s.update(words("no no"), 1.0, &mid);
        s.update(words("no no way"), 1.5, &mid);
        let u = s.update(
            vec![Word::new("no", 1.0, 1.3), Word::new("way", 1.4, 1.8)],
            2.0,
            &mid,
        );
        assert_eq!(u.display.matches("no").count(), 3, "{}", u.display);
    }

    #[test]
    fn mid_sentence_window_starts_are_not_capitalised() {
        let mut s = StreamState::new();
        s.update(words("pogoda i świeci"), 1.5, &mid);
        s.update(words("pogoda i świeci słońce"), 2.0, &mid);
        let u = s.update(
            vec![Word::new("Słońce.", 0.1, 0.6), Word::new("Mówię", 0.8, 1.2)],
            1.3,
            &mid,
        );
        assert!(u.display.contains("słońce."), "{}", u.display);
    }

    #[test]
    fn sentence_starts_keep_their_capital() {
        let mut s = StreamState::new();
        s.update(words("to jest test."), 1.2, &mid);
        s.update(words("to jest test. Dzisiaj"), 1.6, &mid);
        let u = s.update(
            vec![Word::new("Dzisiaj", 0.1, 0.5), Word::new("jest", 0.6, 0.9)],
            1.0,
            &mid,
        );
        assert!(u.display.contains("Dzisiaj"), "{}", u.display);
    }

    #[test]
    fn acronyms_and_names_helpers() {
        assert_eq!(lowercase_first("Słońce."), "słońce.");
        assert_eq!(lowercase_first("NASA"), "NASA");
        assert_eq!(lowercase_first("I"), "I");
        assert!(ends_sentence("test."));
        assert!(ends_sentence("co?"));
        assert!(!ends_sentence("test,"));
        assert!(!ends_sentence("test"));
    }

    #[test]
    fn a_finished_sentence_starts_the_next_line() {
        let mut s = StreamState::new();
        let u = s.update(words("to jest test. Dzisiaj jest ładnie"), 3.0, &mid);
        assert_eq!(u.display, "to jest test.\nDzisiaj jest ładnie");
    }

    #[test]
    fn abbreviations_and_numbers_do_not_break_the_line() {
        let mut s = StreamState::new();
        let u = s.update(words("pan dr. Kowalski kupił 3. książkę"), 3.0, &mid);
        assert_eq!(u.display, "pan dr. Kowalski kupił 3. książkę");
    }

    #[test]
    fn questions_and_exclamations_also_end_a_line() {
        let mut s = StreamState::new();
        let u = s.update(words("co robisz? nic! dobrze"), 2.0, &mid);
        assert_eq!(u.display, "co robisz?\nnic!\ndobrze");
    }

    #[test]
    fn line_breaks_survive_commits_and_never_double_up() {
        let mut s = StreamState::new();
        s.update(words("to jest test. Dzisiaj"), 2.0, &mid);
        // agreement commits "to jest test. Dzisiaj" and cuts the audio
        s.update(words("to jest test. Dzisiaj jest"), 2.4, &mid);
        assert_eq!(s.committed.len(), 4);
        // ...so the next window only contains the words after the cut
        let u = s.update(words("jest ładnie"), 1.0, &mid);
        assert_eq!(u.display, "to jest test.\nDzisiaj jest ładnie");
        assert_eq!(u.display.matches('\n').count(), 1);
    }

    #[test]
    fn cjk_sentences_break_without_spaces() {
        let mut s = StreamState::new();
        let u = s.update(
            vec![Word::new("字幕。", 0.0, 0.5), Word::new("テスト", 0.5, 1.0)],
            1.0,
            &mid,
        );
        assert_eq!(u.display, "字幕。\nテスト");
    }

    #[test]
    fn unusable_timestamps_fall_back_to_proportional_spacing() {
        let mut w = vec![
            Word::new("aa", 0.0, 0.0),
            Word::new("bbbb", 0.0, 0.0),
            Word::new("cc", 0.0, 0.0),
        ];
        sanitize_times(&mut w, 4.0);
        assert!((w[0].t1 - 1.0).abs() < 1e-4);
        assert!((w[1].t0 - 1.0).abs() < 1e-4 && (w[1].t1 - 3.0).abs() < 1e-4);
        assert!((w[2].t1 - 4.0).abs() < 1e-4);
        // good timestamps are left alone
        let mut good = words("a b c");
        let before = good.clone();
        sanitize_times(&mut good, 2.0);
        assert_eq!(good, before);
    }

    #[test]
    fn spaceless_scripts_join_without_spaces() {
        let mut s = StreamState::new();
        let u = s.update(
            vec![Word::new("字幕", 0.0, 0.5), Word::new("テスト", 0.5, 1.0)],
            1.0,
            &mid,
        );
        assert_eq!(u.display, "字幕テスト");
    }

    #[test]
    fn reset_starts_a_new_utterance() {
        let mut s = StreamState::new();
        s.update(words("hello there"), 1.0, &mid);
        s.update(words("hello there friend"), 1.5, &mid);
        assert!(!s.is_empty());
        s.reset();
        assert!(s.is_empty());
        assert_eq!(s.update(words("new start"), 1.0, &mid).display, "new start");
    }

    #[test]
    fn empty_and_blank_hypotheses_are_harmless() {
        let mut s = StreamState::new();
        assert_eq!(s.update(vec![], 1.0, &mid).display, "");
        assert_eq!(
            s.update(vec![Word::new("  ", 0.0, 0.1)], 1.0, &mid).display,
            ""
        );
    }

    #[test]
    fn tail_words_limits_the_translation_snippet() {
        assert_eq!(StreamState::tail_words("a b c d e f", 3), "d e f");
        assert_eq!(StreamState::tail_words("short one", 10), "short one");
        assert_eq!(StreamState::tail_words("", 3), "");
    }
}
