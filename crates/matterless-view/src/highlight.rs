//! A code attachment's colours, for the viewer.
//!
//! Sublime Text's grammars through syntect, chosen by the file's extension.
//! Worked out off the window's thread once the text has arrived, which is
//! shown plain meanwhile: a grammar is a stack of regular expressions run
//! line by line from the top, and a large file takes a moment.

use std::sync::OnceLock;
use syntect::easy::HighlightLines;
use syntect::highlighting::{Theme, ThemeSet};
use syntect::parsing::{SyntaxReference, SyntaxSet};

/// Each line's runs of one colour: byte ranges into the line, and the colour.
pub type Colours = Vec<Vec<(usize, usize, [u8; 3])>>;

/// How much of a file is coloured. Past this it stays plain: nobody reads the
/// ten-thousandth line of a file for its colours, and they would cost seconds.
const COLOURED_BYTES: usize = 2 * 1024 * 1024;

/// The theme, for a dark window.
const THEME: &str = "base16-ocean.dark";

fn syntaxes() -> &'static SyntaxSet {
    static SYNTAXES: OnceLock<SyntaxSet> = OnceLock::new();
    SYNTAXES.get_or_init(SyntaxSet::load_defaults_newlines)
}

fn theme() -> Option<&'static Theme> {
    static THEMES: OnceLock<ThemeSet> = OnceLock::new();
    THEMES
        .get_or_init(ThemeSet::load_defaults)
        .themes
        .get(THEME)
}

/// The grammar for a file of this name, if it is code: none for plain text,
/// which has nothing to colour.
fn grammar(name: &str) -> Option<&'static SyntaxReference> {
    let extension = std::path::Path::new(name)
        .extension()?
        .to_str()?
        .to_ascii_lowercase();
    // What the built-in grammars do not know by name, as the nearest they do:
    // Unreal's shaders are HLSL, which reads as C++, and its project files
    // are JSON.
    let extension = match extension.as_str() {
        "usf" | "ush" | "hlsl" | "glsl" | "vert" | "frag" | "comp" | "inl" => "cpp",
        "uproject" | "uplugin" | "jsonl" => "json",
        "yml" => "yaml",
        "txt" | "log" => return None,
        other => other,
    };
    let found = syntaxes().find_syntax_by_extension(extension)?;
    (found.name != "Plain Text").then_some(found)
}

/// Whether a file of this name is coloured at all.
pub fn colours_for(name: &str) -> bool {
    grammar(name).is_some()
}

/// The colours of `lines`, for a file called `name`; nothing when it is not
/// code the grammars know.
pub fn colours(name: &str, lines: &[String]) -> Option<Colours> {
    let grammar = grammar(name)?;
    let mut painter = HighlightLines::new(grammar, theme()?);
    let mut coloured = Vec::with_capacity(lines.len());
    let mut spent = 0usize;
    let mut with_end = String::new();
    for line in lines {
        spent += line.len() + 1;
        if spent > COLOURED_BYTES {
            break;
        }
        // The grammars are the newline kind: a line is matched with its end.
        with_end.clear();
        with_end.push_str(line);
        with_end.push('\n');
        let Ok(runs) = painter.highlight_line(&with_end, syntaxes()) else {
            break;
        };
        let mut kept: Vec<(usize, usize, [u8; 3])> = Vec::new();
        let mut at = 0usize;
        for (style, piece) in runs {
            let end = (at + piece.len()).min(line.len());
            let colour = [style.foreground.r, style.foreground.g, style.foreground.b];
            if end > at {
                match kept.last_mut() {
                    Some(last) if last.2 == colour && last.1 == at => last.1 = end,
                    _ => kept.push((at, end, colour)),
                }
            }
            at += piece.len();
        }
        coloured.push(kept);
    }
    Some(coloured)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Code is coloured in runs covering its line, a keyword apart from what
    /// follows it; a log is left alone.
    #[test]
    fn code_is_coloured_and_a_log_is_not() {
        let lines = vec!["fn main() { let x = 1; }".to_string(), String::new()];
        let coloured = colours("main.rs", &lines).expect("Rust is known");
        assert_eq!(coloured.len(), 2);
        let first = &coloured[0];
        assert_eq!(first.first().map(|run| run.0), Some(0));
        assert_eq!(first.last().map(|run| run.1), Some(lines[0].len()));
        assert!(first.len() > 3, "more than one colour: {first:?}");
        assert!(coloured[1].is_empty());
        assert!(colours("build.log", &lines).is_none());
        assert!(colours("Shader.usf", &lines).is_some(), "read as C++");
    }
}
