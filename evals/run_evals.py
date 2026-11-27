#!/usr/bin/env python3
"""Keepwright offline agent eval suite.

Runs deterministic castle-design briefs end-to-end through the same
line-delimited JSON-RPC/MCP interface an external LLM agent would use.  The
reference steps in each brief are ordinary MCP tool calls, so no API key or
network access is needed: only the `keep-mcp --offline` sidecar.

Usage:
    python3 evals/run_evals.py
    python3 evals/run_evals.py --brief baltic_island_castle
    python3 evals/run_evals.py --json
    python3 evals/run_evals.py --mcp-bin core/target/release/keep-mcp

Exit status is 0 only when every selected brief passes its thresholds.
"""

from __future__ import annotations

import argparse
import json
import os
import select
import subprocess
import sys
import time
from pathlib import Path
from typing import Any

REPO_ROOT = Path(__file__).resolve().parents[1]
CORE_MANIFEST = REPO_ROOT / "core" / "Cargo.toml"
BRIEF_DIR = Path(__file__).resolve().parent / "briefs"
DEFAULT_PROTOCOL_VERSION = "2025-06-18"
READ_TIMEOUT_SECONDS = 60.0


class EvalError(RuntimeError):
    """A brief could not be executed or the sidecar misbehaved."""


class McpClient:
    """Minimal synchronous MCP stdio client (newline-delimited JSON-RPC 2.0)."""

    def __init__(self, binary: Path, site_id: str) -> None:
        self.binary = binary
        self.site_id = site_id
        self.proc: subprocess.Popen[bytes] | None = None
        self._buffer = b""
        self._next_id = 0
        self._stderr_tail = ""

    def __enter__(self) -> "McpClient":
        self.proc = subprocess.Popen(
            [str(self.binary), "--offline", "--site", self.site_id],
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            bufsize=0,
        )
        init = self.request(
            "initialize",
            {
                "protocolVersion": DEFAULT_PROTOCOL_VERSION,
                "capabilities": {},
                "clientInfo": {"name": "keepwright-evals", "version": "1.0"},
            },
        )
        if not isinstance(init, dict) or "serverInfo" not in init:
            raise EvalError("initialize returned no serverInfo")
        self.notify("notifications/initialized")
        return self

    def __exit__(self, *_: Any) -> None:
        proc = self.proc
        if proc is None:
            return
        try:
            if proc.stdin is not None:
                proc.stdin.close()
            proc.wait(timeout=10)
        except Exception:
            proc.kill()
            proc.wait(timeout=10)
        finally:
            self.proc = None

    def _write(self, message: dict[str, Any]) -> None:
        assert self.proc is not None and self.proc.stdin is not None
        payload = json.dumps(message, separators=(",", ":")).encode("utf-8") + b"\n"
        self.proc.stdin.write(payload)
        self.proc.stdin.flush()

    def notify(self, method: str, params: dict[str, Any] | None = None) -> None:
        message: dict[str, Any] = {"jsonrpc": "2.0", "method": method}
        if params is not None:
            message["params"] = params
        self._write(message)

    def _read_line(self) -> str:
        assert self.proc is not None and self.proc.stdout is not None
        deadline = time.monotonic() + READ_TIMEOUT_SECONDS
        while b"\n" not in self._buffer:
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                self._collect_stderr()
                raise EvalError(
                    f"timed out waiting for {self.binary.name} on site "
                    f"{self.site_id}: {self._stderr_tail or 'no stderr'}"
                )
            ready, _, _ = select.select([self.proc.stdout], [], [], remaining)
            if not ready:
                continue
            chunk = os.read(self.proc.stdout.fileno(), 65536)
            if not chunk:
                self._collect_stderr()
                raise EvalError(
                    f"{self.binary.name} closed stdout unexpectedly: "
                    f"{self._stderr_tail or 'no stderr'}"
                )
            self._buffer += chunk
        line, self._buffer = self._buffer.split(b"\n", 1)
        return line.decode("utf-8", errors="replace")

    def _collect_stderr(self) -> None:
        assert self.proc is not None and self.proc.stderr is not None
        try:
            ready, _, _ = select.select([self.proc.stderr], [], [], 0.1)
            if ready:
                data = os.read(self.proc.stderr.fileno(), 65536)
                self._stderr_tail = data.decode("utf-8", errors="replace").strip()[-2000:]
        except Exception:
            pass

    def request(self, method: str, params: dict[str, Any] | None = None) -> Any:
        self._next_id += 1
        request_id = self._next_id
        message: dict[str, Any] = {
            "jsonrpc": "2.0",
            "id": request_id,
            "method": method,
        }
        if params is not None:
            message["params"] = params
        self._write(message)

        while True:
            line = self._read_line()
            if not line.strip():
                continue
            try:
                response = json.loads(line)
            except json.JSONDecodeError as error:
                raise EvalError(f"invalid JSON-RPC response: {line[:200]!r} ({error})")
            if response.get("id") != request_id:
                # Notifications or unrelated responses; keep waiting for ours.
                continue
            if "error" in response:
                raise EvalError(
                    f"{method} failed: {response['error'].get('message', response['error'])}"
                )
            return response.get("result")

    def call_tool(self, name: str, arguments: dict[str, Any]) -> Any:
        result = self.request("tools/call", {"name": name, "arguments": arguments})
        if not isinstance(result, dict):
            raise EvalError(f"{name} returned a non-object result")
        if result.get("isError"):
            content = result.get("content") or []
            message = content[0].get("text") if content else "tool error"
            raise EvalError(f"{name}: {message}")
        if "structuredContent" not in result:
            raise EvalError(f"{name} returned no structuredContent")
        return result["structuredContent"]


def find_mcp_binary(explicit: str | None) -> Path:
    if explicit:
        candidate = Path(explicit).expanduser().resolve()
        if not candidate.is_file():
            raise EvalError(f"--mcp-bin does not exist: {candidate}")
        return candidate

    for relative in ("core/target/debug/keep-mcp", "core/target/release/keep-mcp"):
        candidate = REPO_ROOT / relative
        if candidate.is_file():
            return candidate

    print(
        "keep-mcp not found; building core/target/debug/keep-mcp ...",
        file=sys.stderr,
    )
    try:
        subprocess.run(
            [
                "cargo",
                "build",
                "--manifest-path",
                str(CORE_MANIFEST),
                "-p",
                "keep-mcp",
            ],
            cwd=REPO_ROOT,
            check=True,
        )
    except (OSError, subprocess.CalledProcessError) as error:
        raise EvalError(f"could not build keep-mcp: {error}") from error

    built = REPO_ROOT / "core/target/debug/keep-mcp"
    if not built.is_file():
        raise EvalError(f"cargo build did not produce {built}")
    return built


def load_briefs(brief_filter: str | None) -> list[dict[str, Any]]:
    paths = sorted(BRIEF_DIR.glob("*.json"))
    if not paths:
        raise EvalError(f"no briefs found in {BRIEF_DIR}")

    briefs: list[dict[str, Any]] = []
    for path in paths:
        try:
            brief = json.loads(path.read_text(encoding="utf-8"))
        except (OSError, json.JSONDecodeError) as error:
            raise EvalError(f"cannot read {path}: {error}") from error

        missing = [
            key
            for key in ("id", "style_id", "site_id", "prompt", "thresholds", "reference_steps")
            if key not in brief
        ]
        if missing:
            raise EvalError(f"{path} is missing required fields: {', '.join(missing)}")
        if not isinstance(brief["reference_steps"], list) or not brief["reference_steps"]:
            raise EvalError(f"{path} must have a non-empty reference_steps list")
        briefs.append(brief)

    if brief_filter:
        selected = [
            brief
            for path, brief in zip(paths, briefs)
            if brief_filter == path.stem or brief_filter == brief.get("id")
        ]
        if not selected:
            available = ", ".join(
                f"{brief.get('id')} ({path.stem})"
                for path, brief in zip(paths, briefs)
            )
            raise EvalError(f"unknown brief '{brief_filter}'; available: {available}")
        briefs = selected

    return briefs


def run_brief(binary: Path, brief: dict[str, Any]) -> dict[str, Any]:
    style_id = brief["style_id"]
    site_id = brief["site_id"]
    thresholds = brief.get("thresholds", {})
    result: dict[str, Any] = {
        "id": brief.get("id"),
        "title": brief.get("title"),
        "style_id": style_id,
        "site_id": site_id,
        "thresholds": thresholds,
        "checks": {},
        "failures": [],
    }

    try:
        with McpClient(binary, site_id) as client:
            for index, step in enumerate(brief.get("reference_steps", []), start=1):
                tool = step.get("tool")
                arguments = step.get("arguments", {})
                if not isinstance(tool, str) or not isinstance(arguments, dict):
                    raise EvalError(f"malformed reference step {index}")
                try:
                    client.call_tool(tool, arguments)
                except EvalError as error:
                    raise EvalError(f"reference step {index} ({tool}): {error}") from error

            assessment = client.call_tool("get_assessment", {"style_id": style_id})
            elements = client.call_tool("list_elements", {})
            plan = client.call_tool("render_plan", {"view": "top_plan"})
    except EvalError as error:
        result["error"] = str(error)
        result["passed"] = False
        return result

    findings = assessment.get("findings") or []
    structure_errors = sum(
        1 for finding in findings if finding.get("severity") == "error"
    )
    historian = assessment.get("historian") or {}
    svg = plan.get("svg") or ""
    svg_bytes = len(svg.encode("utf-8"))
    stripped = svg.strip()
    svg_valid = stripped.startswith("<svg") and stripped.endswith("</svg>") and svg_bytes > 200

    checks = {
        "structure_score": int(assessment.get("score", 0)),
        "structure_errors": structure_errors,
        "structure_passed": bool(assessment.get("passed", False)),
        "historian_score": int(historian.get("overall_score", 0)),
        "authenticity_score": int(historian.get("authenticity_score", 0)),
        "defensive_score": int(historian.get("defensive_score", 0)),
        "domestic_score": int(historian.get("domestic_score", 0)),
        "element_count": int(elements.get("element_count", 0)),
        "opening_count": int(elements.get("opening_count", 0)),
        "svg_bytes": svg_bytes,
        "svg_valid": svg_valid,
    }
    result["checks"] = checks

    failures: list[str] = []
    minimum_structure = int(thresholds.get("min_structure_score", 0))
    maximum_errors = int(thresholds.get("max_structure_errors", 0))
    minimum_historian = int(thresholds.get("min_historian_score", 0))
    minimum_elements = int(thresholds.get("min_elements", 0))

    if checks["structure_score"] < minimum_structure:
        failures.append(
            f"structure score {checks['structure_score']} < {minimum_structure}"
        )
    if checks["structure_errors"] > maximum_errors:
        failures.append(
            f"structure errors {checks['structure_errors']} > {maximum_errors}"
        )
    if checks["historian_score"] < minimum_historian:
        failures.append(
            f"historian score {checks['historian_score']} < {minimum_historian}"
        )
    if checks["element_count"] < minimum_elements:
        failures.append(
            f"elements {checks['element_count']} < {minimum_elements}"
        )
    if thresholds.get("require_svg_plan", True) and not checks["svg_valid"]:
        failures.append("render_plan did not return a valid non-empty SVG")

    result["failures"] = failures
    result["passed"] = not failures
    return result


def print_table(results: list[dict[str, Any]]) -> None:
    header = (
        f"{'BRIEF':34} {'STYLE':16} {'ELEM':>4} {'STRUCT':>6} "
        f"{'HIST':>5} {'ERR':>3} {'SVG_B':>7}  RESULT"
    )
    print(header)
    print("-" * len(header))
    for result in results:
        checks = result.get("checks", {})
        if result.get("error"):
            print(
                f"{str(result.get('id')):34} {str(result.get('style_id')):16} "
                f"{'-':>4} {'-':>6} {'-':>5} {'-':>3} {'-':>7}  FAIL"
            )
            continue
        state = "PASS" if result.get("passed") else "FAIL"
        print(
            f"{str(result.get('id')):34} "
            f"{str(result.get('style_id')):16} "
            f"{checks.get('element_count', 0):>4} "
            f"{checks.get('structure_score', 0):>6} "
            f"{checks.get('historian_score', 0):>5} "
            f"{checks.get('structure_errors', 0):>3} "
            f"{checks.get('svg_bytes', 0):>7}  {state}"
        )

    failures = [
        (result, failure)
        for result in results
        for failure in ([result["error"]] if result.get("error") else result.get("failures", []))
    ]
    if failures:
        print("\nFailures:")
        for result, failure in failures:
            print(f"  - {result.get('id')}: {failure}")


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(
        description="Run the Keepwright offline agent eval suite."
    )
    parser.add_argument(
        "--mcp-bin",
        help="path to keep-mcp (default: auto-detect debug/release, else cargo build)",
    )
    parser.add_argument(
        "--brief",
        help="run one brief by id or filename stem (for example baltic_island_castle)",
    )
    parser.add_argument(
        "--json",
        action="store_true",
        dest="json_output",
        help="emit a machine-readable JSON report instead of the summary table",
    )
    args = parser.parse_args(argv)

    try:
        binary = find_mcp_binary(args.mcp_bin)
        briefs = load_briefs(args.brief)
    except EvalError as error:
        print(f"error: {error}", file=sys.stderr)
        return 2

    results = [run_brief(binary, brief) for brief in briefs]
    all_passed = all(result.get("passed") for result in results)

    if args.json_output:
        report = {
            "passed": all_passed,
            "total": len(results),
            "pass_count": sum(1 for result in results if result.get("passed")),
            "fail_count": sum(1 for result in results if not result.get("passed")),
            "briefs": results,
        }
        print(json.dumps(report, indent=2, sort_keys=True))
    else:
        print(f"keep-mcp: {binary}")
        print(f"briefs:   {len(results)}\n")
        print_table(results)
        print(
            f"\n{sum(1 for result in results if result.get('passed'))}/{len(results)} briefs passed"
        )

    return 0 if all_passed else 1


if __name__ == "__main__":
    sys.exit(main())
