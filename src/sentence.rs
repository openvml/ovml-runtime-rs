//! Language-agnostic sentence splitting and approximate word timings.

use serde::{Deserialize, Serialize};

const SENTENCE_END: &str = ".!?\u{2026}\u{3002}\u{ff01}\u{ff1f}";
const CLOSERS: &str = "\"'\u{00bb}\u{201d}\u{2019})]}\u{300d}\u{300f}";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Word {
    pub text: String,
    pub start_ms: u32,
    pub end_ms: u32,
}

fn prev_nonspace(chars: &[char], index: usize) -> Option<char> {
    let mut pos = index;
    while pos > 0 {
        pos -= 1;
        if !chars[pos].is_whitespace() {
            return Some(chars[pos]);
        }
    }
    None
}

fn next_nonspace(chars: &[char], index: usize) -> Option<char> {
    let mut pos = index;
    while pos < chars.len() {
        if !chars[pos].is_whitespace() {
            return Some(chars[pos]);
        }
        pos += 1;
    }
    None
}

fn is_boundary(chars: &[char], index: usize) -> bool {
    let ch = chars[index];
    if ch == '.' {
        let prev = prev_nonspace(chars, index);
        let next = next_nonspace(chars, index + 1);
        if matches!((prev, next), (Some(p), Some(n)) if p.is_numeric() && n.is_numeric()) {
            return false;
        }
        if let Some(immediate) = chars.get(index + 1) {
            if !immediate.is_whitespace() && immediate.is_alphanumeric() {
                return false;
            }
        }
        return true;
    }
    SENTENCE_END.contains(ch)
}

fn push_part(out: &mut Vec<String>, value: &[char]) {
    let value: String = value.iter().collect();
    let value = value.trim();
    if !value.is_empty() {
        out.push(value.to_string());
    }
}

fn split_long(value: &[char], max_chars: usize, out: &mut Vec<String>) {
    if value.len() <= max_chars {
        out.push(value.iter().collect::<String>().trim().to_string());
        return;
    }
    let mut cut = None;
    for (index, ch) in value.iter().take(max_chars).enumerate() {
        if ch.is_whitespace() || matches!(ch, ',' | ';' | ':' | '\u{2014}' | '\u{2013}') {
            cut = Some(index);
        }
    }
    let Some(cut) = cut.filter(|index| *index > 0) else {
        out.push(value.iter().collect::<String>().trim().to_string());
        return;
    };
    let head: String = value[..cut].iter().collect();
    let head = head.trim();
    if !head.is_empty() {
        out.push(head.to_string());
    }
    split_long(&value[cut + 1..], max_chars, out);
}

pub fn split_sentences(text: &str, max_chars: usize) -> Vec<String> {
    if text.trim().is_empty() {
        return Vec::new();
    }
    let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
    let chars: Vec<char> = normalized.chars().collect();
    let mut parts: Vec<String> = Vec::new();
    let mut start = 0usize;
    let mut index = 0usize;

    while index < chars.len() {
        if is_boundary(&chars, index) {
            let mut end = index + 1;
            while end < chars.len()
                && (SENTENCE_END.contains(chars[end]) || CLOSERS.contains(chars[end]))
            {
                end += 1;
            }
            push_part(&mut parts, &chars[start..end]);
            start = end;
            index = end;
            continue;
        }
        if chars[index] == '\n' {
            push_part(&mut parts, &chars[start..index]);
            start = index + 1;
        }
        index += 1;
    }
    push_part(&mut parts, &chars[start..]);

    let mut result = Vec::new();
    for part in parts {
        let chars: Vec<char> = part.chars().collect();
        split_long(&chars, max_chars.max(1), &mut result);
    }
    result.into_iter().filter(|part| !part.is_empty()).collect()
}

pub fn word_timings(text: &str, duration_ms: u32) -> Vec<Word> {
    let tokens: Vec<&str> = text.split_whitespace().collect();
    if tokens.is_empty() || duration_ms == 0 {
        return Vec::new();
    }
    let weights: Vec<u64> = tokens
        .iter()
        .map(|token| {
            let count = token.chars().filter(|ch| ch.is_alphanumeric()).count() as u64;
            count.max(1)
        })
        .collect();
    let total: u64 = weights.iter().sum();
    let mut words = Vec::with_capacity(tokens.len());
    let mut accumulated = 0u64;
    let mut cursor = 0u32;
    for (token, weight) in tokens.iter().zip(weights.iter()) {
        accumulated += weight;
        let end = if accumulated == total {
            duration_ms
        } else {
            ((u64::from(duration_ms) * accumulated) / total) as u32
        };
        words.push(Word {
            text: (*token).to_string(),
            start_ms: cursor,
            end_ms: end,
        });
        cursor = end;
    }
    words
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_basic_sentences() {
        assert_eq!(
            split_sentences("Hello world. How are you? Fine!", 240),
            vec!["Hello world.", "How are you?", "Fine!"]
        );
    }

    #[test]
    fn keeps_decimal_and_newlines() {
        assert_eq!(
            split_sentences("It is 3.14 now.\nNext line", 240),
            vec!["It is 3.14 now.", "Next line"]
        );
    }

    #[test]
    fn respects_long_text_limit() {
        let parts = split_sentences("one two three four five six", 12);
        assert!(parts.iter().all(|part| part.chars().count() <= 12));
        assert_eq!(parts.join(" "), "one two three four five six");
    }

    #[test]
    fn word_timings_cover_duration() {
        let words = word_timings("Hello world", 1000);
        assert_eq!(words.len(), 2);
        assert_eq!(words[0].start_ms, 0);
        assert_eq!(words[1].end_ms, 1000);
    }
}
