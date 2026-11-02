#!/usr/bin/env python3
"""Diff the compiled UN calendar against one page capture; never publish it."""

import argparse
from collections import Counter
from datetime import date
from html.parser import HTMLParser
import json
from pathlib import Path
import re
import sys
from urllib.request import Request, urlopen

URL = "https://www.un.org/en/observances/list-days-weeks"
MONTHS = "January February March April May June July August September October November December".split()
SHORT_MONTHS = "Jan Feb Mar Apr May Jun Jul Aug Sep Oct Nov Dec".split()
DEFAULT_FILE = Path(__file__).resolve().parents[1] / "assets/un-observances.json"


class Node:
    def __init__(self, tag="", attrs=()):
        self.tag = tag
        self.attrs = dict(attrs)
        self.children = []

    def has_class(self, name):
        return name in self.attrs.get("class", "").split()

    def descendants(self):
        for child in self.children:
            if isinstance(child, Node):
                yield child
                yield from child.descendants()

    def text(self):
        return "".join(c.text() if isinstance(c, Node) else c for c in self.children)


class Page(HTMLParser):
    def __init__(self):
        super().__init__(convert_charrefs=True)
        self.root = Node()
        self.stack = [self.root]

    def handle_starttag(self, tag, attrs):
        node = Node(tag, attrs)
        self.stack[-1].children.append(node)
        if tag not in {"area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "param", "source", "track", "wbr"}:
            self.stack.append(node)

    def handle_startendtag(self, tag, attrs):
        self.stack[-1].children.append(Node(tag, attrs))

    def handle_endtag(self, tag):
        for i in range(len(self.stack) - 1, 0, -1):
            if self.stack[i].tag == tag:
                del self.stack[i:]
                break

    def handle_data(self, data):
        self.stack[-1].children.append(data)


def clean(text):
    return " ".join(text.split())


def field(row, name, required=True):
    fields = [n for n in row.descendants() if n.has_class(name)]
    if len(fields) != 1:
        if not required and not fields:
            return None
        raise ValueError(f"expected one {name}, got {len(fields)}")
    return fields[0]


def parse_page(html):
    # Discard chrome before January, including any unrelated view rows.
    start = re.search(r"<h3\b[^>]*>\s*January\s*</h3>", html)
    if start is None:
        raise ValueError("missing January heading")
    parser = Page()
    parser.feed(html[start.start():])
    parser.close()
    month = 0
    entries = []
    for node in parser.root.descendants():
        if node.tag == "h3":
            heading = clean(node.text())
            if heading in MONTHS:
                if month >= 12 or heading != MONTHS[month]:
                    raise ValueError(f"month heading out of order: {heading}")
                month += 1
            elif month == 12:
                break  # The footer follows the last December row.
            else:
                raise ValueError(f"unexpected heading: {heading}")
        if not node.has_class("views-row"):
            continue
        title = field(field(node, "views-field-title"), "field-content")
        name = clean(title.text())
        if not name:
            raise ValueError("empty observance name")
        resolutions_field = field(node, "views-field-field-url", required=False)
        resolutions = "" if resolutions_field is None else clean(field(resolutions_field, "field-content").text())
        if resolutions:
            if not (resolutions.startswith("(") and resolutions.endswith(")")):
                raise ValueError(f"resolution lacks outer parentheses: {resolutions}")
            resolutions = resolutions[1:-1].strip()
        event = field(field(node, "views-field-field-event-date-1"), "date-display-single")
        stamp = event.attrs.get("content", "")
        if not re.match(r"^\d{4}-\d{2}-\d{2}(?:T|$)", stamp):
            raise ValueError(f"invalid event date: {stamp}")
        parsed = date.fromisoformat(stamp[:10])
        expected = f"{parsed.day:02} {SHORT_MONTHS[parsed.month - 1]}"
        if parsed.month != month or clean(event.text()) != expected:
            raise ValueError(f"date disagrees with month or display: {stamp}, {event.text()}")
        entries.append(dict(month=month, day=parsed.day, name=name, resolutions=resolutions))
    if month != 12 or len(entries) < 200:
        raise ValueError(f"incomplete calendar: {month} months, {len(entries)} entries")
    return entries


def serialize(entries):
    rows = ["    " + json.dumps(e, ensure_ascii=False) for e in entries]
    return '{\n  "source": ' + json.dumps(URL) + ',\n  "entries": [\n' + ",\n".join(rows) + "\n  ]\n}\n"


def key(entry):
    return tuple(entry[k] for k in ("month", "day", "name", "resolutions"))


def line(entry):
    text = entry["name"]
    if entry["resolutions"]:
        text += f" ({entry['resolutions']})"
    return f"{entry['month']}/{entry['day']} {text}"


def main(argv=None):
    args = argparse.ArgumentParser(description=__doc__)
    args.add_argument("--page", type=Path)
    args.add_argument("--file", type=Path, default=DEFAULT_FILE)
    args.add_argument("--write", action="store_true")
    options = args.parse_args(argv)
    try:
        if options.page is not None:
            html = options.page.read_text(encoding="utf-8")
        else:
            request = Request(URL, headers={"User-Agent": "plumage-check/0 (+https://github.com/proof-computer/plumage)"})
            with urlopen(request, timeout=60) as response:
                html = response.read().decode("utf-8")
        entries = parse_page(html)
        if options.write:
            options.file.write_text(serialize(entries), encoding="utf-8")
            return 0
        committed = json.loads(options.file.read_text(encoding="utf-8"))
        if committed["source"] != URL:
            raise ValueError("calendar source does not match URL")
        old = committed["entries"]
        if entries == old:
            dates = len({(e["month"], e["day"]) for e in entries})
            print(f"no drift ({len(entries)} entries on {dates} dates)")
            return 0
        removed = Counter(map(key, old)) - Counter(map(key, entries))
        added = Counter(map(key, entries)) - Counter(map(key, old))
        for prefix, rows, counts in [("-", old, removed.copy()), ("+", entries, added.copy())]:
            for entry in rows:
                identity = key(entry)
                if counts[identity] > 0:
                    print(f"{prefix} {line(entry)}")
                    counts[identity] -= 1
        if not removed and not added:
            print("order changed")
        print(f"drift: {removed.total()} removed, {added.total()} added")
        return 1
    except (OSError, ValueError, KeyError, TypeError) as error:
        print(f"UN observances: {error}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    sys.exit(main())
