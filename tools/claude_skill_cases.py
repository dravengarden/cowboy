"""Skill and custom command cases whose results must match native Claude Code.

Shared by the native-local baseline probe and the packaged remote acceptance.
The fixture is a project's .claude directory. `normalize` replaces the project
root and drops native's token reminder; `listing` keeps the fixture's lines of
native's skill listing.
"""
import json
import re

FILES = {
    "skills/basic/SKILL.md": "---\ndescription: basic fixture skill\n---\n"
    "BASIC dir=${CLAUDE_SKILL_DIR} project=${CLAUDE_PROJECT_DIR} args=$ARGUMENTS first=$1\n",
    "skills/basic/helper.txt": "helper\n",
    "skills/shell-ok/SKILL.md": "---\ndescription: shell ok\n---\nOut: !`echo hello`\n",
    "skills/shell-stderr/SKILL.md": "---\ndescription: shell stderr\n---\nOut: !`echo to-err >&2`\n",
    "skills/shell-both/SKILL.md": "---\ndescription: shell both\n---\nOut: !`echo a; echo b >&2`\n",
    "skills/shell-fail/SKILL.md": "---\ndescription: shell fail\n---\nOut: !`echo partial; exit 3`\n",
    "skills/shell-fail-empty/SKILL.md": "---\ndescription: shell fail empty\n---\nOut: !`exit 1`\n",
    "skills/shell-fenced/SKILL.md": "---\ndescription: shell fenced\n---\nFenced:\n```!\necho one\necho two\n```\nEnd\n",
    "skills/shell-args/SKILL.md": "---\ndescription: shell args\n---\nOut: !`echo got $ARGUMENTS`\n",
    "skills/shell-two/SKILL.md": "---\ndescription: shell two\n---\nA !`echo first` B !`echo second`\n",
    "skills/shell-inert/SKILL.md": "---\ndescription: shell inert\n---\nInline `code` and a!`notrun` and [!`true`]\n",
    "skills/shell-pwd/SKILL.md": "---\ndescription: shell pwd\n---\nOut: !`pwd`\n",
    "skills/shell-cd/SKILL.md": "---\ndescription: shell cd\n---\nOut: !`cd / && pwd`\n",
    "skills/shell-file/SKILL.md": "---\ndescription: shell file\n---\nOut: !`cat ${CLAUDE_SKILL_DIR}/data.txt`\n",
    "skills/shell-file/data.txt": "skill data\n",
    "skills/renamed-dir/SKILL.md": "---\nname: renamed-alias\ndescription: renamed fixture\n---\nRENAMED\n",
    "skills/hidden/SKILL.md": "---\ndescription: hidden fixture\ndisable-model-invocation: true\n---\nHIDDEN\n",
    "skills/no-frontmatter/SKILL.md": "First line describes it.\nNO FRONTMATTER\n",
    "skills/shared/SKILL.md": "---\ndescription: project shared\n---\nPROJECT SHARED\n",
    "commands/pcmd.md": "---\ndescription: project command\n---\nPROJECT CMD $ARGUMENTS\n",
    "commands/grp/inner.md": "---\ndescription: grouped command\n---\nGROUPED $ARGUMENTS\n",
}

# The user's own, in the user's Claude directory: a user skill shadows the
# project's of the same name.
USER_FILES = {
    "skills/shared/SKILL.md": "---\ndescription: user shared\n---\nUSER SHARED dir=${CLAUDE_SKILL_DIR}\n",
    "skills/user-only/SKILL.md": "---\ndescription: user only\n---\nUSER ONLY\n",
    "commands/ucmd.md": "---\ndescription: user command\n---\nUSER CMD $ARGUMENTS\n",
}

NAMES = ["shared", "user-only", "ucmd", "basic", "shell-ok", "shell-stderr", "shell-both", "shell-fail", "shell-fail-empty", "shell-fenced",
         "shell-args", "shell-two", "shell-inert", "shell-pwd", "shell-cd", "shell-file", "renamed-dir", "hidden",
         "no-frontmatter", "pcmd", "grp:inner"]

CASES = [
    ("basic", {"skill": "basic", "args": "one two"}),
    ("basic_again", {"skill": "basic"}),
    ("shell_ok", {"skill": "shell-ok"}),
    ("shell_stderr", {"skill": "shell-stderr"}),
    ("shell_both", {"skill": "shell-both"}),
    ("shell_fail", {"skill": "shell-fail"}),
    ("shell_fail_empty", {"skill": "shell-fail-empty"}),
    ("shell_fenced", {"skill": "shell-fenced"}),
    ("shell_args", {"skill": "shell-args", "args": "X1 X2"}),
    ("shell_two", {"skill": "shell-two"}),
    ("shell_inert", {"skill": "shell-inert"}),
    ("shell_pwd", {"skill": "shell-pwd"}),
    ("shell_cd", {"skill": "shell-cd"}),
    ("shell_file", {"skill": "shell-file"}),
    ("alias", {"skill": "renamed-alias"}),
    ("alias_dir_name", {"skill": "renamed-dir"}),
    ("hidden", {"skill": "hidden"}),
    ("no_frontmatter", {"skill": "no-frontmatter"}),
    ("unknown", {"skill": "no-such-skill"}),
    ("command", {"skill": "pcmd", "args": "c1"}),
    ("grouped_command", {"skill": "grp:inner", "args": "g1"}),
    ("slash_prefixed", {"skill": "/shell-ok"}),
    ("user_shadows_project", {"skill": "shared"}),
    ("user_only", {"skill": "user-only"}),
    ("user_command", {"skill": "ucmd", "args": "u1"}),
]


def setup(project, user):
    """The fixture in a project and the user's Claude directory."""
    for root, files in [(project / ".claude", FILES), (user, USER_FILES)]:
        for name, text in files.items():
            path = root / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(text)


def normalize(value, root, user=None):
    text = value if isinstance(value, str) else json.dumps(value, ensure_ascii=False)
    if user is not None:
        text = text.replace(str(user), "<USER>")
    text = text.replace(str(root), "<ROOT>")
    # Native appends its reminders (tokens left, task list) to a tool result.
    return re.sub(r"\n*<system-reminder>\n.*?\n</system-reminder>\n*", "", text, flags=re.S)


def contents(message, root, user=None):
    """The texts that follow a tool result in its message (the skill's own)."""
    content = message.get("content")
    return [normalize(block["text"], root, user) for block in content if isinstance(content, list)
            and block.get("type") == "text" and not block["text"].startswith("<system-reminder>")]


def listing(text):
    """The fixture's entries of native's skill listing, in order."""
    lines = []
    for line in text.split("\n"):
        match = re.match(r"- ([^:\s]+(?::[^:\s]+)*)(?:: |$)", line)
        if match and match[1] in NAMES:
            lines.append(line)
    return lines
