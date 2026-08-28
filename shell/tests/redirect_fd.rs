// racsh — fd-qualified redirection (`2>file`, `2>&1`).
//
// The parser and AST always modelled Redirect::fd, but the lexer never emitted
// IoNumber, so `cmd 2>/dev/null` lexed as the word "2" plus a plain redirect:
// the command received a bogus "2" operand and stderr was never redirected.
// These pin the lexing rule down.

use racsh::ast::AstNode;
use racsh::lexer::Lexer;
use racsh::parser::Parser;
use racsh::token::TokenKind;

fn lex(input: &str) -> Vec<TokenKind> {
    Lexer::new(input)
        .tokenize()
        .expect("lexing should succeed")
        .into_iter()
        .map(|t| t.kind)
        .collect()
}

fn parse(input: &str) -> AstNode {
    let tokens = Lexer::new(input).tokenize().expect("lexing should succeed");
    Parser::new(tokens).parse().expect("parsing should succeed")
}

fn first_command(node: &AstNode) -> (&Vec<racsh::ast::Word>, &Vec<racsh::ast::Redirect>) {
    match node {
        AstNode::Program { commands } => match &commands[0] {
            AstNode::SimpleCommand {
                words, redirects, ..
            } => (words, redirects),
            other => panic!("expected SimpleCommand, got {:?}", other),
        },
        other => panic!("expected Program, got {:?}", other),
    }
}

#[test]
fn digits_before_a_redirect_lex_as_an_io_number() {
    assert!(
        lex("cmd 2>f").contains(&TokenKind::IoNumber(2)),
        "2> must produce IoNumber(2), got {:?}",
        lex("cmd 2>f")
    );
}

#[test]
fn stderr_redirect_does_not_leak_a_bogus_operand() {
    // The actual bug: `rm x 2>/dev/null` passed "2" to rm as a second operand.
    let ast = parse("rm x 2>/dev/null");
    let (words, redirects) = first_command(&ast);
    assert_eq!(
        words.len(),
        2,
        "only `rm` and `x` are operands: {:?}",
        words
    );
    assert_eq!(redirects.len(), 1);
    assert_eq!(redirects[0].fd, Some(2), "redirect must target fd 2");
}

#[test]
fn plain_redirect_still_has_no_fd() {
    let ast = parse("echo hi >out");
    let (_, redirects) = first_command(&ast);
    assert_eq!(redirects.len(), 1);
    assert_eq!(redirects[0].fd, None);
}

#[test]
fn a_separated_digit_is_an_ordinary_word() {
    // `echo 2 > f` echoes "2"; only digits *immediately* before the operator
    // are an IO number.
    let ast = parse("echo 2 > f");
    let (words, redirects) = first_command(&ast);
    assert_eq!(words.len(), 2, "`2` stays an operand: {:?}", words);
    assert_eq!(redirects[0].fd, None);
}

#[test]
fn digits_not_followed_by_a_redirect_stay_a_word() {
    let toks = lex("echo 22 foo");
    assert!(
        !toks.iter().any(|t| matches!(t, TokenKind::IoNumber(_))),
        "no redirect operator follows, so no IoNumber: {:?}",
        toks
    );
}

#[test]
fn multi_digit_fd_is_supported() {
    let ast = parse("cmd 10>f");
    let (_, redirects) = first_command(&ast);
    assert_eq!(redirects[0].fd, Some(10));
}

#[test]
fn append_and_dup_forms_carry_the_fd_too() {
    let ast = parse("cmd 2>>log");
    let (_, redirects) = first_command(&ast);
    assert_eq!(redirects[0].fd, Some(2));

    let ast = parse("cmd 2>&1");
    let (_, redirects) = first_command(&ast);
    assert_eq!(redirects[0].fd, Some(2));
}
