// racsh — prompt escape expansion (ROADMAP v0.2 §2.2)
//
// `PS1` already went through parameter expansion by the time it reaches here,
// so this stage only handles the backslash escapes a prompt string carries:
//
//   \u  user name          \h  host name        \w  working directory
//   \W  basename of \w     \$  '#' for uid 0, else '$'
//   \n  newline            \\  a literal backslash
//
// An unknown escape is emitted verbatim (backslash included) rather than
// swallowed, so a typo in PS1 is visible instead of silently deleting text.

extern crate alloc;

use alloc::string::String;

/// Everything the prompt needs to know about the session.
///
/// Passed in rather than read here so this stays pure and host-testable: the
/// interactive shell fills it from getuid/getcwd, the tests fill it directly.
pub struct PromptContext<'a> {
    pub user: &'a str,
    pub host: &'a str,
    pub cwd: &'a str,
    pub home: &'a str,
    pub uid: u32,
}

impl<'a> Default for PromptContext<'a> {
    fn default() -> Self {
        PromptContext {
            user: "root",
            host: "racos",
            cwd: "/",
            home: "/",
            uid: 0,
        }
    }
}

/// Expand backslash escapes in a prompt template.
pub fn expand_prompt(template: &str, ctx: &PromptContext) -> String {
    let mut out = String::new();
    let mut chars = template.chars();

    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('u') => out.push_str(ctx.user),
            Some('h') => out.push_str(ctx.host),
            Some('w') => out.push_str(&tilde_abbreviate(ctx.cwd, ctx.home)),
            Some('W') => out.push_str(basename(ctx.cwd)),
            Some('$') => out.push(if ctx.uid == 0 { '#' } else { '$' }),
            Some('n') => out.push('\n'),
            Some('\\') => out.push('\\'),
            // Unknown escape: keep both characters so the mistake shows.
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            // Trailing lone backslash.
            None => out.push('\\'),
        }
    }
    out
}

/// Replace a leading `home` with `~`, the way `\w` is conventionally shown.
///
/// Only a whole path component matches: with home `/home/ada`, the path
/// `/home/adam` is left alone rather than becoming `~m`.
fn tilde_abbreviate(cwd: &str, home: &str) -> String {
    if home.is_empty() || home == "/" {
        return String::from(cwd);
    }
    let home = home.trim_end_matches('/');
    if cwd == home {
        return String::from("~");
    }
    if let Some(rest) = cwd.strip_prefix(home) {
        if rest.starts_with('/') {
            let mut s = String::from("~");
            s.push_str(rest);
            return s;
        }
    }
    String::from(cwd)
}

/// Last component of a path; `/` stays `/`.
fn basename(path: &str) -> &str {
    let trimmed = path.trim_end_matches('/');
    if trimmed.is_empty() {
        return "/";
    }
    match trimmed.rfind('/') {
        Some(i) => &trimmed[i + 1..],
        None => trimmed,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx() -> PromptContext<'static> {
        PromptContext {
            user: "ada",
            host: "racos",
            cwd: "/home/ada/src",
            home: "/home/ada",
            uid: 1000,
        }
    }

    #[test]
    fn expands_the_documented_escapes() {
        assert_eq!(
            expand_prompt("\\u@\\h:\\w\\$ ", &ctx()),
            "ada@racos:~/src$ "
        );
    }

    #[test]
    fn root_gets_a_hash() {
        let mut c = ctx();
        c.uid = 0;
        assert_eq!(expand_prompt("\\$", &c), "#");
    }

    #[test]
    fn w_shows_only_the_basename() {
        assert_eq!(expand_prompt("\\W", &ctx()), "src");
        let mut c = ctx();
        c.cwd = "/";
        assert_eq!(expand_prompt("\\W", &c), "/");
    }

    #[test]
    fn tilde_only_matches_whole_components() {
        let mut c = ctx();
        c.cwd = "/home/adam";
        assert_eq!(expand_prompt("\\w", &c), "/home/adam");
        c.cwd = "/home/ada";
        assert_eq!(expand_prompt("\\w", &c), "~");
    }

    #[test]
    fn unknown_escape_survives_verbatim() {
        assert_eq!(expand_prompt("\\q", &ctx()), "\\q");
        assert_eq!(expand_prompt("a\\", &ctx()), "a\\");
    }

    #[test]
    fn literal_backslash_and_newline() {
        assert_eq!(expand_prompt("\\\\\\n", &ctx()), "\\\n");
    }

    #[test]
    fn plain_text_passes_through() {
        assert_eq!(expand_prompt("racsh$ ", &ctx()), "racsh$ ");
    }
}
