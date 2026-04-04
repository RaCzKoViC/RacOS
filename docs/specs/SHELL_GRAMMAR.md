# RacOS — Shell Grammar Specification (racsh)

> Version: 0.1.0 | Status: Draft | Component: racsh

## 1. Architecture

```
Input → Lexer → Parser → AST → SemanticValidation → Expansion → ExecutionPlan → Runtime
```

Each stage is an independent module. The parser never executes code. Expansion is separate from lexing.

## 2. Token Types

| Token | Examples |
|-------|---------|
| WORD | `hello`, `foo123`, `/usr/bin/ls` |
| ASSIGNMENT_WORD | `FOO=bar` (word containing unquoted `=` before first char) |
| NUMBER | `2` (in redirect context only) |
| NEWLINE | `\n` |
| PIPE | `\|` |
| AND_IF | `&&` |
| OR_IF | `\|\|` |
| SEMI | `;` |
| AMP | `&` |
| LESS | `<` |
| GREAT | `>` |
| DGREAT | `>>` |
| LESSAND | `<&` |
| GREATAND | `>&` |
| LPAREN | `(` |
| RPAREN | `)` |
| LBRACE | `{` |
| RBRACE | `}` |
| IF / THEN / ELSE / ELIF / FI | keywords |
| WHILE / DO / DONE | keywords |
| FOR / IN | keywords |
| CASE / ESAC / DSEMI | keywords |
| FUNCTION | keyword |
| SINGLE_QUOTED | `'...'` |
| DOUBLE_QUOTED | `"..."` |
| BACKQUOTE | `` `...` `` |
| DOLLAR_PAREN | `$(...)` |
| DOLLAR_BRACE | `${...}` |
| DOLLAR_VAR | `$VAR` |
| GLOB_STAR | `*` |
| GLOB_QUESTION | `?` |
| GLOB_BRACKET | `[...]` |
| COMMENT | `# ...` |

## 3. Grammar (BNF-like)

```
program         = linebreak complete_commands linebreak

complete_commands = complete_command (newline_list complete_command)*

complete_command = list separator_op?

list            = and_or (separator_op and_or)*

and_or          = pipeline (AND_IF pipeline | OR_IF pipeline)*

pipeline        = command (PIPE command)*

command         = simple_command
                | compound_command redirect_list?
                | function_def

simple_command  = cmd_prefix? WORD cmd_suffix?

cmd_prefix      = (ASSIGNMENT_WORD | io_redirect)+

cmd_suffix      = (WORD | io_redirect)+

compound_command = brace_group
                 | subshell
                 | if_clause
                 | while_clause
                 | for_clause
                 | case_clause

subshell        = LPAREN complete_commands RPAREN

brace_group     = LBRACE complete_commands RBRACE

if_clause       = IF complete_commands THEN complete_commands
                  (ELIF complete_commands THEN complete_commands)*
                  (ELSE complete_commands)?
                  FI

while_clause    = WHILE complete_commands DO complete_commands DONE

for_clause      = FOR WORD (IN WORD*)? DO complete_commands DONE

case_clause     = CASE WORD IN case_item* ESAC

case_item       = pattern (PIPE pattern)* RPAREN complete_commands DSEMI

function_def    = WORD LPAREN RPAREN compound_command

io_redirect     = io_number? (LESS | GREAT | DGREAT | LESSAND | GREATAND) WORD

io_number       = NUMBER

separator_op    = SEMI | AMP | NEWLINE

redirect_list   = io_redirect+
```

## 4. AST Node Types

```rust
enum AstNode {
    SimpleCommand {
        assignments: Vec<Assignment>,
        words: Vec<Word>,
        redirects: Vec<Redirect>,
    },
    Pipeline {
        commands: Vec<AstNode>,
        bang: bool,            // negated pipeline
    },
    Sequence {
        left: Box<AstNode>,
        right: Box<AstNode>,
        op: SequenceOp,        // Semi, Amp (background)
    },
    And {
        left: Box<AstNode>,
        right: Box<AstNode>,
    },
    Or {
        left: Box<AstNode>,
        right: Box<AstNode>,
    },
    Subshell {
        body: Box<AstNode>,
        redirects: Vec<Redirect>,
    },
    BraceGroup {
        body: Box<AstNode>,
        redirects: Vec<Redirect>,
    },
    If {
        condition: Box<AstNode>,
        then_body: Box<AstNode>,
        elif_parts: Vec<(AstNode, AstNode)>,
        else_body: Option<Box<AstNode>>,
    },
    While {
        condition: Box<AstNode>,
        body: Box<AstNode>,
    },
    For {
        var: String,
        words: Option<Vec<Word>>,
        body: Box<AstNode>,
    },
    Case {
        word: Word,
        items: Vec<CaseItem>,
    },
    FunctionDef {
        name: String,
        body: Box<AstNode>,
    },
    Assignment {
        name: String,
        value: Word,
    },
    Redirect {
        fd: Option<i32>,
        op: RedirectOp,
        target: Word,
    },
}
```

## 5. Quoting Rules

| Syntax | Behavior |
|--------|----------|
| `'...'` | All characters literal, no expansion |
| `"..."` | Variable expansion (`$VAR`), command substitution (`$(...)`), escape `\\` `\"` `\$` `\`` |
| `\x` | Next character literal (except in single quotes) |

## 6. Expansion Order

Applied in this order after parsing, before execution:
1. **Brace expansion** (post-MVP)
2. **Tilde expansion** (`~` → home directory)
3. **Parameter expansion** (`$VAR`, `${VAR}`, `${VAR:-default}`)
4. **Command substitution** (`$(cmd)`)
5. **Arithmetic expansion** (`$((expr))`) — post-MVP
6. **Word splitting** (on unquoted results of expansions)
7. **Pathname expansion / globbing** (`*`, `?`, `[...]`)
8. **Quote removal**

## 7. Builtins

| Builtin | Purpose | Must be in-process |
|---------|---------|-------------------|
| `cd` | Change directory | Yes (affects shell state) |
| `pwd` | Print working directory | Yes |
| `export` | Set environment variable | Yes |
| `unset` | Unset variable | Yes |
| `alias` | Define alias | Yes |
| `unalias` | Remove alias | Yes |
| `set` | Set shell options | Yes |
| `exit` | Exit shell | Yes |
| `jobs` | List background jobs | Yes |
| `fg` | Bring job to foreground | Yes |
| `bg` | Resume job in background | Yes |
| `kill` | Send signal | No (could be external) |
| `history` | Show command history | Yes |
| `source` | Execute file in current shell | Yes |

## 8. Execution Model

### 8.1 Simple Commands (external)

1. Expand words
2. Search PATH for executable
3. `sys_spawn` (fork+exec equivalent)
4. Parent: if foreground, `sys_wait`; if background (`&`), add to job table

### 8.2 Pipelines

1. Create pipes: `sys_pipe` for each `|`
2. Spawn each command, connecting stdin/stdout through pipes
3. Wait for all commands in pipeline
4. Exit status = exit status of last command (or first failed if `pipefail`)

### 8.3 Job Control

- Foreground job: shell waits
- Background job (`&`): shell continues, job added to table
- `Ctrl-C` → SIGINT to foreground process group
- `Ctrl-Z` → SIGSTOP to foreground process group, shell resumes
- `fg %N` → bring job N to foreground (SIGCONT)
- `bg %N` → resume job N in background (SIGCONT)

## 9. Error Handling

| Scenario | Behavior |
|----------|----------|
| Syntax error | Print error with line/column, return status 2 |
| Command not found | Print error, return status 127 |
| Permission denied | Print error, return status 126 |
| Expansion error | Print error, abort command (not whole script unless `set -e`) |
| Pipe write to closed read end | SIGPIPE to writer |

## 10. Script Mode vs Interactive Mode

| Feature | Script | Interactive |
|---------|--------|-------------|
| History | No | Yes |
| Prompt | No | Yes (PS1/PS2) |
| Job control | Optional | Yes |
| `set -e` | Common | Not default |
| Aliases | Loaded from file | Yes |
| Tab completion | No | Yes |

## 11. Test Plan

- Golden tests for lexer: token sequences for known inputs
- Golden tests for parser: AST structure for known inputs
- Quoting edge cases: nested quotes, escaped quotes, empty strings
- Pipeline tests: 2-stage, 3-stage, with redirects
- Redirect tests: stdout, stderr, file, append, dup
- Job control: bg/fg, Ctrl-C, Ctrl-Z
- Variable expansion: simple, default, unset
- Globbing: *, ?, [...], no match
- Script regression suite: collection of scripts with expected output
- Fuzzing: random token sequences, malformed inputs
