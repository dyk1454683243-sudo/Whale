use crate::error::AsmError;

#[derive(Debug, Clone, PartialEq)]
pub enum TokenKind {
    Identifier(String),
    Number(i64),
    StringLiteral(String),
    Comma,
    Colon,
    LBracket,
    RBracket,
    Plus,
    Minus,
    Multiply,
    Divide,
    Newline,
}

#[derive(Debug, Clone)]
pub struct Token {
    pub kind: TokenKind,
    pub position: usize,
    pub line: usize,
    pub column: usize,
}

fn lexer_err<S: Into<String>>(msg: S, line: usize, column: usize) -> AsmError {
    AsmError::LexerError(format!("{} at line {}, column {}", msg.into(), line, column))
}

pub fn tokenize(src: &str) -> Result<Vec<Token>, AsmError> {
    let mut chars = src.chars().peekable();
    let mut tokens = Vec::new();

    let mut pos = 0usize;
    let mut line = 1usize;
    let mut column = 1usize;

    while let Some(&ch) = chars.peek() {
        match ch {
            ';' => {
                while let Some(&c) = chars.peek() {
                    if c == '\n' {
                        break;
                    }
                    chars.next();
                    pos += 1;
                    column += 1;
                }
            }

            ' ' | '\t' | '\r' => {
                chars.next();
                pos += 1;
                column += 1;
            }

            '\n' => {
                tokens.push(Token {
                    kind: TokenKind::Newline,
                    position: pos,
                    line,
                    column,
                });
                chars.next();
                pos += 1;
                line += 1;
                column = 1;
            }

            ',' => {
                tokens.push(Token {
                    kind: TokenKind::Comma,
                    position: pos,
                    line,
                    column,
                });
                chars.next();
                pos += 1;
                column += 1;
            }

            ':' => {
                tokens.push(Token {
                    kind: TokenKind::Colon,
                    position: pos,
                    line,
                    column,
                });
                chars.next();
                pos += 1;
                column += 1;
            }

            '[' => {
                tokens.push(Token {
                    kind: TokenKind::LBracket,
                    position: pos,
                    line,
                    column,
                });
                chars.next();
                pos += 1;
                column += 1;
            }

            ']' => {
                tokens.push(Token {
                    kind: TokenKind::RBracket,
                    position: pos,
                    line,
                    column,
                });
                chars.next();
                pos += 1;
                column += 1;
            }

            '+' => {
                tokens.push(Token {
                    kind: TokenKind::Plus,
                    position: pos,
                    line,
                    column,
                });
                chars.next();
                pos += 1;
                column += 1;
            }

            '-' => {
                tokens.push(Token {
                    kind: TokenKind::Minus,
                    position: pos,
                    line,
                    column,
                });
                chars.next();
                pos += 1;
                column += 1;
            }

            '*' => {
                tokens.push(Token {
                    kind: TokenKind::Multiply,
                    position: pos,
                    line,
                    column,
                });
                chars.next();
                pos += 1;
                column += 1;
            }

            '"' => {
                let start_pos = pos;
                let start_line = line;
                let start_col = column;

                chars.next();
                pos += 1;
                column += 1;

                let mut s = String::new();
                let mut closed = false;
                while let Some(c) = chars.next() {
                    pos += 1;
                    if c == '"' {
                        column += 1;
                        closed = true;
                        break;
                    }
                    if c == '\n' {
                        line += 1;
                        column = 1;
                    } else {
                        column += 1;
                    }
                    s.push(c);
                }
                if !closed {
                    return Err(lexer_err("Unterminated string literal", start_line, start_col));
                }

                tokens.push(Token {
                    kind: TokenKind::StringLiteral(s),
                    position: start_pos,
                    line: start_line,
                    column: start_col,
                });
            }

            c if c.is_ascii_digit() => {
                let start_pos = pos;
                let start_line = line;
                let start_col = column;
                let mut n = String::new();

                if ch == '0' {
                    let mut look = chars.clone();
                    look.next();
                    if let Some(radix_mark) = look.next() {
                        if radix_mark == 'x' || radix_mark == 'X' {
                            chars.next();
                            chars.next();
                            pos += 2;
                            column += 2;

                            while let Some(&d) = chars.peek() {
                                if d.is_ascii_hexdigit() {
                                    n.push(d);
                                    chars.next();
                                    pos += 1;
                                    column += 1;
                                } else {
                                    break;
                                }
                            }
                            if n.is_empty() {
                                return Err(lexer_err("Invalid hex number", start_line, start_col));
                            }
                            let parsed = i64::from_str_radix(&n, 16).map_err(|_| {
                                lexer_err("Hex number out of range", start_line, start_col)
                            })?;
                            tokens.push(Token {
                                kind: TokenKind::Number(parsed),
                                position: start_pos,
                                line: start_line,
                                column: start_col,
                            });
                            continue;
                        } else if radix_mark == 'b' || radix_mark == 'B' {
                            chars.next();
                            chars.next();
                            pos += 2;
                            column += 2;

                            while let Some(&d) = chars.peek() {
                                if d == '0' || d == '1' {
                                    n.push(d);
                                    chars.next();
                                    pos += 1;
                                    column += 1;
                                } else {
                                    break;
                                }
                            }
                            if n.is_empty() {
                                return Err(lexer_err("Invalid binary number", start_line, start_col));
                            }
                            let parsed = i64::from_str_radix(&n, 2).map_err(|_| {
                                lexer_err("Binary number out of range", start_line, start_col)
                            })?;
                            tokens.push(Token {
                                kind: TokenKind::Number(parsed),
                                position: start_pos,
                                line: start_line,
                                column: start_col,
                            });
                            continue;
                        }
                    }
                }

                while let Some(&d) = chars.peek() {
                    if d.is_ascii_digit() {
                        n.push(d);
                        chars.next();
                        pos += 1;
                        column += 1;
                    } else {
                        break;
                    }
                }
                let parsed = n
                    .parse::<i64>()
                    .map_err(|_| lexer_err("Decimal number out of range", start_line, start_col))?;
                tokens.push(Token {
                    kind: TokenKind::Number(parsed),
                    position: start_pos,
                    line: start_line,
                    column: start_col,
                });
            }

            c if c.is_alphanumeric() || c == '_' || c == '.' => {
                let start_pos = pos;
                let start_line = line;
                let start_col = column;
                let mut id = String::new();

                while let Some(&d) = chars.peek() {
                    if d.is_alphanumeric() || d == '_' || d == '.' {
                        id.push(d);
                        chars.next();
                        pos += 1;
                        column += 1;
                    } else {
                        break;
                    }
                }

                tokens.push(Token {
                    kind: TokenKind::Identifier(id),
                    position: start_pos,
                    line: start_line,
                    column: start_col,
                });
            }

            _ => {
                return Err(lexer_err(
                    format!("Unexpected char '{}'", ch),
                    line,
                    column,
                ))
            }
        }
    }

    Ok(tokens)
}
