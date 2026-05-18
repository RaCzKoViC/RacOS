// racsh — Variable / word expansion
//
// Expands WordParts into final strings.
// Order: variable → command substitution (stub) → quote removal

extern crate alloc;

use alloc::string::String;
use alloc::vec::Vec;
use crate::ast::{Word, WordPart};

/// Shell environment: variables + last exit status.
pub struct Env {
    /// Variable storage: (name, value) pairs.
    vars: Vec<(String, String)>,
    /// Last command exit status ($?).
    pub last_status: i32,
    /// Shell PID ($$).
    pub shell_pid: i32,
}

impl Env {
    pub fn new(pid: i32) -> Self {
        Env {
            vars: Vec::new(),
            last_status: 0,
            shell_pid: pid,
        }
    }

    /// Get a variable value.
    pub fn get(&self, name: &str) -> Option<&str> {
        for (k, v) in self.vars.iter().rev() {
            if k == name {
                return Some(v.as_str());
            }
        }
        None
    }

    /// Set a variable.
    pub fn set(&mut self, name: String, value: String) {
        // Update existing or push new
        for entry in self.vars.iter_mut() {
            if entry.0 == name {
                entry.1 = value;
                return;
            }
        }
        self.vars.push((name, value));
    }

    /// Remove a variable.
    pub fn unset(&mut self, name: &str) {
        self.vars.retain(|(k, _)| k != name);
    }

    /// Get PATH as an iterator of directory strings.
    pub fn path_dirs(&self) -> PathDirs<'_> {
        PathDirs {
            path: self.get("PATH").unwrap_or("/bin"),
            pos: 0,
        }
    }

    /// Create a shallow clone for command substitution execution.
    pub fn clone_for_sub(&self) -> Env {
        Env {
            vars: self.vars.clone(),
            last_status: self.last_status,
            shell_pid: self.shell_pid,
        }
    }
}

/// Iterator over colon-separated PATH entries.
pub struct PathDirs<'a> {
    path: &'a str,
    pos: usize,
}

impl<'a> Iterator for PathDirs<'a> {
    type Item = &'a str;

    fn next(&mut self) -> Option<Self::Item> {
        if self.pos >= self.path.len() {
            return None;
        }
        let rest = &self.path[self.pos..];
        match rest.find(':') {
            Some(i) => {
                self.pos += i + 1;
                Some(&rest[..i])
            }
            None => {
                self.pos = self.path.len();
                Some(rest)
            }
        }
    }
}

/// Expand a Word to a string, performing variable + tilde expansion.
pub fn expand_word(word: &Word, env: &Env) -> String {
    let mut result = String::new();
    for part in &word.parts {
        expand_part(part, env, &mut result);
    }
    // Tilde expansion: ~ at start of word → HOME or /
    if result.starts_with('~') {
        let home = env.get("HOME").unwrap_or("/");
        if result.len() == 1 {
            return String::from(home);
        }
        if result.as_bytes()[1] == b'/' {
            let mut expanded = String::from(home);
            expanded.push_str(&result[1..]);
            return expanded;
        }
    }
    result
}

/// Expand a single WordPart.
fn expand_part(part: &WordPart, env: &Env, out: &mut String) {
    match part {
        WordPart::Literal(s) | WordPart::SingleQuoted(s) => {
            out.push_str(s);
        }
        WordPart::DoubleQuoted(s) => {
            // Inside double quotes, expand $VAR inline
            expand_dollar_in_string(s, env, out);
        }
        WordPart::Variable(name) => {
            expand_variable(name, env, out);
        }
        WordPart::CommandSub(cmd) => {
            // Command substitution: run command, capture stdout
            expand_command_sub(cmd, env, out);
        }
        WordPart::BraceExpansion(s) => {
            // Post-MVP — treat as literal for now
            out.push('{');
            out.push_str(s);
            out.push('}');
        }
        WordPart::Glob(_) => {
            // Glob expansion requires filesystem access — pass through as literal
            match part {
                WordPart::Glob(crate::ast::GlobPattern::Star) => out.push('*'),
                WordPart::Glob(crate::ast::GlobPattern::Question) => out.push('?'),
                WordPart::Glob(crate::ast::GlobPattern::Bracket(s)) => {
                    out.push('[');
                    out.push_str(s);
                    out.push(']');
                }
                _ => {}
            }
        }
    }
}

/// Expand a variable name to its value.
fn expand_variable(name: &str, env: &Env, out: &mut String) {
    match name {
        "?" => {
            format_i32(env.last_status, out);
        }
        "$" => {
            format_i32(env.shell_pid, out);
        }
        _ => {
            if let Some(val) = env.get(name) {
                out.push_str(val);
            }
        }
    }
}

/// Expand $VAR references within a double-quoted string.
fn expand_dollar_in_string(s: &str, env: &Env, out: &mut String) {
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'$' {
            i += 1;
            if i >= bytes.len() {
                out.push('$');
                break;
            }
            // Collect variable name (alphanumeric + _)
            let start = i;
            while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_' || (i == start && (bytes[i] == b'?' || bytes[i] == b'$'))) {
                i += 1;
                if i == start + 1 && (bytes[start] == b'?' || bytes[start] == b'$') {
                    break; // Single-char specials
                }
            }
            if i > start {
                let var_name = &s[start..i];
                expand_variable(var_name, env, out);
            } else {
                out.push('$');
            }
        } else if bytes[i] == b'\\' && i + 1 < bytes.len() {
            i += 1;
            match bytes[i] {
                b'\\' => out.push('\\'),
                b'"' => out.push('"'),
                b'$' => out.push('$'),
                b'`' => out.push('`'),
                b'n' => out.push('\n'),
                _ => {
                    out.push('\\');
                    out.push(bytes[i] as char);
                }
            }
            i += 1;
        } else {
            out.push(bytes[i] as char);
            i += 1;
        }
    }
}

/// Format i32 to string without alloc.
fn format_i32(val: i32, out: &mut String) {
    if val < 0 {
        out.push('-');
        format_u32((-val) as u32, out);
    } else {
        format_u32(val as u32, out);
    }
}

fn format_u32(mut val: u32, out: &mut String) {
    if val == 0 {
        out.push('0');
        return;
    }
    let mut buf = [0u8; 10];
    let mut pos = 10;
    while val > 0 {
        pos -= 1;
        buf[pos] = b'0' + (val % 10) as u8;
        val /= 10;
    }
    for &b in &buf[pos..] {
        out.push(b as char);
    }
}

/// Execute command substitution: run inner command string, capture stdout.
///
/// Creates a pipe, redirects child stdout to pipe write end,
/// spawns the command via the shell, reads output from read end.
fn expand_command_sub(cmd: &str, env: &Env, out: &mut String) {
    // Create a pipe
    let mut fds = [0i32; 2];
    if libc_lite::pipe(&mut fds).is_err() {
        return;
    }
    let read_fd = fds[0];
    let write_fd = fds[1];

    // Save current stdout
    let saved_stdout = match libc_lite::dup(1) {
        Ok(fd) => fd,
        Err(_) => {
            let _ = libc_lite::close(read_fd);
            let _ = libc_lite::close(write_fd);
            return;
        }
    };

    // Redirect stdout to write end of pipe
    let _ = libc_lite::dup2(write_fd, 1);
    let _ = libc_lite::close(write_fd);

    // Lex → Parse → Execute the inner command
    {
        use crate::lexer::Lexer;
        use crate::parser::Parser;
        use crate::exec;

        // We need a mutable Env for execution, but we only have &Env.
        // Create a temporary clone of the environment for the subcommand.
        let mut sub_env = env.clone_for_sub();

        let mut lexer = Lexer::new(cmd);
        if let Ok(tokens) = lexer.tokenize() {
            let mut parser = Parser::new(tokens);
            if let Ok(ast) = parser.parse() {
                let _ = exec::execute(&ast, &mut sub_env);
            }
        }
    }

    // Restore stdout
    let _ = libc_lite::dup2(saved_stdout, 1);
    let _ = libc_lite::close(saved_stdout);

    // Read all output from the read end
    let mut buf = [0u8; 256];
    loop {
        match libc_lite::read(read_fd, &mut buf) {
            Ok(0) => break,
            Ok(n) => {
                if let Ok(s) = core::str::from_utf8(&buf[..n]) {
                    out.push_str(s);
                }
            }
            Err(_) => break,
        }
    }
    let _ = libc_lite::close(read_fd);

    // Strip trailing newline(s) — standard shell behavior
    while out.ends_with('\n') {
        out.pop();
    }
}

/// Expand a word, handling glob patterns by matching against filesystem.
/// Returns a `Vec<String>` with all matching files (or single expanded word if no globs).
pub fn expand_word_list(word: &Word, env: &Env) -> Vec<String> {
    // First, handle brace expansion {a,b,c} → multiple words
    
    // Build the complete unexpanded word to check for braces
    let mut complete_word = String::new();
    for part in &word.parts {
        match part {
            WordPart::Literal(s) => complete_word.push_str(s),
            WordPart::SingleQuoted(s) => complete_word.push_str(s),
            WordPart::DoubleQuoted(s) => complete_word.push_str(s),
            WordPart::BraceExpansion(s) => {
                complete_word.push_str(s);
            }
            _ => {}
        }
    }
    
    // Check for brace expansion {a,b,c}
    if let Some(pos) = find_brace_expansion(&complete_word) {
        let (prefix, brace_part, suffix) = extract_brace(&complete_word, pos);
        if let Some(alternatives) = parse_brace_expansion(brace_part) {
            let mut result = Vec::new();
            for alt in alternatives {
                let mut expanded = String::from(prefix);
                expanded.push_str(&alt);
                expanded.push_str(suffix);
                // Now expand glob patterns on this result
                result.extend(expand_single_with_glob(&expanded, env));
            }
            return result;
        }
    }
    
    // No brace expansion — check if word contains any glob patterns
    let has_glob = word.parts.iter().any(|p| matches!(p, WordPart::Glob(_)));
    
    if !has_glob {
        // No glob patterns — expand normally and return single result
        let mut result = Vec::new();
        result.push(expand_word(word, env));
        return result;
    }
    
    // Word contains glob patterns — reconstruct pattern string from parts
    let pattern = reconstruct_pattern(word, env);
    
    // Perform glob expansion
    match glob_expand(&pattern, env) {
        Some(matches) if !matches.is_empty() => matches,
        _ => {
            let mut result = Vec::new();
            result.push(pattern);
            result
        }
    }
}

/// Expand a simple string with glob patterns.
fn expand_single_with_glob(pattern: &str, env: &Env) -> Vec<String> {
    match glob_expand(pattern, env) {
        Some(matches) if !matches.is_empty() => matches,
        _ => {
            let mut result = Vec::new();
            result.push(String::from(pattern));
            result
        }
    }
}

/// Reconstruct the pattern string from word parts (with globs).
fn reconstruct_pattern(word: &Word, env: &Env) -> String {
    let mut pattern = String::new();
    for part in &word.parts {
        match part {
            WordPart::Literal(s) => pattern.push_str(s),
            WordPart::SingleQuoted(s) => pattern.push_str(s),
            WordPart::DoubleQuoted(_s) => pattern.push_str(_s),
            WordPart::Variable(name) => {
                if let Some(val) = env.get(name) {
                    pattern.push_str(val);
                }
            }
            WordPart::CommandSub(cmd) => {
                let mut expanded = String::new();
                expand_command_sub(cmd, env, &mut expanded);
                pattern.push_str(&expanded);
            }
            WordPart::Glob(_) => {
                match part {
                    WordPart::Glob(crate::ast::GlobPattern::Star) => pattern.push('*'),
                    WordPart::Glob(crate::ast::GlobPattern::Question) => pattern.push('?'),
                    WordPart::Glob(crate::ast::GlobPattern::Bracket(s)) => {
                        pattern.push('[');
                        pattern.push_str(s);
                        pattern.push(']');
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }
    pattern
}

/// Find the position and extent of a {...} brace expansion in a string.
/// Returns the starting position of '{' if found.
fn find_brace_expansion(s: &str) -> Option<usize> {
    let mut depth = 0;
    let mut in_quote = false;
    for (i, c) in s.char_indices() {
        match c {
            '\\' => in_quote = true,  // Skip next char
            '{' if !in_quote => {
                if depth == 0 {
                    return Some(i);
                }
                depth += 1;
            }
            '}' if !in_quote => {
                depth -= 1;
            }
            _ if in_quote => in_quote = false,
            _ => {}
        }
    }
    None
}

/// Extract {prefix, brace_content, suffix} from string containing braces at given position.
fn extract_brace(s: &str, start_pos: usize) -> (&str, &str, &str) {
    let prefix = &s[..start_pos];
    
    // Find matching closing brace
    let mut depth = 0;
    let mut end_pos = start_pos;
    for (i, c) in s[start_pos..].char_indices() {
        match c {
            '{' => {
                depth += 1;
                end_pos = start_pos + i;
            }
            '}' => {
                depth -= 1;
                if depth == 0 {
                    end_pos = start_pos + i;
                    break;
                }
            }
            _ => {}
        }
    }
    
    let brace_content = &s[start_pos + 1..end_pos];
    let suffix = if end_pos + 1 < s.len() {
        &s[end_pos + 1..]
    } else {
        ""
    };
    
    (prefix, brace_content, suffix)
}

/// Parse brace expansion content {a,b,c} into alternatives.
fn parse_brace_expansion(content: &str) -> Option<Vec<String>> {
    // Simple comma-separated alternatives (no nesting or ranges for now)
    let mut alternatives = Vec::new();
    let mut current = String::new();
    let mut depth = 0;
    
    for c in content.chars() {
        match c {
            '{' => {
                depth += 1;
                current.push(c);
            }
            '}' => {
                if depth > 0 {
                    depth -= 1;
                    current.push(c);
                }
            }
            ',' if depth == 0 => {
                if !current.is_empty() || alternatives.is_empty() {
                    alternatives.push(current.clone());
                    current.clear();
                }
            }
            _ => current.push(c),
        }
    }
    
    // Add final alternative
    if !current.is_empty() || !alternatives.is_empty() {
        alternatives.push(current);
    }
    
    if alternatives.is_empty() {
        None
    } else if alternatives.len() == 1 && content.find(',').is_none() {
        // Single element with no commas — not a brace expansion
        None
    } else {
        Some(alternatives)
    }
}

/// Expand glob pattern against filesystem.
/// Returns Some(Vec<matches>) or None if pattern is invalid.
fn glob_expand(pattern: &str, _env: &Env) -> Option<Vec<String>> {
    // Determine directory to search
    let (dir_path, glob_part) = split_path_and_pattern(pattern);
    
    // Check if dir_path contains environment variables that need expansion
    let expanded_dir = if dir_path.contains('$') {
        // [TODO] Expand environment variables in path
        alloc::string::String::from(dir_path)
    } else {
        alloc::string::String::from(dir_path)
    };
    
    // Handle special case of pattern with no directory (relative to current dir)
    let dir_to_search = if expanded_dir.is_empty() { "." } else { &expanded_dir };
    
    // Open directory
    let fd = match libc_lite::open(dir_to_search.as_bytes(), 0, 0) {
        Ok(fd) => fd,
        Err(_) => return None,
    };
    
    // Read directory entries
    let mut matches = Vec::new();
    let mut buf = [0u8; 4096];
    
    match libc_lite::getdents(fd, &mut buf) {
        Ok(nbytes) => {
            let mut offset = 0;
            while offset + 10 <= nbytes {
                let name_len = buf[offset + 9] as usize;
                if offset + 10 + name_len > nbytes {
                    break;
                }
                let name_bytes = &buf[offset + 10..offset + 10 + name_len];
                
                // Remove trailing null terminator from name
                let name_end = name_bytes.iter().position(|&b| b == 0).unwrap_or(name_len);
                if name_end > 0 {
                    if let Ok(name) = core::str::from_utf8(&name_bytes[..name_end]) {
                        if glob_match(name, glob_part) {
                            // Build full path
                            let mut full_path = String::from(dir_to_search);
                            if !dir_to_search.ends_with('/') {
                                full_path.push('/');
                            }
                            full_path.push_str(name);
                            matches.push(full_path);
                        }
                    }
                }
                
                offset += 10 + name_len;
            }
        }
        Err(_) => {
            let _ = libc_lite::close(fd);
            return None;
        }
    }
    
    let _ = libc_lite::close(fd);
    
    if matches.is_empty() {
        None
    } else {
        matches.sort();  // Sort for predictable order
        Some(matches)
    }
}

/// Split a path pattern into (directory_path, glob_pattern).
/// E.g., "foo/*.txt" → ("foo", "*.txt"), "*.c" → ("", "*.c")
fn split_path_and_pattern(pattern: &str) -> (&str, &str) {
    if let Some(pos) = pattern.rfind('/') {
        (&pattern[..pos], &pattern[pos + 1..])
    } else {
        ("", pattern)
    }
}

/// Check if filename matches glob pattern.
/// Supports: * (any chars), ? (single char), [abc] (char set)
fn glob_match(name: &str, pattern: &str) -> bool {
    let name_bytes = name.as_bytes();
    let pat_bytes = pattern.as_bytes();
    glob_match_recursive(name_bytes, pat_bytes)
}

/// Recursive glob matching with backtracking.
fn glob_match_recursive(name: &[u8], pattern: &[u8]) -> bool {
    // Base cases
    if pattern.is_empty() {
        return name.is_empty();
    }
    if pattern.len() == 1 && pattern[0] == b'*' {
        return true;  // * matches everything
    }
    
    // Handle current pattern character
    match pattern[0] {
        b'*' => {
            // * matches zero or more characters
            // Try matching rest of pattern at each position in name
            for i in 0..=name.len() {
                if glob_match_recursive(&name[i..], &pattern[1..]) {
                    return true;
                }
            }
            false
        }
        b'?' => {
            // ? matches exactly one character
            if name.is_empty() {
                false
            } else {
                glob_match_recursive(&name[1..], &pattern[1..])
            }
        }
        b'[' => {
            // [abc] or [a-z] character set
            if name.is_empty() {
                return false;
            }
            
            // Find closing bracket
            if let Some(close_pos) = pattern[1..].iter().position(|&b| b == b']') {
                let char_set = &pattern[1..1 + close_pos];
                let name_char = name[0];
                
                let matches = if char_set.len() >= 2 && char_set[1] == b'-' {
                    // Range like [a-z]
                    let start = char_set[0];
                    let end = char_set[2];
                    name_char >= start && name_char <= end
                } else {
                    // Explicit set like [abc]
                    char_set.iter().any(|&c| c == name_char)
                };
                
                if matches {
                    glob_match_recursive(&name[1..], &pattern[2 + close_pos..])
                } else {
                    false
                }
            } else {
                false  // Unclosed bracket
            }
        }
        c => {
            // Literal character must match
            if name.is_empty() || name[0] != c {
                false
            } else {
                glob_match_recursive(&name[1..], &pattern[1..])
            }
        }
    }
}
