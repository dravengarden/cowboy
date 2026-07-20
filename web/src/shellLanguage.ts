export const SHELL_SYNTAX_LANGUAGE = "cowboy-shell";

// Bash starts a comment only where a new word can begin. Keeping this pattern
// shared makes the Prism grammar's boundary semantics independently testable.
export const SHELL_COMMENT_PATTERN = /(^|[\t ;|&()])#.*/m;
