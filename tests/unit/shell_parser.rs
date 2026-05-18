//! Unit tests for shell parser

#[cfg(test)]
mod tests {
    #[derive(Debug, Clone, PartialEq)]
    enum AstNode {
        Program { commands: Vec<AstNode> },
        SimpleCommand { words: Vec<String>, redirects: Vec<Redirect> },
        Sequence { left: Box<AstNode>, right: Box<AstNode>, op: SequenceOp },
    }

    #[derive(Debug, Clone, PartialEq)]
    struct Redirect {
        fd: i32,
        target: String,
    }

    #[derive(Debug, Clone, PartialEq)]
    enum SequenceOp {
        Semi,
        Amp,
    }

    #[derive(Debug, Clone)]
    struct ParseError {
        message: String,
    }

    // Mock parser for testing
    struct MockParser;

    impl MockParser {
        fn parse_simple_command(input: &str) -> Result<AstNode, ParseError> {
            let words: Vec<String> = input.split_whitespace()
                .map(|s| s.to_string())
                .collect();
            
            if words.is_empty() {
                return Err(ParseError { message: "Empty command".to_string() });
            }

            Ok(AstNode::SimpleCommand {
                words,
                redirects: Vec::new(),
            })
        }

        fn parse_sequence(input: &str) -> Result<AstNode, ParseError> {
            if input.contains(";") {
                let parts: Vec<&str> = input.split(";").collect();
                if parts.len() == 2 {
                    let left = Self::parse_simple_command(parts[0].trim())?;
                    let right = Self::parse_simple_command(parts[1].trim())?;
                    
                    return Ok(AstNode::Sequence {
                        left: Box::new(left),
                        right: Box::new(right),
                        op: SequenceOp::Semi,
                    });
                }
            }
            
            Self::parse_simple_command(input)
        }
    }

    #[test]
    fn test_parse_simple_command() {
        let result = MockParser::parse_simple_command("ls -l");
        assert!(result.is_ok());
        
        match result.unwrap() {
            AstNode::SimpleCommand { words, .. } => {
                assert_eq!(words.len(), 2);
                assert_eq!(words[0], "ls");
                assert_eq!(words[1], "-l");
            }
            _ => panic!("Expected SimpleCommand"),
        }
    }

    #[test]
    fn test_parse_command_with_args() {
        let result = MockParser::parse_simple_command("echo hello world");
        assert!(result.is_ok());
        
        match result.unwrap() {
            AstNode::SimpleCommand { words, .. } => {
                assert_eq!(words.len(), 3);
                assert_eq!(words[0], "echo");
                assert_eq!(words[1], "hello");
                assert_eq!(words[2], "world");
            }
            _ => panic!("Expected SimpleCommand"),
        }
    }

    #[test]
    fn test_parse_sequence() {
        let result = MockParser::parse_sequence("ls ; pwd");
        assert!(result.is_ok());
        
        match result.unwrap() {
            AstNode::Sequence { left, right, op } => {
                assert_eq!(op, SequenceOp::Semi);
                
                match *left {
                    AstNode::SimpleCommand { words, .. } => {
                        assert_eq!(words[0], "ls");
                    }
                    _ => panic!("Expected SimpleCommand in left"),
                }
                
                match *right {
                    AstNode::SimpleCommand { words, .. } => {
                        assert_eq!(words[0], "pwd");
                    }
                    _ => panic!("Expected SimpleCommand in right"),
                }
            }
            _ => panic!("Expected Sequence"),
        }
    }

    #[test]
    fn test_parse_empty_command() {
        let result = MockParser::parse_simple_command("");
        assert!(result.is_err());
    }
}