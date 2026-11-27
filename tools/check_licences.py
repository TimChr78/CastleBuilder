#!/usr/bin/env python3
"""Validate the Keepwright licence manifest and keep docs/CREDITS.md in sync.

Zero third-party dependencies: only the Python 3 standard library is used.

Usage:
    python3 tools/check_licences.py --check
    python3 tools/check_licences.py --write-credits

``--check`` validates every manifest entry and fails (exit 1) if any policy is
broken or if docs/CREDITS.md differs from the generated content.
``--write-credits`` performs the same validation and then regenerates
docs/CREDITS.md. It refuses to write when validation fails.
"""

from __future__ import annotations

import argparse
import json
import re
import sys
from pathlib import Path
from typing import Any

MANIFEST_REL = Path("assets-src/LICENCE_MANIFEST.json")
CREDITS_REL = Path("docs/CREDITS.md")
SCHEMA_VERSION = 1

REQUIRED_FIELDS = (
    "path",
    "kind",
    "source",
    "author",
    "licence",
    "url",
    "shipped",
    "ai_generated",
)

KIND_ORDER = (
    "procedural_shader",
    "procedural_audio",
    "data_pack",
    "texture",
    "model",
    "audio",
)

KIND_LABELS = {
    "procedural_shader": "Procedural shaders",
    "procedural_audio": "Procedural audio",
    "data_pack": "Data packs",
    "texture": "Textures",
    "model": "Models",
    "audio": "Audio",
}

APPROVED_LICENCES = ("MIT", "CC0-1.0", "CC-BY-4.0", "self-made")
APPROVED_LICENCE_SET = set(APPROVED_LICENCES)


def is_non_commercial(licence: str) -> bool:
    """Return True for NC licence strings such as CC-BY-NC-4.0."""
    upper = licence.upper()
    tokens = re.split(r"[^A-Z0-9]+", upper)
    if "NC" in tokens:
        return True
    return "NONCOMMERCIAL" in "".join(tokens)


def _require_string(entry: dict[str, Any], field: str, index: int, errors: list[str]) -> str:
    value = entry.get(field)
    if not isinstance(value, str) or not value.strip():
        errors.append(f"entry {index}: field '{field}' must be a non-empty string")
        return ""
    return value


def validate_manifest(root: Path, data: Any) -> list[str]:
    """Return a list of human-readable policy violations (empty when valid)."""
    errors: list[str] = []

    if not isinstance(data, dict):
        return ["manifest root must be a JSON object"]

    if data.get("schema_version") != SCHEMA_VERSION:
        errors.append(
            f"manifest schema_version must be {SCHEMA_VERSION}, got {data.get('schema_version')!r}"
        )

    entries = data.get("entries")
    if not isinstance(entries, list) or not entries:
        return errors + ["manifest 'entries' must be a non-empty array"]

    seen_paths: set[str] = set()

    for index, entry in enumerate(entries):
        if not isinstance(entry, dict):
            errors.append(f"entry {index}: must be a JSON object")
            continue

        missing = [field for field in REQUIRED_FIELDS if field not in entry]
        if missing:
            errors.append(f"entry {index}: missing required fields: {', '.join(missing)}")
            continue

        path_text = _require_string(entry, "path", index, errors)
        kind = _require_string(entry, "kind", index, errors)
        _require_string(entry, "source", index, errors)
        _require_string(entry, "author", index, errors)
        licence = _require_string(entry, "licence", index, errors)

        url = entry.get("url")
        if not isinstance(url, str):
            errors.append(f"entry {index}: field 'url' must be a string (use \"\" when none)")

        shipped = entry.get("shipped")
        if not isinstance(shipped, bool):
            errors.append(f"entry {index}: field 'shipped' must be a boolean")
            shipped = False

        ai_generated = entry.get("ai_generated")
        if not isinstance(ai_generated, bool):
            errors.append(f"entry {index}: field 'ai_generated' must be a boolean")
            ai_generated = False

        if kind and kind not in KIND_ORDER:
            errors.append(
                f"entry {index}: unknown kind '{kind}' (expected one of {', '.join(KIND_ORDER)})"
            )

        if path_text:
            if path_text in seen_paths:
                errors.append(f"entry {index}: duplicate path '{path_text}'")
            seen_paths.add(path_text)

            relative = Path(path_text)
            if relative.is_absolute() or ".." in relative.parts:
                errors.append(
                    f"entry {index}: path '{path_text}' must be a repo-relative path without '..'"
                )
            else:
                candidate = (root / relative).resolve()
                try:
                    candidate.relative_to(root.resolve())
                except ValueError:
                    errors.append(f"entry {index}: path '{path_text}' escapes the repository")
                else:
                    if not candidate.is_file():
                        errors.append(f"entry {index}: path '{path_text}' does not exist in the repo")

        if licence:
            if shipped:
                if licence not in APPROVED_LICENCE_SET:
                    errors.append(
                        f"entry {index}: shipped asset '{path_text}' uses unapproved licence '{licence}' "
                        f"(allowed: {', '.join(APPROVED_LICENCES)})"
                    )
                if is_non_commercial(licence):
                    errors.append(
                        f"entry {index}: shipped asset '{path_text}' uses non-commercial licence '{licence}'"
                    )
            elif is_non_commercial(licence):
                print(
                    f"warning: entry {index}: development-only asset '{path_text}' uses "
                    f"non-commercial licence '{licence}'; it cannot be shipped",
                    file=sys.stderr,
                )

        if shipped and ai_generated:
            errors.append(
                f"entry {index}: shipped asset '{path_text}' is ai_generated; "
                "AI-generated assets are not allowed in shipped builds (ARCHITECTURE 9.1)"
            )

    return errors


def _credit_source(entry: dict[str, Any]) -> str:
    url = entry.get("url", "").strip()
    if not url:
        return "In repository"
    label = url.replace("https://", "").replace("http://", "")
    return f"[{label}]({url})"


def _credit_table(entries: list[dict[str, Any]]) -> list[str]:
    lines = [
        "| Asset | Path | Author | Licence | Source |",
        "| --- | --- | --- | --- | --- |",
    ]
    for entry in sorted(entries, key=lambda item: item["path"]):
        path = entry["path"]
        name = Path(path).name
        lines.append(
            "| {name} | `{path}` | {author} | {licence} | {source} |".format(
                name=name,
                path=path,
                author=entry["author"],
                licence=entry["licence"],
                source=_credit_source(entry),
            )
        )
    return lines


def render_credits(manifest: dict[str, Any]) -> str:
    """Render deterministic CREDITS.md content from a validated manifest."""
    entries = manifest["entries"]
    shipped = [entry for entry in entries if entry["shipped"]]
    development = [entry for entry in entries if not entry["shipped"]]

    lines = [
        "# Credits and licence manifest",
        "",
        "<!-- Generated by tools/check_licences.py from assets-src/LICENCE_MANIFEST.json. Do not edit by hand. -->",
        "",
        "Keepwright ships only procedural, self-made, MIT, CC0-1.0, or CC-BY-4.0 assets. "
        "Every shipped entry is declared `ai_generated: false`; non-commercial licences and "
        "AI-generated assets are excluded from shipped builds.",
        "",
        "## Shipped assets",
        "",
    ]

    for kind in KIND_ORDER:
        group = [entry for entry in shipped if entry["kind"] == kind]
        if not group:
            continue
        lines.append(f"### {KIND_LABELS[kind]}")
        lines.append("")
        lines.extend(_credit_table(group))
        lines.append("")

    if development:
        lines.extend(
            [
                "## Development-only assets",
                "",
                "These manifest entries are not part of a shipped build.",
                "",
            ]
        )
        for kind in KIND_ORDER:
            group = [entry for entry in development if entry["kind"] == kind]
            if not group:
                continue
            lines.append(f"### {KIND_LABELS[kind]}")
            lines.append("")
            lines.extend(_credit_table(group))
            lines.append("")

    counts: dict[str, int] = {}
    for entry in shipped:
        counts[entry["licence"]] = counts.get(entry["licence"], 0) + 1

    lines.extend(
        [
            "## Licence summary",
            "",
            "| Licence | Shipped entries |",
            "| --- | ---: |",
        ]
    )
    for licence in APPROVED_LICENCES:
        if licence in counts:
            lines.append(f"| {licence} | {counts[licence]} |")
    lines.extend(
        [
            "",
            f"Manifest schema version {manifest['schema_version']}.",
            "",
        ]
    )
    return "\n".join(lines)


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(
        description="Validate assets-src/LICENCE_MANIFEST.json and keep docs/CREDITS.md in sync."
    )
    mode = parser.add_mutually_exclusive_group()
    mode.add_argument(
        "--check",
        action="store_true",
        help="validate the manifest and fail if docs/CREDITS.md is out of date (default)",
    )
    mode.add_argument(
        "--write-credits",
        action="store_true",
        help="validate the manifest, then regenerate docs/CREDITS.md",
    )
    args = parser.parse_args(argv)
    write_credits = args.write_credits
    root = Path(__file__).resolve().parent.parent
    manifest_path = root / MANIFEST_REL

    try:
        data = json.loads(manifest_path.read_text(encoding="utf-8"))
    except FileNotFoundError:
        print(f"error: manifest not found at {manifest_path}", file=sys.stderr)
        return 1
    except json.JSONDecodeError as exc:
        print(f"error: invalid JSON in {manifest_path}: {exc}", file=sys.stderr)
        return 1

    errors = validate_manifest(root, data)
    if errors:
        for message in errors:
            print(f"error: {message}", file=sys.stderr)
        print(
            f"FAILED: {len(errors)} manifest error(s); docs/CREDITS.md was not changed.",
            file=sys.stderr,
        )
        return 1

    expected = render_credits(data)
    credits_path = root / CREDITS_REL

    if write_credits:
        credits_path.parent.mkdir(parents=True, exist_ok=True)
        credits_path.write_text(expected, encoding="utf-8", newline="\n")
        print(
            f"OK: validated {len(data['entries'])} manifest entries and wrote "
            f"{CREDITS_REL.as_posix()}."
        )
        return 0

    if not credits_path.is_file():
        print(
            f"error: {CREDITS_REL.as_posix()} is missing; run "
            "python3 tools/check_licences.py --write-credits",
            file=sys.stderr,
        )
        return 1

    existing = credits_path.read_text(encoding="utf-8").replace("\r\n", "\n")
    if existing != expected:
        print(
            f"error: {CREDITS_REL.as_posix()} is out of date; run "
            "python3 tools/check_licences.py --write-credits",
            file=sys.stderr,
        )
        return 1

    print(
        f"OK: {len(data['entries'])} manifest entries valid and "
        f"{CREDITS_REL.as_posix()} is up to date."
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
