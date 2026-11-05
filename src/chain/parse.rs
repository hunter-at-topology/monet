//! A tiny chain grammar:
//!
//! ```text
//! program := (stmt NEWLINE)*
//! stmt    := IDENT '=' expr  |  def  |  'wgsl' BLOCK  |  ['$' ':'] expr
//! def     := ('picture' | 'effect') IDENT '(' [param (',' param)*] ')' BLOCK
//! param   := IDENT ['=' expr]
//! expr    := term (('+' | '-') term)*
//! term    := unary (('*' | '/') unary)*
//! unary   := '-' unary | postfix
//! postfix := primary ('.' IDENT ['(' args ')'])*
//! primary := NUMBER | STRING | IDENT ['(' args ')'] | '(' expr ')'
//! ```
//!
//! A line starting with `.` continues the previous one, so chains can be
//! written one effect per line. A BLOCK is `{ … }` of native WGSL, kept as text.

use std::ops::Range;

use super::Error;
use super::lex::{Tok, Token, lex};

#[derive(Clone, Debug, PartialEq)]
pub struct Expr {
    pub kind: Kind,
    pub span: Range<usize>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Kind {
    Num(f64),
    /// Contents and the byte offset where they start (just past the quote).
    Str(String, usize),
    Ident(String),
    /// `f(args)` — `name_span` covers just the name.
    Call {
        name: String,
        name_span: Range<usize>,
        args: Vec<Expr>,
    },
    /// `recv.name(args)` / `recv.name`
    Method {
        recv: Box<Expr>,
        name: String,
        name_span: Range<usize>,
        args: Vec<Expr>,
        called: bool,
    },
    Bin(char, Box<Expr>, Box<Expr>),
    Neg(Box<Expr>),
}

#[derive(Clone, Debug, PartialEq)]
pub enum Stmt {
    Let {
        name: String,
        span: Range<usize>,
        value: Expr,
    },
    /// `picture name(…) { wgsl }` or `effect name(…) { wgsl }`
    Def(Def),
    /// `wgsl { … }`: helper functions for your shaders
    Wgsl {
        code: String,
        /// Byte offset of the code (just past the `{`).
        start: usize,
    },
    Expr(Expr),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DefKind {
    /// A source: the body returns the colour at `uv`.
    Picture,
    /// Chained with `.`: the body returns the new colour at `uv`, reading the
    /// picture so far with `pic(p)`.
    Effect,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Param {
    pub name: String,
    pub span: Range<usize>,
    pub default: Option<Expr>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Def {
    pub kind: DefKind,
    pub name: String,
    pub name_span: Range<usize>,
    pub params: Vec<Param>,
    pub body: String,
    /// Byte offset of the body (just past the `{`).
    pub body_start: usize,
    /// From `picture` to the closing `}`.
    pub span: Range<usize>,
}

struct P {
    toks: Vec<Token>,
    i: usize,
}

impl P {
    fn peek(&self) -> &Tok {
        &self.toks[self.i].tok
    }
    fn peek_at(&self, k: usize) -> &Tok {
        &self.toks[(self.i + k).min(self.toks.len() - 1)].tok
    }
    fn span(&self) -> Range<usize> {
        self.toks[self.i].span.clone()
    }
    fn bump(&mut self) -> Token {
        let t = self.toks[self.i].clone();
        if self.i < self.toks.len() - 1 {
            self.i += 1;
        }
        t
    }
    fn eat(&mut self, s: &str) -> bool {
        if matches!(self.peek(), Tok::Sym(x) if *x == s) {
            self.bump();
            true
        } else {
            false
        }
    }
    fn expect(&mut self, s: &str, ctx: &str) -> Result<Range<usize>, Error> {
        if self.eat(s) {
            Ok(self.toks[self.i - 1].span.clone())
        } else {
            Err(Error::new(
                self.span(),
                format!("expected `{s}` {ctx}{}", self.found()),
            ))
        }
    }
    fn found(&self) -> String {
        match self.peek() {
            Tok::Eof => ", but the code ended".into(),
            Tok::Newline => ", but the line ended".into(),
            Tok::Num(n) => format!(", found `{n}`"),
            Tok::Str(s) => format!(", found \"{s}\""),
            Tok::Ident(s) => format!(", found `{s}`"),
            Tok::Sym(s) => format!(", found `{s}`"),
            Tok::Block(_) => ", found a `{` block".into(),
        }
    }
    /// Skip a newline when the next line starts with `.` (a continued chain).
    fn continue_chain(&mut self) {
        if *self.peek() == Tok::Newline && matches!(self.peek_at(1), Tok::Sym(".")) {
            self.bump();
        }
    }

    fn block(&mut self, what: &str) -> Result<(String, usize, usize), Error> {
        if let Tok::Block(code) = self.peek().clone() {
            let span = self.bump().span;
            return Ok((code, span.start + 1, span.end));
        }
        Err(Error::new(
            self.span(),
            format!("expected `{{` and the shader code {what}{}", self.found()),
        ))
    }

    fn def(&mut self, kind: DefKind) -> Result<Stmt, Error> {
        let start = self.bump().span.start;
        let t = self.bump();
        let Tok::Ident(name) = t.tok else { unreachable!() };
        let name_span = t.span;
        self.expect("(", "after the name")?;
        let mut params = Vec::new();
        if !self.eat(")") {
            loop {
                let t = self.bump();
                let Tok::Ident(pname) = t.tok else {
                    self.i -= 1;
                    return Err(Error::new(
                        t.span,
                        format!(
                            "expected a parameter name, like amount = .5{}",
                            self.found()
                        ),
                    ));
                };
                let default = if self.eat("=") {
                    Some(self.expr()?)
                } else {
                    None
                };
                params.push(Param {
                    name: pname,
                    span: t.span,
                    default,
                });
                if self.eat(",") {
                    continue;
                }
                self.expect(")", "to close the parameters")?;
                break;
            }
        }
        let (body, body_start, end) = self.block("of this shader")?;
        Ok(Stmt::Def(Def {
            kind,
            name,
            name_span,
            params,
            body,
            body_start,
            span: start..end,
        }))
    }

    fn stmt(&mut self) -> Result<Stmt, Error> {
        if let (Tok::Ident(kw), Tok::Ident(_), Tok::Sym("(")) =
            (self.peek().clone(), self.peek_at(1).clone(), self.peek_at(2).clone())
        {
            match kw.as_str() {
                "picture" => return self.def(DefKind::Picture),
                "effect" => return self.def(DefKind::Effect),
                _ => {}
            }
        }
        if matches!((self.peek(), self.peek_at(1)), (Tok::Ident(kw), Tok::Block(_)) if kw == "wgsl") {
            self.bump();
            let (code, start, _) = self.block("")?;
            return Ok(Stmt::Wgsl { code, start });
        }
        if let (Tok::Ident(kw), Tok::Ident(name)) = (self.peek().clone(), self.peek_at(1).clone())
            && matches!(kw.as_str(), "picture" | "effect")
        {
            let span = self.toks[self.i + 1].span.clone();
            return Err(Error::new(
                span.end..span.end,
                format!("expected `(` after `{name}`: {kw} {name}(…) {{ shader code }}"),
            ));
        }
        if let (Tok::Ident(name), Tok::Sym("=")) = (self.peek().clone(), self.peek_at(1).clone()) {
            let span = self.bump().span;
            self.bump();
            let value = self.expr()?;
            return Ok(Stmt::Let { name, span, value });
        }
        if matches!(self.peek(), Tok::Sym("$")) && matches!(self.peek_at(1), Tok::Sym(":")) {
            self.bump();
            self.bump();
        }
        Ok(Stmt::Expr(self.expr()?))
    }

    fn expr(&mut self) -> Result<Expr, Error> {
        let mut a = self.term()?;
        loop {
            self.continue_chain();
            let op = match self.peek() {
                Tok::Sym("+") => '+',
                Tok::Sym("-") => '-',
                _ => return Ok(a),
            };
            self.bump();
            let b = self.term()?;
            let span = a.span.start..b.span.end;
            a = Expr {
                kind: Kind::Bin(op, Box::new(a), Box::new(b)),
                span,
            };
        }
    }

    fn term(&mut self) -> Result<Expr, Error> {
        let mut a = self.unary()?;
        loop {
            let op = match self.peek() {
                Tok::Sym("*") => '*',
                Tok::Sym("/") => '/',
                _ => return Ok(a),
            };
            self.bump();
            let b = self.unary()?;
            let span = a.span.start..b.span.end;
            a = Expr {
                kind: Kind::Bin(op, Box::new(a), Box::new(b)),
                span,
            };
        }
    }

    fn unary(&mut self) -> Result<Expr, Error> {
        if matches!(self.peek(), Tok::Sym("-")) {
            let s = self.bump().span;
            let e = self.unary()?;
            let span = s.start..e.span.end;
            // Fold literal negatives so `-0.5` scrubs as one number.
            if let Kind::Num(n) = e.kind {
                return Ok(Expr {
                    kind: Kind::Num(-n),
                    span,
                });
            }
            return Ok(Expr {
                kind: Kind::Neg(Box::new(e)),
                span,
            });
        }
        self.postfix()
    }

    fn args(&mut self) -> Result<(Vec<Expr>, usize), Error> {
        let mut args = Vec::new();
        if self.eat(")") {
            return Ok((args, self.toks[self.i - 1].span.end));
        }
        loop {
            args.push(self.expr()?);
            if self.eat(",") {
                continue;
            }
            let end = self.expect(")", "to close the arguments")?;
            return Ok((args, end.end));
        }
    }

    fn postfix(&mut self) -> Result<Expr, Error> {
        let mut e = self.primary()?;
        loop {
            self.continue_chain();
            if !matches!(self.peek(), Tok::Sym(".")) {
                return Ok(e);
            }
            self.bump();
            let t = self.bump();
            let Tok::Ident(name) = t.tok else {
                return Err(Error::new(t.span, "expected an effect name after `.`"));
            };
            let name_span = t.span.clone();
            let (args, end, called) = if self.eat("(") {
                let (a, end) = self.args()?;
                (a, end, true)
            } else {
                (Vec::new(), name_span.end, false)
            };
            let span = e.span.start..end;
            e = Expr {
                kind: Kind::Method {
                    recv: Box::new(e),
                    name,
                    name_span,
                    args,
                    called,
                },
                span,
            };
        }
    }

    fn primary(&mut self) -> Result<Expr, Error> {
        let t = self.bump();
        let span = t.span.clone();
        match t.tok {
            Tok::Num(n) => Ok(Expr {
                kind: Kind::Num(n),
                span,
            }),
            Tok::Str(s) => Ok(Expr {
                kind: Kind::Str(s, span.start + 1),
                span,
            }),
            Tok::Ident(name) => {
                if self.eat("(") {
                    let (args, end) = self.args()?;
                    Ok(Expr {
                        kind: Kind::Call {
                            name,
                            name_span: span.clone(),
                            args,
                        },
                        span: span.start..end,
                    })
                } else {
                    Ok(Expr {
                        kind: Kind::Ident(name),
                        span,
                    })
                }
            }
            Tok::Sym("(") => {
                let e = self.expr()?;
                self.expect(")", "to close the bracket")?;
                Ok(e)
            }
            _ => {
                self.i -= 1;
                Err(Error::new(
                    span,
                    format!("expected a value{}", self.found()),
                ))
            }
        }
    }
}

pub fn parse(src: &str) -> Result<Vec<Stmt>, Error> {
    let mut p = P {
        toks: lex(src)?,
        i: 0,
    };
    let mut out = Vec::new();
    loop {
        while *p.peek() == Tok::Newline {
            p.bump();
        }
        if *p.peek() == Tok::Eof {
            return Ok(out);
        }
        out.push(p.stmt()?);
        match p.peek() {
            Tok::Newline | Tok::Eof => {}
            _ => {
                return Err(Error::new(
                    p.span(),
                    format!("expected a new line{}. Chain effects with `.`", p.found()),
                ));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chain_over_lines() {
        let s = parse("mesh(\"dawn\")\n  .flow(.3)\n  .grain()\nstars()").unwrap();
        assert_eq!(s.len(), 2);
        let Stmt::Expr(e) = &s[0] else { panic!() };
        let Kind::Method { name, recv, .. } = &e.kind else {
            panic!()
        };
        assert_eq!(name, "grain");
        assert!(matches!(&recv.kind, Kind::Method { name, .. } if name == "flow"));
    }

    #[test]
    fn lets_and_arithmetic() {
        let s = parse("p = \"dawn\"\nx = sine * 2 + 1\n$: mesh(p)").unwrap();
        assert!(matches!(&s[0], Stmt::Let { name, .. } if name == "p"));
        let Stmt::Let { value, .. } = &s[1] else {
            panic!()
        };
        assert!(matches!(value.kind, Kind::Bin('+', _, _)));
        assert!(matches!(&s[2], Stmt::Expr(_)));
    }

    #[test]
    fn negative_literals_fold() {
        let s = parse("julia(\"x\", -0.8)").unwrap();
        let Stmt::Expr(Expr {
            kind: Kind::Call { args, .. },
            ..
        }) = &s[0]
        else {
            panic!()
        };
        assert_eq!(args[1].kind, Kind::Num(-0.8));
        assert_eq!(args[1].span, 11..15);
    }

    #[test]
    fn shader_definitions() {
        let src = "// fades\neffect fade(amount = .5, tint = \"dawn\", other) {\n  return pic(uv) * amount;\n}\nwgsl { fn two() -> f32 { return 2.0; } }\nmesh().fade(.2)";
        let s = parse(src).unwrap();
        let Stmt::Def(d) = &s[0] else { panic!("{s:?}") };
        assert_eq!((d.kind, d.name.as_str()), (DefKind::Effect, "fade"));
        assert_eq!(&src[d.name_span.clone()], "fade");
        let names: Vec<_> = d.params.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["amount", "tint", "other"]);
        assert!(d.params[2].default.is_none());
        assert_eq!(&src[d.body_start..d.body_start + d.body.len()], d.body);
        assert!(d.body.contains("pic(uv)"));
        assert!(matches!(&s[1], Stmt::Wgsl { code, .. } if code.contains("fn two")));
        assert!(matches!(&s[2], Stmt::Expr(_)));
        // `picture = 1` is still a variable
        assert!(matches!(&parse("picture = 1").unwrap()[0], Stmt::Let { .. }));
        assert!(parse("picture p()").unwrap_err().msg.contains("shader code"));
        assert!(parse("effect e {").unwrap_err().msg.contains("never ends"));
        assert!(parse("effect e { }").unwrap_err().msg.contains("expected `(`"));
        assert!(parse("picture p(1) { }").unwrap_err().msg.contains("parameter name"));
    }

    #[test]
    fn friendly_errors() {
        assert!(
            parse("mesh(\"dawn\" flow()")
                .unwrap_err()
                .msg
                .contains("expected `)`")
        );
        assert!(
            parse("mesh() grain()")
                .unwrap_err()
                .msg
                .contains("Chain effects with `.`")
        );
    }
}
