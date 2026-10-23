//! Tokens for the text form. Newlines are tokens (statements end at them); `//` starts a comment.

#[derive(Debug, Clone, PartialEq)]
pub enum Tok {
    /// Identifier, keyword, dotted id (`windows.list`), icon name (`list-add`) or `$payload`.
    Word(String),
    Str(String),
    Num(f64),
    Sym(char),
    Newline,
    Eof,
}

#[derive(Debug, Clone)]
pub struct Token {
    pub tok: Tok,
    pub line: usize,
    pub col: usize,
}

fn word_start(c: char) -> bool {
    c.is_ascii_alphabetic() || c == '_' || c == '$'
}

fn word_part(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | '$')
}

pub fn lex(src: &str) -> Result<Vec<Token>, (usize, usize, String)> {
    let chars: Vec<char> = src.chars().collect();
    let (mut i, mut line, mut col) = (0usize, 1usize, 1usize);
    let mut out: Vec<Token> = Vec::new();
    macro_rules! push {
        ($t:expr, $l:expr, $c:expr) => {
            out.push(Token { tok: $t, line: $l, col: $c })
        };
    }
    while i < chars.len() {
        let c = chars[i];
        let (l0, c0) = (line, col);
        match c {
            '\n' => {
                push!(Tok::Newline, l0, c0);
                i += 1;
                line += 1;
                col = 1;
            }
            ' ' | '\t' | '\r' => {
                i += 1;
                col += 1;
            }
            '/' if chars.get(i + 1) == Some(&'/') => {
                while i < chars.len() && chars[i] != '\n' {
                    i += 1;
                    col += 1;
                }
            }
            '"' => {
                let mut s = String::new();
                i += 1;
                col += 1;
                loop {
                    let Some(&ch) = chars.get(i) else { return Err((l0, c0, "unterminated string".into())) };
                    i += 1;
                    col += 1;
                    match ch {
                        '"' => break,
                        '\n' => return Err((l0, c0, "unterminated string".into())),
                        '\\' => {
                            let Some(&e) = chars.get(i) else { return Err((l0, c0, "unterminated string".into())) };
                            i += 1;
                            col += 1;
                            s.push(match e {
                                'n' => '\n',
                                't' => '\t',
                                other => other,
                            });
                        }
                        other => s.push(other),
                    }
                }
                push!(Tok::Str(s), l0, c0);
            }
            d if d.is_ascii_digit() || (d == '-' && chars.get(i + 1).is_some_and(|n| n.is_ascii_digit())) => {
                let start = i;
                i += 1;
                while i < chars.len()
                    && (chars[i].is_ascii_digit()
                        || (chars[i] == '.' && chars.get(i + 1).is_some_and(|n| n.is_ascii_digit())))
                {
                    i += 1;
                }
                let text: String = chars[start..i].iter().collect();
                col += i - start;
                match text.parse::<f64>() {
                    Ok(n) => push!(Tok::Num(n), l0, c0),
                    Err(_) => return Err((l0, c0, format!("bad number `{text}`"))),
                }
            }
            w if word_start(w) => {
                let start = i;
                i += 1;
                // A trailing '.' belongs to a modifier (`).visible`), never to a word.
                while i < chars.len()
                    && word_part(chars[i])
                    && !(chars[i] == '.' && !chars.get(i + 1).is_some_and(|n| word_start(*n)))
                {
                    i += 1;
                }
                col += i - start;
                push!(Tok::Word(chars[start..i].iter().collect()), l0, c0);
            }
            '{' | '}' | '(' | ')' | '[' | ']' | ',' | ':' | '=' | '.' => {
                push!(Tok::Sym(c), l0, c0);
                i += 1;
                col += 1;
            }
            other => return Err((l0, c0, format!("unexpected character `{other}`"))),
        }
    }
    push!(Tok::Eof, line, col);
    Ok(out)
}
