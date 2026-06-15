// racsh — AST node types (SHELL_GRAMMAR.md §4)

extern crate alloc;

use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec::Vec;

/// A word that may contain literal text and/or expansion parts.
#[derive(Debug, Clone, PartialEq)]
pub enum WordPart {
    Literal(String),
    SingleQuoted(String),
    DoubleQuoted(String),
    Variable(String),
    CommandSub(String),
    BraceExpansion(String),
    Glob(GlobPattern),
}

#[derive(Debug, Clone, PartialEq)]
pub enum GlobPattern {
    Star,
    Question,
    Bracket(String),
}

/// A compound word (sequence of parts).
#[derive(Debug, Clone, PartialEq)]
pub struct Word {
    pub parts: Vec<WordPart>,
}

impl Word {
    pub fn literal(s: &str) -> Self {
        Word {
            parts: alloc::vec![WordPart::Literal(String::from(s))],
        }
    }

    pub fn from_parts(parts: Vec<WordPart>) -> Self {
        Word { parts }
    }
}

/// Redirect direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RedirectOp {
    Input,     // <
    Output,    // >
    Append,    // >>
    DupInput,  // <&
    DupOutput, // >&
}

/// I/O redirect.
#[derive(Debug, Clone, PartialEq)]
pub struct Redirect {
    pub fd: Option<i32>,
    pub op: RedirectOp,
    pub target: Word,
}

/// Variable assignment.
#[derive(Debug, Clone, PartialEq)]
pub struct Assignment {
    pub name: String,
    pub value: Word,
}

/// Sequence operator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SequenceOp {
    Semi,       // ;
    Background, // &
}

/// A case item (pattern + body).
#[derive(Debug, Clone)]
pub struct CaseItem {
    pub patterns: Vec<Word>,
    pub body: Option<Box<AstNode>>,
}

/// AST node — represents a parsed shell construct.
#[derive(Debug, Clone)]
pub enum AstNode {
    /// A simple command: optional assignments, command words, redirects.
    SimpleCommand {
        assignments: Vec<Assignment>,
        words: Vec<Word>,
        redirects: Vec<Redirect>,
    },

    /// A pipeline: cmd1 | cmd2 | cmd3
    Pipeline {
        commands: Vec<AstNode>,
        negated: bool,
    },

    /// Sequential execution: left ; right  or  left & right
    Sequence {
        left: Box<AstNode>,
        right: Box<AstNode>,
        op: SequenceOp,
    },

    /// Logical AND: left && right
    And {
        left: Box<AstNode>,
        right: Box<AstNode>,
    },

    /// Logical OR: left || right
    Or {
        left: Box<AstNode>,
        right: Box<AstNode>,
    },

    /// Subshell: ( commands )
    Subshell {
        body: Box<AstNode>,
        redirects: Vec<Redirect>,
    },

    /// Brace group: { commands; }
    BraceGroup {
        body: Box<AstNode>,
        redirects: Vec<Redirect>,
    },

    /// If statement
    If {
        condition: Box<AstNode>,
        then_body: Box<AstNode>,
        elif_parts: Vec<(Box<AstNode>, Box<AstNode>)>,
        else_body: Option<Box<AstNode>>,
    },

    /// While loop
    While {
        condition: Box<AstNode>,
        body: Box<AstNode>,
    },

    /// For loop
    For {
        var: String,
        words: Option<Vec<Word>>,
        body: Box<AstNode>,
    },

    /// Case statement
    Case { word: Word, items: Vec<CaseItem> },

    /// Function definition
    FunctionDef { name: String, body: Box<AstNode> },

    /// A list of commands (program root).
    Program { commands: Vec<AstNode> },
}
