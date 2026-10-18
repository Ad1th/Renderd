"""Fails when the built site uses words the site must not use.

Checks the visible text of every page in site/public (not /api/, which is
rustdoc): the banned words from the website plan, exclamation marks, em
dashes, and the latency claim that has no benchmark yet.
Usage: python3 site/scripts/check_words.py [site/public]
"""
import pathlib
import re
import sys
from html.parser import HTMLParser

BANNED = [
    "seamless", "seamlessly", "effortless", "effortlessly", "unleash", "supercharge",
    "revolutionize", "revolutionise", "cutting-edge", "next-gen", "blazing", "game-changer",
    "elevate", "empower", "robust", "leverage", "unlock", "look no further", "whether you're",
]
RULES = [(re.compile(r"\b" + re.escape(w) + r"\b", re.I), f'banned word "{w}"') for w in BANNED]
RULES += [
    (re.compile("!"), "exclamation mark"),
    (re.compile("—"), "em dash"),
    (re.compile(r"sub[- ]?16\s?ms", re.I), "unmeasured latency claim"),
]


class Text(HTMLParser):
    """Collects visible text, plus alt, title and meta content."""

    SKIP = {"script", "style", "pre", "code"}

    def __init__(self):
        super().__init__()
        self.depth = 0
        self.parts = []

    def handle_starttag(self, tag, attrs):
        if tag in self.SKIP:
            self.depth += 1
        for k, v in attrs:
            if v and (k in ("alt", "title", "aria-label") or (tag == "meta" and k == "content")):
                self.parts.append(v)

    def handle_endtag(self, tag):
        if tag in self.SKIP and self.depth:
            self.depth -= 1

    def handle_data(self, data):
        if not self.depth:
            self.parts.append(data)


def main():
    root = pathlib.Path(sys.argv[1] if len(sys.argv) > 1 else "site/public")
    pages = [p for p in root.rglob("*.html") if "api" not in p.relative_to(root).parts[:1]]
    if not pages:
        sys.exit(f"no pages found in {root}")
    bad = 0
    for page in sorted(pages):
        t = Text()
        t.feed(page.read_text(encoding="utf-8"))
        for line in " ".join(t.parts).split("\n"):
            for rule, what in RULES:
                for m in rule.finditer(line):
                    bad += 1
                    near = line[max(0, m.start() - 40): m.end() + 40].strip()
                    print(f"{page.relative_to(root)}: {what}: ...{near}...")
    print(f"checked {len(pages)} pages, {bad} problems")
    sys.exit(1 if bad else 0)


if __name__ == "__main__":
    main()
