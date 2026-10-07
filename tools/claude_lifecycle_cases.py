"""Bash process lifetimes whose results and effects must match native Claude Code.

Shared by the native-local baseline probe and the packaged remote acceptance.
Each process a command leaves behind beats into its own `lc-<name>.beat` file;
which of them still beat is compared after the sequence, independently of the
process namespace the target runs commands in. A `lc.stop` file ends them all.
`TaskStop` arguments name the most recent task a result reported. `normalize`
removes task identities, output locations and the project root.
"""
from pathlib import Path
import re
import time

STOP = "TaskStop"


def beat(name):
    """A shell loop that beats for about a minute unless told to stop."""
    return (f"(i=0; while [ $i -lt 300 ] && [ ! -e lc.stop ]; do echo $i > lc-{name}.beat; "
            "i=$((i+1)); sleep 0.2; done)")


CASES = [
    ("child_holds_output", ("Bash", {"command": f"{beat('held')} & echo started"})),
    ("child_quiet", ("Bash", {"command": f"{beat('quiet')} >/dev/null 2>&1 & echo started"})),
    ("child_new_session", ("Bash", {"command": f"setsid bash -c '{beat('session')}' >/dev/null 2>&1 </dev/null & echo started"})),
    ("exit_leaves_child", ("Bash", {"command": f"{beat('exit')} & exit 3"})),
    ("exec_replaces_shell", ("Bash", {"command": "exec echo replaced"})),
    ("exit_trap", ("Bash", {"command": "trap 'echo trapped' EXIT; echo body"})),
    ("timeout_kills_tree", ("Bash", {"command": f"sleep 0.1; {beat('timeout')} & sleep 46", "timeout": 1500})),
    ("timeout_moves", ("Bash", {"command": f"{beat('moved')} & wait; echo done", "timeout": 1500})),
    ("stop_moved", (STOP, {})),
    ("background_tree", ("Bash", {"command": f"{beat('background')} & wait", "run_in_background": True})),
    ("stop_background", (STOP, {})),
    ("background_shell_exits", ("Bash", {"command": f"{beat('left')} & echo left", "run_in_background": True})),
    ("pause", ("Bash", {"command": "sleep 2; echo paused"})),
    ("stop_finished", (STOP, {})),
]

TASK_ID = re.compile(r"(?:with ID: |\(ID: )([A-Za-z0-9-]+)")


def task_id(text):
    match = TASK_ID.search(text or "")
    return match.group(1) if match else None


def alive(directory):
    """Which recorded processes still beat, by beat file name."""
    def beats():
        return {path.name: path.read_text() for path in sorted(Path(directory).glob("lc-*.beat"))}
    before = beats()
    time.sleep(1.5)
    after = beats()
    return {name: after.get(name) != value for name, value in before.items()}


def stop_all(directory):
    (Path(directory) / "lc.stop").write_text("stop\n")


def left_running_notified(requests):
    """Whether the model was told the command that left a child running completed."""
    def strings(value):
        if isinstance(value, str):
            yield value
        elif isinstance(value, dict):
            for item in value.values():
                yield from strings(item)
        elif isinstance(value, list):
            for item in value:
                yield from strings(item)
    return any(re.search(r"<task-notification>(?:(?!</task-notification>).)*lc-left\.beat; .*?echo left\" "
                         r"completed \(exit code 0\)", text, re.S)
               for request in requests for text in strings(request.get("messages")))


def normalize(text, project, ids=()):
    """Placement-independent form of one tool result's text."""
    text = re.sub(r"\n*<system-reminder>.*?</system-reminder>\n*", "", text, flags=re.S)
    text = text.replace(str(project), "<project>")
    for identity in ids:
        text = text.replace(identity, "<task>")
    text = re.sub(r"Output is being written to: \S+?\. ", "Output is being written to: <file>. ", text)
    return text
