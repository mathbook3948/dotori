//! Reads tokens into statements and expressions: the part of JavaScript that Playwright scripts
//! use. Anything else is refused by name, so the model can write it another way.

use std::sync::Arc;

use crate::script::lex::Lexed;
use crate::script::lex::Piece;
use crate::script::lex::Token;
use crate::script::lex::lex;

#[derive(Clone, Debug)]
pub(crate) enum Stmt {
    Let {
        names: Pattern,
        value: Option<Expr>,
        constant: bool,
    },
    Expr(Expr),
    If {
        test: Expr,
        then: Box<Stmt>,
        otherwise: Option<Box<Stmt>>,
    },
    ForOf {
        name: Pattern,
        constant: bool,
        items: Expr,
        body: Box<Stmt>,
    },
    For {
        init: Option<Box<Stmt>>,
        test: Option<Expr>,
        update: Option<Expr>,
        body: Box<Stmt>,
    },
    While {
        test: Expr,
        body: Box<Stmt>,
    },
    Block(Vec<Stmt>),
    Return(Option<Expr>),
    Break,
    Continue,
    Throw(Expr),
    Try {
        body: Vec<Stmt>,
        name: Option<String>,
        catch: Option<Vec<Stmt>>,
        finally: Option<Vec<Stmt>>,
    },
    Function {
        name: String,
        function: Arc<Function>,
    },
    Empty,
}

#[derive(Clone, Debug)]
pub(crate) enum Pattern {
    Name(String),
    Array(Vec<Option<String>>),
    Object(Vec<(String, String)>),
}

#[derive(Clone, Debug)]
pub(crate) struct Function {
    pub(crate) params: Vec<Pattern>,
    pub(crate) body: Body,
    pub(crate) source: String,
}

#[derive(Clone, Debug)]
pub(crate) enum Body {
    Expr(Box<Expr>),
    Block(Vec<Stmt>),
}

#[derive(Clone, Debug)]
pub(crate) enum Expr {
    Number(f64),
    Str(String),
    Template(Vec<Expr>),
    Regex {
        source: String,
        flags: String,
    },
    Bool(bool),
    Null,
    Undefined,
    Name(String),
    Array(Vec<Item>),
    Object(Vec<(Key, Expr)>),
    Member {
        object: Box<Expr>,
        property: Box<Key>,
        optional: bool,
    },
    Call {
        callee: Box<Expr>,
        args: Vec<Item>,
        optional: bool,
    },
    New {
        callee: Box<Expr>,
        args: Vec<Item>,
    },
    Unary {
        op: &'static str,
        value: Box<Expr>,
    },
    Binary {
        op: &'static str,
        left: Box<Expr>,
        right: Box<Expr>,
    },
    Logical {
        op: &'static str,
        left: Box<Expr>,
        right: Box<Expr>,
    },
    Conditional {
        test: Box<Expr>,
        then: Box<Expr>,
        otherwise: Box<Expr>,
    },
    Assign {
        op: &'static str,
        target: Box<Expr>,
        value: Box<Expr>,
    },
    Update {
        op: &'static str,
        prefix: bool,
        target: Box<Expr>,
    },
    Function(Arc<Function>),
    Await(Box<Expr>),
}

#[derive(Clone, Debug)]
pub(crate) enum Item {
    One(Expr),
    Spread(Expr),
}

#[derive(Clone, Debug)]
pub(crate) enum Key {
    Name(String),
    Computed(Expr),
    Spread,
}

/// Reads a whole script.
pub(crate) fn parse(source: &str) -> Result<Vec<Stmt>, String> {
    let tokens = lex(source)?;
    let mut parser = Parser {
        tokens,
        at: 0,
        source,
    };
    let mut body = Vec::new();
    while !parser.is_end() {
        body.push(parser.statement()?);
    }
    Ok(body)
}

struct Parser<'a> {
    tokens: Vec<Lexed>,
    at: usize,
    source: &'a str,
}

const UNSUPPORTED: [&str; 8] = [
    "class", "switch", "with", "yield", "import", "export", "do", "label",
];

impl Parser<'_> {
    fn peek(&self) -> &Token {
        &self.tokens[self.at].token
    }

    fn peek_at(&self, ahead: usize) -> &Token {
        &self.tokens[(self.at + ahead).min(self.tokens.len() - 1)].token
    }

    fn line(&self) -> usize {
        self.tokens[self.at].line
    }

    fn is_end(&self) -> bool {
        matches!(self.peek(), Token::End)
    }

    fn next(&mut self) -> Token {
        let token = self.tokens[self.at].token.clone();
        if self.at < self.tokens.len() - 1 {
            self.at += 1;
        }
        token
    }

    fn is(&self, punct: &str) -> bool {
        matches!(self.peek(), Token::Punct(p) if *p == punct)
    }

    fn is_word(&self, word: &str) -> bool {
        matches!(self.peek(), Token::Name(name) if name == word)
    }

    fn eat(&mut self, punct: &str) -> bool {
        if self.is(punct) {
            self.next();
            true
        } else {
            false
        }
    }

    fn eat_word(&mut self, word: &str) -> bool {
        if self.is_word(word) {
            self.next();
            true
        } else {
            false
        }
    }

    fn expect(&mut self, punct: &str) -> Result<(), String> {
        if self.eat(punct) {
            Ok(())
        } else {
            Err(self.error(&format!("expected `{punct}`")))
        }
    }

    fn error(&self, what: &str) -> String {
        let found = match self.peek() {
            Token::End => String::from("the end"),
            Token::Name(name) => format!("`{name}`"),
            Token::Punct(punct) => format!("`{punct}`"),
            Token::Number(n) => format!("{n}"),
            Token::Str(_) | Token::Template(_) => String::from("a string"),
            Token::Regex { .. } => String::from("a regular expression"),
        };
        format!("line {}: {what}, found {found}", self.line())
    }

    fn name(&mut self) -> Result<String, String> {
        match self.next() {
            Token::Name(name) => Ok(name),
            _ => {
                self.at -= 1;
                Err(self.error("expected a name"))
            }
        }
    }

    fn end_statement(&mut self) {
        self.eat(";");
    }

    fn block(&mut self) -> Result<Vec<Stmt>, String> {
        self.expect("{")?;
        let mut body = Vec::new();
        while !self.is("}") {
            if self.is_end() {
                return Err(self.error("expected `}`"));
            }
            body.push(self.statement()?);
        }
        self.next();
        Ok(body)
    }

    fn statement(&mut self) -> Result<Stmt, String> {
        if let Token::Name(word) = self.peek()
            && UNSUPPORTED.contains(&word.as_str())
            && word != "label"
        {
            return Err(self.error(&format!("`{word}` is not supported")));
        }
        if self.eat(";") {
            return Ok(Stmt::Empty);
        }
        if self.is("{") {
            return Ok(Stmt::Block(self.block()?));
        }
        if self.is_word("const") || self.is_word("let") || self.is_word("var") {
            let stmt = self.declaration()?;
            self.end_statement();
            return Ok(stmt);
        }
        if self.eat_word("if") {
            self.expect("(")?;
            let test = self.expression()?;
            self.expect(")")?;
            let then = Box::new(self.statement()?);
            let otherwise = if self.eat_word("else") {
                Some(Box::new(self.statement()?))
            } else {
                None
            };
            return Ok(Stmt::If {
                test,
                then,
                otherwise,
            });
        }
        if self.eat_word("for") {
            return self.for_loop();
        }
        if self.eat_word("while") {
            self.expect("(")?;
            let test = self.expression()?;
            self.expect(")")?;
            let body = Box::new(self.statement()?);
            return Ok(Stmt::While { test, body });
        }
        if self.eat_word("return") {
            let value = if self.is(";") || self.is("}") || self.is_end() {
                None
            } else {
                Some(self.expression()?)
            };
            self.end_statement();
            return Ok(Stmt::Return(value));
        }
        if self.eat_word("break") {
            self.end_statement();
            return Ok(Stmt::Break);
        }
        if self.eat_word("continue") {
            self.end_statement();
            return Ok(Stmt::Continue);
        }
        if self.eat_word("throw") {
            let value = self.expression()?;
            self.end_statement();
            return Ok(Stmt::Throw(value));
        }
        if self.eat_word("try") {
            let body = self.block()?;
            let mut name = None;
            let mut catch = None;
            if self.eat_word("catch") {
                if self.eat("(") {
                    name = Some(self.name()?);
                    self.expect(")")?;
                }
                catch = Some(self.block()?);
            }
            let finally = if self.eat_word("finally") {
                Some(self.block()?)
            } else {
                None
            };
            if catch.is_none() && finally.is_none() {
                return Err(self.error("expected `catch` or `finally`"));
            }
            return Ok(Stmt::Try {
                body,
                name,
                catch,
                finally,
            });
        }
        if self.is_word("async") && matches!(self.peek_at(1), Token::Name(n) if n == "function") {
            self.next();
        }
        if self.is_word("function") && matches!(self.peek_at(1), Token::Name(_)) {
            let start = self.tokens[self.at].start;
            self.next();
            let name = self.name()?;
            let params = self.params()?;
            let body = Body::Block(self.block()?);
            let source = self.source_from(start);
            return Ok(Stmt::Function {
                name,
                function: Arc::new(Function {
                    params,
                    body,
                    source,
                }),
            });
        }
        let value = self.expression()?;
        self.end_statement();
        Ok(Stmt::Expr(value))
    }

    fn declaration(&mut self) -> Result<Stmt, String> {
        let constant = matches!(self.next(), Token::Name(word) if word == "const");
        let names = self.pattern()?;
        let value = if self.eat("=") {
            Some(self.assignment()?)
        } else {
            None
        };
        if self.is(",") {
            return Err(self.error("declare one name per statement"));
        }
        Ok(Stmt::Let {
            names,
            value,
            constant,
        })
    }

    fn pattern(&mut self) -> Result<Pattern, String> {
        if self.eat("[") {
            let mut names = Vec::new();
            while !self.eat("]") {
                if self.eat(",") {
                    names.push(None);
                    continue;
                }
                names.push(Some(self.name()?));
                if !self.is("]") {
                    self.expect(",")?;
                }
            }
            return Ok(Pattern::Array(names));
        }
        if self.eat("{") {
            let mut names = Vec::new();
            while !self.eat("}") {
                let key = self.name()?;
                let local = if self.eat(":") {
                    self.name()?
                } else {
                    key.clone()
                };
                names.push((key, local));
                if !self.is("}") {
                    self.expect(",")?;
                }
            }
            return Ok(Pattern::Object(names));
        }
        Ok(Pattern::Name(self.name()?))
    }

    fn for_loop(&mut self) -> Result<Stmt, String> {
        if self.is_word("await") {
            return Err(self.error("`for await` is not supported"));
        }
        self.expect("(")?;
        let declares = self.is_word("const") || self.is_word("let") || self.is_word("var");
        if declares && matches!(self.peek_at(2), Token::Name(of) if of == "of")
            || declares && matches!(self.peek_at(1), Token::Punct("[" | "{"))
        {
            let saved = self.at;
            let constant = matches!(self.next(), Token::Name(word) if word == "const");
            let name = self.pattern()?;
            if self.eat_word("of") {
                let items = self.expression()?;
                self.expect(")")?;
                let body = Box::new(self.statement()?);
                return Ok(Stmt::ForOf {
                    name,
                    constant,
                    items,
                    body,
                });
            }
            if self.is_word("in") {
                return Err(self.error(
                    "`for in` is not supported; use `for (const key of Object.keys(...))`",
                ));
            }
            self.at = saved;
        }
        let init = if self.eat(";") {
            None
        } else {
            let init = if declares {
                self.declaration()?
            } else {
                Stmt::Expr(self.expression()?)
            };
            if self.is_word("in") {
                return Err(self.error(
                    "`for in` is not supported; use `for (const key of Object.keys(...))`",
                ));
            }
            self.expect(";")?;
            Some(Box::new(init))
        };
        let test = if self.is(";") {
            None
        } else {
            Some(self.expression()?)
        };
        self.expect(";")?;
        let update = if self.is(")") {
            None
        } else {
            Some(self.expression()?)
        };
        self.expect(")")?;
        let body = Box::new(self.statement()?);
        Ok(Stmt::For {
            init,
            test,
            update,
            body,
        })
    }

    fn expression(&mut self) -> Result<Expr, String> {
        let first = self.assignment()?;
        if self.is(",") {
            return Err(self.error("the comma operator is not supported"));
        }
        Ok(first)
    }

    fn assignment(&mut self) -> Result<Expr, String> {
        if let Some(function) = self.arrow()? {
            return Ok(function);
        }
        let target = self.conditional()?;
        for op in [
            "=", "+=", "-=", "*=", "/=", "%=", "**=", "&&=", "||=", "??=",
        ] {
            if self.is(op) {
                if !matches!(target, Expr::Name(_) | Expr::Member { .. }) {
                    return Err(self.error("this cannot be assigned to"));
                }
                self.next();
                let value = self.assignment()?;
                return Ok(Expr::Assign {
                    op,
                    target: Box::new(target),
                    value: Box::new(value),
                });
            }
        }
        Ok(target)
    }

    fn arrow(&mut self) -> Result<Option<Expr>, String> {
        let saved = self.at;
        let start = self.tokens[self.at].start;
        if self.is_word("async")
            && matches!(self.peek_at(1), Token::Name(_) | Token::Punct("("))
            && !matches!(self.peek_at(1), Token::Name(n) if n == "function")
        {
            self.next();
        }
        let params = if matches!(self.peek(), Token::Name(_))
            && matches!(self.peek_at(1), Token::Punct("=>"))
        {
            vec![Pattern::Name(self.name()?)]
        } else if self.is("(") && self.closes_into_arrow() {
            self.params()?
        } else {
            self.at = saved;
            return Ok(None);
        };
        if !self.eat("=>") {
            self.at = saved;
            return Ok(None);
        }
        let body = if self.is("{") {
            Body::Block(self.block()?)
        } else {
            Body::Expr(Box::new(self.assignment()?))
        };
        let source = self.source_from(start);
        Ok(Some(Expr::Function(Arc::new(Function {
            params,
            body,
            source,
        }))))
    }

    fn closes_into_arrow(&self) -> bool {
        let mut depth = 0;
        let mut at = self.at;
        while at < self.tokens.len() {
            match &self.tokens[at].token {
                Token::Punct("(" | "[" | "{") => depth += 1,
                Token::Punct(")" | "]" | "}") => {
                    depth -= 1;
                    if depth == 0 {
                        return matches!(
                            self.tokens.get(at + 1).map(|lexed| &lexed.token),
                            Some(Token::Punct("=>"))
                        );
                    }
                }
                Token::End => return false,
                _ => {}
            }
            at += 1;
        }
        false
    }

    fn params(&mut self) -> Result<Vec<Pattern>, String> {
        self.expect("(")?;
        let mut params = Vec::new();
        while !self.eat(")") {
            if self.is("...") {
                return Err(self.error("rest parameters are not supported"));
            }
            params.push(self.pattern()?);
            if self.is("=") {
                return Err(self.error("default parameters are not supported"));
            }
            if !self.is(")") {
                self.expect(",")?;
            }
        }
        Ok(params)
    }

    fn source_from(&self, start: usize) -> String {
        let end = self.tokens[self.at.saturating_sub(1)].end;
        self.source[start..end].to_owned()
    }

    fn conditional(&mut self) -> Result<Expr, String> {
        let test = self.binary(0)?;
        if self.eat("?") {
            let then = self.assignment()?;
            self.expect(":")?;
            let otherwise = self.assignment()?;
            return Ok(Expr::Conditional {
                test: Box::new(test),
                then: Box::new(then),
                otherwise: Box::new(otherwise),
            });
        }
        Ok(test)
    }

    fn binary(&mut self, level: usize) -> Result<Expr, String> {
        const LEVELS: [&[&str]; 9] = [
            &["??"],
            &["||"],
            &["&&"],
            &["===", "!==", "==", "!="],
            &["<", ">", "<=", ">=", "instanceof", "in"],
            &["<<", ">>", ">>>"],
            &["+", "-"],
            &["*", "/", "%"],
            &["**"],
        ];
        if level == LEVELS.len() {
            return self.unary();
        }
        let mut left = self.binary(level + 1)?;
        loop {
            let found = LEVELS[level].iter().find(|op| match self.peek() {
                Token::Punct(p) => p == *op,
                Token::Name(n) => n == *op,
                _ => false,
            });
            let Some(op) = found.copied() else {
                return Ok(left);
            };
            self.next();
            if op == "**" {
                let right = self.binary(level)?;
                return Ok(Expr::Binary {
                    op,
                    left: Box::new(left),
                    right: Box::new(right),
                });
            }
            let right = self.binary(level + 1)?;
            left = if matches!(op, "&&" | "||" | "??") {
                Expr::Logical {
                    op,
                    left: Box::new(left),
                    right: Box::new(right),
                }
            } else {
                Expr::Binary {
                    op,
                    left: Box::new(left),
                    right: Box::new(right),
                }
            };
        }
    }

    fn unary(&mut self) -> Result<Expr, String> {
        for op in ["!", "-", "+", "++", "--"] {
            if self.is(op) {
                self.next();
                let value = self.unary()?;
                if matches!(op, "++" | "--") {
                    return Ok(Expr::Update {
                        op,
                        prefix: true,
                        target: Box::new(value),
                    });
                }
                return Ok(Expr::Unary {
                    op,
                    value: Box::new(value),
                });
            }
        }
        for word in ["typeof", "void"] {
            if self.eat_word(word) {
                let value = self.unary()?;
                return Ok(Expr::Unary {
                    op: word_op(word),
                    value: Box::new(value),
                });
            }
        }
        if self.eat_word("await") {
            let value = self.unary()?;
            return Ok(Expr::Await(Box::new(value)));
        }
        if self.is_word("delete") {
            return Err(self.error("`delete` is not supported"));
        }
        let value = self.postfix()?;
        for op in ["++", "--"] {
            if self.is(op) {
                self.next();
                return Ok(Expr::Update {
                    op,
                    prefix: false,
                    target: Box::new(value),
                });
            }
        }
        Ok(value)
    }

    fn postfix(&mut self) -> Result<Expr, String> {
        let mut value = if self.eat_word("new") {
            let callee = self.primary()?;
            let callee = self.members(callee, false)?;
            let args = if self.is("(") {
                self.args()?
            } else {
                Vec::new()
            };
            Expr::New {
                callee: Box::new(callee),
                args,
            }
        } else {
            self.primary()?
        };
        value = self.members(value, true)?;
        Ok(value)
    }

    fn members(&mut self, mut value: Expr, calls: bool) -> Result<Expr, String> {
        loop {
            if self.eat(".") {
                let property = self.name()?;
                value = Expr::Member {
                    object: Box::new(value),
                    property: Box::new(Key::Name(property)),
                    optional: false,
                };
            } else if self.is("?.") {
                self.next();
                if self.is("(") && calls {
                    let args = self.args()?;
                    value = Expr::Call {
                        callee: Box::new(value),
                        args,
                        optional: true,
                    };
                } else if self.eat("[") {
                    let key = self.expression()?;
                    self.expect("]")?;
                    value = Expr::Member {
                        object: Box::new(value),
                        property: Box::new(Key::Computed(key)),
                        optional: true,
                    };
                } else {
                    let property = self.name()?;
                    value = Expr::Member {
                        object: Box::new(value),
                        property: Box::new(Key::Name(property)),
                        optional: true,
                    };
                }
            } else if self.eat("[") {
                let key = self.expression()?;
                self.expect("]")?;
                value = Expr::Member {
                    object: Box::new(value),
                    property: Box::new(Key::Computed(key)),
                    optional: false,
                };
            } else if self.is("(") && calls {
                let args = self.args()?;
                value = Expr::Call {
                    callee: Box::new(value),
                    args,
                    optional: false,
                };
            } else if matches!(self.peek(), Token::Template(_)) && calls {
                return Err(self.error("tagged templates are not supported"));
            } else {
                return Ok(value);
            }
        }
    }

    fn args(&mut self) -> Result<Vec<Item>, String> {
        self.expect("(")?;
        let mut args = Vec::new();
        while !self.eat(")") {
            if self.eat("...") {
                args.push(Item::Spread(self.assignment()?));
            } else {
                args.push(Item::One(self.assignment()?));
            }
            if !self.is(")") {
                self.expect(",")?;
            }
        }
        Ok(args)
    }

    fn primary(&mut self) -> Result<Expr, String> {
        let start = self.tokens[self.at].start;
        match self.next() {
            Token::Number(n) => Ok(Expr::Number(n)),
            Token::Str(text) => Ok(Expr::Str(text)),
            Token::Regex { source, flags } => Ok(Expr::Regex { source, flags }),
            Token::Template(pieces) => {
                let mut parts = Vec::new();
                for piece in pieces {
                    match piece {
                        Piece::Text(text) => parts.push(Expr::Str(text)),
                        Piece::Hole(hole) => {
                            let mut inner = Parser {
                                tokens: lex(&hole)?,
                                at: 0,
                                source: &hole,
                            };
                            let value = inner.expression()?;
                            if !inner.is_end() {
                                return Err(inner.error("expected the end of `${}`"));
                            }
                            parts.push(value);
                        }
                    }
                }
                Ok(Expr::Template(parts))
            }
            Token::Punct("(") => {
                let value = self.expression()?;
                self.expect(")")?;
                Ok(value)
            }
            Token::Punct("[") => {
                let mut items = Vec::new();
                while !self.eat("]") {
                    if self.eat("...") {
                        items.push(Item::Spread(self.assignment()?));
                    } else {
                        items.push(Item::One(self.assignment()?));
                    }
                    if !self.is("]") {
                        self.expect(",")?;
                    }
                }
                Ok(Expr::Array(items))
            }
            Token::Punct("{") => {
                let mut fields = Vec::new();
                while !self.eat("}") {
                    if self.eat("...") {
                        fields.push((Key::Spread, self.assignment()?));
                    } else {
                        let key = match self.next() {
                            Token::Name(name) => Key::Name(name),
                            Token::Str(text) => Key::Name(text),
                            Token::Number(n) => Key::Name(crate::script::run::number_text(n)),
                            Token::Punct("[") => {
                                let key = self.expression()?;
                                self.expect("]")?;
                                Key::Computed(key)
                            }
                            _ => {
                                self.at -= 1;
                                return Err(self.error("expected a property name"));
                            }
                        };
                        let value = if self.eat(":") {
                            self.assignment()?
                        } else if self.is("(") {
                            let params = self.params()?;
                            let body = Body::Block(self.block()?);
                            Expr::Function(Arc::new(Function {
                                params,
                                body,
                                source: self.source_from(start),
                            }))
                        } else {
                            match &key {
                                Key::Name(name) => Expr::Name(name.clone()),
                                _ => return Err(self.error("expected `:`")),
                            }
                        };
                        fields.push((key, value));
                    }
                    if !self.is("}") {
                        self.expect(",")?;
                    }
                }
                Ok(Expr::Object(fields))
            }
            Token::Name(name) => match name.as_str() {
                "true" => Ok(Expr::Bool(true)),
                "false" => Ok(Expr::Bool(false)),
                "null" => Ok(Expr::Null),
                "undefined" => Ok(Expr::Undefined),
                "function" | "async" => {
                    if name == "async" && !self.eat_word("function") {
                        self.at -= 1;
                        return Err(self.error("expected `function` or an arrow function"));
                    }
                    if matches!(self.peek(), Token::Name(_)) {
                        self.next();
                    }
                    let params = self.params()?;
                    let body = Body::Block(self.block()?);
                    Ok(Expr::Function(Arc::new(Function {
                        params,
                        body,
                        source: self.source_from(start),
                    })))
                }
                word if UNSUPPORTED.contains(&word) || word == "this" || word == "super" => {
                    self.at -= 1;
                    Err(self.error(&format!("`{word}` is not supported")))
                }
                _ => Ok(Expr::Name(name)),
            },
            _ => {
                self.at -= 1;
                Err(self.error("expected a value"))
            }
        }
    }
}

fn word_op(word: &str) -> &'static str {
    match word {
        "typeof" => "typeof",
        _ => "void",
    }
}

#[cfg(test)]
#[path = "parse.test.rs"]
mod tests;
