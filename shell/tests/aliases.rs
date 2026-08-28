// racsh — alias storage and expansion (ROADMAP v0.2 §2.2).
//
// Expansion itself is a pure function over Env, so it is exercised here on the
// host; the `alias` / `unalias` builtins are covered end-to-end by racos-test
// in the QEMU smoke.

use racsh::exec::expand_aliases;
use racsh::expand::Env;

fn words(items: &[&str]) -> Vec<String> {
    items.iter().map(|s| String::from(*s)).collect()
}

fn env_with(aliases: &[(&str, &str)]) -> Env {
    let mut env = Env::new(1);
    for (name, value) in aliases {
        env.set_alias(String::from(*name), String::from(*value));
    }
    env
}

#[test]
fn expands_a_leading_alias_and_keeps_the_arguments() {
    let env = env_with(&[("ll", "ls -la")]);
    assert_eq!(
        expand_aliases(words(&["ll", "/bin"]), &env),
        words(&["ls", "-la", "/bin"])
    );
}

#[test]
fn only_the_command_word_is_expanded() {
    // `ll` in argument position is data, not a command.
    let env = env_with(&[("ll", "ls -la")]);
    assert_eq!(
        expand_aliases(words(&["echo", "ll"]), &env),
        words(&["echo", "ll"])
    );
}

#[test]
fn expansion_recurses_through_chained_aliases() {
    let env = env_with(&[("ll", "ls -la"), ("la", "ll")]);
    assert_eq!(expand_aliases(words(&["la"]), &env), words(&["ls", "-la"]));
}

#[test]
fn self_referential_alias_terminates() {
    // The idiom `alias ls='ls --color'` must expand exactly once. Expanding it
    // again would loop forever.
    let env = env_with(&[("ls", "ls --color")]);
    assert_eq!(
        expand_aliases(words(&["ls", "/tmp"]), &env),
        words(&["ls", "--color", "/tmp"])
    );
}

#[test]
fn mutually_recursive_aliases_terminate() {
    let env = env_with(&[("a", "b"), ("b", "a")]);
    let out = expand_aliases(words(&["a"]), &env);
    assert!(
        out == words(&["a"]) || out == words(&["b"]),
        "got {:?}",
        out
    );
}

#[test]
fn empty_alias_drops_the_command_word() {
    let env = env_with(&[("nope", "")]);
    assert_eq!(expand_aliases(words(&["nope", "x"]), &env), words(&["x"]));
}

#[test]
fn unknown_command_is_left_alone() {
    let env = env_with(&[("ll", "ls -la")]);
    assert_eq!(
        expand_aliases(words(&["cat", "f"]), &env),
        words(&["cat", "f"])
    );
}

#[test]
fn lookup_set_and_remove_round_trip() {
    let mut env = Env::new(1);
    assert_eq!(env.lookup_alias("ll"), None);

    env.set_alias(String::from("ll"), String::from("ls -la"));
    assert_eq!(env.lookup_alias("ll"), Some("ls -la"));

    // Redefinition replaces rather than duplicating.
    env.set_alias(String::from("ll"), String::from("ls -l"));
    assert_eq!(env.lookup_alias("ll"), Some("ls -l"));
    assert_eq!(env.aliases().count(), 1);

    assert!(env.remove_alias("ll"));
    assert!(!env.remove_alias("ll"));
    assert_eq!(env.lookup_alias("ll"), None);
}

#[test]
fn listing_is_sorted_by_name() {
    let env = env_with(&[("zz", "1"), ("aa", "2"), ("mm", "3")]);
    let names: Vec<&str> = env.aliases().map(|(n, _)| n).collect();
    assert_eq!(names, vec!["aa", "mm", "zz"]);
}

#[test]
fn aliases_survive_into_command_substitution() {
    let env = env_with(&[("ll", "ls -la")]);
    let sub = env.clone_for_sub();
    assert_eq!(sub.lookup_alias("ll"), Some("ls -la"));
}
