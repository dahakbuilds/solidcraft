//! A minimal G-code reader: the linear moves and tool changes a post produces, turned
//! into [CL points](ClPoint) the [simulator](sim) can replay.
//!
//! The supported dialect is the common flat subset of RS-274/ISO 6983: absolute
//! (G90) and incremental (G91) linear moves (G0 rapids, G1 feeds), millimetres or
//! inches (G21/G20), `;` and `(…)` comments, tool words (T) with tool changes (M6),
//! and program ends (M2/M30). Everything else is either ignored with a warning
//! (spindle and coolant words, feed rates, work-offset and plane selectors) or refused
//! with a clear "not supported yet" error — arc moves (G2/G3), canned cycles (G81 and
//! friends) and return-to-home (G28) included, because silently dropping a cutting
//! move from a simulator would hide exactly the gouge the reader came to find.
//!
//! Coordinates are taken as machine coordinates in the file's unit; the caller places
//! the stock in the same frame. Everything is bounded: file size, line count, point
//! count and warning count all have caps.

use crate::{Result, invalid, sim::ClPoint};

/// Largest G-code text one read may parse.
pub const MAX_BYTES: usize = 4 << 20;
/// Largest number of lines one read may parse.
pub const MAX_LINES: usize = 100_000;
/// Largest number of CL points one read may produce.
pub const MAX_POINTS: usize = 100_000;
/// Warnings a read keeps before dropping the rest.
pub const MAX_WARNINGS: usize = 64;

/// What a G-code file decoded to: the tip path, the tool numbers seen, and the words
/// that were ignored (capped at [`MAX_WARNINGS`]).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct GcodePath {
    pub points: Vec<ClPoint>,
    /// Tool words in the order they appeared; a `T` followed by `M6` is one change.
    pub tools: Vec<String>,
    pub warnings: Vec<String>,
}

/// One `(letter, value)` word of a block.
struct Word {
    letter: char,
    value: f64,
}

/// Decode G-code text into a replayable path.
pub fn read(text: &str) -> Result<GcodePath> {
    if text.len() > MAX_BYTES {
        return Err(invalid(format!("the G-code file is over {} MiB", MAX_BYTES >> 20)));
    }
    let mut out = GcodePath::default();
    let mut absolute = true;
    let mut scale = 1.0;
    let mut motion = 0u32;
    let (mut x, mut y, mut z) = (0.0, 0.0, 0.0);
    let mut pending_tool: Option<String> = None;
    for (index, raw) in text.lines().enumerate() {
        if index >= MAX_LINES {
            return Err(invalid(format!("the G-code file has more than {MAX_LINES} lines")));
        }
        let line = index + 1;
        let words = match block(&strip_comment(raw)) {
            Ok(words) => words,
            Err(why) => return Err(invalid(format!("line {line}: {why}"))),
        };
        if words.is_empty() {
            continue;
        }
        let mut gs: Vec<u32> = Vec::new();
        let mut m: Option<u32> = None;
        let mut target = [None, None, None];
        let mut has_move = false;
        for word in words {
            match word.letter {
                'g' => gs.push(word.value as u32),
                'm' => m = Some(word.value as u32),
                't' => pending_tool = Some(format!("T{}", word.value as u32)),
                'x' | 'y' | 'z' => {
                    if !word.value.is_finite() {
                        return Err(invalid(format!("line {line}: `{}` is not a finite coordinate", word.letter.to_uppercase())));
                    }
                    target[if word.letter == 'x' {
                        0
                    } else if word.letter == 'y' {
                        1
                    } else {
                        2
                    }] = Some(word.value);
                    has_move = true;
                }
                // Program, feed, spindle, block numbers and tool-length compensation
                // carry no geometry for the replay.
                'o' | 'f' | 's' | 'n' | 'h' | 'd' => {}
                other => {
                    if out.warnings.len() < MAX_WARNINGS {
                        out.warnings.push(format!("line {line}: word `{}` is ignored", other.to_uppercase()));
                    }
                }
            }
        }
        // A block can carry several G words (`G90 G1 X…`); each applies in order.
        for g in gs {
            match g {
                0 | 1 => motion = g,
                20 => scale = 25.4,
                21 => scale = 1.0,
                90 => absolute = true,
                91 => absolute = false,
                // Plane, feed mode, dwell, compensation, work offsets and stops: no geometry.
                4 | 17 | 18 | 19 | 40 | 43 | 44 | 49 | 53 | 54..=59 | 65..=72 | 80 | 94 | 95 | 98 | 99 => {}
                2 | 3 => return Err(invalid(format!("not supported yet: arc moves (G{g}) at line {line}"))),
                28 => return Err(invalid(format!("not supported yet: return-to-home (G28) at line {line}"))),
                73..=89 => return Err(invalid(format!("not supported yet: canned cycles (G{g}) at line {line}"))),
                other => {
                    if out.warnings.len() < MAX_WARNINGS {
                        out.warnings.push(format!("line {line}: G{other} is ignored"));
                    }
                }
            }
        }
        if has_move {
            // Apply the moves so each axis sees its own current value.
            if let Some(v) = target[0] {
                x = if absolute { v * scale } else { x + v * scale };
            }
            if let Some(v) = target[1] {
                y = if absolute { v * scale } else { y + v * scale };
            }
            if let Some(v) = target[2] {
                z = if absolute { v * scale } else { z + v * scale };
            }
            if out.points.len() >= MAX_POINTS {
                return Err(invalid(format!("the G-code file produces more than {MAX_POINTS} moves")));
            }
            out.points.push(ClPoint { x, y, z, cut: motion == 1 });
        }
        // A stop ends the program after its own block's move; a tool change lands
        // before any later move.
        if let Some(m) = m {
            match m {
                2 | 30 => return Ok(finish(out, pending_tool)),
                6 => {
                    if let Some(tool) = pending_tool.take() {
                        out.tools.push(tool);
                    }
                }
                _ => {}
            }
        }
    }
    Ok(finish(out, pending_tool))
}

fn finish(mut out: GcodePath, pending_tool: Option<String>) -> GcodePath {
    if let Some(tool) = pending_tool {
        out.tools.push(tool);
    }
    out
}

/// Drop `; …` and `( … )` comments.
fn strip_comment(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut in_paren = false;
    for c in line.chars() {
        match c {
            '(' => in_paren = true,
            ')' => in_paren = false,
            ';' => break,
            _ if !in_paren => out.push(c),
            _ => {}
        }
    }
    out
}

/// Split a block into `(letter, value)` words: `G1 X10.5 Y-2 F120`.
fn block(line: &str) -> std::result::Result<Vec<Word>, String> {
    let chars: Vec<char> = line.chars().collect();
    let mut words = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let letter = chars[i];
        if letter.is_whitespace() {
            i += 1;
            continue;
        }
        if !letter.is_ascii_alphabetic() {
            return Err(format!("unexpected `{letter}`"));
        }
        let letter = letter.to_ascii_lowercase();
        i += 1;
        let start = i;
        if i < chars.len() && (chars[i] == '-' || chars[i] == '+') {
            i += 1;
        }
        let digits_start = i;
        while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '.') {
            i += 1;
        }
        if i == digits_start {
            return Err(format!("`{}` has no number", letter.to_uppercase()));
        }
        let number: String = chars[start..i].iter().collect();
        let value = number.parse::<f64>().map_err(|_| format!("`{}` is not a number", letter.to_uppercase()))?;
        words.push(Word { letter, value });
    }
    Ok(words)
}

#[cfg(test)]
#[path = "gcode_tests.rs"]
mod tests;
