// racsh — Lexer (SHELL_GRAMMAR.md §2)
//
// Converts a source string into a stream of tokens.
// Handles quoting, escape sequences, comments, and operator recognition.

extern crate alloc;

use alloc::string::String;
use alloc::vec::Vec;

use crate::token::{Span, Token, TokenKind};

/// Lexer error.
#[derive(Debug, Clone)]
pub struct LexerError {
    pub message: String,
    pub line: u32,
    pub col: u32,
}

/// The racsh lexer.
pub struct Lexer<'a> {
    src: &'a [u8],
    pos: usize,
    line: u32,
    col: u32,
}

impl<'a> Lexer<'a> {
    pub fn new(source: &'a str) -> Self {
        Lexer {
            src: source.as_bytes(),
            pos: 0,
            line: 1,
            col: 1,
        }
    }

    fn peek(&self) -> Option<u8> {
        self.src.get(self.pos).copied()
    }

    fn peek_at(&self, offset: usize) -> Option<u8> {
        self.src.get(self.pos + offset).copied()
    }

    fn advance(&mut self) -> Option<u8> {
        let ch = self.src.get(self.pos).copied()?;
        self.pos += 1;
        if ch == b'\n' {
            self.line += 1;
            self.col = 1;
        } else {
            self.col += 1;
        }
        Some(ch)
    }

    fn span_from(&self, start_offset: usize, start_line: u32, start_col: u32) -> Span {
        Span {
            line: start_line,
            col: start_col,
            offset: start_offset,
            len: self.pos - start_offset,
        }
    }

    fn skip_whitespace(&mut self) {
        while let Some(ch) = self.peek() {
            if ch == b' ' || ch == b'\t' || ch == b'\r' {
                self.advance();
            } else {
                break;
            }
        }
    }

    /// Tokenize the entire input.
    pub fn tokenize(&mut self) -> Result<Vec<Token>, LexerError> {
        let mut tokens = Vec::new();

        loop {
            self.skip_whitespace();

            let start = self.pos;
            let start_line = self.line;
            let start_col = self.col;

            let ch = match self.peek() {
                Some(ch) => ch,
                None => {
                    tokens.push(Token {
                        kind: TokenKind::Eof,
                        span: self.span_from(start, start_line, start_col),
                    });
                    break;
                }
            };

            let kind = match ch {
                b'\n' => {
                    self.advance();
                    TokenKind::Newline
                }

                b'#' => {
                    // Comment — consume until newline
                    self.advance();
                    let mut text = String::new();
                    while let Some(c) = self.peek() {
                        if c == b'\n' {
                            break;
                        }
                        text.push(self.advance().unwrap() as char);
                    }
                    TokenKind::Comment(text)
                }

                b'|' => {
                    self.advance();
                    if self.peek() == Some(b'|') {
                        self.advance();
                        TokenKind::OrIf
                    } else {
                        TokenKind::Pipe
                    }
                }

                b'&' => {
                    self.advance();
                    if self.peek() == Some(b'&') {
                        self.advance();
                        TokenKind::AndIf
                    } else {
                        TokenKind::Amp
                    }
                }

                b';' => {
                    self.advance();
                    if self.peek() == Some(b';') {
                        self.advance();
                        TokenKind::DSemi
                    } else {
                        TokenKind::Semi
                    }
                }

                b'(' => {
                    self.advance();
                    TokenKind::LParen
                }
                b')' => {
                    self.advance();
                    TokenKind::RParen
                }

                b'<' => {
                    self.advance();
                    if self.peek() == Some(b'&') {
                        self.advance();
                        TokenKind::LessAnd
                    } else {
                        TokenKind::Less
                    }
                }

                b'>' => {
                    self.advance();
                    if self.peek() == Some(b'>') {
                        self.advance();
                        TokenKind::DGreat
                    } else if self.peek() == Some(b'&') {
                        self.advance();
                        TokenKind::GreatAnd
                    } else {
                        TokenKind::Great
                    }
                }

                b'\'' => self.lex_single_quoted()?,

                b'"' => self.lex_double_quoted()?,

                b'$' => self.lex_dollar()?,

                b'`' => self.lex_backquote()?,

                b'\\' => {
                    // Line continuation or escaped character
                    self.advance();
                    if self.peek() == Some(b'\n') {
                        self.advance();
                        // Line continuation — skip and continue
                        continue;
                    }
                    // Escaped character — becomes part of a word
                    let escaped = self.advance().unwrap_or(b'\\') as char;
                    let mut word = String::new();
                    word.push(escaped);
                    self.lex_word_rest(&mut word);
                    self.classify_word(word)
                }

                _ => {
                    // Word (including paths, commands, etc.)
                    let mut word = String::new();
                    word.push(self.advance().unwrap() as char);
                    self.lex_word_rest(&mut word);
                    self.classify_word(word)
                }
            };

            tokens.push(Token {
                kind,
                span: self.span_from(start, start_line, start_col),
            });
        }

        Ok(tokens)
    }

    /// Continue lexing a word (unquoted).
    fn lex_word_rest(&mut self, word: &mut String) {
        loop {
            match self.peek() {
                None => break,
                Some(c) => {
                    if c == b' '
                        || c == b'\t'
                        || c == b'\n'
                        || c == b'\r'
                        || c == b'|'
                        || c == b'&'
                        || c == b';'
                        || c == b'('
                        || c == b')'
                        || c == b'<'
                        || c == b'>'
                        || c == b'#'
                    {
                        break;
                    }
                    if c == b'\\' {
                        self.advance();
                        if let Some(n) = self.advance() {
                            word.push(n as char);
                        }
                    } else if c == b'\'' {
                        // Inline single-quote in word
                        if let TokenKind::SingleQuoted(s) = self.lex_single_quoted().unwrap_or(TokenKind::Word(String::new())) {
                            word.push_str(&s);
                        }
                    } else if c == b'"' {
                        // Inline double-quote in word
                        if let TokenKind::DoubleQuoted(s) = self.lex_double_quoted().unwrap_or(TokenKind::Word(String::new())) {
                            word.push_str(&s);
                        }
                    } else {
                        word.push(self.advance().unwrap() as char);
                    }
                }
            }
        }
    }

    /// Classify a word: keyword, assignment, or plain word.
    fn classify_word(&self, word: String) -> TokenKind {
        // Check for assignment word: NAME=VALUE
        if let Some(eq_pos) = word.find('=') {
            if eq_pos > 0 {
                let name = &word[..eq_pos];
                // Name must start with letter or underscore
                let first = name.as_bytes()[0];
                if (first.is_ascii_alphabetic() || first == b'_')
                    && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
                {
                    return TokenKind::AssignmentWord {
                        name: String::from(name),
                        value: String::from(&word[eq_pos + 1..]),
                    };
                }
            }
        }

        // Check for keywords
        if let Some(kw) = TokenKind::keyword_from_str(&word) {
            return kw;
        }

        // Check for { and }
        if word == "{" {
            return TokenKind::LBrace;
        }
        if word == "}" {
            return TokenKind::RBrace;
        }

        TokenKind::Word(word)
    }

    fn lex_single_quoted(&mut self) -> Result<TokenKind, LexerError> {
        self.advance(); // consume opening '
        let mut text = String::new();
        loop {
            match self.advance() {
                Some(b'\'') => return Ok(TokenKind::SingleQuoted(text)),
                Some(c) => text.push(c as char),
                None => {
                    return Err(LexerError {
                        message: String::from("Unterminated single quote"),
                        line: self.line,
                        col: self.col,
                    })
                }
            }
        }
    }

    fn lex_double_quoted(&mut self) -> Result<TokenKind, LexerError> {
        self.advance(); // consume opening "
        let mut text = String::new();
        loop {
            match self.advance() {
                Some(b'"') => return Ok(TokenKind::DoubleQuoted(text)),
                Some(b'\\') => {
                    // In double quotes, only \\ \" \$ \` \newline are special
                    match self.peek() {
                        Some(b'\\') | Some(b'"') | Some(b'$') | Some(b'`') => {
                            text.push(self.advance().unwrap() as char);
                        }
                        Some(b'\n') => {
                            self.advance(); // line continuation
                        }
                        _ => {
                            text.push('\\');
                        }
                    }
                }
                Some(c) => text.push(c as char),
                None => {
                    return Err(LexerError {
                        message: String::from("Unterminated double quote"),
                        line: self.line,
                        col: self.col,
                    })
                }
            }
        }
    }

    fn lex_dollar(&mut self) -> Result<TokenKind, LexerError> {
        self.advance(); // consume $

        match self.peek() {
            Some(b'(') => {
                self.advance(); // consume (
                let mut text = String::new();
                let mut depth = 1;
                while depth > 0 {
                    match self.advance() {
                        Some(b'(') => {
                            depth += 1;
                            text.push('(');
                        }
                        Some(b')') => {
                            depth -= 1;
                            if depth > 0 {
                                text.push(')');
                            }
                        }
                        Some(c) => text.push(c as char),
                        None => {
                            return Err(LexerError {
                                message: String::from("Unterminated $()"),
                                line: self.line,
                                col: self.col,
                            })
                        }
                    }
                }
                Ok(TokenKind::DollarParen(text))
            }
            Some(b'{') => {
                self.advance(); // consume {
                let mut text = String::new();
                let mut depth = 1;
                while depth > 0 {
                    match self.advance() {
                        Some(b'{') => {
                            depth += 1;
                            text.push('{');
                        }
                        Some(b'}') => {
                            depth -= 1;
                            if depth > 0 {
                                text.push('}');
                            }
                        }
                        Some(c) => text.push(c as char),
                        None => {
                            return Err(LexerError {
                                message: String::from("Unterminated ${}"),
                                line: self.line,
                                col: self.col,
                            })
                        }
                    }
                }
                Ok(TokenKind::DollarBrace(text))
            }
            Some(ch) if ch.is_ascii_alphabetic() || ch == b'_' => {
                let mut name = String::new();
                while let Some(c) = self.peek() {
                    if c.is_ascii_alphanumeric() || c == b'_' {
                        name.push(self.advance().unwrap() as char);
                    } else {
                        break;
                    }
                }
                Ok(TokenKind::DollarVar(name))
            }
            Some(ch) if ch == b'?' || ch == b'$' || ch == b'!' || ch == b'#'
                || ch == b'0' || ch == b'@' || ch == b'*' =>
            {
                let mut name = String::new();
                name.push(self.advance().unwrap() as char);
                Ok(TokenKind::DollarVar(name))
            }
            _ => {
                // Lone $ — treat as literal word character
                let mut word = String::from("$");
                self.lex_word_rest(&mut word);
                Ok(self.classify_word(word))
            }
        }
    }

    fn lex_backquote(&mut self) -> Result<TokenKind, LexerError> {
        self.advance(); // consume `
        let mut text = String::new();
        loop {
            match self.advance() {
                Some(b'`') => return Ok(TokenKind::Backquote(text)),
                Some(b'\\') => {
                    if let Some(c) = self.advance() {
                        text.push(c as char);
                    }
                }
                Some(c) => text.push(c as char),
                None => {
                    return Err(LexerError {
                        message: String::from("Unterminated backquote"),
                        line: self.line,
                        col: self.col,
                    })
                }
            }
        }
    }
}
