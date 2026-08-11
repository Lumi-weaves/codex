#!/usr/bin/env python3
"""Dispatch a one-shot JIT runner for the manual Lumi shadow workflow.

Owned exclusively by .github/workflows/lumi-release-shadow-worker.yml (locked
by scripts/release/lumi_shadow_workflow_static_test.sh); no credentials are
stored in this repository.

The controller is a human-run manual step, not automation in this repo.
Given an authorized workflow_dispatch run of
.github/workflows/lumi-release-shadow-worker.yml on Lumi-weaves/codex main,
this dispatcher:

  * fail-closed verifies the exact run/attempt, event, branch, head SHA,
    workflow id -> exact workflow path, refs/heads/main, live run state, the
    completed/successful gate job, and the chosen queued/unassigned build job
    carrying exactly the run's deterministic per-run label;
  * re-reads and revalidates the run, workflow/main identity, and attempt
    jobs immediately before the single non-retried generate-jitconfig POST,
    which requests a deterministic runner name (`name`), the explicit runner
    group, exactly the expected label, and work folder `_work` (HTTP 201
    required);
  * validates the returned runner (name, idle, exactly the expected custom
    label) and the encoded_jit_config (nonempty, bounded, canonical base64),
    then streams the encoded value exactly once plus one newline to the stdin
    of the provided runner command and propagates its exit code.

Security contract:

  * the token comes only from the LUMI_GITHUB_TOKEN environment variable; it
    is never accepted as an argument, never written to a file, never printed,
    and never included in any error message;
  * the runner child environment strips LUMI_GITHUB_TOKEN, GH_TOKEN,
    GITHUB_TOKEN, and every other TOKEN/AUTHORIZATION-bearing variable;
  * encoded_jit_config is never decoded, logged, stored, or echoed; it is
    streamed to the child stdin and nothing else;
  * the single POST is never retried; any ambiguous transport failure after
    the POST fails the dispatch;
  * all API requests use the fixed public base https://api.github.com with
    the official API version header 2026-03-10. Tests inject a mock transport
    only; there is no runtime base-URL override.

No polling, background daemons, runner registration, or live API calls happen
here beyond the fixed GETs and the single POST.
"""

from __future__ import annotations

import argparse
import json
import os
import re
import shutil
import subprocess
import sys
import urllib.error
import urllib.request
from collections.abc import Callable
from typing import Any

API_BASE = "https://api.github.com"
API_VERSION = "2026-03-10"
USER_AGENT = "lumi-shadow-dispatcher"
REPO = "Lumi-weaves/codex"
WORKFLOW_NAME = "lumi-release-shadow-worker"
WORKFLOW_PATH = ".github/workflows/lumi-release-shadow-worker.yml"
GATE_JOB_NAME = "Resolve source to exact commit"
JOB_NAMES = {
    "arm64": "Build and validate shadow packages (aarch64)",
    "x86_64": "Build and validate shadow package (x86_64)",
}
LABEL_PREFIX = {
    "arm64": "lumi-shadow-arm64-",
    "x86_64": "lumi-shadow-x86_64-",
}
# GitHub-added read-only labels that may accompany the workflow's single
# custom label in the job's labels list. Anything outside this documented
# allowlist fails closed (no other custom routing label is ever accepted).
GITHUB_READ_ONLY_LABELS = frozenset({"self-hosted"})
PER_PAGE = 100
REQUEST_TIMEOUT_SECONDS = 30.0
# Conservative portable hard cap shared by both host controllers: an
# exported env value at 262144 bytes already fails Linux execve with
# "Argument list too long" (single env/argv string limit near 128KiB)
# before Runner.Listener starts, so the dispatcher never creates or
# streams a JIT config above 65536 bytes.
MAX_ENCODED_JIT_CONFIG_LENGTH = 65536
TOKEN_ENV_NAMES = frozenset(
    {
        "LUMI_GITHUB_TOKEN",
        "GH_TOKEN",
        "GITHUB_TOKEN",
        "GH_ENTERPRISE_TOKEN",
        "GITHUB_ENTERPRISE_TOKEN",
    }
)
SHA_PATTERN = re.compile(r"^[0-9a-f]{40}$")
BASE64_PATTERN = re.compile(r"^[A-Za-z0-9+/]+={0,2}$")
RUNNER_NAME_PATTERN = re.compile(r"^[A-Za-z0-9-_]{1,64}$")

Headers = dict[str, str]
# Injected transport: (method, url, headers, body) -> (status, headers, body).
# The base URL is fixed; tests mock the transport and never override it.
Transport = Callable[[str, str, Headers, bytes | None], tuple[int, dict[str, str], bytes]]
PopenFactory = Callable[..., Any]


class DispatchError(Exception):
    """Base error; the message is safe to print (never the token or JIT config)."""


class ApiError(DispatchError):
    """GitHub API request/response failure (transport or malformed payload)."""


class RejectError(DispatchError):
    """A fail-closed gate rejected the run/job/response."""


class ChildError(DispatchError):
    """The runner child process could not be started, fed, or waited on."""


def _require(condition: bool, message: str) -> None:
    if not condition:
        raise RejectError(message)


def _positive_int(value: str) -> int:
    try:
        number = int(value)
    except ValueError:
        raise argparse.ArgumentTypeError("must be a positive integer") from None
    if number <= 0:
        raise argparse.ArgumentTypeError("must be a positive integer")
    return number


def _headers(token: str, json_body: bool = False) -> Headers:
    headers: Headers = {
        "Accept": "application/vnd.github+json",
        "Authorization": f"Bearer {token}",
        "X-GitHub-Api-Version": API_VERSION,
        "User-Agent": USER_AGENT,
    }
    if json_body:
        headers["Content-Type"] = "application/json"
    return headers


def default_transport(
    method: str, url: str, headers: Headers, body: bytes | None
) -> tuple[int, dict[str, str], bytes]:
    """Real transport: urllib with a finite timeout; never includes secrets."""
    request = urllib.request.Request(url, data=body, method=method, headers=headers)
    try:
        with urllib.request.urlopen(request, timeout=REQUEST_TIMEOUT_SECONDS) as response:
            return response.status, dict(response.headers.items()), response.read()
    except urllib.error.HTTPError as error:
        raise ApiError(f"GitHub API returned HTTP {error.code} for {url}") from None
    except urllib.error.URLError:
        raise ApiError(f"GitHub API unreachable for {url}") from None
    except (OSError, TimeoutError):
        raise ApiError(f"GitHub API request failed for {url}") from None


def _request(
    transport: Transport,
    method: str,
    path: str,
    headers: Headers,
    body: bytes | None = None,
) -> tuple[int, dict[str, str], bytes]:
    """Invoke the transport and convert any failure into a redacted ApiError."""
    url = f"{API_BASE}{path}"
    try:
        return transport(method, url, headers, body)
    except DispatchError:
        raise
    except Exception:  # noqa: BLE001 - redaction is the point of this catch
        # Never propagate transport internals: they could contain the token.
        raise ApiError(f"GitHub API request failed for {path}") from None


def _parse_json(status: int, body: bytes, path: str) -> Any:
    if not 200 <= status < 300:
        raise ApiError(f"GitHub API returned HTTP {status} for {path}")
    try:
        return json.loads(body.decode("utf-8"))
    except (UnicodeDecodeError, json.JSONDecodeError):
        raise ApiError(f"GitHub API returned invalid JSON for {path}") from None


def _get_json(
    transport: Transport, method: str, path: str, headers: Headers
) -> Any:
    status, _resp_headers, body = _request(transport, method, path, headers)
    return _parse_json(status, body, path)


def _rel_next(headers: dict[str, str]) -> str | None:
    """Return the Link header value when it declares rel=\"next\", else None."""
    link = ""
    for key, value in headers.items():
        if key.lower() == "link":
            link = value
            break
    for part in link.split(","):
        if 'rel="next"' in part:
            return link
    return None


def _fetch_all_jobs(
    transport: Transport, run_id: int, run_attempt: int, headers: Headers
) -> list[dict[str, Any]]:
    """Fetch every job of the exact attempt; reject truncation or mismatch."""
    page = 1
    total_count: int | None = None
    collected: list[dict[str, Any]] = []
    while True:
        path = (
            f"/repos/{REPO}/actions/runs/{run_id}/attempts/{run_attempt}/jobs"
            f"?per_page={PER_PAGE}&page={page}"
        )
        status, resp_headers, body = _request(transport, "GET", path, headers)
        data = _parse_json(status, body, path)
        _require(isinstance(data, dict), "jobs payload is not an object")
        count = data.get("total_count")
        jobs = data.get("jobs")
        _require(isinstance(count, int) and count >= 0, "jobs total_count is invalid")
        _require(isinstance(jobs, list), "jobs list is missing")
        if total_count is None:
            total_count = count
        elif count != total_count:
            raise RejectError("jobs total_count changed between pages")
        collected.extend(jobs)
        if len(jobs) < PER_PAGE or len(collected) >= total_count:
            break
        if _rel_next(resp_headers) is None:
            raise RejectError("jobs pagination is truncated (next page without a link)")
        page += 1
    if len(collected) != total_count:
        raise RejectError(
            f"jobs pagination mismatch: got {len(collected)} of {total_count} jobs"
        )
    return collected


def _check_run(
    run: Any, run_id: int, run_attempt: int
) -> tuple[str, int]:
    """Verify the run payload; return (head_sha, workflow_id)."""
    _require(isinstance(run, dict), "run payload is not an object")
    _require(run.get("id") == run_id, "run id does not match the requested run id")
    _require(
        run.get("run_attempt") == run_attempt,
        "run attempt does not match the requested attempt",
    )
    _require(
        run.get("event") == "workflow_dispatch",
        "run event is not workflow_dispatch",
    )
    _require(
        run.get("pull_requests") == [],
        "run must have no pull requests",
    )
    _require(run.get("name") == WORKFLOW_NAME, "run workflow name mismatch")
    _require(run.get("head_branch") == "main", "run head branch is not main")
    head_sha = run.get("head_sha")
    _require(
        isinstance(head_sha, str) and SHA_PATTERN.fullmatch(head_sha) is not None,
        "run head_sha is not an exact 40-hex commit",
    )
    workflow_id = run.get("workflow_id")
    _require(
        isinstance(workflow_id, int) and workflow_id > 0,
        "run workflow_id is invalid",
    )
    expected_repo_url = f"{API_BASE}/repos/{REPO}"
    for field in ("repository", "head_repository"):
        repo = run.get(field)
        _require(isinstance(repo, dict), f"run {field} is missing")
        _require(repo.get("name") == "codex", f"run {field} name mismatch")
        _require(
            repo.get("full_name") == REPO, f"run {field} full name mismatch"
        )
        _require(
            repo.get("url") == expected_repo_url, f"run {field} url mismatch"
        )
    _require(
        run.get("status") in {"queued", "requested", "waiting", "in_progress"},
        "run is not in a live state",
    )
    _require(run.get("conclusion") is None, "run already concluded (completed/cancelled)")
    return head_sha, workflow_id


def _check_workflow(workflow: Any, expected_workflow_id: int) -> None:
    _require(isinstance(workflow, dict), "workflow payload is not an object")
    _require(
        workflow.get("id") == expected_workflow_id,
        "workflow id does not match the run workflow_id",
    )
    _require(
        workflow.get("path") == WORKFLOW_PATH,
        f"workflow id resolves to {workflow.get('path')!r}, not the shadow workflow",
    )
    _require(
        workflow.get("name") == WORKFLOW_NAME,
        f"workflow id resolves to {workflow.get('name')!r}, not the shadow workflow",
    )
    _require(workflow.get("state") == "active", "workflow is not active")


def _check_main_ref(main_ref: Any, head_sha: str) -> None:
    _require(isinstance(main_ref, dict), "refs/heads/main payload is not an object")
    obj = main_ref.get("object")
    _require(
        isinstance(obj, dict) and obj.get("type") == "commit",
        "refs/heads/main does not point to a commit",
    )
    _require(
        obj.get("sha") == head_sha,
        "refs/heads/main does not match the run head_sha",
    )


def _select_job(
    jobs: list[dict[str, Any]],
    run_id: int,
    run_attempt: int,
    target: str,
    expected_label: str,
    head_sha: str,
) -> dict[str, Any]:
    """Verify gate + chosen job and return the chosen job (fail closed)."""
    _require(isinstance(jobs, list), "jobs payload is not a list")
    gate_jobs = [
        job for job in jobs
        if isinstance(job, dict) and job.get("name") == GATE_JOB_NAME
    ]
    _require(len(gate_jobs) == 1, "gate job must appear exactly once")
    gate = gate_jobs[0]
    _require(gate.get("status") == "completed", "gate job is not completed")
    _require(gate.get("conclusion") == "success", "gate job did not succeed")

    chosen_name = JOB_NAMES[target]
    chosen = [
        job for job in jobs
        if isinstance(job, dict) and job.get("name") == chosen_name
    ]
    _require(len(chosen) == 1, f"chosen job {chosen_name!r} must appear exactly once")
    job = chosen[0]
    _require(job.get("status") == "queued", "chosen job is not queued")
    _require(job.get("conclusion") is None, "chosen job already concluded")
    # GitHub fills started_at when a self-hosted job enters the queue, before
    # any runner is assigned. Assignment is represented by the runner fields
    # below; status=queued plus empty runner identity is the safe boundary.
    _require(
        job.get("runner_name") in (None, ""),
        "chosen job already has a runner assigned",
    )
    _require(
        job.get("runner_id") in (None, "", 0),
        "chosen job already has a runner assigned",
    )
    _require(
        job.get("runner_group_id") in (None, "", 0),
        "chosen job already has a runner group assigned",
    )
    _require(job.get("run_id") == run_id, "chosen job run_id mismatch")
    if job.get("run_attempt") is not None:
        _require(job.get("run_attempt") == run_attempt, "chosen job run_attempt mismatch")
    _require(
        job.get("workflow_name") == WORKFLOW_NAME,
        "chosen job workflow_name mismatch",
    )
    _require(job.get("head_branch") == "main", "chosen job head_branch mismatch")
    _require(job.get("head_sha") == head_sha, "chosen job head_sha mismatch")
    labels = job.get("labels")
    _require(isinstance(labels, list), "chosen job labels are missing")
    _require(
        labels.count(expected_label) == 1,
        "chosen job must carry the expected label exactly once",
    )
    extras = [label for label in labels if label != expected_label]
    _require(
        all(
            isinstance(label, str) and label in GITHUB_READ_ONLY_LABELS
            for label in extras
        ),
        "chosen job carries an unexpected routing label",
    )
    holders = sum(
        1
        for other in jobs
        if isinstance(other, dict)
        and isinstance(other.get("labels"), list)
        and expected_label in other["labels"]
    )
    _require(holders == 1, "another job requests the same derived label")
    return job


def _expected_label(target: str, run_id: int, run_attempt: int) -> str:
    return f"{LABEL_PREFIX[target]}{run_id}-{run_attempt}"


def _runner_name(target: str, run_id: int, run_attempt: int) -> str:
    name = f"lumi-shadow-{target}-{run_id}-{run_attempt}"
    _require(
        RUNNER_NAME_PATTERN.fullmatch(name) is not None,
        "derived runner name is not a safe GitHub runner name",
    )
    return name


def _check_jit_response(data: Any, runner_name: str, expected_label: str) -> str:
    """Validate the generate-jitconfig response; return encoded_jit_config."""
    _require(isinstance(data, dict), "generate-jitconfig response is not an object")
    runner = data.get("runner")
    _require(isinstance(runner, dict), "generate-jitconfig response has no runner")
    runner_id = runner.get("id")
    _require(
        isinstance(runner_id, int)
        and not isinstance(runner_id, bool)
        and runner_id > 0,
        "generated runner id is invalid",
    )
    _require(
        runner.get("name") == runner_name,
        "generated runner name does not match the requested name",
    )
    _require(runner.get("busy") is False, "generated runner is busy")
    # The documented JIT creation response marks the not-yet-connected runner
    # as offline (official example: "status": "offline", os "unknown").
    _require(
        runner.get("status") == "offline",
        "generated runner is not in the documented offline state",
    )
    labels = runner.get("labels")
    _require(isinstance(labels, list), "generated runner labels are missing")
    names: list[str] = []
    for label in labels:
        _require(
            isinstance(label, dict)
            and isinstance(label.get("name"), str)
            and label.get("type") in ("custom", "read-only"),
            "generated runner has an invalid label",
        )
        name = label["name"]
        names.append(name)
        if label["type"] == "custom":
            _require(
                name == expected_label,
                "generated runner carries an unexpected custom label",
            )
    _require(
        names.count(expected_label) == 1,
        "generated runner does not carry the expected label exactly once",
    )
    encoded = data.get("encoded_jit_config")
    _require(
        isinstance(encoded, str) and encoded,
        "encoded_jit_config is missing or empty",
    )
    _require(
        len(encoded) <= MAX_ENCODED_JIT_CONFIG_LENGTH,
        "encoded_jit_config exceeds the bounded size",
    )
    _require(
        BASE64_PATTERN.fullmatch(encoded) is not None and len(encoded) % 4 == 0,
        "encoded_jit_config is not canonical base64",
    )
    return encoded


def _sanitize_env(env: dict[str, str]) -> dict[str, str]:
    """Child environment without token/authorization-bearing variables."""
    sanitized: dict[str, str] = {}
    for key, value in env.items():
        upper = key.upper()
        if (
            upper in TOKEN_ENV_NAMES
            or "TOKEN" in upper
            or "AUTHORIZATION" in upper
        ):
            continue
        sanitized[key] = value
    return sanitized


def _validate_child_command(command: list[str], env: dict[str, str]) -> None:
    """Fail before the irreversible POST when the runner command is not
    locally resolvable/executable on the sanitized child PATH.

    Uses shutil.which only: no execution and no shell. A missing local
    executable (for example a missing `ssh` or controller command) must be
    caught before a JIT config is consumed, not after the POST.
    """
    if not command:
        raise ChildError("runner command is empty")
    child_env = _sanitize_env(env)
    path = child_env.get("PATH")
    if path is None:
        # exec*p*/spawn*p* semantics: no PATH in the environment means the
        # default search path.
        path = os.defpath
    if shutil.which(command[0], path=path) is None:
        raise ChildError(
            f"runner executable {command[0]!r} is not resolvable/executable "
            "on the sanitized child PATH"
        )


def _run_child(
    popen: PopenFactory,
    command: list[str],
    env: dict[str, str],
    payload: bytes,
) -> int:
    """Spawn the runner command, stream the payload once, propagate exit code."""
    if not command:
        raise ChildError("runner command is empty")
    sanitized = _sanitize_env(env)
    try:
        process = popen(command, stdin=subprocess.PIPE, env=sanitized)
    except OSError:
        raise ChildError("could not start the runner command") from None
    try:
        process.stdin.write(payload)
        process.stdin.close()
    except BrokenPipeError:
        raise ChildError("runner exited before consuming the JIT config") from None
    except OSError:
        raise ChildError("could not write the JIT config to the runner stdin") from None
    try:
        return process.wait()
    except OSError:
        raise ChildError("could not wait for the runner process") from None


def _preflight(
    transport: Transport,
    run_id: int,
    run_attempt: int,
    target: str,
    headers: Headers,
) -> str:
    """One full fail-closed verification pass; returns the expected label."""
    run = _get_json(transport, "GET", f"/repos/{REPO}/actions/runs/{run_id}", headers)
    head_sha, workflow_id = _check_run(run, run_id, run_attempt)

    workflow = _get_json(
        transport, "GET", f"/repos/{REPO}/actions/workflows/{workflow_id}", headers
    )
    _check_workflow(workflow, workflow_id)

    main_ref = _get_json(transport, "GET", f"/repos/{REPO}/git/ref/heads/main", headers)
    _check_main_ref(main_ref, head_sha)

    expected_label = _expected_label(target, run_id, run_attempt)
    jobs = _fetch_all_jobs(transport, run_id, run_attempt, headers)
    _select_job(jobs, run_id, run_attempt, target, expected_label, head_sha)
    return expected_label


def dispatch(
    run_id: int,
    run_attempt: int,
    target: str,
    runner_group_id: int,
    runner_command: list[str],
    token: str,
    env: dict[str, str],
    transport: Transport,
    popen: PopenFactory,
) -> int:
    """Run the fail-closed preflights, the single POST, and the child."""
    headers = _headers(token)

    expected_label = _preflight(transport, run_id, run_attempt, target, headers)
    # Final re-read (run, workflow/main identity, and jobs) immediately
    # before the single non-retried POST.
    expected_label = _preflight(transport, run_id, run_attempt, target, headers)

    # Catch a missing local executable before the irreversible POST.
    _validate_child_command(runner_command, env)

    runner_name = _runner_name(target, run_id, run_attempt)
    body = {
        "name": runner_name,
        "runner_group_id": runner_group_id,
        "labels": [expected_label],
        "work_folder": "_work",
    }
    status, _resp_headers, resp_body = _request(
        transport,
        "POST",
        f"/repos/{REPO}/actions/runners/generate-jitconfig",
        _headers(token, json_body=True),
        json.dumps(body, separators=(",", ":")).encode("utf-8"),
    )
    if status != 201:
        raise ApiError(
            f"generate-jitconfig returned HTTP {status}, expected 201"
        )
    data = _parse_json(status, resp_body, f"/repos/{REPO}/actions/runners/generate-jitconfig")
    encoded = _check_jit_response(data, runner_name, expected_label)

    return _run_child(popen, runner_command, env, encoded.encode("ascii") + b"\n")


def main(
    argv: list[str] | None = None,
    environ: dict[str, str] | None = None,
    transport: Transport | None = None,
    popen: PopenFactory | None = None,
) -> int:
    raw_argv = list(sys.argv[1:] if argv is None else argv)
    env = dict(os.environ if environ is None else environ)

    parser = argparse.ArgumentParser(
        description=(
            "Verify an authorized lumi-release-shadow-worker run and stream a "
            "one-shot JIT runner config to the provided runner command."
        )
    )
    parser.add_argument("--run-id", type=_positive_int, required=True, metavar="RUN_ID")
    parser.add_argument(
        "--run-attempt", type=_positive_int, required=True, metavar="ATTEMPT"
    )
    parser.add_argument("--target", required=True, metavar="arm64|x86_64")
    parser.add_argument(
        "--runner-group-id", type=_positive_int, required=True, metavar="GROUP_ID"
    )
    parser.add_argument(
        "runner_command",
        nargs=argparse.REMAINDER,
        metavar="CMD [ARG...]",
        help="runner command, given literally after --",
    )
    if "--" not in raw_argv:
        parser.error("a runner command is required after --")
    args = parser.parse_args(raw_argv)
    command = list(args.runner_command)
    if command and command[0] == "--":
        command = command[1:]
    if not command:
        parser.error("a runner command is required after --")
    if args.target not in JOB_NAMES:
        parser.error(f"--target must be one of: {', '.join(sorted(JOB_NAMES))}")

    token = env.get("LUMI_GITHUB_TOKEN", "")
    if not token:
        print("LUMI_GITHUB_TOKEN is not set; refusing to dispatch", file=sys.stderr)
        return 1

    try:
        code = dispatch(
            args.run_id,
            args.run_attempt,
            args.target,
            args.runner_group_id,
            command,
            token,
            env,
            transport if transport is not None else default_transport,
            popen if popen is not None else subprocess.Popen,
        )
    except DispatchError as error:
        print(f"dispatch failed: {error}", file=sys.stderr)
        return 1
    print(f"JIT config streamed to the runner; child exited with {code}", file=sys.stderr)
    return code


if __name__ == "__main__":
    raise SystemExit(main())
