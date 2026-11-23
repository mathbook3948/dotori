//! Splits script text into tokens. A `/` starts a regular expression only where a value can
//! begin, the way JavaScript decides.

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Token {
    Number(f64),
    Str(String),
    Template(Vec<Piece>),
    Regex { source: String, flags: String },
    Name(String),
    Punct(&'static str),
    End,
}

/// One part of a template literal: text as written, or the source of a `${...}` hole.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Piece {
    Text(String),
    Hole(String),
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Lexed {
    pub(crate) token: Token,
    pub(crate) start: usize,
    pub(crate) end: usize,
    pub(crate) line: usize,
}

const PUNCTS: &[&str] = &[
    ">>>=", "...", "===", "!==", "**=", "<<=", ">>=", ">>>", "&&=", "||=", "??=", "=>", "==", "!=",
    "<=", ">=", "&&", "||", "??", "?.", "++", "--", "+=", "-=", "*=", "/=", "%=", "**", "<<", ">>",
    "{", "}", "(", ")", "[", "]", ";", ",", "<", ">", "+", "-", "*", "/", "%", "!", "?", ":", "=",
    ".", "&", "|",
];

pub(crate) fn lex(source: &str) -> Result<Vec<Lexed>, String> {
    let chars: Vec<(usize, char)> = source.char_indices().collect();
    let mut tokens: Vec<Lexed> = Vec::new();
    let mut i = 0;
    let mut line = 1;
    let at = |i: usize| chars.get(i).map(|(_, c)| *c);
    let offset = |i: usize| chars.get(i).map_or(source.len(), |(at, _)| *at);
    while i < chars.len() {
        let c = chars[i].1;
        if c == '\n' {
            line += 1;
            i += 1;
            continue;
        }
        if c.is_whitespace() {
            i += 1;
            continue;
        }
        if c == '/' && at(i + 1) == Some('/') {
            while i < chars.len() && chars[i].1 != '\n' {
                i += 1;
            }
            continue;
        }
        if c == '/' && at(i + 1) == Some('*') {
            i += 2;
            while i < chars.len() && !(chars[i].1 == '*' && at(i + 1) == Some('/')) {
                if chars[i].1 == '\n' {
                    line += 1;
                }
                i += 1;
            }
            i += 2;
            continue;
        }
        let start = i;
        let token = if c.is_ascii_digit()
            || (c == '.' && at(i + 1).is_some_and(|d| d.is_ascii_digit()))
        {
            let mut text = String::new();
            while let Some(d) =
                at(i).filter(|d| d.is_ascii_alphanumeric() || *d == '.' || *d == '_')
            {
                if d != '_' {
                    text.push(d);
                }
                i += 1;
            }
            Token::Number(number(&text).ok_or_else(|| format!("line {line}: bad number {text}"))?)
        } else if c == '"' || c == '\'' {
            i += 1;
            let mut text = String::new();
            loop {
                match at(i) {
                    None | Some('\n') => return Err(format!("line {line}: unclosed string")),
                    Some(q) if q == c => {
                        i += 1;
                        break;
                    }
                    Some('\\') => {
                        let (escaped, used) = escape(&chars, i + 1)
                            .ok_or_else(|| format!("line {line}: bad escape"))?;
                        text.push_str(&escaped);
                        i += 1 + used;
                    }
                    Some(other) => {
                        text.push(other);
                        i += 1;
                    }
                }
            }
            Token::Str(text)
        } else if c == '`' {
            i += 1;
            let mut pieces = Vec::new();
            let mut text = String::new();
            loop {
                match at(i) {
                    None => return Err(format!("line {line}: unclosed template")),
                    Some('`') => {
                        i += 1;
                        break;
                    }
                    Some('\\') => {
                        let (escaped, used) = escape(&chars, i + 1)
                            .ok_or_else(|| format!("line {line}: bad escape"))?;
                        text.push_str(&escaped);
                        i += 1 + used;
                    }
                    Some('$') if at(i + 1) == Some('{') => {
                        pieces.push(Piece::Text(std::mem::take(&mut text)));
                        i += 2;
                        let hole_start = i;
                        let mut depth = 1;
                        while depth > 0 {
                            match at(i) {
                                None => {
                                    return Err(format!("line {line}: unclosed ${{ in template"));
                                }
                                Some('{') => depth += 1,
                                Some('}') => depth -= 1,
                                Some('\n') => line += 1,
                                _ => {}
                            }
                            i += 1;
                        }
                        pieces.push(Piece::Hole(
                            source[offset(hole_start)..offset(i - 1)].to_owned(),
                        ));
                    }
                    Some(other) => {
                        if other == '\n' {
                            line += 1;
                        }
                        text.push(other);
                        i += 1;
                    }
                }
            }
            pieces.push(Piece::Text(text));
            Token::Template(pieces)
        } else if c == '/' && value_can_start(tokens.last().map(|lexed| &lexed.token)) {
            i += 1;
            let mut body = String::new();
            let mut class = false;
            loop {
                match at(i) {
                    None | Some('\n') => {
                        return Err(format!("line {line}: unclosed regular expression"));
                    }
                    Some('/') if !class => {
                        i += 1;
                        break;
                    }
                    Some('\\') => {
                        body.push('\\');
                        if let Some(next) = at(i + 1) {
                            body.push(next);
                        }
                        i += 2;
                    }
                    Some(other) => {
                        if other == '[' {
                            class = true;
                        } else if other == ']' {
                            class = false;
                        }
                        body.push(other);
                        i += 1;
                    }
                }
            }
            let mut flags = String::new();
            while let Some(f) = at(i).filter(char::is_ascii_alphabetic) {
                flags.push(f);
                i += 1;
            }
            Token::Regex {
                source: body,
                flags,
            }
        } else if c.is_alphabetic() || c == '_' || c == '$' {
            let mut name = String::new();
            while let Some(n) = at(i).filter(|n| n.is_alphanumeric() || *n == '_' || *n == '$') {
                name.push(n);
                i += 1;
            }
            Token::Name(name)
        } else {
            let rest = &source[offset(i)..];
            let punct = PUNCTS
                .iter()
                .find(|punct| rest.starts_with(**punct))
                .ok_or_else(|| format!("line {line}: unexpected {c:?}"))?;
            i += punct.chars().count();
            Token::Punct(punct)
        };
        tokens.push(Lexed {
            token,
            start: offset(start),
            end: offset(i),
            line,
        });
    }
    tokens.push(Lexed {
        token: Token::End,
        start: source.len(),
        end: source.len(),
        line,
    });
    Ok(tokens)
}

fn number(text: &str) -> Option<f64> {
    let lower = text.to_ascii_lowercase();
    if let Some(hex) = lower.strip_prefix("0x") {
        return u64::from_str_radix(hex, 16).ok().map(|n| n as f64);
    }
    lower.parse().ok()
}

fn escape(chars: &[(usize, char)], i: usize) -> Option<(String, usize)> {
    let c = chars.get(i)?.1;
    Some(match c {
        'n' => (String::from("\n"), 1),
        't' => (String::from("\t"), 1),
        'r' => (String::from("\r"), 1),
        '0' => (String::from("\0"), 1),
        'u' if chars.get(i + 1).map(|(_, c)| *c) == Some('{') => {
            let digits: String = chars[i + 2..]
                .iter()
                .map(|(_, c)| *c)
                .take_while(|c| *c != '}')
                .collect();
            let code = u32::from_str_radix(&digits, 16).ok()?;
            (char::from_u32(code)?.to_string(), digits.len() + 3)
        }
        'u' => {
            let digits: String = chars.get(i + 1..i + 5)?.iter().map(|(_, c)| *c).collect();
            let code = u32::from_str_radix(&digits, 16).ok()?;
            (char::from_u32(code)?.to_string(), 5)
        }
        'x' => {
            let digits: String = chars.get(i + 1..i + 3)?.iter().map(|(_, c)| *c).collect();
            let code = u32::from_str_radix(&digits, 16).ok()?;
            (char::from_u32(code)?.to_string(), 3)
        }
        '\n' => (String::new(), 1),
        other => (other.to_string(), 1),
    })
}

fn value_can_start(previous: Option<&Token>) -> bool {
    match previous {
        None => true,
        Some(Token::Punct(punct)) => !matches!(*punct, ")" | "]" | "}" | "++" | "--"),
        Some(Token::Name(name)) => matches!(
            name.as_str(),
            "return" | "typeof" | "await" | "case" | "in" | "of" | "throw" | "void" | "delete"
        ),
        Some(_) => false,
    }
}

#[cfg(test)]
#[path = "lex.test.rs"]
mod tests;
