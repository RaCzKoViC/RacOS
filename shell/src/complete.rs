// racsh — Tab completion (ROADMAP v0.2 §2.2)
//
// Two kinds of completion, chosen by where the word sits:
//
//   * command position  -> builtins + every executable on $PATH
//   * anywhere else     -> file and directory names
//
// A word containing '/' is always completed as a path, even in command
// position, so `./scr<Tab>` and `/bin/ec<Tab>` behave the way they read.
//
// The decision logic and prefix arithmetic are pure functions with tests; only
// candidate gathering touches the filesystem.

extern crate alloc;

use alloc::string::String;
use alloc::vec::Vec;

/// Byte range of the word being completed, and whether it sits in command
/// position.
pub struct WordSpan {
    pub start: usize,
    pub end: usize,
    pub command_position: bool,
}

/// Locate the word the cursor is in or at the end of.
///
/// Word boundaries are whitespace. Command position means the word is the
/// first of a command: nothing before it on the line, or only a separator
/// (`|`, `;`, `&`) since the last one.
pub fn word_span(line: &str, cursor: usize) -> WordSpan {
    let bytes = line.as_bytes();
    let cursor = cursor.min(bytes.len());

    let mut start = cursor;
    while start > 0 && !is_word_break(bytes[start - 1]) {
        start -= 1;
    }
    let mut end = cursor;
    while end < bytes.len() && !is_word_break(bytes[end]) {
        end += 1;
    }

    // Walk back over blanks; whatever precedes decides command position.
    let mut i = start;
    while i > 0 && (bytes[i - 1] == b' ' || bytes[i - 1] == b'\t') {
        i -= 1;
    }
    let command_position = i == 0 || matches!(bytes[i - 1], b'|' | b';' | b'&' | b'(');

    WordSpan {
        start,
        end,
        command_position,
    }
}

fn is_word_break(b: u8) -> bool {
    matches!(
        b,
        b' ' | b'\t' | b'|' | b';' | b'&' | b'(' | b')' | b'<' | b'>'
    )
}

/// Split a path prefix into (directory to scan, partial basename).
///
/// `"/bi"` -> ("/", "bi");  `"src/ma"` -> ("src/", "ma");  `"ma"` -> ("", "ma").
/// The directory keeps its trailing slash so callers can concatenate directly.
pub fn split_path_prefix(word: &str) -> (&str, &str) {
    match word.rfind('/') {
        Some(i) => (&word[..i + 1], &word[i + 1..]),
        None => ("", word),
    }
}

/// Longest common prefix of all candidates; empty when they share none.
pub fn common_prefix(candidates: &[String]) -> String {
    let first = match candidates.first() {
        Some(f) => f.as_str(),
        None => return String::new(),
    };
    let mut len = first.len();
    for c in &candidates[1..] {
        len = len.min(shared_len(first, c));
        if len == 0 {
            return String::new();
        }
    }
    // Do not split a UTF-8 character in half.
    while len > 0 && !first.is_char_boundary(len) {
        len -= 1;
    }
    String::from(&first[..len])
}

fn shared_len(a: &str, b: &str) -> usize {
    let (ab, bb) = (a.as_bytes(), b.as_bytes());
    let mut i = 0;
    while i < ab.len() && i < bb.len() && ab[i] == bb[i] {
        i += 1;
    }
    i
}

/// Names in `dir` starting with `prefix`, sorted and de-duplicated.
///
/// `dir` may be "" for the current directory. Unreadable directories yield
/// nothing rather than an error: completion is a convenience, never fatal.
pub fn dir_entries_with_prefix(dir: &str, prefix: &str) -> Vec<String> {
    let scan = if dir.is_empty() { "." } else { dir };
    let mut cpath = String::from(scan);
    cpath.push('\0');

    let fd = match libc_lite::open(cpath.as_bytes(), 0, 0) {
        Ok(fd) => fd,
        Err(_) => return Vec::new(),
    };

    let mut buf = [0u8; 4096];
    // sys_getdents emits every entry in one call and keeps no cursor, so a
    // second call would repeat them forever. One call, parse it all.
    let n = match libc_lite::getdents(fd, &mut buf) {
        Ok(n) => n,
        Err(_) => {
            let _ = libc_lite::close(fd);
            return Vec::new();
        }
    };
    let _ = libc_lite::close(fd);

    let mut out: Vec<String> = Vec::new();
    // Layout: [0..8) ino u64, [8] file_type u8, [9] name_len u8, [10..] name.
    let mut off = 0usize;
    while off + 10 <= n {
        let name_len = buf[off + 9] as usize;
        let entry_size = 10 + name_len;
        if off + entry_size > n {
            break;
        }
        let name = &buf[off + 10..off + 10 + name_len];
        off += entry_size;

        if name == b"." || name == b".." {
            continue;
        }
        if let Ok(s) = core::str::from_utf8(name) {
            if s.starts_with(prefix) && !out.iter().any(|e| e == s) {
                out.push(String::from(s));
            }
        }
    }
    out.sort();
    out
}

/// Everything runnable whose name starts with `prefix`: builtins first, then
/// each $PATH directory in order.
pub fn command_candidates(prefix: &str, path: &str, builtins: &[&str]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();

    for b in builtins {
        if b.starts_with(prefix) {
            out.push(String::from(*b));
        }
    }
    for dir in path.split(':').filter(|d| !d.is_empty()) {
        for name in dir_entries_with_prefix(dir, prefix) {
            if !out.iter().any(|e| *e == name) {
                out.push(name);
            }
        }
    }
    out.sort();
    out
}

/// Parse `/proc/mounts` content into the mountpoints that sit directly inside
/// `dir` and start with `prefix`, returned as bare names.
///
/// Mountpoints are not directory entries. The kernel creates `/proc`, `/dev`,
/// `/tmp`, `/mnt` and friends in the mount table, while `/` in the initramfs
/// carries only `bin`, `etc` and `sbin` — so completing `/pro` by listing `/`
/// finds nothing. Folding the mount table in fixes that.
///
/// Split from the I/O so it can be tested on the host.
pub fn mountpoints_in(mounts: &str, dir: &str, prefix: &str) -> Vec<String> {
    // `dir` arrives with its trailing slash ("/" or "/mnt/"); normalise to the
    // form a mountpoint's parent takes ("" for root, "/mnt" otherwise).
    let parent = if dir == "/" {
        ""
    } else {
        dir.trim_end_matches('/')
    };

    let mut out: Vec<String> = Vec::new();
    for line in mounts.split('\n') {
        // Field 1 is the mountpoint: "device mountpoint fstype opts 0 0".
        let mp = match line.split_whitespace().nth(1) {
            Some(m) => m,
            None => continue,
        };
        if mp == "/" {
            continue; // root has no parent to complete under
        }
        let (mp_parent, name) = match mp.rfind('/') {
            Some(i) => (&mp[..i], &mp[i + 1..]),
            None => continue,
        };
        if mp_parent == parent && name.starts_with(prefix) && !out.iter().any(|e| e == name) {
            out.push(String::from(name));
        }
    }
    out
}

fn read_proc_mounts() -> String {
    let fd = match libc_lite::open(b"/proc/mounts\0", 0, 0) {
        Ok(fd) => fd,
        Err(_) => return String::new(),
    };
    let mut contents: Vec<u8> = Vec::new();
    let mut chunk = [0u8; 512];
    loop {
        match libc_lite::read(fd, &mut chunk) {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                contents.extend_from_slice(&chunk[..n]);
                if contents.len() > 8192 {
                    break;
                }
            }
        }
    }
    let _ = libc_lite::close(fd);
    match core::str::from_utf8(&contents) {
        Ok(s) => String::from(s),
        Err(_) => String::new(),
    }
}

/// Candidates for a path-shaped word, each returned with its directory prefix
/// re-attached so it can replace the word wholesale.
pub fn path_candidates(word: &str) -> Vec<String> {
    let (dir, partial) = split_path_prefix(word);

    let mut names = dir_entries_with_prefix(dir, partial);
    for mp in mountpoints_in(&read_proc_mounts(), dir, partial) {
        if !names.iter().any(|n| *n == mp) {
            names.push(mp);
        }
    }
    names.sort();

    names
        .into_iter()
        .map(|name| {
            let mut full = String::from(dir);
            full.push_str(&name);
            full
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn owned(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| String::from(*s)).collect()
    }

    #[test]
    fn word_at_end_of_line_is_command_position() {
        let s = word_span("ls", 2);
        assert_eq!((s.start, s.end), (0, 2));
        assert!(s.command_position);
    }

    #[test]
    fn second_word_is_not_command_position() {
        let s = word_span("ls /bi", 6);
        assert_eq!((s.start, s.end), (3, 6));
        assert!(!s.command_position);
    }

    #[test]
    fn word_after_a_pipe_is_command_position_again() {
        let s = word_span("cat f | gr", 10);
        assert_eq!(s.start, 8);
        assert!(s.command_position);
    }

    #[test]
    fn word_after_a_semicolon_is_command_position() {
        let s = word_span("cd /; ls", 8);
        assert!(s.command_position);
    }

    #[test]
    fn empty_line_completes_a_command() {
        let s = word_span("", 0);
        assert_eq!((s.start, s.end), (0, 0));
        assert!(s.command_position);
    }

    #[test]
    fn cursor_mid_word_still_spans_the_whole_word() {
        let s = word_span("echo hello", 7);
        assert_eq!((s.start, s.end), (5, 10));
    }

    #[test]
    fn splits_a_path_prefix_at_the_last_slash() {
        assert_eq!(split_path_prefix("/bi"), ("/", "bi"));
        assert_eq!(split_path_prefix("src/ma"), ("src/", "ma"));
        assert_eq!(split_path_prefix("ma"), ("", "ma"));
        assert_eq!(split_path_prefix("/"), ("/", ""));
    }

    #[test]
    fn common_prefix_of_several_candidates() {
        assert_eq!(
            common_prefix(&owned(&["mkfs.fat32", "mkfs.racfs"])),
            "mkfs."
        );
        assert_eq!(common_prefix(&owned(&["cat"])), "cat");
        assert_eq!(common_prefix(&owned(&["cat", "echo"])), "");
        assert_eq!(common_prefix(&[]), "");
    }

    const MOUNTS: &str = "initramfs / initramfs rw 0 0\n\
                          devfs /dev devfs rw 0 0\n\
                          tmpfs /tmp tmpfs rw 0 0\n\
                          proc /proc proc rw 0 0\n\
                          /dev/ram0 /var racfs rw 0 0\n\
                          none /fat fat32 rw 0 0\n\
                          /dev/sda /mnt racfs rw 0 0\n";

    #[test]
    fn mountpoints_complete_under_root() {
        // The case that started this: `/pro` finds nothing by listing `/`,
        // because /proc is a mount, not a directory entry.
        assert_eq!(mountpoints_in(MOUNTS, "/", "pro"), owned(&["proc"]));
        assert_eq!(mountpoints_in(MOUNTS, "/", "m"), owned(&["mnt"]));
    }

    #[test]
    fn root_itself_is_never_a_candidate() {
        // "/" has no parent directory to be completed under.
        let all = mountpoints_in(MOUNTS, "/", "");
        assert!(!all.iter().any(|m| m.is_empty() || m == "/"));
        assert_eq!(all.len(), 6, "six mounts sit directly under /: {:?}", all);
    }

    #[test]
    fn mountpoints_do_not_leak_into_other_directories() {
        assert!(mountpoints_in(MOUNTS, "/mnt/", "").is_empty());
        assert!(mountpoints_in(MOUNTS, "/bin/", "p").is_empty());
    }

    #[test]
    fn builtins_are_offered_in_command_position() {
        // No filesystem involved: an empty PATH exercises the builtin half.
        let got = command_candidates("ex", "", &["exit", "export", "echo"]);
        assert_eq!(got, owned(&["exit", "export"]));
    }
}
