//! Turns a stream of repeated whole-window hypotheses into stable, growing
//! captions (YouTube style).
//!
//! * Words become **frozen** once they are stable: either two consecutive passes
//!   agree on them and they are at least [`AGREE_MIN_AGE`] old, or they are older
//!   than [`FREEZE_AGE`] (Whisper practically never revises text that far back).
//!   Frozen words never change again; only the short tail after them is tentative.
//! * Freezing is independent of trimming the audio. Audio is cut (so it is not
//!   transcribed again) only during a real pause at the *end* of the buffer, when
//!   every word is complete. Whisper's word timestamps are too inaccurate to tell
//!   which words lie before a cut in the middle of speech, and a word whose audio
//!   is cut before it was frozen is simply lost.
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

/// Most frozen words kept for display (older ones scroll off anyway).
const MAX_COMMITTED_WORDS: usize = 80;
/// A word two passes agree on must also be this old (seconds before the end of
/// the audio) before it freezes: a wrong guess can repeat for a pass or two.
const AGREE_MIN_AGE: f32 = 1.0;
/// Words older than this freeze regardless of agreement.
const FREEZE_AGE: f32 = 2.5;
/// A pause at the end of the audio must last this long before everything is
/// frozen and the audio is cut.
pub const PAUSE_SECS: f32 = 0.6;
/// If speech never pauses, accept a hard cut at the end of the frozen words once
/// the window is this long.
const HARD_CUT_SECS: f32 = 12.0;
/// A hard cut is taken this much *before* the end of the last frozen word: a
/// leftover sliver of it is dropped by the repeat check, whereas cutting too late
/// would clip the next word.
const HARD_CUT_BACKOFF: f32 = 0.2;

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
    /// Everything to show: frozen words followed by the tentative tail.
    pub display: String,
    /// Drop this many seconds from the start of the audio window (it holds only
    /// frozen words and will not be transcribed again).
    pub cut_secs: Option<f32>,
}

#[derive(Default)]
pub struct StreamState {
    committed: Vec<String>,
    /// End (window-relative seconds) of the last frozen word; `0` if none in this window.
    frozen_end: f32,
    /// Previous tentative hypothesis, in current-window coordinates.
    prev: Vec<Word>,
    spaceless: bool,
    /// The last frozen word did not end a sentence, so the next window's first
    /// word must not keep the capital letter Whisper gives every window start.
    lower_first: bool,
}

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
        self.frozen_end = 0.0;
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

    /// Words at the start of `hyp` that repeat the last frozen words (the audio
    /// kept after a cut can still hold a sliver of them, and timestamps differ a
    /// little between passes).
    fn repeated_prefix(&self, hyp: &[Word]) -> usize {
        for k in (1..=3).rev() {
            if hyp.len() >= k
                && self.committed.len() >= k
                && hyp[..k]
                    .iter()
                    .zip(&self.committed[self.committed.len() - k..])
                    .all(|(h, c)| {
                        let n = norm(&h.text);
                        !n.is_empty() && n == norm(c)
                    })
            {
                return k;
            }
        }
        0
    }

    /// Feed the latest hypothesis for the current audio window (`audio_secs` long).
    ///
    /// `trailing_silence` is where (window-relative seconds) the silence at the
    /// very end of the audio began, if there is one of at least [`PAUSE_SECS`].
    /// The speaker has stopped, so every word is complete: they are all frozen and
    /// the audio is cut inside the silence. Otherwise the audio is kept (frozen
    /// words stay frozen either way).
    pub fn update(
        &mut self,
        mut hyp: Vec<Word>,
        audio_secs: f32,
        trailing_silence: Option<f32>,
    ) -> Update {
        hyp.retain(|w| !w.text.trim().is_empty());
        self.spaceless = hyp.iter().any(|w| w.text.chars().any(is_cjk));
        sanitize_times(&mut hyp, audio_secs);

        // Words that are already frozen are not shown again.
        if self.frozen_end > 0.0 {
            let end = self.frozen_end;
            // A word that ends where the frozen text ends (or earlier) is a repeat.
            hyp.retain(|w| (w.t0 + w.t1) / 2.0 > end - 0.1 && w.t1 > end + 0.02);
        }
        let repeated = self.repeated_prefix(&hyp);
        hyp.drain(..repeated);

        if self.lower_first && !self.spaceless {
            if let Some(first) = hyp.first_mut() {
                first.text = lowercase_first(&first.text);
            }
        }

        // Freeze the stable prefix.
        let agreed = common_prefix_len(&self.prev, &hyp);
        let mut freeze = 0;
        for (i, w) in hyp.iter().enumerate() {
            let age = audio_secs - w.t1;
            let stable = (i < agreed && age >= AGREE_MIN_AGE) || age >= FREEZE_AGE;
            if !stable {
                break;
            }
            freeze = i + 1;
        }
        if freeze > 0 {
            self.committed
                .extend(hyp[..freeze].iter().map(|w| w.text.clone()));
            let excess = self.committed.len().saturating_sub(MAX_COMMITTED_WORDS);
            self.committed.drain(..excess);
            self.lower_first = self.committed.last().is_some_and(|w| !ends_sentence(w));
            self.frozen_end = hyp[freeze - 1].t1;
            hyp.drain(..freeze);
        }

        // A pause at the end: the speaker stopped, so nothing in `hyp` is still
        // being said. Freeze it all and cut inside the silence.
        let mut cut_secs = None;
        let limit = (audio_secs - 0.2).max(0.0);
        if let Some(start) = trailing_silence.filter(|s| audio_secs - s >= PAUSE_SECS) {
            if !hyp.is_empty() {
                self.committed.extend(hyp.iter().map(|w| w.text.clone()));
                let excess = self.committed.len().saturating_sub(MAX_COMMITTED_WORDS);
                self.committed.drain(..excess);
                self.lower_first = self.committed.last().is_some_and(|w| !ends_sentence(w));
                hyp.clear();
            }
            let cut = (start + 0.3).min(limit);
            if cut > 0.0 {
                cut_secs = Some(cut);
                self.frozen_end = 0.0;
            }
        } else if audio_secs > HARD_CUT_SECS && self.frozen_end > 0.0 {
            // Continuous speech for a long time: cut at the end of the frozen words.
            let cut = (self.frozen_end - HARD_CUT_BACKOFF).clamp(0.0, limit);
            if cut > 0.0 {
                cut_secs = Some(cut);
                self.frozen_end = (self.frozen_end - cut).max(0.0);
                for w in &mut hyp {
                    w.t0 = (w.t0 - cut).max(0.0);
                    w.t1 = (w.t1 - cut).max(0.0);
                }
            }
        }

        self.prev = hyp;
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

    /// No pause at the end of the audio (the speaker is still talking).
    const TALKING: Option<f32> = None;

    /// Words spaced 0.4 s apart starting at `start`.
    fn words_at(text: &str, start: f32) -> Vec<Word> {
        text.split_whitespace()
            .enumerate()
            .map(|(i, w)| {
                let t = start + i as f32 * 0.4;
                Word::new(w, t, t + 0.35)
            })
            .collect()
    }

    fn words(text: &str) -> Vec<Word> {
        words_at(text, 0.0)
    }

    #[test]
    fn young_words_are_shown_but_not_frozen() {
        let mut s = StreamState::new();
        let u = s.update(words("hello this is"), 1.5, TALKING);
        assert_eq!(u.display, "hello this is");
        assert_eq!(u.cut_secs, None);
        assert!(s.committed.is_empty());
        // agreement alone is not enough while the words are young
        let u = s.update(words("hello this is a"), 1.25, TALKING);
        assert_eq!(u.display, "hello this is a");
        assert!(s.committed.is_empty(), "{:?}", s.committed);
    }

    #[test]
    fn agreeing_old_enough_words_freeze_but_the_audio_is_not_cut_mid_speech() {
        let mut s = StreamState::new();
        s.update(words("hello this is a test"), 2.2, TALKING);
        let u = s.update(words("hello this is a test of it"), 2.8, TALKING);
        // words that ended more than 1 s ago and were agreed on are frozen...
        assert!(s.committed.len() >= 2, "{:?}", s.committed);
        assert_eq!(u.display, "hello this is a test of it");
        // ...but cutting mid-speech could lose a word, so the audio is kept
        assert_eq!(u.cut_secs, None);
    }

    #[test]
    fn frozen_words_never_change_even_if_later_passes_disagree() {
        let mut s = StreamState::new();
        s.update(words("hello this is a test"), 2.2, TALKING);
        s.update(words("hello this is a test of it"), 2.8, TALKING);
        let frozen = s.committed.clone();
        assert!(!frozen.is_empty());
        // a later pass rewrites the early words completely: the display must not follow
        let u = s.update(words("jello miss was the best of it again"), 3.4, TALKING);
        assert!(
            u.display.starts_with(&frozen.join(" ")),
            "frozen head changed: {:?} vs {frozen:?}",
            u.display
        );
    }

    #[test]
    fn words_older_than_the_freeze_age_freeze_without_agreement() {
        let mut s = StreamState::new();
        // first ever hypothesis: nothing to agree with, but the early words are old
        let u = s.update(
            words("one two three four five six seven eight"),
            4.0,
            TALKING,
        );
        assert!(!s.committed.is_empty(), "old words must freeze");
        assert!(u.display.starts_with("one two"));
    }

    #[test]
    fn freezing_works_without_any_silent_gap_to_cut_at() {
        // continuous speech: the audio is never cut, yet the head stays put
        let mut s = StreamState::new();
        let mut shown: Vec<String> = Vec::new();
        for step in 0..12 {
            let secs = 2.0 + step as f32 * 0.5;
            let n = ((secs / 0.4) as usize).min(20);
            let text = (1..=n)
                .map(|i| format!("w{i}"))
                .collect::<Vec<_>>()
                .join(" ");
            let u = s.update(words(&text), secs, TALKING);
            assert_eq!(u.cut_secs, None);
            shown.push(u.display);
        }
        for pair in shown.windows(2) {
            let head = pair[0]
                .split_whitespace()
                .take(5)
                .collect::<Vec<_>>()
                .join(" ");
            assert!(
                pair[1].starts_with(&head),
                "head moved: {:?} -> {:?}",
                pair[0],
                pair[1]
            );
        }
    }

    #[test]
    fn a_pause_freezes_every_word_and_cuts_inside_the_silence() {
        let mut s = StreamState::new();
        // speech until ~2.0 s, then the speaker stops; audio is now 3.2 s long
        s.update(words("alpha beta gamma delta"), 1.9, TALKING);
        let u = s.update(words("alpha beta gamma delta"), 3.2, Some(2.0));
        assert_eq!(u.display, "alpha beta gamma delta");
        assert_eq!(s.committed.len(), 4, "everything is final after a pause");
        let cut = u.cut_secs.expect("cut during the pause");
        assert!(
            cut > 2.0 && cut < 3.1,
            "cut must lie inside the silence: {cut}"
        );
        // the next utterance starts from a clean window and appends
        let u = s.update(words("epsilon zeta"), 1.0, TALKING);
        assert_eq!(u.display, "alpha beta gamma delta epsilon zeta");
    }

    #[test]
    fn a_pause_that_is_too_short_changes_nothing() {
        let mut s = StreamState::new();
        let u = s.update(words("alpha beta"), 1.5, Some(1.2)); // only 0.3 s of silence
        assert_eq!(u.cut_secs, None);
        assert!(s.committed.is_empty());
    }

    #[test]
    fn a_pause_with_nothing_new_still_cuts_the_leftover_audio() {
        let mut s = StreamState::new();
        s.update(words("alpha beta gamma delta"), 1.9, TALKING);
        s.update(words("alpha beta gamma delta"), 3.2, Some(2.0));
        // the window was cut but silence keeps arriving: an empty hypothesis cuts again
        let u = s.update(vec![], 1.5, Some(0.0));
        assert_eq!(u.display, "alpha beta gamma delta");
    }

    #[test]
    fn continuous_speech_is_cut_only_when_the_window_gets_very_long() {
        let mut s = StreamState::new();
        let text = (1..=12)
            .map(|i| format!("w{i}"))
            .collect::<Vec<_>>()
            .join(" ");
        assert_eq!(s.update(words(&text), 5.0, TALKING).cut_secs, None);
        let mut s = StreamState::new();
        let long = (1..=40)
            .map(|i| format!("w{i}"))
            .collect::<Vec<_>>()
            .join(" ");
        let u = s.update(words(&long), 16.5, TALKING);
        let cut = u.cut_secs.expect("a very long window is cut");
        // backed off from the end of the last frozen word, not past it
        assert!(cut > 0.0 && cut < 16.5 - 2.0, "cut = {cut}");
    }

    #[test]
    fn a_repeat_of_the_last_frozen_word_after_a_cut_is_dropped() {
        let mut s = StreamState::new();
        s.update(words("mówię powoli żeby sprawdzić czy"), 2.0, TALKING);
        // the speaker pauses: everything is frozen and the audio is cut
        let u = s.update(words("mówię powoli żeby sprawdzić czy"), 3.4, Some(2.1));
        assert!(u.cut_secs.is_some());
        let last = s.committed.last().cloned().unwrap();
        // the next window starts with a sliver of the last frozen word again
        let again = vec![
            Word::new(&last, 0.0, 0.1),
            Word::new("podpisy", 0.3, 0.8),
            Word::new("są", 0.9, 1.2),
        ];
        let u = s.update(again, 1.4, TALKING);
        assert_eq!(u.display.matches(last.as_str()).count(), 1, "{}", u.display);
        assert!(u.display.contains("są"), "{}", u.display);
    }

    #[test]
    fn a_genuinely_repeated_word_in_the_first_window_is_kept() {
        let mut s = StreamState::new();
        let u = s.update(words("no no way no"), 1.6, TALKING);
        assert_eq!(u.display, "no no way no");
    }

    #[test]
    fn punctuation_and_case_do_not_break_agreement() {
        let mut s = StreamState::new();
        s.update(
            vec![Word::new("Hello,", 0.0, 0.4), Word::new("world", 0.5, 0.9)],
            2.5,
            TALKING,
        );
        s.update(
            vec![
                Word::new("hello", 0.0, 0.4),
                Word::new("world.", 0.5, 0.9),
                Word::new("again", 1.0, 1.4),
            ],
            2.6,
            TALKING,
        );
        assert_eq!(s.committed.len(), 2, "{:?}", s.committed);
    }

    #[test]
    fn mid_sentence_window_starts_are_not_capitalised() {
        let mut s = StreamState::new();
        s.update(words("pogoda i świeci słońce dziś"), 3.0, TALKING);
        s.update(words("pogoda i świeci słońce dziś jest"), 3.4, TALKING);
        assert!(!s.committed.is_empty());
        let u = s.update(
            vec![Word::new("Słońce", 0.1, 0.5), Word::new("dziś", 0.6, 0.9)],
            1.0,
            TALKING,
        );
        assert!(!u.display.contains("Słońce"), "{}", u.display);
    }

    #[test]
    fn sentence_starts_keep_their_capital() {
        let mut s = StreamState::new();
        s.update(words("to jest test. Dzisiaj"), 3.0, TALKING);
        s.update(words("to jest test. Dzisiaj jest"), 3.4, TALKING);
        let u = s.update(
            vec![Word::new("Dzisiaj", 0.1, 0.5), Word::new("jest", 0.6, 0.9)],
            1.0,
            TALKING,
        );
        assert!(u.display.contains("Dzisiaj"), "{}", u.display);
    }

    #[test]
    fn a_finished_sentence_starts_the_next_line() {
        let mut s = StreamState::new();
        let u = s.update(words("to jest test. Dzisiaj jest ładnie"), 3.0, TALKING);
        assert_eq!(u.display, "to jest test.\nDzisiaj jest ładnie");
    }

    #[test]
    fn abbreviations_and_numbers_do_not_break_the_line() {
        let mut s = StreamState::new();
        let u = s.update(words("pan dr. Kowalski kupił 3. książkę"), 3.0, TALKING);
        assert_eq!(u.display, "pan dr. Kowalski kupił 3. książkę");
    }

    #[test]
    fn questions_and_exclamations_also_end_a_line() {
        let mut s = StreamState::new();
        let u = s.update(words("co robisz? nic! dobrze"), 2.0, TALKING);
        assert_eq!(u.display, "co robisz?\nnic!\ndobrze");
    }

    #[test]
    fn line_breaks_never_double_up_across_commits() {
        let mut s = StreamState::new();
        s.update(words("to jest test. Dzisiaj jest"), 3.0, TALKING);
        let u = s.update(words("to jest test. Dzisiaj jest ładnie"), 3.4, TALKING);
        assert_eq!(u.display.matches('\n').count(), 1, "{:?}", u.display);
    }

    #[test]
    fn cjk_sentences_break_without_spaces() {
        let mut s = StreamState::new();
        let u = s.update(
            vec![Word::new("字幕。", 0.0, 0.5), Word::new("テスト", 0.5, 1.0)],
            1.0,
            TALKING,
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
        let mut good = words("a b c");
        let before = good.clone();
        sanitize_times(&mut good, 2.0);
        assert_eq!(good, before);
    }

    #[test]
    fn reset_starts_a_new_utterance() {
        let mut s = StreamState::new();
        s.update(words("hello there friend"), 1.5, TALKING);
        assert!(!s.is_empty());
        s.reset();
        assert!(s.is_empty());
        assert_eq!(
            s.update(words("new start"), 1.0, TALKING).display,
            "new start"
        );
    }

    #[test]
    fn empty_and_blank_hypotheses_are_harmless() {
        let mut s = StreamState::new();
        assert_eq!(s.update(vec![], 1.0, TALKING).display, "");
        assert_eq!(
            s.update(vec![Word::new("  ", 0.0, 0.1)], 1.0, TALKING)
                .display,
            ""
        );
    }

    #[test]
    fn tail_words_limits_the_translation_snippet() {
        assert_eq!(StreamState::tail_words("a b c d e f", 3), "d e f");
        assert_eq!(StreamState::tail_words("short one", 10), "short one");
        assert_eq!(StreamState::tail_words("", 3), "");
    }

    // ---------------------------------------------------------------- simulation

    /// Deterministic pseudo-random numbers in 0..1.
    struct Lcg(u64);
    impl Lcg {
        fn next(&mut self) -> f32 {
            self.0 = self
                .0
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            ((self.0 >> 33) as f32) / (u32::MAX >> 1) as f32
        }
    }

    /// A noisy recogniser: every pass hears the truth, except that words within
    /// the last 2 s are often wrong and words 2-3.5 s back are occasionally wrong.
    /// The speaker pauses for 1.5 s after `pause_after` words (if given).
    /// Returns violations of two properties: text older than ~2.6 s on screen never
    /// changes, and no spoken word is ever lost.
    fn simulate(pause_after: Option<usize>, seed: u64) -> Vec<String> {
        let truth: Vec<String> = (1..=45).map(|i| format!("slowo{i}")).collect();
        let word_secs = 0.35;
        let pause = 1.5f32;
        // absolute start/end of word i (a pause is inserted after `pause_after` words)
        let times = |i: usize| -> (f32, f32) {
            let shift = match pause_after {
                Some(n) if i >= n => pause,
                _ => 0.0,
            };
            let t0 = i as f32 * word_secs + shift;
            (t0, t0 + 0.3)
        };
        let mut rng = Lcg(seed);
        let mut state = StreamState::new();
        let mut window_start = 0.0f32; // absolute time where the audio window starts
        let mut shown: Vec<(f32, String)> = Vec::new();

        let mut now = 1.0f32;
        while now < 22.0 {
            let spoken: Vec<usize> = (0..truth.len()).filter(|i| times(*i).1 <= now).collect();
            let hyp: Vec<Word> = spoken
                .iter()
                .filter(|i| (times(**i).0 + times(**i).1) / 2.0 > window_start)
                .map(|i| {
                    let (t0, t1) = times(*i);
                    let age = now - t1;
                    let wrong = (age < 2.0 && rng.next() < 0.45)
                        || ((2.0..3.5).contains(&age) && rng.next() < 0.08);
                    let text = if wrong {
                        format!("zle{}", (rng.next() * 100.0) as u32)
                    } else {
                        truth[*i].clone()
                    };
                    Word::new(
                        &text,
                        (t0 - window_start).max(0.0),
                        (t1 - window_start).max(0.0),
                    )
                })
                .collect();
            // silence at the end of the audio, if the last spoken word ended a while ago
            let last_end = spoken.last().map(|i| times(*i).1).unwrap_or(0.0);
            let trailing =
                (now - last_end >= PAUSE_SECS).then_some((last_end - window_start).max(0.0));
            let audio_secs = now - window_start;
            let u = state.update(hyp, audio_secs, trailing);
            if let Some(cut) = u.cut_secs {
                window_start += cut;
            }
            shown.push((now, u.display));
            now += 0.4;
        }

        let mut violations = Vec::new();
        // 1. old text never changes
        for pair in shown.windows(2) {
            let (t, ref prev) = pair[0];
            let (_, ref cur) = pair[1];
            let words_prev: Vec<&str> = prev.split_whitespace().collect();
            let stable = words_prev
                .len()
                .saturating_sub((2.6 / word_secs) as usize + 1);
            let head = words_prev[..stable].join(" ");
            if !cur.starts_with(&head) {
                violations.push(format!("t={t:.1}: {head:?} no longer a prefix of {cur:?}"));
            }
        }
        // 2. in the end every spoken word is on screen, in order, exactly once
        let last = shown.last().map(|(_, d)| d.clone()).unwrap_or_default();
        let got: Vec<&str> = last.split_whitespace().collect();
        let expected_len = truth.len();
        let indices: Vec<usize> = got
            .iter()
            .filter_map(|w| w.strip_prefix("slowo").and_then(|n| n.parse().ok()))
            .collect();
        // correctly recognised words must appear in order and never twice
        if indices.windows(2).any(|p| p[1] <= p[0]) {
            violations.push(format!("duplicate or out-of-order words in {last:?}"));
        }
        if got.len() != expected_len {
            violations.push(format!(
                "expected {expected_len} words, final caption has {}: {last:?}",
                got.len()
            ));
        }
        violations
    }

    #[test]
    fn continuous_speech_with_a_noisy_recogniser_is_stable_and_loses_nothing() {
        for seed in 1..=25 {
            let v = simulate(None, seed);
            assert!(v.is_empty(), "seed {seed}: {v:#?}");
        }
    }

    #[test]
    fn speech_with_pauses_is_stable_and_loses_nothing() {
        for pause_after in [8, 15, 22] {
            for seed in 1..=15 {
                let v = simulate(Some(pause_after), seed);
                assert!(
                    v.is_empty(),
                    "pause after {pause_after}, seed {seed}: {v:#?}"
                );
            }
        }
    }
}
