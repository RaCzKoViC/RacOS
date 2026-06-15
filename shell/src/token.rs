// racsh — Token types (SHELL_GRAMMAR.md §2)

extern crate alloc;
use alloc::string::String;

/// Source location for error reporting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span {
    pub line: u32,
    pub col: u32,
    pub offset: usize,
    pub len: usize,
}

/// Token with type and source location.
#[derive(Debug, Clone, PartialEq)]
pub struct Token {
    pub kind: TokenKind,
    pub span: Span,
}

/// All token types in racsh.
#[derive(Debug, Clone, PartialEq)]
pub enum TokenKind {
    // Literals
    Word(String),
    AssignmentWord { name: String, value: String },

    // Operators
    Pipe,  // |
    AndIf, // &&
    OrIf,  // ||
    Semi,  // ;
    Amp,   // &
    Newline,

    // Redirections
    Less,     // <
    Great,    // >
    DGreat,   // >>
    LessAnd,  // <&
    GreatAnd, // >&
    IoNumber(u32),

    // Grouping
    LParen, // (
    RParen, // )
    LBrace, // {
    RBrace, // }

    // Keywords
    If,
    Then,
    Else,
    Elif,
    Fi,
    While,
    Do,
    Done,
    For,
    In,
    Case,
    Esac,
    DSemi, // ;;
    Function,

    // Quoting
    SingleQuoted(String),
    DoubleQuoted(String),
    Backquote(String),

    // Expansions
    DollarParen(String),
    DollarBrace(String),
    DollarVar(String),

    // Glob patterns (as part of words)
    GlobStar,
    GlobQuestion,
    GlobBracket(String),

    // Special
    Comment(String),
    Eof,
}

impl TokenKind {
    /// Check if this token is a keyword.
    pub fn is_keyword(&self) -> bool {
        matches!(
            self,
            TokenKind::If
                | TokenKind::Then
                | TokenKind::Else
                | TokenKind::Elif
                | TokenKind::Fi
                | TokenKind::While
                | TokenKind::Do
                | TokenKind::Done
                | TokenKind::For
                | TokenKind::In
                | TokenKind::Case
                | TokenKind::Esac
                | TokenKind::Function
        )
    }

    /// Try to convert a word into a keyword token.
    pub fn keyword_from_str(s: &str) -> Option<TokenKind> {
        match s {
            "if" => Some(TokenKind::If),
            "then" => Some(TokenKind::Then),
            "else" => Some(TokenKind::Else),
            "elif" => Some(TokenKind::Elif),
            "fi" => Some(TokenKind::Fi),
            "while" => Some(TokenKind::While),
            "do" => Some(TokenKind::Do),
            "done" => Some(TokenKind::Done),
            "for" => Some(TokenKind::For),
            "in" => Some(TokenKind::In),
            "case" => Some(TokenKind::Case),
            "esac" => Some(TokenKind::Esac),
            "function" => Some(TokenKind::Function),
            _ => None,
        }
    }
}
