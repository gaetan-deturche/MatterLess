//! What a message parses into, as a tree.
//!
//! Between raw markdown and rows on screen there are two steps that can each be
//! wrong in ways that look the same: the parse, and the layout that walks it.
//! This prints the first, so a wrong shape on screen can be blamed on the right
//! one.
//!
//!     cargo run -p matterless-render --example what_markdown -- <file>

use matterless_render::markdown::Node;

fn main() {
    let Some(path) = std::env::args().nth(1) else {
        return eprintln!("give it a file holding one message");
    };
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(error) => return eprintln!("could not read {path}: {error}"),
    };
    for node in matterless_render::markdown::parse(&text) {
        show(&node, 0);
    }
}

fn show(node: &Node, depth: usize) {
    let pad = "  ".repeat(depth);
    match node {
        Node::Text { value } => println!("{pad}Text {:?}", cut(value)),
        Node::UserMention { username, everyone } => {
            println!(
                "{pad}UserMention @{username}{}",
                if *everyone { " (everyone)" } else { "" }
            )
        }
        Node::ChannelLink { name } => println!("{pad}ChannelLink ~{name}"),
        Node::Emoji { name, unicode } => println!("{pad}Emoji :{name}: {unicode:?}"),
        Node::InlineCode { value } => println!("{pad}InlineCode {:?}", cut(value)),
        Node::CodeBlock { language, value } => {
            println!("{pad}CodeBlock {language:?} {:?}", cut(value))
        }
        Node::Image { url, alt } => println!("{pad}Image {:?} {:?}", cut(url), cut(alt)),
        Node::SoftBreak => println!("{pad}SoftBreak"),
        Node::HardBreak => println!("{pad}HardBreak"),
        Node::Rule => println!("{pad}Rule"),
        Node::Link { href, children } => {
            println!("{pad}Link {:?}", cut(href));
            for child in children {
                show(child, depth + 1);
            }
        }
        Node::List { ordered, items } => {
            println!("{pad}List ordered={ordered} ({} items)", items.len());
            for (at, item) in items.iter().enumerate() {
                println!("{pad}  item {at}");
                for child in item {
                    show(child, depth + 2);
                }
            }
        }
        Node::Heading { level, children } => {
            println!("{pad}Heading {level}");
            for child in children {
                show(child, depth + 1);
            }
        }
        Node::Paragraph { children } => {
            println!("{pad}Paragraph");
            for child in children {
                show(child, depth + 1);
            }
        }
        Node::Emphasis { children } => nested(pad, "Emphasis", children, depth),
        Node::Strong { children } => nested(pad, "Strong", children, depth),
        Node::Strike { children } => nested(pad, "Strike", children, depth),
        Node::Blockquote { children } => nested(pad, "Blockquote", children, depth),
        other => println!("{pad}{other:?}"),
    }
}

fn nested(pad: String, what: &str, children: &[Node], depth: usize) {
    println!("{pad}{what}");
    for child in children {
        show(child, depth + 1);
    }
}

fn cut(value: &str) -> String {
    match value.chars().count() > 44 {
        true => format!("{}\u{2026}", value.chars().take(44).collect::<String>()),
        false => value.to_string(),
    }
}
