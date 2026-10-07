"""Bash cases whose model-visible results must match native Claude Code.

Shared by the native-local baseline probe and the packaged remote acceptance.
The calls run in order within one turn, so shell state carries between them as
it does natively. `normalize` removes what legitimately differs by placement:
the project root, the shell path and native's persisted output location.
"""
import re

CASES = [
    ("echo", {"command": "echo hi"}),
    ("no_output", {"command": "true"}),
    ("no_newline", {"command": "printf 'no newline'"}),
    ("streams", {"command": "echo out1; echo err1 >&2; echo out2"}),
    ("stderr_only", {"command": "echo only-err >&2"}),
    ("fail_streams", {"command": "echo out1; echo err1 >&2; exit 7"}),
    ("fail_silent", {"command": "exit 4"}),
    ("trailing_blank_lines", {"command": "printf 'a\\n\\n\\n'"}),
    ("leading_blank_lines", {"command": "printf '\\n\\nb\\n'"}),
    ("inner_blank_lines", {"command": "printf '  a\\n\\n  b  \\n'"}),
    ("cd_sub", {"command": "cd sub && pwd"}),
    ("pwd_after_cd", {"command": "pwd"}),
    ("cd_outside", {"command": "cd / && pwd"}),
    ("pwd_after_outside", {"command": "pwd"}),
    ("export", {"command": "export PROBE_VAR=set-in-shell; echo $PROBE_VAR"}),
    ("env_after_export", {"command": "echo \"var=${PROBE_VAR:-unset}\""}),
    ("function_after", {"command": "probe_fn() { echo fn; }; probe_fn; bash -c 'type probe_fn' >/dev/null 2>&1 || echo child-unset"}),
    ("stdin", {"command": "cat; echo after-cat"}),
    ("tty", {"command": "test -t 0 && echo stdin-tty || echo stdin-not-tty; test -t 1 && echo stdout-tty || echo stdout-not-tty"}),
    ("pipefail", {"command": "false | true; echo pipe=$?"}),
    ("set_e", {"command": "set -e; false; echo not-reached"}),
    ("nul", {"command": "printf 'a\\0b\\n'"}),
    ("ansi", {"command": "printf '\\033[31mred\\033[0m\\n'"}),
    ("cr", {"command": "printf 'one\\rtwo\\n'"}),
    ("invalid_utf8", {"command": "printf 'x\\377y\\n'"}),
    ("long_lines", {"command": "seq 1 20000"}),
    ("long_line", {"command": "head -c 40000 /dev/zero | tr '\\0' x; echo"}),
    ("env_vars", {"command": "env | cut -d= -f1 | grep -E '^(CLAUDECODE|CLAUDE_CODE_CHILD_SESSION|CLAUDE_CODE_ENTRYPOINT|CLAUDE_CODE_SESSION_ATTENDED|COREPACK_ENABLE_AUTO_PIN|GIT_EDITOR|CLAUDE_PROJECT_DIR|NO_COLOR|FORCE_COLOR)$' | sort"}),
    ("env_values", {"command": "echo \"$CLAUDECODE $CLAUDE_CODE_CHILD_SESSION $CLAUDE_CODE_SESSION_ATTENDED $COREPACK_ENABLE_AUTO_PIN $GIT_EDITOR $CLAUDE_CODE_ENTRYPOINT ${CLAUDE_CODE_SESSION_ID:+session} ${CLAUDE_EFFORT:+effort} ${AI_AGENT:+agent}\""}),
    ("cd_then_fail", {"command": "cd sub; false"}),
    ("pwd_after_failed_cd", {"command": "pwd"}),
    ("fail_large", {"command": "head -c 31000 /dev/zero | tr '\\0' z; exit 2"}),
    ("reset_large", {"command": "cd / && seq 1 20000"}),
    ("size_30001", {"command": "head -c 30001 /dev/zero | tr '\\0' y"}),
    ("description_only", {"command": "echo described", "description": "Print a word"}),
    ("umask", {"command": "umask"}),
]


def normalize(text, project, shell_paths=()):
    """Placement-independent form of one tool result's text."""
    text = re.sub(r"\n*<system-reminder>.*?</system-reminder>\n*", "", text, flags=re.S)
    text = text.replace(str(project), "<project>")
    for path in shell_paths:
        text = text.replace(path, "<shell>")
    text = re.sub(r"Full output saved to: \S+", "Full output saved to: <file>", text)
    return text
