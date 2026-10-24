//! Post processors: turning a toolpath into the G-code one machine wants.
//!
//! A post is a small JSON description — the motion words, the unit line, header and
//! footer lines — that [`emit`] applies to a [Toolpath]. The built-ins cover the
//! common ISO mill in millimetres and inches; users write their own as JSON and the
//! engine commands keep them in a posts folder. Arcs whose plane is not parallel to
//! XY are tessellated into feeds, because most 3-axis controls only arc in XY.

use crate::toolpath::{Segment, Toolpath};
use crate::{Result, invalid};
use serde::{Deserialize, Serialize};

/// Longest header or footer a post may carry.
pub const MAX_POST_LINES: usize = 64;
/// Longest single line of a post's text.
const MAX_LINE_LEN: usize = 120;
/// Longest emitted program.
const MAX_PROGRAM_LEN: usize = 16_000_000;

fn is_word(s: &str) -> bool {
    !s.is_empty() && s.len() <= 8 && !s.chars().any(char::is_control)
}

fn is_text(lines: &[String]) -> bool {
    lines.len() <= MAX_POST_LINES && lines.iter().all(|l| l.len() <= MAX_LINE_LEN && !l.chars().any(char::is_control))
}

/// One post processor: the words a machine answers to.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Post {
    pub id: String,
    pub name: String,
    pub machine: String,
    /// Emit absolute moves (G90); otherwise incremental (G91).
    pub absolute: bool,
    /// The unit line, e.g. `G21` (millimetres) or `G20` (inches). Empty omits it.
    pub unit_line: String,
    pub rapid: String,
    pub feed: String,
    pub arc_cw: String,
    pub arc_ccw: String,
    /// The feed-rate word, usually `F`.
    pub feed_word: String,
    /// The spindle line; `{rpm}` is replaced. Empty omits it.
    pub spindle: String,
    pub header: Vec<String>,
    pub footer: Vec<String>,
}

impl Default for Post {
    fn default() -> Self {
        Post {
            id: "iso_mill".into(),
            name: "ISO 3-axis mill".into(),
            machine: "3-axis mill".into(),
            absolute: true,
            unit_line: "G21".into(),
            rapid: "G0".into(),
            feed: "G1".into(),
            arc_cw: "G2".into(),
            arc_ccw: "G3".into(),
            feed_word: "F".into(),
            spindle: "M3 S{rpm}".into(),
            header: vec!["(SolidCraft)".into()],
            footer: vec!["M5".into(), "G0 Z50".into(), "M30".into()],
        }
    }
}

impl Post {
    /// Refuse anything a writer could not safely emit.
    pub fn check(&self) -> Result<()> {
        if self.id.is_empty()
            || self.id.len() > 64
            || !self.id.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
        {
            return Err(invalid("post id must be 1..64 letters, digits, dot, dash or underscore"));
        }
        if self.name.is_empty() || self.name.len() > 120 || self.name.chars().any(char::is_control) {
            return Err(invalid("post name must be 1..120 printable characters"));
        }
        if self.machine.len() > 120 || self.machine.chars().any(char::is_control) {
            return Err(invalid("post machine must be at most 120 printable characters"));
        }
        for (what, word) in
            [("unit line", &self.unit_line), ("rapid", &self.rapid), ("feed", &self.feed), ("arc cw", &self.arc_cw), ("arc ccw", &self.arc_ccw), ("feed word", &self.feed_word)]
        {
            if !is_word(word) {
                return Err(invalid(format!("post {what} must be 1..8 printable characters")));
            }
        }
        if self.spindle.len() > 40 || self.spindle.chars().any(char::is_control) {
            return Err(invalid("post spindle line must be at most 40 printable characters"));
        }
        if !is_text(&self.header) || !is_text(&self.footer) {
            return Err(invalid(format!("post header and footer are at most {MAX_POST_LINES} lines of {MAX_LINE_LEN} characters")));
        }
        Ok(())
    }
}

/// The posts that ship with SolidCraft.
pub fn builtin_posts() -> Vec<Post> {
    let mm = Post::default();
    let inch = Post {
        id: "inch_mill".into(),
        name: "Inch 3-axis mill".into(),
        unit_line: "G20".into(),
        footer: vec!["M5".into(), "G0 Z2".into(), "M30".into()],
        ..Default::default()
    };
    vec![mm, inch]
}

/// How to write the moves: a feed rate, a spindle speed and an optional tool note.
#[derive(Clone, Debug)]
pub struct EmitOptions {
    /// Feed in millimetres per minute (or inches, if the post says inches).
    pub feed: f64,
    pub rpm: f64,
    /// A tool label to name in a comment; `None` omits it.
    pub tool: Option<String>,
}

impl Default for EmitOptions {
    fn default() -> Self {
        EmitOptions { feed: 500.0, rpm: 8000.0, tool: None }
    }
}

/// A coordinate with trailing zeros trimmed, so programs read like a human wrote them.
fn num(v: f64) -> String {
    if !v.is_finite() {
        return "0".into();
    }
    let s = format!("{v:.4}");
    let t = s.trim_end_matches('0').trim_end_matches('.');
    if t == "-0" || t.is_empty() {
        "0".into()
    } else {
        t.to_string()
    }
}

/// Write `path` as a program in `post`'s dialect.
pub fn emit(post: &Post, path: &Toolpath, o: &EmitOptions) -> Result<String> {
    post.check()?;
    if !o.feed.is_finite() || o.feed <= 0.0 || o.feed > 1e6 {
        return Err(invalid("feed must be a positive number"));
    }
    if !o.rpm.is_finite() || o.rpm < 0.0 || o.rpm > 1e6 {
        return Err(invalid("rpm must be a non-negative number"));
    }
    let mut out = String::new();
    let mut push = |line: &str| -> Result<()> {
        if out.len() + line.len() + 1 > MAX_PROGRAM_LEN {
            return Err(invalid("the emitted program is too large"));
        }
        out.push_str(line);
        out.push('\n');
        Ok(())
    };
    for line in &post.header {
        push(line)?;
    }
    if !post.unit_line.is_empty() {
        push(&post.unit_line)?;
    }
    push(if post.absolute { "G90" } else { "G91" })?;
    if !post.spindle.is_empty() && o.rpm > 0.0 {
        push(&post.spindle.replace("{rpm}", &num(o.rpm)))?;
    }
    if let Some(tool) = &o.tool
        && !tool.is_empty()
        && tool.len() <= 64
        && !tool.chars().any(char::is_control)
    {
        push(&format!("({tool})"))?;
    }
    let feed = num(o.feed);
    for seg in &path.segments {
        match *seg {
            Segment::Line { from, to, cut } => {
                if cut {
                    push(&format!("{} X{} Y{} Z{} {}{}", post.feed, num(to[0]), num(to[1]), num(to[2]), post.feed_word, feed))?;
                } else {
                    // A rapid: clear Z first when descending, XY first when rising —
                    // the classic safe-move order so the tool never drags.
                    if to[2] <= from[2] {
                        push(&format!("{} Z{}", post.rapid, num(to[2])))?;
                        push(&format!("{} X{} Y{}", post.rapid, num(to[0]), num(to[1])))?;
                    } else {
                        push(&format!("{} X{} Y{}", post.rapid, num(to[0]), num(to[1])))?;
                        push(&format!("{} Z{}", post.rapid, num(to[2])))?;
                    }
                }
            }
            Segment::Arc { from, to, center, normal, clockwise, cut } => {
                let planar = normal[2].abs() > 0.999;
                if cut && planar {
                    // G2/G3 read clockwise looking down the +Z axis; a normal of -Z
                    // flips what our `clockwise` means.
                    let cw_top = clockwise != (normal[2] < 0.0);
                    let word = if cw_top { &post.arc_cw } else { &post.arc_ccw };
                    push(&format!(
                        "{word} X{} Y{} I{} J{} {}{feed}",
                        num(to[0]),
                        num(to[1]),
                        num(center[0] - from[0]),
                        num(center[1] - from[1]),
                        post.feed_word
                    ))?;
                } else {
                    // Non-cut arcs and tilted arcs: chords, as the control cannot arc them.
                    let one = Toolpath { segments: vec![*seg] };
                    let Ok(lines) = one.tessellate(crate::toolpath::DEFAULT_TOLERANCE) else { continue };
                    for line in lines {
                        for w in line.points.windows(2) {
                            let b = w[1];
                            if cut {
                                push(&format!("{} X{} Y{} Z{} {}{feed}", post.feed, num(b.x), num(b.y), num(b.z), post.feed_word))?;
                            } else {
                                push(&format!("{} X{} Y{} Z{}", post.rapid, num(b.x), num(b.y), num(b.z)))?;
                            }
                        }
                    }
                }
            }
        }
    }
    for line in &post.footer {
        push(line)?;
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::toolpath::Segment;

    fn path() -> Toolpath {
        Toolpath {
            segments: vec![
                Segment::Line { from: [0.0, 0.0, 5.0], to: [10.0, 0.0, 5.0], cut: false },
                Segment::Line { from: [10.0, 0.0, 5.0], to: [10.0, 0.0, -1.0], cut: true },
                Segment::Line { from: [10.0, 0.0, -1.0], to: [20.0, 0.0, -1.0], cut: true },
                Segment::Arc { from: [20.0, 0.0, -1.0], to: [20.0, 4.0, -1.0], center: [20.0, 2.0, -1.0], normal: [0.0, 0.0, 1.0], clockwise: false, cut: true },
            ],
        }
    }

    #[test]
    fn emits_a_program() {
        let text = emit(&Post::default(), &path(), &EmitOptions::default()).unwrap();
        assert!(text.starts_with("(SolidCraft)\nG21\nG90\nM3 S8000\n"), "{text}");
        assert!(text.contains("G0 Z5\nG0 X10 Y0\n"), "rapid clears Z first: {text}");
        assert!(text.contains("G1 X10 Y0 Z-1 F500"), "{text}");
        assert!(text.contains("G3 X20 Y4 I0 J2 F500"), "ccw arc in XY: {text}");
        assert!(text.ends_with("M5\nG0 Z50\nM30\n"), "{text}");
    }

    #[test]
    fn tilted_arcs_tessellate() {
        let p = Toolpath {
            segments: vec![Segment::Arc {
                from: [0.0, 0.0, 0.0],
                to: [0.0, 0.0, 2.0],
                center: [0.0, 0.0, 1.0],
                normal: [1.0, 0.0, 0.0],
                clockwise: false,
                cut: true,
            }],
        };
        let text = emit(&Post::default(), &p, &EmitOptions::default()).unwrap();
        // Many short feeds instead of one G3.
        assert!(text.matches("G1 ").count() > 3, "{text}");
        assert!(!text.contains("G3"), "{text}");
    }

    #[test]
    fn hostile_posts_are_refused() {
        let bad = Post { id: String::new(), ..Default::default() };
        assert!(emit(&bad, &path(), &EmitOptions::default()).is_err());
        let bad = Post { rapid: "G0\x01".into(), ..Default::default() };
        assert!(emit(&bad, &path(), &EmitOptions::default()).is_err());
        let bad = Post { header: vec!["x".repeat(200)], ..Default::default() };
        assert!(emit(&bad, &path(), &EmitOptions::default()).is_err());
        assert!(emit(&Post::default(), &path(), &EmitOptions { feed: f64::NAN, ..Default::default() }).is_err());
        assert!(emit(&Post::default(), &path(), &EmitOptions { feed: -1.0, ..Default::default() }).is_err());
    }

    #[test]
    fn posts_round_trip_through_json() {
        let json = serde_json::to_string(&Post::default()).unwrap();
        let back: Post = serde_json::from_str(&json).unwrap();
        assert_eq!(back, Post::default());
        // Unknown fields are refused so typos do not silently default.
        assert!(serde_json::from_str::<Post>(r#"{"id":"x","unknown":1}"#).is_err());
    }
}
