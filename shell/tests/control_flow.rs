// racsh — host-side coverage for control-flow parsing, field splitting, and
// the source/. builtin's env-restore contract. The actual command-execution
// paths (fork/exec/write) need real syscalls and are exercised by racos-test
// in the QEMU smoke; these tests cover everything reachable without I/O.

use core::cell::RefCell;
use racsh::ast::{AstNode, Word, WordPart};
use racsh::builtin::{is_builtin, run_source_in_env};
use racsh::expand::{expand_word_list, Env};
use racsh::lexer::Lexer;
use racsh::parser::Parser;

fn parse(input: &str) -> AstNode {
    let mut lexer = Lexer::new(input);
    let tokens = lexer.tokenize().expect("lexing should succeed");
    let mut parser = Parser::new(tokens);
    parser.parse().expect("parsing should succeed")
}

#[test]
fn parser_builds_if_then_else_ast() {
    match parse("if true; then echo yes; else echo no; fi") {
        AstNode::Program { commands } => match &commands[0] {
            AstNode::If {
                condition: _,
                then_body: _,
                elif_parts,
                else_body,
            } => {
                assert!(elif_parts.is_empty(), "no elif expected");
                assert!(else_body.is_some(), "else branch should be parsed");
            }
            other => panic!("expected If, got {:?}", other),
        },
        other => panic!("expected Program, got {:?}", other),
    }
}

#[test]
fn parser_builds_while_ast() {
    match parse("while false; do echo loop; done") {
        AstNode::Program { commands } => {
            assert!(matches!(&commands[0], AstNode::While { .. }));
        }
        other => panic!("expected Program, got {:?}", other),
    }
}

#[test]
fn parser_builds_for_ast_with_word_list() {
    match parse("for x in a b c; do echo $x; done") {
        AstNode::Program { commands } => match &commands[0] {
            AstNode::For { var, words, .. } => {
                assert_eq!(var, "x");
                let list = words.as_ref().expect("for loop should carry a word list");
                assert_eq!(list.len(), 3);
            }
            other => panic!("expected For, got {:?}", other),
        },
        other => panic!("expected Program, got {:?}", other),
    }
}

#[test]
fn parser_builds_case_ast_with_multiple_arms() {
    match parse("case $x in foo) echo f;; bar|baz) echo b;; *) echo other;; esac") {
        AstNode::Program { commands } => match &commands[0] {
            AstNode::Case { items, .. } => {
                assert_eq!(items.len(), 3);
                assert_eq!(items[1].patterns.len(), 2, "bar|baz is two patterns");
            }
            other => panic!("expected Case, got {:?}", other),
        },
        other => panic!("expected Program, got {:?}", other),
    }
}

#[test]
fn parser_builds_function_def() {
    match parse("greet() { echo hi; }") {
        AstNode::Program { commands } => {
            assert!(matches!(&commands[0], AstNode::FunctionDef { .. }));
        }
        other => panic!("expected Program, got {:?}", other),
    }
}

#[test]
fn unquoted_variable_expansion_splits_on_whitespace() {
    let mut env = Env::new(1);
    env.set(String::from("LIST"), String::from("a b c"));

    let word = Word::from_parts(vec![WordPart::Variable(String::from("LIST"))]);
    let expanded = expand_word_list(&word, &env);
    assert_eq!(expanded, vec!["a", "b", "c"]);
}

#[test]
fn unquoted_variable_expansion_collapses_runs_of_whitespace() {
    let mut env = Env::new(1);
    env.set(String::from("LIST"), String::from("  a   b\tc\n"));

    let word = Word::from_parts(vec![WordPart::Variable(String::from("LIST"))]);
    let expanded = expand_word_list(&word, &env);
    assert_eq!(expanded, vec!["a", "b", "c"]);
}

#[test]
fn empty_unquoted_variable_produces_no_words() {
    let env = Env::new(1);
    let word = Word::from_parts(vec![WordPart::Variable(String::from("UNSET"))]);
    let expanded = expand_word_list(&word, &env);
    assert!(expanded.is_empty(), "empty unquoted $VAR should drop");
}

#[test]
fn quoted_variable_expansion_does_not_split() {
    let mut env = Env::new(1);
    env.set(String::from("LIST"), String::from("a b c"));

    let word = Word::from_parts(vec![WordPart::DoubleQuoted(String::from("$LIST"))]);
    let expanded = expand_word_list(&word, &env);
    assert_eq!(expanded, vec!["a b c"], "\"$LIST\" must NOT word-split");
}

#[test]
fn literal_word_is_never_split() {
    let env = Env::new(1);
    let word = Word::literal("hello world");
    let expanded = expand_word_list(&word, &env);
    assert_eq!(expanded, vec!["hello world"]);
}

#[test]
fn builtin_table_recognizes_source_and_dot() {
    assert!(is_builtin("source"));
    assert!(is_builtin("."));
    assert!(!is_builtin("nonexistent"));
}

#[test]
fn run_source_in_env_sets_then_restores_arg0_and_positional() {
    let mut env = Env::new(99);
    env.arg0 = String::from("parent");
    env.positional = vec![String::from("p1"), String::from("p2")];

    // A no-op assignment script; doesn't invoke any I/O.
    let src = "X=1";
    let status = run_source_in_env(
        src,
        &mut env,
        Some(String::from("/etc/sourced.sh")),
        Some(vec![
            String::from("s1"),
            String::from("s2"),
            String::from("s3"),
        ]),
        &|_| {},
    );
    assert_eq!(status, 0);
    assert_eq!(env.arg0, "parent", "arg0 must be restored after sourcing");
    assert_eq!(
        env.positional,
        vec![String::from("p1"), String::from("p2")],
        "positional must be restored after sourcing"
    );
    assert_eq!(
        env.get("X"),
        Some("1"),
        "variable assignments leak to parent env per source semantics"
    );
}

#[test]
fn run_source_in_env_returns_2_on_lex_or_parse_error() {
    let mut env = Env::new(1);
    let captured: RefCell<Vec<u8>> = RefCell::new(Vec::new());
    let status = run_source_in_env(
        "this is ; ; ; not valid (",
        &mut env,
        None,
        None,
        &|bytes| captured.borrow_mut().extend_from_slice(bytes),
    );
    assert_eq!(status, 2, "parse/lex errors must surface as status 2");
    assert!(
        env.last_status == 2,
        "env.last_status must reflect source failure"
    );
    assert!(
        !captured.borrow().is_empty(),
        "an error message should be written to the provided sink"
    );
}

#[test]
fn run_source_in_env_preserves_parent_arg0_when_not_overridden() {
    let mut env = Env::new(1);
    env.arg0 = String::from("racsh");

    let status = run_source_in_env("", &mut env, None, None, &|_| {});
    assert_eq!(status, 0);
    assert_eq!(
        env.arg0, "racsh",
        "passing script_name=None must NOT alter arg0"
    );
}
