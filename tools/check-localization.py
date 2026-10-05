#!/usr/bin/env python3
"""Check that every localizable string in the Mac and Linux apps has a zh-Hans translation.

The app's copy is keyed in English: SwiftUI literals (Text, Label, Button, Toggle,
LabeledContent) and String(localized:) calls. English needs no table because the
key is the English text; app/Localization/zh-Hans.lproj/Localizable.strings maps
each key to Chinese. This script extracts the keys from the Swift sources the way
the compiler builds them and reports keys missing from the table, table entries no
source uses any more, and translations whose placeholders don't match their key.

Source keys are matched by placeholder count only: the extractor can't tell
whether `\\(x)` is an Int (%lld) or a String (%@), so the table author picks the
specifier in the table key. Each translation must then take the same specifier
types in the same argument order as its key (positional `%2$@` forms are ordered
by index), because a mismatch makes the runtime lookup miss and silently show
English.

The Linux app (desktop/src) shares the table: its keys are the first argument of
`tr("...")`, written already in table form (`%@`, `%lld`).

Usage: tools/check-localization.py   (exit 1 on any problem)
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
SOURCES = [ROOT / "app/Sources/VibeBuddy", ROOT / "app/Sources/VibeBuddyCore"]
RUST_SOURCES = ROOT / "desktop/src"
TABLE = ROOT / "app/Localization/zh-Hans.lproj/Localizable.strings"

# SwiftUI initialisers whose first argument is a LocalizedStringKey.
VIEW_CALLS = ("Text(", "Label(", "Button(", "Toggle(", "LabeledContent(")
# Keys that read the same in every language.
UNTRANSLATED = {"App", "daemon", "—", "›"}
PLACEHOLDER = "\0"
SPECIFIER = re.compile(r"%(?:\d+\$)?(?:@|lld|ld|d|lf|f)")
SPECIFIER_PARTS = re.compile(r"%(?:(\d+)\$)?(@|lld|ld|d|lf|f)")


def read_literal(code: str, start: int) -> tuple[str, int]:
    """Parse the Swift string literal opening at code[start]; return (key, end).

    Interpolations become PLACEHOLDER, and nested literals inside them are skipped.
    """
    assert code[start] == '"'
    out: list[str] = []
    i = start + 1
    while code[i] != '"':
        if code[i] == "\\":
            nxt = code[i + 1]
            if nxt == "(":
                depth, i = 1, i + 2
                while depth:
                    if code[i] == '"':
                        _, i = read_literal(code, i)
                        continue
                    depth += {"(": 1, ")": -1}.get(code[i], 0)
                    i += 1
                out.append(PLACEHOLDER)
                continue
            out.append({"n": "\n", "t": "\t", '"': '"', "\\": "\\"}.get(nxt, nxt))
            i += 2
            continue
        out.append(code[i])
        i += 1
    return "".join(out), i + 1


def first_argument_literals(code: str, start: int, whole_list: bool = False) -> list[str]:
    """Literals in the first argument of a call whose '(' is at code[start - 1].

    With whole_list, collect every element of the bracket that opens at code[start - 1].
    """
    literals, depth, i = [], 0, start
    while i < len(code):
        ch = code[i]
        if ch == '"':
            text, i = read_literal(code, i)
            if depth == 0:
                literals.append(text)
            continue
        if ch in "([{":
            depth += 1
        elif ch in ")]}":
            if depth == 0:
                break
            depth -= 1
        elif ch == "," and depth == 0 and not whole_list:
            break
        i += 1
    return literals


def strip_comments(code: str) -> str:
    """Blank out // comments while leaving string literals (and URLs in them) alone."""
    out, i = [], 0
    while i < len(code):
        if code[i] == '"':
            _, end = read_literal(code, i)
            out.append(code[i:end])
            i = end
        elif code.startswith("//", i):
            while i < len(code) and code[i] != "\n":
                i += 1
        else:
            out.append(code[i])
            i += 1
    return "".join(out)


def source_keys() -> dict[str, str]:
    """Normalised key -> where it was first seen."""
    keys: dict[str, str] = {}
    for directory in SOURCES:
        for path in sorted(directory.glob("*.swift")):
            code = strip_comments(path.read_text(encoding="utf-8"))
            found: list[tuple[int, str]] = []
            for match in re.finditer(r'String\(localized:\s*"', code):
                found.append((match.start(), read_literal(code, match.end() - 1)[0]))
            for call in VIEW_CALLS:
                for match in re.finditer(r"(?<![\w.])" + re.escape(call), code):
                    if code.startswith("verbatim:", match.end()):
                        continue  # Text(verbatim:) is shown as written, never looked up
                    for text in first_argument_literals(code, match.end()):
                        found.append((match.start(), text))
            for match in re.finditer(r"\[LocalizedStringKey\]\s*=\s*\[", code):
                found.extend((match.start(), t) for t in first_argument_literals(code, match.end(), whole_list=True))
            for offset, text in found:
                if not text.replace(PLACEHOLDER, "").strip() or text in UNTRANSLATED:
                    continue
                line = code.count("\n", 0, offset) + 1
                keys.setdefault(text, f"{path.relative_to(ROOT)}:{line}")
    for path in sorted(RUST_SOURCES.glob("*.rs")):
        code = path.read_text(encoding="utf-8")
        for match in re.finditer(r'(?<![\w.])tr\(\s*"((?:[^"\\]|\\.)*)"', code):
            text = SPECIFIER.sub(PLACEHOLDER, unescape(match.group(1)))
            line = code.count("\n", 0, match.start()) + 1
            keys.setdefault(text, f"{path.relative_to(ROOT)}:{line}")
    return keys


def unescape(text: str) -> str:
    return re.sub(r'\\(.)', lambda m: {"n": "\n", "t": "\t"}.get(m.group(1), m.group(1)), text)


def table_entries() -> dict[str, str]:
    body = TABLE.read_text(encoding="utf-8")
    body = re.sub(r"/\*.*?\*/", "", body, flags=re.S)
    literal = r'"((?:[^"\\]|\\.)*)"'
    entries = {}
    for key, value in re.findall(literal + r"\s*=\s*" + literal + r"\s*;", body):
        entries[unescape(key)] = unescape(value)
    return entries


def argument_types(text: str) -> list[str]:
    """Specifier types in argument order; `%2$@` counts as the second argument."""
    slots = []
    for position, (index, kind) in enumerate(SPECIFIER_PARTS.findall(text), start=1):
        slots.append((int(index) if index else position, kind))
    return [kind for _, kind in sorted(slots)]


def main() -> int:
    keys = source_keys()
    table = table_entries()
    normalised = {SPECIFIER.sub(PLACEHOLDER, key): key for key in table}
    problems = []
    for key, where in sorted(keys.items(), key=lambda item: item[1]):
        if key not in normalised:
            problems.append(f"missing zh-Hans translation ({where}): {key.replace(PLACEHOLDER, '%@')!r}")
    for norm, key in normalised.items():
        if norm not in keys:
            problems.append(f"unused table entry: {key!r}")
        if argument_types(key) != argument_types(table[key]):
            problems.append(
                f"placeholders differ: {key!r} takes {argument_types(key)}, "
                f"translation {table[key]!r} takes {argument_types(table[key])}"
            )
    for problem in problems:
        print(problem, file=sys.stderr)
    if problems:
        print(f"{len(problems)} localization problem(s) in {TABLE.relative_to(ROOT)}", file=sys.stderr)
        return 1
    print(f"localization ok: {len(keys)} keys")
    return 0


if __name__ == "__main__":
    sys.exit(main())
