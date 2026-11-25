//! Local wake-word matching for the always-on household assistant.
//!
//! The appliance never sends microphone audio to a cloud service. A local
//! listener (the optional `voice/wakeword.py` helper, or a future on-device
//! streaming STT engine) transcribes speech and posts the text to the daemon;
//! this module decides whether the configured assistant name was called.
//!
//! Matching is token based (so `casamiento` does not wake `Casa`), accent and
//! case insensitive, and tolerant to small typos. When the wake word is
//! followed by a request in the same sentence (`"oye Casa, enciende la luz"`),
//! the remaining words are returned as the trailing query.

use crate::config::VoiceConfig;

/// Minimum similarity required before the configured name is considered called.
///
/// Exact token matches always pass; fuzzy matches need to be close enough to
/// tolerate STT typos while keeping false positives low.
pub const MIN_FUZZY_SIMILARITY: f32 = 0.75;

/// Result of a successful wake-word match.
#[derive(Debug, Clone, PartialEq)]
pub struct WakeMatch {
    /// Normalized name variant that matched (`casa`, `oye casa`).
    pub phrase: String,
    /// Similarity in `0.0..=1.0` (1.0 for an exact token match).
    pub score: f32,
    /// Words spoken after the wake word, when the user chained a request.
    pub trailing_query: Option<String>,
}

/// Matches transcripts against the configured assistant name.
#[derive(Debug, Clone)]
pub struct WakeWordMatcher {
    assistant_name: String,
    normalized_name: String,
    name_tokens: Vec<String>,
    threshold: f32,
}

impl WakeWordMatcher {
    /// Build a matcher for the configured assistant name.
    ///
    /// `sensitivity` in `0.0..=1.0` lowers the fuzzy threshold as it grows:
    /// 0.0 is strict (almost exact), 1.0 tolerates edits on longer names.
    /// Wake-word matching only runs on locally produced transcripts.
    #[must_use]
    pub fn from_config(config: &VoiceConfig) -> Self {
        let threshold = (0.85 - config.sensitivity.clamp(0.0, 1.0) * 0.25).clamp(0.55, 0.9);
        Self::new(&config.assistant_name, threshold)
    }

    /// Build a matcher with an explicit similarity threshold (0.5..=1.0).
    #[must_use]
    pub fn new(assistant_name: &str, threshold: f32) -> Self {
        let normalized_name = normalize_phrase(assistant_name);
        let name_tokens = tokenize(&normalized_name);
        Self {
            assistant_name: assistant_name.trim().to_owned(),
            normalized_name,
            name_tokens,
            threshold: threshold.clamp(0.5, 1.0),
        }
    }

    /// Configured assistant name (display form).
    #[must_use]
    pub fn assistant_name(&self) -> &str {
        &self.assistant_name
    }

    /// Whether `transcript` calls the assistant by name.
    #[must_use]
    pub fn matches(&self, transcript: &str) -> Option<WakeMatch> {
        if self.name_tokens.is_empty() {
            return None;
        }
        let normalized = normalize_phrase(transcript);
        let tokens = tokenize(&normalized);
        if tokens.is_empty() {
            return None;
        }

        let name_len = self.name_tokens.len();
        if tokens.len() < name_len {
            return None;
        }

        let mut best: Option<(usize, usize, f32)> = None;
        for start in 0..=(tokens.len() - name_len) {
            let window = &tokens[start..start + name_len];
            let similarity = window_similarity(window, &self.name_tokens);
            if similarity < self.threshold {
                continue;
            }
            let exact = similarity >= 0.999;
            let improved = match best {
                None => true,
                Some((_, _, best_score)) => {
                    similarity > best_score + f32::EPSILON
                        || ((similarity - best_score).abs() <= f32::EPSILON && exact)
                }
            };
            if improved {
                best = Some((start, start + name_len, similarity));
            }
        }

        let (start, end, score) = best?;
        let phrase = tokens[start..end].join(" ");
        let trailing = tokens[end..].join(" ");
        Some(WakeMatch {
            phrase,
            score,
            trailing_query: if trailing.is_empty() {
                None
            } else {
                Some(trailing)
            },
        })
    }

    /// Normalized name used for diagnostics and the UI hint.
    #[must_use]
    pub fn normalized_name(&self) -> &str {
        &self.normalized_name
    }
}

/// Lowercase, strip diacritics and reduce punctuation to single spaces.
///
/// This makes `"¡Oye, Cása!"` and `"oye casa"` equivalent for wake detection.
#[must_use]
pub fn normalize_phrase(input: &str) -> String {
    let mut output = String::with_capacity(input.len());
    let mut pending_space = false;
    for character in input.chars() {
        let folded = match character.to_lowercase().next().unwrap_or(character) {
            'á' | 'à' | 'ä' | 'â' | 'ã' | 'å' => Some('a'),
            'é' | 'è' | 'ë' | 'ê' => Some('e'),
            'í' | 'ì' | 'ï' | 'î' => Some('i'),
            'ó' | 'ò' | 'ö' | 'ô' | 'õ' => Some('o'),
            'ú' | 'ù' | 'ü' | 'û' => Some('u'),
            'ñ' => Some('n'),
            'ç' => Some('c'),
            other if other.is_alphanumeric() => Some(other),
            _ => None,
        };
        match folded {
            Some(character) => {
                if pending_space && !output.is_empty() {
                    output.push(' ');
                }
                output.push(character);
                pending_space = false;
            }
            None => pending_space = true,
        }
    }
    output
}

fn tokenize(normalized: &str) -> Vec<String> {
    normalized.split_whitespace().map(str::to_owned).collect()
}

fn window_similarity(window: &[String], name: &[String]) -> f32 {
    if window.len() != name.len() {
        return 0.0;
    }
    let total: usize = window
        .iter()
        .zip(name)
        .map(|(left, right)| levenshtein(left, right))
        .sum();
    let max_len: usize = window.iter().map(String::len).sum();
    if max_len == 0 {
        return 1.0;
    }
    1.0 - (total as f32 / max_len as f32)
}

/// Levenshtein distance between two words (character level).
fn levenshtein(left: &str, right: &str) -> usize {
    let right_chars: Vec<char> = right.chars().collect();
    let mut previous: Vec<usize> = (0..=right_chars.len()).collect();
    let mut current = vec![0usize; right_chars.len() + 1];

    for (row, left_char) in left.chars().enumerate() {
        current[0] = row + 1;
        for (column, right_char) in right_chars.iter().enumerate() {
            let substitution = previous[column] + usize::from(left_char != *right_char);
            let insertion = current[column] + 1;
            let deletion = previous[column + 1] + 1;
            current[column + 1] = substitution.min(insertion).min(deletion);
        }
        std::mem::swap(&mut previous, &mut current);
    }
    previous[right_chars.len()]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::VoiceConfig;

    fn matcher(name: &str) -> WakeWordMatcher {
        WakeWordMatcher::new(name, MIN_FUZZY_SIMILARITY)
    }

    #[test]
    fn exact_name_matches() {
        let matcher = matcher("Casa");
        let matched = matcher.matches("casa").expect("wake");
        assert_eq!(matched.phrase, "casa");
        assert!((matched.score - 1.0).abs() < f32::EPSILON);
        assert!(matched.trailing_query.is_none());
    }

    #[test]
    fn greetings_case_and_accents_are_ignored() {
        let matcher = matcher("Cása");
        assert!(matcher.matches("Oye, CASA").is_some());
        assert!(matcher.matches("¡Hola Cása!").is_some());
        assert!(matcher.matches("  oye   casa  ").is_some());
    }

    #[test]
    fn chained_request_is_returned_as_trailing_query() {
        let matched = matcher("Aura")
            .matches("Oye Aura enciende la luz del salón")
            .expect("wake with request");
        assert_eq!(
            matched.trailing_query.as_deref(),
            Some("enciende la luz del salon")
        );
    }

    #[test]
    fn substrings_do_not_wake_the_assistant() {
        let matcher = matcher("Casa");
        assert!(matcher.matches("casamiento").is_none());
        assert!(matcher.matches("esto es una casa bonita").is_some());
        assert!(matcher.matches("cas").is_none());
    }

    #[test]
    fn small_stt_typos_still_match() {
        let matcher = matcher("Aura");
        assert!(matcher.matches("oye aura").is_some());
        assert!(matcher.matches("oye ara").is_none());
        assert!(matcher.matches("oye auro").is_some());
    }

    #[test]
    fn multiword_names_match_contiguously() {
        let matcher = matcher("Hey Home");
        let matched = matcher.matches("hey home pon musica").expect("wake");
        assert_eq!(matched.trailing_query.as_deref(), Some("pon musica"));
        assert!(matcher.matches("home hey").is_none());
    }

    #[test]
    fn sensitivity_lowers_the_fuzzy_threshold() {
        let strict = VoiceConfig {
            assistant_name: "Casa".to_owned(),
            sensitivity: 0.0,
            ..VoiceConfig::default()
        };
        let permissive = VoiceConfig {
            assistant_name: "Casa".to_owned(),
            sensitivity: 1.0,
            ..VoiceConfig::default()
        };
        let threshold_strict = WakeWordMatcher::from_config(&strict);
        let threshold_permissive = WakeWordMatcher::from_config(&permissive);
        assert!(threshold_strict.matches("cassa").is_none());
        assert!(threshold_permissive.matches("cassa").is_some());
    }

    #[test]
    fn empty_transcript_or_name_never_matches() {
        assert!(matcher("Aura").matches("").is_none());
        assert!(matcher("Aura").matches("   ").is_none());
        assert!(matcher("").matches("hola").is_none());
    }

    #[test]
    fn normalization_is_stable() {
        assert_eq!(
            normalize_phrase("¡Oye, Cása!  ¿qué tal?"),
            "oye casa que tal"
        );
        assert_eq!(normalize_phrase("AURA"), "aura");
        assert_eq!(normalize_phrase("..."), "");
    }
}
