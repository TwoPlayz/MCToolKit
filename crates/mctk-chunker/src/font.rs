//! Text width in Minecraft's default font, for wrapping sign text. Ported from Chunker's
//! `FontUtil` (MIT, Hive Games); the width table is Chunker's
//! `default_font_width_codepoints.json` (`data/font_widths.json`).
//!
//! Bedrock stores a sign as one string that the game wraps by itself, while Java stores
//! four separate lines. Splitting a Bedrock string into lines needs the pixel width of
//! every character.

use std::collections::HashMap;
use std::sync::OnceLock;

const SECTION: char = '§';

fn widths() -> &'static HashMap<u32, u8> {
    static W: OnceLock<HashMap<u32, u8>> = OnceLock::new();
    W.get_or_init(|| {
        let groups: Vec<Vec<u32>> = serde_json::from_str(include_str!("../data/font_widths.json"))
            .expect("built-in font width table is valid");
        let mut map = HashMap::new();
        // The array index is the width.
        for (width, code_points) in groups.iter().enumerate() {
            for cp in code_points {
                map.insert(*cp, width as u8);
            }
        }
        map
    })
}

pub fn char_width(c: char) -> u32 {
    widths().get(&(c as u32)).copied().unwrap_or(0) as u32
}

pub fn text_width(s: &str) -> u32 {
    s.chars().map(char_width).sum()
}

/// Wraps one line of text at word boundaries so no line is wider than `max_width`.
/// `§` formatting codes take no space.
pub fn split_line(max_width: u32, input: &str) -> Vec<String> {
    let space = char_width(' ');
    let mut lines = Vec::new();
    let (mut line, mut word) = (String::new(), String::new());
    let (mut line_width, mut word_width) = (0u32, 0u32);
    let mut skip_next = false;
    for c in input.chars() {
        let width = if skip_next || c == SECTION { 0 } else { char_width(c) };
        if skip_next {
            skip_next = false;
        } else if c == SECTION {
            skip_next = true;
        }
        if c.is_whitespace() {
            if line_width + word_width + space > max_width {
                lines.push(std::mem::take(&mut line));
                line_width = 0;
            } else if !line.is_empty() {
                line.push(' ');
                line_width += space;
            }
            line.push_str(&word);
            line_width += word_width;
            word.clear();
            word_width = 0;
        } else {
            word.push(c);
            word_width += width;
        }
    }
    if !word.is_empty() {
        if line_width + word_width + space > max_width {
            lines.push(std::mem::take(&mut line));
        } else if !line.is_empty() {
            line.push(' ');
        }
        line.push_str(&word);
    }
    if !line.is_empty() {
        lines.push(line);
    }
    lines
}

/// Splits `original` lines (already separated at `\n`) so the text fills up to `max_lines`
/// lines. Text that already uses all lines is left alone.
pub fn split(max_width: u32, max_lines: usize, original: &[String]) -> Vec<String> {
    if original.len() >= max_lines || original.is_empty() {
        return original.to_vec();
    }
    let mut out: Vec<String> = original
        .iter()
        .flat_map(|l| split_line(max_width, l))
        .collect();
    out.truncate(max_lines);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn widths_match_the_game_font() {
        assert_eq!(char_width(' '), 4);
        assert_eq!(char_width('i'), 2);
        assert_eq!(char_width('W'), 6);
        assert_eq!(text_width("Hi"), char_width('H') + 2);
    }

    #[test]
    fn long_text_wraps_at_words() {
        let lines = split_line(88, "The quick brown fox jumps over the lazy dog");
        assert!(lines.len() >= 3, "{lines:?}");
        assert!(lines.iter().all(|l| text_width(l) <= 88), "{lines:?}");
        assert_eq!(lines.join(" "), "The quick brown fox jumps over the lazy dog");
    }

    #[test]
    fn formatting_codes_take_no_room() {
        assert_eq!(split_line(88, "§cHello"), vec!["§cHello".to_string()]);
    }

    #[test]
    fn full_signs_are_kept_as_they_are() {
        let four: Vec<String> = ["a", "b", "c", "d"].iter().map(|s| s.to_string()).collect();
        assert_eq!(split(88, 4, &four), four);
    }
}
