use racsh::ast::{AstNode, SequenceOp, Word, WordPart};
use racsh::expand::{expand_word, pattern_match, Env};
use racsh::lexer::Lexer;
use racsh::parser::Parser;

fn parse(input: &str) -> AstNode {
    let mut lexer = Lexer::new(input);
    let tokens = lexer.tokenize().expect("lexing should succeed");
    let mut parser = Parser::new(tokens);
    parser.parse().expect("parsing should succeed")
}

fn literal(word: &Word) -> &str {
    match word.parts.as_slice() {
        [WordPart::Literal(value)] => value.as_str(),
        other => panic!("expected literal word, got {:?}", other),
    }
}

#[test]
fn parser_builds_simple_command_ast() {
    match parse("echo hello world") {
        AstNode::Program { commands } => {
            assert_eq!(commands.len(), 1);
            match &commands[0] {
                AstNode::SimpleCommand {
                    words, redirects, ..
                } => {
                    assert_eq!(redirects.len(), 0);
                    assert_eq!(words.len(), 3);
                    assert_eq!(literal(&words[0]), "echo");
                    assert_eq!(literal(&words[1]), "hello");
                    assert_eq!(literal(&words[2]), "world");
                }
                other => panic!("expected simple command, got {:?}", other),
            }
        }
        other => panic!("expected program, got {:?}", other),
    }
}

#[test]
fn parser_builds_sequence_ast() {
    match parse("ls ; pwd") {
        AstNode::Program { commands } => {
            assert_eq!(commands.len(), 1);
            match &commands[0] {
                AstNode::Sequence { left, right, op } => {
                    assert_eq!(*op, SequenceOp::Semi);
                    match &**left {
                        AstNode::SimpleCommand { words, .. } => {
                            assert_eq!(literal(&words[0]), "ls");
                        }
                        other => panic!("expected left simple command, got {:?}", other),
                    }
                    match &**right {
                        AstNode::SimpleCommand { words, .. } => {
                            assert_eq!(literal(&words[0]), "pwd");
                        }
                        other => panic!("expected right simple command, got {:?}", other),
                    }
                }
                other => panic!("expected sequence, got {:?}", other),
            }
        }
        other => panic!("expected program, got {:?}", other),
    }
}

#[test]
fn expander_uses_real_environment_and_word_parts() {
    let mut env = Env::new(99);
    env.set(String::from("PROJECT"), String::from("RacOS"));
    env.last_status = 23;

    let word = Word::from_parts(vec![
        WordPart::DoubleQuoted(String::from("$PROJECT")),
        WordPart::Literal(String::from(":")),
        WordPart::Variable(String::from("?")),
    ]);

    assert_eq!(expand_word(&word, &env), "RacOS:23");
}

#[test]
fn expander_case_patterns_are_real_globs() {
    assert!(pattern_match("bootx64.efi", "boot*.efi"));
    assert!(pattern_match("tty7", "tty[0-9]"));
    assert!(!pattern_match("tty12", "tty?"));
}
