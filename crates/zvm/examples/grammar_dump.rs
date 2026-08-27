//! Dump a story's grammar table, in `infodump -g`'s spelling.
//!
//! A development tool, not a feature: it exists so `zvm::grammar` can be
//! diffed line-for-line against ztools' `infodump`, which is the reference
//! implementation for every table shape this module reads.
//!
//!     cargo run -p zvm --example grammar_dump -- stories/zork1.z3
//!     cargo run -p zvm --example grammar_dump -- --sentences stories/zork1.z3
//!
//! `--sentences` prints one sentence per line and nothing else, which is what
//! the comparison script diffs against infodump's quoted renderings.
//!
//! The library's own `SyntaxLine::describe` deliberately uses neutral names;
//! infodump's uppercase-for-GV1 / lowercase-for-GV2 / `OBJ`-for-Infocom
//! spelling is reproduced here rather than in the crate.

use zvm::grammar::{Grammar, GrammarFormat, RoutineRef, SyntaxLine, Token};
use zvm::memory::Memory;

fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let sentences_only = args.iter().any(|a| a == "--sentences");
    args.retain(|a| a != "--sentences");
    let Some(path) = args.first() else {
        eprintln!("usage: grammar_dump [--sentences] <story-file>");
        std::process::exit(2);
    };

    let bytes = match std::fs::read(path) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("{path}: {e}");
            std::process::exit(2);
        }
    };
    let mem = match Memory::new(bytes) {
        Ok(m) => m,
        Err(e) => {
            eprintln!("{path}: {e:?}");
            std::process::exit(2);
        }
    };

    let grammar = match Grammar::load(&mem) {
        Ok(g) => g,
        Err(e) => {
            eprintln!("{path}: no grammar ({e:?})");
            std::process::exit(1);
        }
    };

    if !sentences_only {
        eprintln!("format = {:?}, verbs = {}", grammar.format(), grammar.verbs().len());
    }

    for verb in grammar.verbs() {
        let name = verb.word().unwrap_or("no-verb");
        if !sentences_only {
            println!(
                "{:3}. {} entr{}, verb = {:?}{}",
                verb.number,
                verb.lines.len(),
                if verb.lines.len() == 1 { "y" } else { "ies" },
                name,
                if verb.words.len() > 1 {
                    format!(", synonyms = {}", verb.words[1..].join(", "))
                } else {
                    String::new()
                }
            );
        }
        for line in &verb.lines {
            let text = render(grammar.format(), name, line);
            if sentences_only {
                println!("{text}");
            } else {
                println!("    {text}");
            }
        }
    }
}

/// Render one line the way infodump prints it, for the format in hand.
fn render(format: GrammarFormat, verb: &str, line: &SyntaxLine) -> String {
    let mut out = String::from(verb);
    for slot in &line.slots {
        for (i, tok) in slot.alternatives.iter().enumerate() {
            out.push(' ');
            if i > 0 {
                out.push_str("/ ");
            }
            out.push_str(&render_token(format, tok));
        }
    }
    if line.reverse {
        out.push_str(" REVERSE");
    }
    out
}

// Every public type in `zvm::grammar` is `#[non_exhaustive]`, so an out-of-crate
// consumer — which this example is — matches with a wildcard. That is the shape
// an embedder's own renderer will have.
fn render_token(format: GrammarFormat, tok: &Token) -> String {
    match tok {
        Token::Word(w) => w.clone(),
        Token::InfocomObject { .. } => "OBJ".to_string(),
        Token::Noun(k) => match format {
            GrammarFormat::InformGv2 => k.name().to_string(),
            GrammarFormat::Inform5 | GrammarFormat::InformGv1 => k.name().to_uppercase(),
            _ => "OBJ".to_string(),
        },
        Token::Attribute(a) => format!("ATTRIBUTE({a})"),
        Token::FilteredNoun(r) => match r {
            RoutineRef::Index(i) => format!("NOUN [parse {i}]"),
            RoutineRef::Packed(a) => format!("noun = [parse ${a:04x}]"),
            _ => "NOUN [parse ?]".to_string(),
        },
        Token::Routine(r) => match r {
            RoutineRef::Index(i) => format!("TEXT [parse {i}]"),
            RoutineRef::Packed(a) => format!("[parse ${a:04x}]"),
            _ => "[parse ?]".to_string(),
        },
        Token::Scope(r) => match r {
            RoutineRef::Index(i) => format!("SCOPE [parse {i}]"),
            RoutineRef::Packed(a) => format!("scope = [parse ${a:04x}]"),
            _ => "scope = [parse ?]".to_string(),
        },
        _ => "UNKNOWN".to_string(),
    }
}
