#!/usr/bin/env python3
"""Tests for lumi_shadow_dispatch_jit.py (fail-closed JIT dispatcher).

Run with: python3 scripts/release/lumi_shadow_dispatch_jit_test.py

Every API interaction is served by an injected mock transport and the runner
child is an injected fake process; no test contacts GitHub and no test
spawns a real subprocess.
"""

from __future__ import annotations

import contextlib
import io
import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import lumi_shadow_dispatch_jit as jit

RUN_ID = 123
ATTEMPT = 1
GROUP = 9
TOKEN = "test-token-abc"
SHA = "a" * 40

RUN_URL = f"{jit.API_BASE}/repos/{jit.REPO}/actions/runs/{RUN_ID}"
WORKFLOW_URL = f"{jit.API_BASE}/repos/{jit.REPO}/actions/workflows/555"
REF_URL = f"{jit.API_BASE}/repos/{jit.REPO}/git/ref/heads/main"
JOBS_URL = (
    f"{jit.API_BASE}/repos/{jit.REPO}/actions/runs/{RUN_ID}"
    f"/attempts/{ATTEMPT}/jobs?per_page=100&page=1"
)
POST_URL = f"{jit.API_BASE}/repos/{jit.REPO}/actions/runners/generate-jitconfig"

# A locally resolvable/executable command for success-path tests; the new
# pre-POST validation requires a real executable (mocks never run it).
RUNNER_CMD = [sys.executable, "-c", "pass"]

EXPECTED_HEADERS = {
    "Accept": "application/vnd.github+json",
    "Authorization": f"Bearer {TOKEN}",
    "X-GitHub-Api-Version": "2026-03-10",
    "User-Agent": "lumi-shadow-dispatcher",
}


def expected_label(target: str) -> str:
    return f"lumi-shadow-{target}-{RUN_ID}-{ATTEMPT}"


def expected_runner_name(target: str) -> str:
    return f"lumi-shadow-{target}-{RUN_ID}-{ATTEMPT}"


def make_run(run_id: int = RUN_ID, attempt: int = ATTEMPT, **overrides):
    repo = {
        "name": "codex",
        "full_name": jit.REPO,
        "url": f"{jit.API_BASE}/repos/{jit.REPO}",
    }
    run = {
        "id": run_id,
        "name": jit.WORKFLOW_NAME,
        "run_attempt": attempt,
        "event": "workflow_dispatch",
        "pull_requests": [],
        "head_branch": "main",
        "head_sha": SHA,
        "workflow_id": 555,
        "status": "in_progress",
        "conclusion": None,
        "repository": dict(repo),
        "head_repository": dict(repo),
    }
    run.update(overrides)
    return run


def make_workflow(**overrides):
    workflow = {
        "id": 555,
        "name": jit.WORKFLOW_NAME,
        "path": jit.WORKFLOW_PATH,
        "state": "active",
    }
    workflow.update(overrides)
    return workflow


def make_main_ref(sha: str = SHA, **overrides):
    ref = {"ref": "refs/heads/main", "object": {"type": "commit", "sha": sha}}
    ref.update(overrides)
    return ref


def make_job(
    name: str,
    status: str = "queued",
    conclusion=None,
    labels=None,
    started_at=None,
    runner_id=None,
    runner_name=None,
    runner_group_id=None,
    run_id: int = RUN_ID,
    run_attempt: int = ATTEMPT,
    workflow_name=None,
    head_branch=None,
    head_sha=None,
):
    return {
        "name": name,
        "status": status,
        "conclusion": conclusion,
        "started_at": started_at,
        "runner_id": runner_id,
        "runner_name": runner_name,
        "runner_group_id": runner_group_id,
        "labels": list(labels) if labels is not None else [],
        "run_id": run_id,
        "run_attempt": run_attempt,
        "workflow_name": workflow_name
        if workflow_name is not None
        else jit.WORKFLOW_NAME,
        "head_branch": head_branch if head_branch is not None else "main",
        "head_sha": head_sha if head_sha is not None else SHA,
    }


def make_gate(**overrides):
    job = make_job(
        jit.GATE_JOB_NAME,
        status="completed",
        conclusion="success",
        labels=["ubuntu-24.04"],
    )
    job.update(overrides)
    return job


def make_build_job(target: str = "arm64", labels=None, **overrides):
    job = make_job(
        jit.JOB_NAMES[target],
        status="queued",
        labels=labels if labels is not None else ["self-hosted", expected_label(target)],
    )
    job.update(overrides)
    return job


def make_jobs(target: str = "arm64", jobs=None):
    return jobs if jobs is not None else [make_gate(), make_build_job(target)]


def make_jit_response(
    target: str = "arm64",
    runner_overrides=None,
    labels=None,
    encoded: str = "QUJDRA==",
    **overrides,
):
    runner = {
        "id": 7,
        "name": expected_runner_name(target),
        "runner_group_id": GROUP,
        "os": "unknown",
        "status": "offline",
        "labels": labels
        if labels is not None
        else [
            {"id": 1, "name": "self-hosted", "type": "read-only"},
            {"id": 2, "name": expected_label(target), "type": "custom"},
        ],
        "busy": False,
    }
    if runner_overrides:
        runner.update(runner_overrides)
    data = {"runner": runner, "encoded_jit_config": encoded}
    data.update(overrides)
    return data


def jobs_payload(jobs) -> dict:
    return {"total_count": len(jobs), "jobs": jobs}


class MockTransport:
    """Scripted transport; records every request for exact-order assertions."""

    def __init__(self, responses):
        self._responses = list(responses)
        self.requests = []

    def __call__(self, method, url, headers, body):
        self.requests.append(
            {"method": method, "url": url, "headers": dict(headers), "body": body}
        )
        if not self._responses:
            raise AssertionError(f"unexpected request: {method} {url}")
        status, resp_headers, payload = self._responses.pop(0)
        if isinstance(payload, (dict, list)):
            payload = json.dumps(payload).encode("utf-8")
        return status, dict(resp_headers), payload

    @property
    def post_count(self) -> int:
        return sum(1 for request in self.requests if request["method"] == "POST")


class FakeStdin:
    def __init__(self):
        self.written = b""
        self.closed = False

    def write(self, data):
        self.written += data

    def close(self):
        self.closed = True


class RaisingStdin(FakeStdin):
    def __init__(self, error):
        super().__init__()
        self._error = error

    def write(self, data):
        raise self._error


class FakeProcess:
    def __init__(self, exit_code: int, stdin: FakeStdin):
        self.stdin = stdin
        self._exit_code = exit_code
        self.waited = False

    def wait(self) -> int:
        self.waited = True
        return self._exit_code


class FakePopen:
    def __init__(self, exit_code: int = 0, spawn_error=None, write_error=None):
        self.exit_code = exit_code
        self.spawn_error = spawn_error
        self.write_error = write_error
        self.calls = []
        self.process = None

    def __call__(self, argv, **kwargs):
        self.calls.append({"argv": argv, "kwargs": kwargs})
        if self.spawn_error is not None:
            raise self.spawn_error
        stdin = RaisingStdin(self.write_error) if self.write_error else FakeStdin()
        self.process = FakeProcess(self.exit_code, stdin)
        return self.process


def responses_for(
    target: str = "arm64",
    jobs=None,
    jit_response=None,
    jobs_payload_override=None,
    jobs_headers=None,
):
    jobs = make_jobs(target, jobs)
    payload = jobs_payload_override if jobs_payload_override is not None else jobs_payload(jobs)
    jobs_read = (200, jobs_headers or {}, payload)
    return [
        (200, {}, make_run()),
        (200, {}, make_workflow()),
        (200, {}, make_main_ref()),
        jobs_read,
        (200, {}, make_run()),
        (200, {}, make_workflow()),
        (200, {}, make_main_ref()),
        jobs_read,
        (201, {}, jit_response if jit_response is not None else make_jit_response(target)),
    ]


def run_cli(
    target: str = "arm64",
    responses=None,
    command=None,
    environ=None,
    exit_code: int = 0,
    popen=None,
    transport=None,
    argv=None,
):
    transport = transport if transport is not None else MockTransport(
        responses if responses is not None else responses_for(target)
    )
    fake_popen = popen if popen is not None else FakePopen(exit_code=exit_code)
    env = {
        "PATH": "/usr/bin:/bin",
        "HOME": "/home/controller",
        "http_proxy": "http://proxy.local:7890",
        "LUMI_GITHUB_TOKEN": TOKEN,
        "GH_TOKEN": "gh-token-value",
        "GITHUB_TOKEN": "github-token-value",
        "SOME_TOKEN_VAR": "another-secret",
        "FOO_AUTHORIZATION": "auth-value",
    }
    if environ is not None:
        env = dict(environ)
    cli_argv = argv if argv is not None else [
        "--run-id", str(RUN_ID),
        "--run-attempt", str(ATTEMPT),
        "--target", target,
        "--runner-group-id", str(GROUP),
        "--",
    ] + (command if command is not None else list(RUNNER_CMD))
    err = io.StringIO()
    with contextlib.redirect_stderr(err):
        code = jit.main(cli_argv, environ=env, transport=transport, popen=fake_popen)
    return code, transport, fake_popen, err.getvalue()


def expected_post_body(target: str) -> bytes:
    return json.dumps(
        {
            "name": expected_runner_name(target),
            "runner_group_id": GROUP,
            "labels": [expected_label(target)],
            "work_folder": "_work",
        },
        separators=(",", ":"),
    ).encode("utf-8")


class SuccessDispatchTest(unittest.TestCase):
    def _assert_success_common(self, code, transport, popen, err, target):
        self.assertEqual(code, 0)
        self.assertNotIn(TOKEN, err)
        self.assertEqual(transport.post_count, 1)
        post = transport.requests[-1]
        self.assertEqual(post["url"], POST_URL)
        self.assertEqual(post["body"], expected_post_body(target))
        # The official generate-jitconfig body key is `name`; the wrong
        # `runner_name` key must never be sent.
        self.assertNotIn(b"runner_name", post["body"])
        self.assertIn(b'"name":"lumi-shadow-', post["body"])
        self.assertEqual(popen.calls[0]["argv"], list(RUNNER_CMD))
        self.assertIs(popen.calls[0]["kwargs"]["stdin"], subprocess.PIPE)
        child_env = popen.calls[0]["kwargs"]["env"]
        for key in (
            "LUMI_GITHUB_TOKEN", "GH_TOKEN", "GITHUB_TOKEN",
            "SOME_TOKEN_VAR", "FOO_AUTHORIZATION",
        ):
            self.assertNotIn(key, child_env)
        for key in ("PATH", "HOME", "http_proxy"):
            self.assertIn(key, child_env)
        self.assertEqual(popen.process.stdin.written, b"QUJDRA==\n")
        self.assertTrue(popen.process.stdin.closed)
        self.assertTrue(popen.process.waited)

    def test_arm64_full_success_exact_request_order(self):
        code, transport, popen, err = run_cli("arm64")
        self.assertEqual(
            [request["url"] for request in transport.requests],
            [
                RUN_URL, WORKFLOW_URL, REF_URL, JOBS_URL,
                RUN_URL, WORKFLOW_URL, REF_URL, JOBS_URL,
                POST_URL,
            ],
        )
        for request in transport.requests[:8]:
            self.assertEqual(request["method"], "GET")
            self.assertIsNone(request["body"])
            self.assertEqual(request["headers"], EXPECTED_HEADERS)
        post = transport.requests[8]
        self.assertEqual(post["method"], "POST")
        self.assertEqual(
            post["headers"],
            {**EXPECTED_HEADERS, "Content-Type": "application/json"},
        )
        self.assertEqual(post["body"], expected_post_body("arm64"))
        self._assert_success_common(code, transport, popen, err, "arm64")

    def test_x86_64_full_success(self):
        code, transport, popen, err = run_cli("x86_64")
        self.assertEqual(code, 0)
        self.assertEqual(transport.post_count, 1)
        self.assertEqual(
            transport.requests[-1]["body"],
            expected_post_body("x86_64"),
        )
        self._assert_success_common(code, transport, popen, err, "x86_64")

    def test_queued_job_started_at_without_runner_is_eligible(self):
        jobs = [
            make_gate(),
            make_build_job(
                started_at="2026-08-11T02:51:14Z",
                runner_id=0,
                runner_name="",
                runner_group_id=0,
            ),
        ]
        code, transport, popen, err = run_cli(
            "arm64", responses=responses_for("arm64", jobs=jobs)
        )
        self._assert_success_common(code, transport, popen, err, "arm64")

    def test_child_exit_code_propagated(self):
        code, _transport, _popen, _err = run_cli(exit_code=7)
        self.assertEqual(code, 7)

    def test_pagination_over_one_hundred_success(self):
        jobs = [make_gate(), make_build_job("arm64")] + [
            make_job(f"filler-{i}", labels=["self-hosted"]) for i in range(148)
        ]
        page1, page2 = jobs[:100], jobs[100:]
        link = (
            f'<{jit.API_BASE}/repos/{jit.REPO}/actions/runs/{RUN_ID}'
            f'/attempts/{ATTEMPT}/jobs?per_page=100&page=2>; rel="next"'
        )
        # GitHub reports the overall total_count on every page.
        page1_payload = {"total_count": 150, "jobs": page1}
        page2_payload = {"total_count": 150, "jobs": page2}
        responses = [
            (200, {}, make_run()),
            (200, {}, make_workflow()),
            (200, {}, make_main_ref()),
            (200, {"Link": link}, page1_payload),
            (200, {}, page2_payload),
            (200, {}, make_run()),
            (200, {}, make_workflow()),
            (200, {}, make_main_ref()),
            (200, {"Link": link}, page1_payload),
            (200, {}, page2_payload),
            (201, {}, make_jit_response("arm64")),
        ]
        code, transport, _popen, _err = run_cli(responses=responses)
        self.assertEqual(code, 0)
        self.assertEqual(transport.post_count, 1)
        urls = [request["url"] for request in transport.requests]
        self.assertEqual(urls.count(JOBS_URL), 2)
        self.assertEqual(urls.count(JOBS_URL.replace("&page=1", "&page=2")), 2)

    def test_exactly_one_hundred_jobs_without_next_link_success(self):
        jobs = [make_gate(), make_build_job("arm64")] + [
            make_job(f"filler-{i}", labels=["self-hosted"]) for i in range(98)
        ]
        responses = [
            (200, {}, make_run()),
            (200, {}, make_workflow()),
            (200, {}, make_main_ref()),
            (200, {}, jobs_payload(jobs)),
            (200, {}, make_run()),
            (200, {}, make_workflow()),
            (200, {}, make_main_ref()),
            (200, {}, jobs_payload(jobs)),
            (201, {}, make_jit_response("arm64")),
        ]
        code, transport, _popen, _err = run_cli(responses=responses)
        self.assertEqual(code, 0)
        self.assertEqual(transport.post_count, 1)


class FailClosedTest(unittest.TestCase):
    def assert_rejected(self, responses, expected_gets, target="arm64"):
        code, transport, _popen, err = run_cli(target=target, responses=responses)
        self.assertEqual(code, 1)
        self.assertEqual(transport.post_count, 0)
        self.assertEqual(len(transport.requests), expected_gets)
        self.assertNotIn(TOKEN, err)
        return err

    # -- run payload gates (one GET) -------------------------------------
    def test_run_id_mismatch(self):
        responses = responses_for()
        responses[0] = (200, {}, make_run(run_id=124))
        self.assert_rejected(responses, 1)

    def test_run_attempt_mismatch(self):
        responses = responses_for()
        responses[0] = (200, {}, make_run(attempt=2))
        self.assert_rejected(responses, 1)

    def test_event_pull_request_rejected(self):
        responses = responses_for()
        responses[0] = (200, {}, make_run(event="pull_request"))
        self.assert_rejected(responses, 1)

    def test_event_push_rejected(self):
        responses = responses_for()
        responses[0] = (200, {}, make_run(event="push"))
        self.assert_rejected(responses, 1)

    def test_run_pull_requests_non_empty_rejected(self):
        responses = responses_for()
        responses[0] = (200, {}, make_run(pull_requests=[{"number": 1}]))
        self.assert_rejected(responses, 1)

    def test_run_workflow_name_mismatch(self):
        responses = responses_for()
        responses[0] = (200, {}, make_run(name="other-workflow"))
        self.assert_rejected(responses, 1)

    def test_head_branch_not_main(self):
        responses = responses_for()
        responses[0] = (200, {}, make_run(head_branch="dev"))
        self.assert_rejected(responses, 1)

    def test_head_sha_short(self):
        responses = responses_for()
        responses[0] = (200, {}, make_run(head_sha="a" * 39))
        self.assert_rejected(responses, 1)

    def test_head_sha_not_hex(self):
        responses = responses_for()
        responses[0] = (200, {}, make_run(head_sha="z" * 40))
        self.assert_rejected(responses, 1)

    def test_workflow_id_invalid(self):
        responses = responses_for()
        responses[0] = (200, {}, make_run(workflow_id=0))
        self.assert_rejected(responses, 1)

    def test_repository_name_mismatch(self):
        responses = responses_for()
        responses[0] = (
            200,
            {},
            make_run(repository={"name": "other", "full_name": "Other/codex",
                                 "url": f"{jit.API_BASE}/repos/Other/codex"}),
        )
        self.assert_rejected(responses, 1)

    def test_head_repository_mismatch(self):
        responses = responses_for()
        responses[0] = (
            200,
            {},
            make_run(head_repository={"name": "other", "full_name": "Other/codex",
                                      "url": f"{jit.API_BASE}/repos/Other/codex"}),
        )
        self.assert_rejected(responses, 1)

    def test_repository_url_mismatch(self):
        responses = responses_for()
        responses[0] = (
            200,
            {},
            make_run(repository={"name": "codex", "full_name": jit.REPO,
                                 "url": "https://api.github.com/repos/other/codex"}),
        )
        self.assert_rejected(responses, 1)

    def test_run_status_completed(self):
        responses = responses_for()
        responses[0] = (200, {}, make_run(status="completed"))
        self.assert_rejected(responses, 1)

    def test_run_status_cancelled(self):
        responses = responses_for()
        responses[0] = (200, {}, make_run(status="cancelled"))
        self.assert_rejected(responses, 1)

    def test_run_conclusion_cancelled(self):
        responses = responses_for()
        responses[0] = (200, {}, make_run(conclusion="cancelled"))
        self.assert_rejected(responses, 1)

    def test_run_conclusion_success(self):
        responses = responses_for()
        responses[0] = (200, {}, make_run(conclusion="success"))
        self.assert_rejected(responses, 1)

    # -- workflow id resolution (two GETs) --------------------------------
    def test_workflow_path_mismatch(self):
        responses = responses_for()
        responses[1] = (200, {}, make_workflow(path=".github/workflows/other.yml"))
        self.assert_rejected(responses, 2)

    def test_workflow_name_mismatch(self):
        responses = responses_for()
        responses[1] = (200, {}, make_workflow(name="other-name"))
        self.assert_rejected(responses, 2)

    def test_workflow_id_mismatch(self):
        responses = responses_for()
        responses[1] = (200, {}, make_workflow(id=556))
        self.assert_rejected(responses, 2)

    def test_workflow_state_inactive(self):
        responses = responses_for()
        responses[1] = (200, {}, make_workflow(state="disabled_manually"))
        self.assert_rejected(responses, 2)

    # -- refs/heads/main (three GETs) -------------------------------------
    def test_main_ref_sha_mismatch(self):
        responses = responses_for()
        responses[2] = (200, {}, make_main_ref(sha="b" * 40))
        self.assert_rejected(responses, 3)

    def test_main_ref_not_a_commit(self):
        responses = responses_for()
        responses[2] = (200, {}, make_main_ref(object={"type": "tag", "sha": SHA}))
        self.assert_rejected(responses, 3)

    # -- attempt jobs (four GETs) -----------------------------------------
    def test_gate_job_missing(self):
        responses = responses_for(jobs=[make_build_job()])
        self.assert_rejected(responses, 4)

    def test_gate_job_duplicated(self):
        responses = responses_for(jobs=[make_gate(), make_gate(), make_build_job()])
        self.assert_rejected(responses, 4)

    def test_gate_job_not_completed(self):
        responses = responses_for(jobs=[make_gate(status="in_progress"), make_build_job()])
        self.assert_rejected(responses, 4)

    def test_gate_job_failed(self):
        responses = responses_for(
            jobs=[make_gate(conclusion="failure"), make_build_job()]
        )
        self.assert_rejected(responses, 4)

    def test_chosen_job_missing(self):
        responses = responses_for(jobs=[make_gate()])
        self.assert_rejected(responses, 4)

    def test_chosen_job_duplicated(self):
        responses = responses_for(
            jobs=[make_gate(), make_build_job(), make_build_job()]
        )
        self.assert_rejected(responses, 4)

    def test_chosen_job_not_queued(self):
        responses = responses_for(jobs=[make_gate(), make_build_job(status="in_progress")])
        self.assert_rejected(responses, 4)

    def test_chosen_job_concluded(self):
        responses = responses_for(
            jobs=[make_gate(), make_build_job(conclusion="success")]
        )
        self.assert_rejected(responses, 4)

    def test_chosen_job_runner_assigned(self):
        responses = responses_for(
            jobs=[make_gate(), make_build_job(runner_name="some-runner")]
        )
        self.assert_rejected(responses, 4)

    def test_chosen_job_runner_id_assigned(self):
        responses = responses_for(
            jobs=[make_gate(), make_build_job(runner_id=5)]
        )
        self.assert_rejected(responses, 4)

    def test_chosen_job_runner_group_assigned(self):
        responses = responses_for(
            jobs=[make_gate(), make_build_job(runner_group_id=3)]
        )
        self.assert_rejected(responses, 4)

    def test_chosen_job_run_id_mismatch(self):
        responses = responses_for(
            jobs=[make_gate(), make_build_job(run_id=999)]
        )
        self.assert_rejected(responses, 4)

    def test_chosen_job_workflow_name_mismatch(self):
        responses = responses_for(
            jobs=[make_gate(), make_build_job(workflow_name="other-workflow")]
        )
        self.assert_rejected(responses, 4)

    def test_chosen_job_head_branch_mismatch(self):
        responses = responses_for(
            jobs=[make_gate(), make_build_job(head_branch="dev")]
        )
        self.assert_rejected(responses, 4)

    def test_chosen_job_head_sha_mismatch(self):
        responses = responses_for(
            jobs=[make_gate(), make_build_job(head_sha="b" * 40)]
        )
        self.assert_rejected(responses, 4)

    def test_chosen_job_missing_expected_label(self):
        responses = responses_for(
            jobs=[make_gate(), make_build_job(labels=["self-hosted"])]
        )
        self.assert_rejected(responses, 4)

    def test_chosen_job_duplicate_expected_label(self):
        label = expected_label("arm64")
        responses = responses_for(
            jobs=[make_gate(), make_build_job(labels=["self-hosted", label, label])]
        )
        self.assert_rejected(responses, 4)

    def test_chosen_job_extra_custom_label(self):
        label = expected_label("arm64")
        responses = responses_for(
            jobs=[
                make_gate(),
                make_build_job(labels=["self-hosted", label, "lumi-shadow-arm64-999-9"]),
            ]
        )
        self.assert_rejected(responses, 4)

    def test_chosen_job_unallowlisted_read_only_label(self):
        label = expected_label("arm64")
        responses = responses_for(
            jobs=[make_gate(), make_build_job(labels=["self-hosted", "Linux", label])]
        )
        self.assert_rejected(responses, 4)

    def test_another_job_requests_derived_label(self):
        label = expected_label("arm64")
        responses = responses_for(
            jobs=[
                make_gate(),
                make_build_job(),
                make_job("filler", labels=[label]),
            ]
        )
        self.assert_rejected(responses, 4)

    def test_jobs_total_count_mismatch(self):
        responses = responses_for(
            jobs_payload_override={"total_count": 5, "jobs": make_jobs()}
        )
        self.assert_rejected(responses, 4)

    def test_jobs_pagination_truncated(self):
        jobs = [make_gate(), make_build_job("arm64")] + [
            make_job(f"filler-{i}", labels=["self-hosted"]) for i in range(148)
        ]
        responses = [
            (200, {}, make_run()),
            (200, {}, make_workflow()),
            (200, {}, make_main_ref()),
            (200, {}, {"total_count": 150, "jobs": jobs[:100]}),
        ]
        self.assert_rejected(responses, 4)

    def test_re_read_catches_job_pickup(self):
        # First read: queued. Final re-read: job got picked up -> reject.
        responses = responses_for()
        responses[7] = (
            200,
            {},
            jobs_payload([make_gate(), make_build_job(status="in_progress")]),
        )
        self.assert_rejected(responses, 8)

    def test_re_read_catches_run_completion(self):
        # First read: live. Final re-read: run completed -> reject.
        responses = responses_for()
        responses[4] = (200, {}, make_run(status="completed"))
        self.assert_rejected(responses, 5)

    def test_re_read_catches_workflow_change(self):
        responses = responses_for()
        responses[5] = (200, {}, make_workflow(state="disabled_manually"))
        self.assert_rejected(responses, 6)

    def test_re_read_catches_main_ref_change(self):
        responses = responses_for()
        responses[6] = (200, {}, make_main_ref(sha="b" * 40))
        self.assert_rejected(responses, 7)


class JitResponseValidationTest(unittest.TestCase):
    def assert_response_rejected(self, jit_response):
        responses = responses_for(jit_response=jit_response)
        code, transport, popen, err = run_cli(responses=responses)
        self.assertEqual(code, 1)
        self.assertEqual(transport.post_count, 1)
        self.assertEqual(popen.calls, [])
        self.assertNotIn(TOKEN, err)
        return err

    def assert_post_status_rejected(self, status):
        responses = responses_for()
        responses[8] = (status, {}, make_jit_response("arm64"))
        code, transport, popen, err = run_cli(responses=responses)
        self.assertEqual(code, 1)
        self.assertEqual(transport.post_count, 1)
        self.assertEqual(popen.calls, [])
        self.assertNotIn(TOKEN, err)
        return err

    def test_post_status_200_rejected(self):
        self.assert_post_status_rejected(200)

    def test_post_status_202_rejected(self):
        self.assert_post_status_rejected(202)

    def test_post_status_500_rejected(self):
        self.assert_post_status_rejected(500)

    def test_encoded_jit_config_missing(self):
        self.assert_response_rejected(make_jit_response(encoded_jit_config=None))

    def test_encoded_jit_config_empty(self):
        self.assert_response_rejected(make_jit_response(encoded=""))

    def test_encoded_jit_config_oversized(self):
        self.assert_response_rejected(
            make_jit_response(encoded="A" * (jit.MAX_ENCODED_JIT_CONFIG_LENGTH + 1))
        )

    def test_encoded_jit_config_at_limit_accepted(self):
        # Boundary at the shared conservative 64KiB hard cap (65536 bytes,
        # well under the Linux execve single-string limit that broke an
        # exported 262144-byte env value with E2BIG).
        responses = responses_for(
            jit_response=make_jit_response(
                encoded="A" * jit.MAX_ENCODED_JIT_CONFIG_LENGTH
            )
        )
        code, transport, _popen, err = run_cli(responses=responses)
        self.assertEqual(code, 0)
        self.assertEqual(transport.post_count, 1)
        self.assertNotIn(TOKEN, err)

    def test_encoded_jit_config_not_base64(self):
        self.assert_response_rejected(make_jit_response(encoded="%QUJDRA=="))

    def test_encoded_jit_config_wrong_length(self):
        self.assert_response_rejected(make_jit_response(encoded="QUJDRA"))

    def test_encoded_jit_config_excess_padding(self):
        self.assert_response_rejected(make_jit_response(encoded="QUJDRA==="))

    def test_runner_name_mismatch(self):
        self.assert_response_rejected(
            make_jit_response(runner_overrides={"name": "lumi-shadow-arm64-123-2"})
        )

    def test_runner_id_missing(self):
        self.assert_response_rejected(
            make_jit_response(runner_overrides={"id": None})
        )

    def test_runner_id_zero(self):
        self.assert_response_rejected(
            make_jit_response(runner_overrides={"id": 0})
        )

    def test_runner_status_online_rejected(self):
        # The documented JIT creation response marks the runner offline.
        self.assert_response_rejected(
            make_jit_response(runner_overrides={"status": "online"})
        )

    def test_runner_status_missing(self):
        self.assert_response_rejected(
            make_jit_response(runner_overrides={"status": None})
        )

    def test_runner_busy(self):
        self.assert_response_rejected(make_jit_response(runner_overrides={"busy": True}))

    def test_runner_missing_expected_custom_label(self):
        self.assert_response_rejected(
            make_jit_response(
                labels=[{"id": 1, "name": "self-hosted", "type": "read-only"}]
            )
        )

    def test_runner_live_repository_jit_read_only_label_accepted(self):
        label = expected_label("arm64")
        responses = responses_for(
            jit_response=make_jit_response(
                labels=[{"id": 0, "name": label, "type": "read-only"}]
            )
        )
        code, transport, _popen, err = run_cli(responses=responses)
        self.assertEqual(code, 0)
        self.assertEqual(transport.post_count, 1)
        self.assertNotIn(TOKEN, err)

    def test_runner_extra_custom_label(self):
        label = expected_label("arm64")
        self.assert_response_rejected(
            make_jit_response(
                labels=[
                    {"id": 1, "name": "self-hosted", "type": "read-only"},
                    {"id": 2, "name": label, "type": "custom"},
                    {"id": 3, "name": "extra-label", "type": "custom"},
                ]
            )
        )

    def test_runner_duplicate_custom_label(self):
        label = expected_label("arm64")
        self.assert_response_rejected(
            make_jit_response(
                labels=[
                    {"id": 1, "name": label, "type": "custom"},
                    {"id": 2, "name": label, "type": "custom"},
                ]
            )
        )

    def test_runner_invalid_label_type(self):
        label = expected_label("arm64")
        self.assert_response_rejected(
            make_jit_response(
                labels=[{"id": 1, "name": label, "type": "weird"}]
            )
        )

    def test_runner_underscore_label_type_rejected(self):
        # The official response type string is "read-only" (hyphen); the
        # underscore form must be rejected.
        label = expected_label("arm64")
        self.assert_response_rejected(
            make_jit_response(
                labels=[
                    {"id": 1, "name": "self-hosted", "type": "read_only"},
                    {"id": 2, "name": label, "type": "custom"},
                ]
            )
        )

    def test_runner_read_only_labels_allowed(self):
        label = expected_label("arm64")
        responses = responses_for(
            jit_response=make_jit_response(
                labels=[
                    {"id": 1, "name": "self-hosted", "type": "read-only"},
                    {"id": 2, "name": "X64", "type": "read-only"},
                    {"id": 3, "name": "macOS", "type": "read-only"},
                    {"id": 4, "name": label, "type": "custom"},
                ]
            )
        )
        code, transport, _popen, err = run_cli(responses=responses)
        self.assertEqual(code, 0)
        self.assertEqual(transport.post_count, 1)
        self.assertNotIn(TOKEN, err)


class SecretRedactionTest(unittest.TestCase):
    def _cli(self, transport, popen=None):
        err = io.StringIO()
        argv = [
            "--run-id", str(RUN_ID), "--run-attempt", str(ATTEMPT),
            "--target", "arm64", "--runner-group-id", str(GROUP), "--",
            *RUNNER_CMD,
        ]
        with contextlib.redirect_stderr(err):
            code = jit.main(
                argv,
                environ={"LUMI_GITHUB_TOKEN": TOKEN, "PATH": "/usr/bin"},
                transport=transport,
                popen=popen if popen is not None else FakePopen(),
            )
        return code, err.getvalue()

    def test_transport_exception_redacted(self):
        def exploding(_method, _url, _headers, _body):
            raise RuntimeError(f"transport exploded with {TOKEN}")

        code, err = self._cli(exploding)
        self.assertEqual(code, 1)
        self.assertNotIn(TOKEN, err)
        self.assertIn("dispatch failed", err)

    def test_http_error_body_never_printed(self):
        def http_error(_method, _url, _headers, _body):
            return 500, {}, json.dumps({"message": f"server saw {TOKEN}"}).encode()

        code, err = self._cli(http_error)
        self.assertEqual(code, 1)
        self.assertNotIn(TOKEN, err)
        self.assertIn("HTTP 500", err)

    def test_malformed_json_body_never_printed(self):
        def bad_json(_method, _url, _headers, _body):
            return 200, {}, f"not json {TOKEN}".encode()

        code, err = self._cli(bad_json)
        self.assertEqual(code, 1)
        self.assertNotIn(TOKEN, err)
        self.assertIn("invalid JSON", err)

    def test_child_spawn_failure_redacted(self):
        popen = FakePopen(spawn_error=FileNotFoundError())
        responses = responses_for()
        transport = MockTransport(responses)
        err = io.StringIO()
        argv = [
            "--run-id", str(RUN_ID), "--run-attempt", str(ATTEMPT),
            "--target", "arm64", "--runner-group-id", str(GROUP), "--",
            *RUNNER_CMD,
        ]
        with contextlib.redirect_stderr(err):
            code = jit.main(
                argv,
                environ={"LUMI_GITHUB_TOKEN": TOKEN, "PATH": "/usr/bin"},
                transport=transport,
                popen=popen,
            )
        self.assertEqual(code, 1)
        self.assertNotIn(TOKEN, err.getvalue())
        self.assertEqual(transport.post_count, 1)

    def test_child_broken_pipe_redacted(self):
        popen = FakePopen(write_error=BrokenPipeError())
        responses = responses_for()
        transport = MockTransport(responses)
        err = io.StringIO()
        argv = [
            "--run-id", str(RUN_ID), "--run-attempt", str(ATTEMPT),
            "--target", "arm64", "--runner-group-id", str(GROUP), "--",
            *RUNNER_CMD,
        ]
        with contextlib.redirect_stderr(err):
            code = jit.main(
                argv,
                environ={"LUMI_GITHUB_TOKEN": TOKEN, "PATH": "/usr/bin"},
                transport=transport,
                popen=popen,
            )
        self.assertEqual(code, 1)
        self.assertNotIn(TOKEN, err.getvalue())


class CliTest(unittest.TestCase):
    def _main(self, argv, environ=None):
        return jit.main(
            argv,
            environ=environ if environ is not None else {"LUMI_GITHUB_TOKEN": TOKEN},
            transport=MockTransport([]),
            popen=FakePopen(),
        )

    def test_missing_double_dash(self):
        with self.assertRaises(SystemExit) as cm:
            self._main([
                "--run-id", str(RUN_ID), "--run-attempt", str(ATTEMPT),
                "--target", "arm64", "--runner-group-id", str(GROUP), "./run.sh",
            ])
        self.assertEqual(cm.exception.code, 2)

    def test_empty_command_after_double_dash(self):
        with self.assertRaises(SystemExit) as cm:
            self._main([
                "--run-id", str(RUN_ID), "--run-attempt", str(ATTEMPT),
                "--target", "arm64", "--runner-group-id", str(GROUP), "--",
            ])
        self.assertEqual(cm.exception.code, 2)

    def test_zero_run_id(self):
        with self.assertRaises(SystemExit) as cm:
            self._main([
                "--run-id", "0", "--run-attempt", str(ATTEMPT),
                "--target", "arm64", "--runner-group-id", str(GROUP), "--", "./run.sh",
            ])
        self.assertEqual(cm.exception.code, 2)

    def test_non_integer_run_id(self):
        with self.assertRaises(SystemExit) as cm:
            self._main([
                "--run-id", "abc", "--run-attempt", str(ATTEMPT),
                "--target", "arm64", "--runner-group-id", str(GROUP), "--", "./run.sh",
            ])
        self.assertEqual(cm.exception.code, 2)

    def test_unknown_target(self):
        with self.assertRaises(SystemExit) as cm:
            self._main([
                "--run-id", str(RUN_ID), "--run-attempt", str(ATTEMPT),
                "--target", "mips", "--runner-group-id", str(GROUP), "--", "./run.sh",
            ])
        self.assertEqual(cm.exception.code, 2)

    def test_token_absent_makes_no_api_calls(self):
        transport = MockTransport([])
        err = io.StringIO()
        argv = [
            "--run-id", str(RUN_ID), "--run-attempt", str(ATTEMPT),
            "--target", "arm64", "--runner-group-id", str(GROUP), "--", "./run.sh",
        ]
        with contextlib.redirect_stderr(err):
            code = jit.main(argv, environ={}, transport=transport, popen=FakePopen())
        self.assertEqual(code, 1)
        self.assertEqual(transport.requests, [])
        self.assertIn("LUMI_GITHUB_TOKEN is not set", err.getvalue())


class ChildCommandValidationTest(unittest.TestCase):
    def _cli(self, argv, path=None, environ=None):
        transport = MockTransport(responses_for())
        popen = FakePopen()
        err = io.StringIO()
        env = {"LUMI_GITHUB_TOKEN": TOKEN}
        if path is not None:
            env["PATH"] = path
        if environ is not None:
            env = dict(environ)
        with contextlib.redirect_stderr(err):
            code = jit.main(argv, environ=env, transport=transport, popen=popen)
        return code, transport, popen, err.getvalue()

    def _argv(self, command):
        return [
            "--run-id", str(RUN_ID), "--run-attempt", str(ATTEMPT),
            "--target", "arm64", "--runner-group-id", str(GROUP), "--",
            *command,
        ]

    def test_missing_local_command_no_post(self):
        # Relative path with a directory component: checked directly, not
        # via PATH; missing on disk -> reject before the POST.
        code, transport, popen, err = self._cli(
            self._argv(["./definitely-missing-runner"]), path="/usr/bin:/bin"
        )
        self.assertEqual(code, 1)
        self.assertEqual(transport.post_count, 0)
        self.assertEqual(transport.requests[-1]["method"], "GET")
        self.assertEqual(popen.calls, [])
        self.assertIn("not resolvable", err)
        self.assertNotIn(TOKEN, err)

    def test_missing_bare_command_on_child_path_no_post(self):
        code, transport, popen, err = self._cli(
            self._argv(["lumi-definitely-missing-cmd"]), path="/usr/bin:/bin"
        )
        self.assertEqual(code, 1)
        self.assertEqual(transport.post_count, 0)
        self.assertEqual(transport.requests[-1]["method"], "GET")
        self.assertEqual(popen.calls, [])
        self.assertIn("not resolvable", err)

    def test_no_path_var_no_post(self):
        # No PATH in the sanitized child environment: exec semantics fall
        # back to the default search path, where the command is absent.
        code, transport, popen, err = self._cli(
            self._argv(["lumi-definitely-missing-cmd"]),
            environ={"LUMI_GITHUB_TOKEN": TOKEN},
        )
        self.assertEqual(code, 1)
        self.assertEqual(transport.post_count, 0)
        self.assertEqual(popen.calls, [])
        self.assertIn("not resolvable", err)

    def test_non_executable_command_no_post(self):
        with tempfile.TemporaryDirectory() as tmp:
            script = Path(tmp) / "runner.sh"
            script.write_text("#!/bin/sh\nexit 0\n", encoding="utf-8")
            script.chmod(0o644)  # readable but not executable
            code, transport, popen, err = self._cli(
                self._argv([str(script)]), path="/usr/bin:/bin"
            )
        self.assertEqual(code, 1)
        self.assertEqual(transport.post_count, 0)
        self.assertEqual(transport.requests[-1]["method"], "GET")
        self.assertEqual(popen.calls, [])
        self.assertIn("not resolvable", err)


if __name__ == "__main__":
    unittest.main()
