//! Tokens. Spans are byte ranges into the source, so the editor can highlight
//! and scrub exactly what the evaluator saw.

use std::ops::Range;

use super::Error;

#[derive(Clone, Debug, PartialEq)]
pub enum Tok {
    Num(f64),
    /// String contents (without quotes); the span covers the quotes.
    Str(String),
    Ident(String),
    Sym(&'static str),
    /// `{ … }`: native WGSL, kept as written (without the braces); the span covers them.
    Block(String),
    Newline,
    Eof,
}

#[derive(Clone, Debug)]
pub struct Token {
    pub tok: Tok,
    pub span: Range<usize>,
}

const SYMS: &[&str] = &["(", ")", ",", ".", "=", "+", "-", "*", "/", "$", ":"];

pub fn lex(src: &str) -> Result<Vec<Token>, Error> {
    let b = src.as_bytes();
    let mut out: Vec<Token> = Vec::new();
    let mut i = 0;
    let mut depth = 0i32;
    while i < b.len() {
        let c = b[i];
        if c == b'\n' {
            if depth <= 0 && !matches!(out.last().map(|t| &t.tok), None | Some(Tok::Newline)) {
                out.push(Token {
                    tok: Tok::Newline,
                    span: i..i + 1,
                });
            }
            i += 1;
            continue;
        }
        if c.is_ascii_whitespace() {
            i += 1;
            continue;
        }
        if c == b'/' && b.get(i + 1) == Some(&b'/') {
            while i < b.len() && b[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        if c == b'/' && b.get(i + 1) == Some(&b'*') {
            i += 2;
            while i < b.len() && !(b[i] == b'*' && b.get(i + 1) == Some(&b'/')) {
                i += 1;
            }
            i = (i + 2).min(b.len());
            continue;
        }
        let start = i;
        let after_value = out.last().is_some_and(|t| {
            t.span.end == i && matches!(t.tok, Tok::Ident(_) | Tok::Num(_) | Tok::Sym(")"))
        });
        if c.is_ascii_digit()
            || (c == b'.' && b.get(i + 1).is_some_and(u8::is_ascii_digit) && !after_value)
        {
            while i < b.len() && b[i].is_ascii_digit() {
                i += 1;
            }
            if i < b.len() && b[i] == b'.' && b.get(i + 1).is_some_and(u8::is_ascii_digit) {
                i += 1;
                while i < b.len() && b[i].is_ascii_digit() {
                    i += 1;
                }
            }
            let text = &src[start..i];
            let v = text
                .parse::<f64>()
                .map_err(|_| Error::new(start..i, format!("`{text}` isn't a number")))?;
            out.push(Token {
                tok: Tok::Num(v),
                span: start..i,
            });
            continue;
        }
        if c == b'"' || c == b'\'' || c == b'`' {
            i += 1;
            while i < b.len() && b[i] != c {
                i += 1;
            }
            if i >= b.len() {
                return Err(Error::new(
                    start..b.len(),
                    "this string never ends: add a closing quote",
                ));
            }
            i += 1;
            out.push(Token {
                tok: Tok::Str(src[start + 1..i - 1].to_string()),
                span: start..i,
            });
            continue;
        }
        if c.is_ascii_alphabetic() || c == b'_' {
            while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'_') {
                i += 1;
            }
            out.push(Token {
                tok: Tok::Ident(src[start..i].to_string()),
                span: start..i,
            });
            continue;
        }
        if c == b'{' {
            let end = block_end(b, i).ok_or_else(|| {
                Error::new(i..i + 1, "this shader block never ends: add a closing `}`")
            })?;
            out.push(Token {
                tok: Tok::Block(src[i + 1..end].to_string()),
                span: i..end + 1,
            });
            i = end + 1;
            continue;
        }
        if c == b'}' {
            return Err(Error::new(
                i..i + 1,
                "this `}` closes nothing: shader code goes after picture name(…) { or effect name(…) {",
            ));
        }
        let Some(sym) = SYMS.iter().find(|s| src[i..].starts_with(**s)) else {
            let ch = src[i..].chars().next().unwrap_or('?');
            return Err(Error::new(
                i..i + ch.len_utf8(),
                format!("unexpected `{ch}`"),
            ));
        };
        match *sym {
            "(" => depth += 1,
            ")" => depth -= 1,
            _ => {}
        }
        out.push(Token {
            tok: Tok::Sym(sym),
            span: i..i + sym.len(),
        });
        i += sym.len();
    }
    out.push(Token {
        tok: Tok::Eof,
        span: b.len()..b.len(),
    });
    Ok(out)
}

/// The `}` matching the `{` at `open`, skipping WGSL comments. `None` if it never closes.
pub fn block_end(b: &[u8], open: usize) -> Option<usize> {
    let mut depth = 0usize;
    let mut i = open;
    while i < b.len() {
        match b[i] {
            b'/' if b.get(i + 1) == Some(&b'/') => {
                while i < b.len() && b[i] != b'\n' {
                    i += 1;
                }
                continue;
            }
            b'/' if b.get(i + 1) == Some(&b'*') => {
                i += 2;
                while i < b.len() && !(b[i] == b'*' && b.get(i + 1) == Some(&b'/')) {
                    i += 1;
                }
                i += 2;
                continue;
            }
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn toks(s: &str) -> Vec<Tok> {
        lex(s).unwrap().into_iter().map(|t| t.tok).collect()
    }

    #[test]
    fn chains_and_numbers() {
        assert_eq!(
            toks("mesh(\"dawn\").flow(.3)"),
            vec![
                Tok::Ident("mesh".into()),
                Tok::Sym("("),
                Tok::Str("dawn".into()),
                Tok::Sym(")"),
                Tok::Sym("."),
                Tok::Ident("flow".into()),
                Tok::Sym("("),
                Tok::Num(0.3),
                Tok::Sym(")"),
                Tok::Eof
            ]
        );
    }

    #[test]
    fn newlines_inside_parens_vanish() {
        let t = toks("a(1,\n2)\n\n\nb // hi\n");
        assert_eq!(t.iter().filter(|t| **t == Tok::Newline).count(), 2);
    }

    #[test]
    fn spans_are_bytes() {
        let t = lex("x  \"0 1\"").unwrap();
        assert_eq!(t[1].span, 3..8);
    }

    #[test]
    fn shader_blocks_are_one_token() {
        let src = "picture p() {\n  let a = 1.0; // }\n  if (a > 0.0) { return vec4f(a); }\n}\nmesh()";
        let t = lex(src).unwrap();
        let block = t.iter().find(|t| matches!(t.tok, Tok::Block(_))).unwrap();
        assert_eq!(&src[block.span.clone()], &src[12..src.len() - 7]);
        assert!(src[block.span.clone()].ends_with("}\n}"));
        assert!(lex("effect e() { return pic(uv);").unwrap_err().msg.contains("never ends"));
        assert!(lex("mesh() }").unwrap_err().msg.contains("closes nothing"));
    }

    #[test]
    fn unterminated_string() {
        assert!(
            lex("mesh(\"dawn)")
                .unwrap_err()
                .msg
                .contains("closing quote")
        );
    }
}
