// racsh — Parser (SHELL_GRAMMAR.md §3)
//
// Recursive descent parser: tokens → AST.
// Implements the grammar from SHELL_GRAMMAR.md §3.

extern crate alloc;

use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec::Vec;

use crate::ast::*;
use crate::token::{Token, TokenKind};

/// Parser error.
#[derive(Debug, Clone)]
pub struct ParseError {
    pub message: String,
    pub line: u32,
    pub col: u32,
}

/// The racsh parser.
pub struct Parser {
    tokens: Vec<Token>,
    pos: usize,
}

impl Parser {
    pub fn new(tokens: Vec<Token>) -> Self {
        Parser { tokens, pos: 0 }
    }

    fn peek(&self) -> &TokenKind {
        self.tokens
            .get(self.pos)
            .map(|t| &t.kind)
            .unwrap_or(&TokenKind::Eof)
    }

    fn current_token(&self) -> &Token {
        &self.tokens[self.pos.min(self.tokens.len() - 1)]
    }

    fn advance(&mut self) -> &Token {
        let tok = &self.tokens[self.pos.min(self.tokens.len() - 1)];
        if self.pos < self.tokens.len() {
            self.pos += 1;
        }
        tok
    }

    fn expect(&mut self, expected: &TokenKind) -> Result<(), ParseError> {
        if self.peek() == expected {
            self.advance();
            Ok(())
        } else {
            let tok = self.current_token();
            Err(ParseError {
                message: alloc::format!("Expected {:?}, got {:?}", expected, tok.kind),
                line: tok.span.line,
                col: tok.span.col,
            })
        }
    }

    fn at_end(&self) -> bool {
        matches!(self.peek(), TokenKind::Eof)
    }

    /// Skip newlines and comments.
    fn skip_newlines(&mut self) {
        while matches!(self.peek(), TokenKind::Newline | TokenKind::Comment(_)) {
            self.advance();
        }
    }

    fn error(&self, msg: &str) -> ParseError {
        let tok = self.current_token();
        ParseError {
            message: String::from(msg),
            line: tok.span.line,
            col: tok.span.col,
        }
    }

    // ─────────────────────────────────────────────
    // Grammar entry point
    // ─────────────────────────────────────────────

    /// Parse the entire token stream into a Program AST node.
    pub fn parse(&mut self) -> Result<AstNode, ParseError> {
        self.skip_newlines();
        let mut commands = Vec::new();

        while !self.at_end() {
            let cmd = self.parse_list()?;
            commands.push(cmd);

            // Consume separator (;, &, newline)
            match self.peek() {
                TokenKind::Semi | TokenKind::Amp | TokenKind::Newline => {
                    self.advance();
                    self.skip_newlines();
                }
                TokenKind::Eof => break,
                _ => {
                    self.skip_newlines();
                    if !self.at_end() {
                        // Allow consecutive commands
                    }
                }
            }
        }

        Ok(AstNode::Program { commands })
    }

    // ─────────────────────────────────────────────
    // list = and_or (separator_op and_or)*
    // ─────────────────────────────────────────────

    fn parse_list(&mut self) -> Result<AstNode, ParseError> {
        let mut left = self.parse_and_or()?;

        loop {
            match self.peek() {
                TokenKind::Semi => {
                    self.advance();
                    self.skip_newlines();
                    if self.at_end()
                        || matches!(
                            self.peek(),
                            TokenKind::RParen
                                | TokenKind::RBrace
                                | TokenKind::Then
                                | TokenKind::Do
                                | TokenKind::Fi
                                | TokenKind::Done
                                | TokenKind::Else
                                | TokenKind::Elif
                                | TokenKind::Esac
                                | TokenKind::DSemi
                        )
                    {
                        break;
                    }
                    let right = self.parse_and_or()?;
                    left = AstNode::Sequence {
                        left: Box::new(left),
                        right: Box::new(right),
                        op: SequenceOp::Semi,
                    };
                }
                TokenKind::Amp => {
                    self.advance();
                    self.skip_newlines();
                    if self.at_end()
                        || matches!(
                            self.peek(),
                            TokenKind::RParen
                                | TokenKind::RBrace
                                | TokenKind::Then
                                | TokenKind::Do
                                | TokenKind::Fi
                                | TokenKind::Done
                                | TokenKind::Esac
                        )
                    {
                        // Trailing & — background the left
                        left = AstNode::Sequence {
                            left: Box::new(left),
                            right: Box::new(AstNode::SimpleCommand {
                                assignments: Vec::new(),
                                words: Vec::new(),
                                redirects: Vec::new(),
                            }),
                            op: SequenceOp::Background,
                        };
                        break;
                    }
                    let right = self.parse_and_or()?;
                    left = AstNode::Sequence {
                        left: Box::new(left),
                        right: Box::new(right),
                        op: SequenceOp::Background,
                    };
                }
                _ => break,
            }
        }

        Ok(left)
    }

    // ─────────────────────────────────────────────
    // and_or = pipeline (AND_IF pipeline | OR_IF pipeline)*
    // ─────────────────────────────────────────────

    fn parse_and_or(&mut self) -> Result<AstNode, ParseError> {
        let mut left = self.parse_pipeline()?;

        loop {
            match self.peek() {
                TokenKind::AndIf => {
                    self.advance();
                    self.skip_newlines();
                    let right = self.parse_pipeline()?;
                    left = AstNode::And {
                        left: Box::new(left),
                        right: Box::new(right),
                    };
                }
                TokenKind::OrIf => {
                    self.advance();
                    self.skip_newlines();
                    let right = self.parse_pipeline()?;
                    left = AstNode::Or {
                        left: Box::new(left),
                        right: Box::new(right),
                    };
                }
                _ => break,
            }
        }

        Ok(left)
    }

    // ─────────────────────────────────────────────
    // pipeline = command (PIPE command)*
    // ─────────────────────────────────────────────

    fn parse_pipeline(&mut self) -> Result<AstNode, ParseError> {
        let first = self.parse_command()?;

        if !matches!(self.peek(), TokenKind::Pipe) {
            return Ok(first);
        }

        let mut commands = alloc::vec![first];
        while matches!(self.peek(), TokenKind::Pipe) {
            self.advance();
            self.skip_newlines();
            let cmd = self.parse_command()?;
            commands.push(cmd);
        }

        Ok(AstNode::Pipeline {
            commands,
            negated: false,
        })
    }

    // ─────────────────────────────────────────────
    // command = simple_command | compound_command | function_def
    // ─────────────────────────────────────────────

    fn parse_command(&mut self) -> Result<AstNode, ParseError> {
        match self.peek() {
            TokenKind::LParen => self.parse_subshell(),
            TokenKind::LBrace => self.parse_brace_group(),
            TokenKind::If => self.parse_if(),
            TokenKind::While => self.parse_while(),
            TokenKind::For => self.parse_for(),
            TokenKind::Case => self.parse_case(),
            TokenKind::Function => self.parse_function_def(),
            // POSIX function definition: `name () compound-command`.
            TokenKind::Word(_) if self.is_posix_function_def() => self.parse_posix_function_def(),
            _ => self.parse_simple_command(),
        }
    }

    /// Lookahead for the POSIX `name ( )` function-definition prefix.
    fn is_posix_function_def(&self) -> bool {
        matches!(self.peek(), TokenKind::Word(_))
            && matches!(
                self.tokens.get(self.pos + 1).map(|t| &t.kind),
                Some(TokenKind::LParen)
            )
            && matches!(
                self.tokens.get(self.pos + 2).map(|t| &t.kind),
                Some(TokenKind::RParen)
            )
    }

    /// Parse `name () compound-command` (POSIX function definition).
    fn parse_posix_function_def(&mut self) -> Result<AstNode, ParseError> {
        let name = match self.advance().kind.clone() {
            TokenKind::Word(s) => s,
            _ => return Err(self.error("Expected function name")),
        };
        self.expect(&TokenKind::LParen)?;
        self.expect(&TokenKind::RParen)?;
        self.skip_newlines();
        let body = self.parse_command()?;
        Ok(AstNode::FunctionDef {
            name,
            body: Box::new(body),
        })
    }

    // ─────────────────────────────────────────────
    // simple_command = cmd_prefix? WORD cmd_suffix?
    // ─────────────────────────────────────────────

    fn parse_simple_command(&mut self) -> Result<AstNode, ParseError> {
        let mut assignments = Vec::new();
        let mut words = Vec::new();
        let mut redirects = Vec::new();

        // Parse prefix: assignments and redirects before the command word
        loop {
            match self.peek() {
                TokenKind::AssignmentWord { .. } => {
                    let tok = self.advance().clone();
                    if let TokenKind::AssignmentWord { name, value } = tok.kind {
                        // The value may continue into adjacent expansion tokens
                        // (e.g. `PATH=$PATH:/x` lexes as AssignmentWord{PATH,""}
                        // followed by $PATH). Build a multi-part value word.
                        let mut parts = Vec::new();
                        if !value.is_empty() {
                            parts.push(WordPart::Literal(value));
                        }
                        let end = tok.span.offset + tok.span.len;
                        self.merge_adjacent(&mut parts, end);
                        if parts.is_empty() {
                            parts.push(WordPart::Literal(String::new()));
                        }
                        assignments.push(Assignment {
                            name,
                            value: Word::from_parts(parts),
                        });
                    }
                }
                TokenKind::Less
                | TokenKind::Great
                | TokenKind::DGreat
                | TokenKind::LessAnd
                | TokenKind::GreatAnd
                | TokenKind::IoNumber(_) => {
                    redirects.push(self.parse_redirect()?);
                }
                _ => break,
            }
        }

        // Parse command word and arguments
        loop {
            match self.peek() {
                TokenKind::Word(_)
                | TokenKind::AssignmentWord { .. }
                | TokenKind::SingleQuoted(_)
                | TokenKind::DoubleQuoted(_)
                | TokenKind::DollarVar(_)
                | TokenKind::DollarParen(_)
                | TokenKind::DollarBrace(_)
                | TokenKind::Backquote(_)
                | TokenKind::GlobStar
                | TokenKind::GlobQuestion
                | TokenKind::GlobBracket(_) => {
                    words.push(self.parse_word()?);
                }
                TokenKind::Less
                | TokenKind::Great
                | TokenKind::DGreat
                | TokenKind::LessAnd
                | TokenKind::GreatAnd
                | TokenKind::IoNumber(_) => {
                    redirects.push(self.parse_redirect()?);
                }
                _ => break,
            }
        }

        if assignments.is_empty() && words.is_empty() && redirects.is_empty() {
            return Err(self.error("Expected command"));
        }

        Ok(AstNode::SimpleCommand {
            assignments,
            words,
            redirects,
        })
    }

    /// Parse a single shell word. Tokens that abut with no intervening
    /// whitespace (tracked via source spans) are concatenated into one word —
    /// so `pre${VAR}.txt`, `"a"b`, `$x$y` and `def=${VAR}` each become a single
    /// multi-part word, matching shell word-splitting semantics.
    fn parse_word(&mut self) -> Result<Word, ParseError> {
        self.parse_word_inner(false)
    }

    /// Like [`Self::parse_word`], but a reserved word is taken as a literal
    /// instead of being rejected. Only valid where the grammar cannot contain
    /// a keyword — currently `case` patterns (see [`keyword_spelling`]).
    fn parse_pattern_word(&mut self) -> Result<Word, ParseError> {
        self.parse_word_inner(true)
    }

    fn parse_word_inner(&mut self, allow_keywords: bool) -> Result<Word, ParseError> {
        let mut parts = Vec::new();

        let first = self.advance().clone();
        let word_end = first.span.offset + first.span.len;
        if !push_word_part(&first.kind, &mut parts) {
            match keyword_spelling(&first.kind) {
                Some(text) if allow_keywords => {
                    parts.push(WordPart::Literal(String::from(text)));
                }
                _ => {
                    return Err(self.error(&alloc::format!("Expected word, got {:?}", first.kind)));
                }
            }
        }

        // Greedily merge immediately-adjacent word-like tokens.
        self.merge_adjacent(&mut parts, word_end);

        Ok(Word::from_parts(parts))
    }

    /// Consume tokens that begin exactly where the previous one ended (no
    /// intervening whitespace) and append their word parts to `parts`.
    fn merge_adjacent(&mut self, parts: &mut Vec<WordPart>, mut word_end: usize) {
        loop {
            let tok = self.current_token();
            if tok.span.offset != word_end {
                break;
            }
            let mut more = Vec::new();
            if push_word_part(&tok.kind, &mut more) {
                word_end = tok.span.offset + tok.span.len;
                parts.extend(more);
                self.advance();
            } else {
                break;
            }
        }
    }

    /// Parse a redirect: [N]< | > | >> | <& | >& WORD
    fn parse_redirect(&mut self) -> Result<Redirect, ParseError> {
        let fd = if let TokenKind::IoNumber(n) = self.peek() {
            let n = *n;
            self.advance();
            Some(n as i32)
        } else {
            None
        };

        let op = match self.peek() {
            TokenKind::Less => {
                self.advance();
                RedirectOp::Input
            }
            TokenKind::Great => {
                self.advance();
                RedirectOp::Output
            }
            TokenKind::DGreat => {
                self.advance();
                RedirectOp::Append
            }
            TokenKind::LessAnd => {
                self.advance();
                RedirectOp::DupInput
            }
            TokenKind::GreatAnd => {
                self.advance();
                RedirectOp::DupOutput
            }
            _ => return Err(self.error("Expected redirect operator")),
        };

        let target = self.parse_word()?;

        Ok(Redirect { fd, op, target })
    }

    // ─────────────────────────────────────────────
    // Compound commands
    // ─────────────────────────────────────────────

    fn parse_subshell(&mut self) -> Result<AstNode, ParseError> {
        self.expect(&TokenKind::LParen)?;
        self.skip_newlines();
        let body = self.parse_list()?;
        self.skip_newlines();
        self.expect(&TokenKind::RParen)?;

        let mut redirects = Vec::new();
        while matches!(
            self.peek(),
            TokenKind::Less
                | TokenKind::Great
                | TokenKind::DGreat
                | TokenKind::LessAnd
                | TokenKind::GreatAnd
        ) {
            redirects.push(self.parse_redirect()?);
        }

        Ok(AstNode::Subshell {
            body: Box::new(body),
            redirects,
        })
    }

    fn parse_brace_group(&mut self) -> Result<AstNode, ParseError> {
        self.expect(&TokenKind::LBrace)?;
        self.skip_newlines();
        let body = self.parse_list()?;
        self.skip_newlines();
        self.expect(&TokenKind::RBrace)?;

        let mut redirects = Vec::new();
        while matches!(
            self.peek(),
            TokenKind::Less
                | TokenKind::Great
                | TokenKind::DGreat
                | TokenKind::LessAnd
                | TokenKind::GreatAnd
        ) {
            redirects.push(self.parse_redirect()?);
        }

        Ok(AstNode::BraceGroup {
            body: Box::new(body),
            redirects,
        })
    }

    fn parse_if(&mut self) -> Result<AstNode, ParseError> {
        self.expect(&TokenKind::If)?;
        self.skip_newlines();
        let condition = self.parse_list()?;
        self.skip_newlines();
        self.expect(&TokenKind::Then)?;
        self.skip_newlines();
        let then_body = self.parse_list()?;
        self.skip_newlines();

        let mut elif_parts = Vec::new();
        while matches!(self.peek(), TokenKind::Elif) {
            self.advance();
            self.skip_newlines();
            let elif_cond = self.parse_list()?;
            self.skip_newlines();
            self.expect(&TokenKind::Then)?;
            self.skip_newlines();
            let elif_body = self.parse_list()?;
            self.skip_newlines();
            elif_parts.push((Box::new(elif_cond), Box::new(elif_body)));
        }

        let else_body = if matches!(self.peek(), TokenKind::Else) {
            self.advance();
            self.skip_newlines();
            Some(Box::new(self.parse_list()?))
        } else {
            None
        };

        self.skip_newlines();
        self.expect(&TokenKind::Fi)?;

        Ok(AstNode::If {
            condition: Box::new(condition),
            then_body: Box::new(then_body),
            elif_parts,
            else_body,
        })
    }

    fn parse_while(&mut self) -> Result<AstNode, ParseError> {
        self.expect(&TokenKind::While)?;
        self.skip_newlines();
        let condition = self.parse_list()?;
        self.skip_newlines();
        self.expect(&TokenKind::Do)?;
        self.skip_newlines();
        let body = self.parse_list()?;
        self.skip_newlines();
        self.expect(&TokenKind::Done)?;

        Ok(AstNode::While {
            condition: Box::new(condition),
            body: Box::new(body),
        })
    }

    fn parse_for(&mut self) -> Result<AstNode, ParseError> {
        self.expect(&TokenKind::For)?;

        let var = match self.peek().clone() {
            TokenKind::Word(s) => {
                let s = s.clone();
                self.advance();
                s
            }
            _ => return Err(self.error("Expected variable name after 'for'")),
        };

        self.skip_newlines();

        let words = if matches!(self.peek(), TokenKind::In) {
            self.advance();
            let mut wlist = Vec::new();
            loop {
                match self.peek() {
                    TokenKind::Semi | TokenKind::Newline | TokenKind::Do => break,
                    TokenKind::Eof => break,
                    _ => wlist.push(self.parse_word()?),
                }
            }
            // consume separator
            if matches!(self.peek(), TokenKind::Semi | TokenKind::Newline) {
                self.advance();
            }
            Some(wlist)
        } else {
            None
        };

        self.skip_newlines();
        self.expect(&TokenKind::Do)?;
        self.skip_newlines();
        let body = self.parse_list()?;
        self.skip_newlines();
        self.expect(&TokenKind::Done)?;

        Ok(AstNode::For {
            var,
            words,
            body: Box::new(body),
        })
    }

    fn parse_case(&mut self) -> Result<AstNode, ParseError> {
        self.expect(&TokenKind::Case)?;
        let word = self.parse_word()?;
        self.skip_newlines();
        self.expect(&TokenKind::In)?;
        self.skip_newlines();

        let mut items = Vec::new();
        while !matches!(self.peek(), TokenKind::Esac | TokenKind::Eof) {
            // Patterns are plain words: `done`, `in`, `esac`… carry no
            // reserved meaning here, so accept keyword tokens as literals.
            let mut patterns = alloc::vec![self.parse_pattern_word()?];
            while matches!(self.peek(), TokenKind::Pipe) {
                self.advance();
                patterns.push(self.parse_pattern_word()?);
            }
            self.expect(&TokenKind::RParen)?;
            self.skip_newlines();

            let body = if matches!(self.peek(), TokenKind::DSemi | TokenKind::Esac) {
                None
            } else {
                Some(Box::new(self.parse_list()?))
            };

            if matches!(self.peek(), TokenKind::DSemi) {
                self.advance();
            }
            self.skip_newlines();

            items.push(CaseItem { patterns, body });
        }

        self.expect(&TokenKind::Esac)?;

        Ok(AstNode::Case { word, items })
    }

    fn parse_function_def(&mut self) -> Result<AstNode, ParseError> {
        self.expect(&TokenKind::Function)?;

        let name = match self.peek().clone() {
            TokenKind::Word(s) => {
                let s = s.clone();
                self.advance();
                s
            }
            _ => return Err(self.error("Expected function name")),
        };

        // Optional ()
        if matches!(self.peek(), TokenKind::LParen) {
            self.advance();
            self.expect(&TokenKind::RParen)?;
        }

        self.skip_newlines();
        let body = self.parse_command()?;

        Ok(AstNode::FunctionDef {
            name,
            body: Box::new(body),
        })
    }
}

/// Append a token's word part(s) to `parts`. Returns `false` if the token is
/// not word-like (so callers know to stop merging). An `AssignmentWord` in word
/// position (i.e. after the command name) is a plain `name=value` literal.
/// Spelling of a reserved-word token, or `None` if `kind` isn't a keyword.
///
/// POSIX only recognises reserved words in command-word position. Everywhere
/// else — notably a `case` pattern — they are ordinary words, so
/// `case $x in done) ...; esac` is legal and must not be parsed as the `done`
/// that closes a loop. The lexer classifies keywords unconditionally, so the
/// parser maps them back to literals where a plain word is what's expected.
fn keyword_spelling(kind: &TokenKind) -> Option<&'static str> {
    match kind {
        TokenKind::If => Some("if"),
        TokenKind::Then => Some("then"),
        TokenKind::Else => Some("else"),
        TokenKind::Elif => Some("elif"),
        TokenKind::Fi => Some("fi"),
        TokenKind::While => Some("while"),
        TokenKind::Do => Some("do"),
        TokenKind::Done => Some("done"),
        TokenKind::For => Some("for"),
        TokenKind::In => Some("in"),
        TokenKind::Case => Some("case"),
        TokenKind::Esac => Some("esac"),
        TokenKind::Function => Some("function"),
        _ => None,
    }
}

fn push_word_part(kind: &TokenKind, parts: &mut Vec<WordPart>) -> bool {
    match kind {
        TokenKind::Word(s) => parts.push(WordPart::Literal(s.clone())),
        TokenKind::AssignmentWord { name, value } => {
            let mut lit = name.clone();
            lit.push('=');
            lit.push_str(value);
            parts.push(WordPart::Literal(lit));
        }
        TokenKind::SingleQuoted(s) => parts.push(WordPart::SingleQuoted(s.clone())),
        TokenKind::DoubleQuoted(s) => parts.push(WordPart::DoubleQuoted(s.clone())),
        TokenKind::DollarVar(s) => parts.push(WordPart::Variable(s.clone())),
        TokenKind::DollarParen(s) => parts.push(WordPart::CommandSub(s.clone())),
        TokenKind::DollarBrace(s) => parts.push(WordPart::BraceExpansion(s.clone())),
        TokenKind::Backquote(s) => parts.push(WordPart::CommandSub(s.clone())),
        TokenKind::GlobStar => parts.push(WordPart::Glob(GlobPattern::Star)),
        TokenKind::GlobQuestion => parts.push(WordPart::Glob(GlobPattern::Question)),
        TokenKind::GlobBracket(s) => parts.push(WordPart::Glob(GlobPattern::Bracket(s.clone()))),
        _ => return false,
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::{Word, WordPart};
    use crate::lexer::Lexer;

    fn parse_str(input: &str) -> Result<AstNode, ParseError> {
        let mut lexer = Lexer::new(input);
        let tokens = lexer.tokenize().unwrap();
        let mut parser = Parser::new(tokens);
        parser.parse()
    }

    fn literal(word: &Word) -> &str {
        match word.parts.as_slice() {
            [WordPart::Literal(value)] => value.as_str(),
            other => panic!("expected literal word, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_simple_command() {
        let ast = parse_str("ls -l").unwrap();
        match ast {
            AstNode::Program { commands } => {
                assert_eq!(commands.len(), 1);
                match &commands[0] {
                    AstNode::SimpleCommand { words, .. } => {
                        assert_eq!(words.len(), 2);
                        assert_eq!(literal(&words[0]), "ls");
                        assert_eq!(literal(&words[1]), "-l");
                    }
                    _ => panic!("Expected SimpleCommand"),
                }
            }
            _ => panic!("Expected Program"),
        }
    }

    #[test]
    fn test_parse_command_with_args() {
        let ast = parse_str("echo hello world").unwrap();
        match ast {
            AstNode::Program { commands } => {
                assert_eq!(commands.len(), 1);
                match &commands[0] {
                    AstNode::SimpleCommand { words, .. } => {
                        assert_eq!(words.len(), 3);
                        assert_eq!(literal(&words[0]), "echo");
                        assert_eq!(literal(&words[1]), "hello");
                        assert_eq!(literal(&words[2]), "world");
                    }
                    _ => panic!("Expected SimpleCommand"),
                }
            }
            _ => panic!("Expected Program"),
        }
    }

    #[test]
    fn test_parse_sequence() {
        let ast = parse_str("ls ; pwd").unwrap();
        match ast {
            AstNode::Program { commands } => {
                assert_eq!(commands.len(), 1);
                match &commands[0] {
                    AstNode::Sequence { left, right, op } => {
                        assert_eq!(*op, SequenceOp::Semi);
                        // Check left command
                        match &**left {
                            AstNode::SimpleCommand { words, .. } => {
                                assert_eq!(literal(&words[0]), "ls");
                            }
                            _ => panic!("Expected SimpleCommand in left"),
                        }
                        // Check right command
                        match &**right {
                            AstNode::SimpleCommand { words, .. } => {
                                assert_eq!(literal(&words[0]), "pwd");
                            }
                            _ => panic!("Expected SimpleCommand in right"),
                        }
                    }
                    _ => panic!("Expected Sequence"),
                }
            }
            _ => panic!("Expected Program"),
        }
    }
}
