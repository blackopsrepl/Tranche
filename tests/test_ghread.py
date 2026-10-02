"""Offline tests for the bounded read transport: URL policy, bounds, request shape.

No network and no `gh` invocation: `run_gh` is replaced with recorded output and
`invoke` with recorded responses, so the assertions are about the request that
would be constructed and the rules applied to the answer.
"""
import json
import sys
import unittest
from unittest.mock import patch

import ghread


def block(status, headers=(), body=b"", version=b"HTTP/2.0"):
    lines = [version + b" " + status]
    lines += [f"{key}: {value}".encode() for key, value in headers]
    return b"\r\n".join(lines) + b"\r\n\r\n" + body


class UrlPolicyTests(unittest.TestCase):
    def test_accepts_only_https_api_github_com_without_fragment_or_credentials(self):
        self.assertEqual(ghread.validate_url("https://api.github.com/repos/o/r/pulls/1"),
                         "https://api.github.com/repos/o/r/pulls/1")
        for url in ("http://api.github.com/repos/o/r",
                    "https://api.github.com.evil.example/repos/o/r",
                    "https://evil.example/repos/o/r",
                    "https://token@api.github.com/repos/o/r",
                    "https://api.github.com/repos/o/r#frag",
                    "https://api.github.com/repos/o/r?access_token=abc",
                    "https://api.github.com/repos/o/r?client_secret=x",
                    "https://api.github.com/repos/o/r\nHost: evil",
                    "",
                    None,
                    7):
            with self.subTest(url=url), self.assertRaises(ghread.ReadError):
                ghread.validate_url(url)

    def test_repository_prefix_is_enforced_when_requested(self):
        ghread.validate_url("https://api.github.com/repos/o/r/pulls/1", "o/r")
        for url in ("https://api.github.com/repos/o/other/pulls/1",
                    "https://api.github.com/repos/other/r/pulls/1",
                    "https://api.github.com/graphql"):
            with self.subTest(url=url), self.assertRaises(ghread.ReadError):
                ghread.validate_url(url, "o/r")

    def test_next_link_reads_only_the_next_relation_and_validates_it(self):
        header = ('<https://api.github.com/repos/o/r/pulls?page=2>; rel="next", '
                  '<https://api.github.com/repos/o/r/pulls?page=9>; rel="last"')
        self.assertEqual(ghread.parse_next_link(header),
                         "https://api.github.com/repos/o/r/pulls?page=2")
        self.assertEqual(ghread.parse_next_link('<https://api.github.com/x>; rel="last"'), None)
        self.assertEqual(ghread.parse_next_link(None), None)
        self.assertEqual(ghread.parse_next_link(""), None)
        for hostile in ('<https://evil.example/x>; rel="next"',
                        '<http://api.github.com/x>; rel="next"',
                        '<https://api.github.com/x?token=abc>; rel="next"'):
            with self.subTest(hostile=hostile), self.assertRaises(ghread.ReadError):
                ghread.parse_next_link(hostile)


class BudgetTests(unittest.TestCase):
    def test_every_attempt_counts_including_failures_and_reserved_capacity(self):
        budget = ghread.Budget(3, reserved=1)
        budget.charge()
        budget.charge()
        with self.assertRaises(ghread.BudgetExhausted):
            budget.charge()  # the third would eat the reserve
        budget.charge(reserve=0)
        self.assertEqual(budget.used, 3)
        self.assertEqual(budget.remaining, 0)
        with self.assertRaises(ghread.BudgetExhausted):
            budget.charge(reserve=0)
        budget.failures += 2
        summary = budget.summary()
        self.assertEqual((summary["limit"], summary["used"], summary["reserved"],
                          summary["failures"]), (3, 3, 1, 2))
        for limit, reserved in ((-1, 0), (2, 5), (2, -1), (2.5, 0), (True, 0)):
            with self.subTest(limit=limit, reserved=reserved), self.assertRaises(ghread.ReadError):
                ghread.Budget(limit, reserved=reserved)


class ResponseParsingTests(unittest.TestCase):
    def test_multi_hop_include_output_uses_the_final_status_and_headers(self):
        raw = (block(b"301 Moved Permanently", [("location", "https://api.github.com/b")], b"")
               + block(b"200 OK", [("content-type", "application/json; charset=utf-8"),
                                   ("link", '<https://api.github.com/repos/o/r/pulls?page=2>; rel="next"')],
                       b'[{"number":1}]'))
        blocks, body = ghread._split_response(raw)
        self.assertEqual(len(blocks), 2)
        self.assertEqual(ghread._status(blocks[-1]), 200)
        self.assertEqual(body, b'[{"number":1}]')
        self.assertEqual(ghread._headers(blocks[-1])["content-type"],
                         "application/json; charset=utf-8")
        response = ghread.Response(200, "https://api.github.com/x", ghread._headers(blocks[-1]), body)
        self.assertEqual(response.next_url, "https://api.github.com/repos/o/r/pulls?page=2")
        self.assertEqual(response.json(), [{"number": 1}])

    def test_truncated_or_unreadable_bodies_are_never_parsed_as_json(self):
        response = ghread.Response(200, "u", {}, b'{"partial"', truncated=True)
        with self.assertRaises(ghread.ResponseTooLarge):
            response.json()
        with self.assertRaises(ghread.ReadError):
            ghread.Response(200, "u", {}, b"not json").json()
        with self.assertRaises(ghread.ReadError):
            ghread._split_response(b"not an http response")

    def test_declared_oversized_body_is_refused_before_it_is_read(self):
        result = ghread.RawResult(0, block(b"200 OK", [("content-length", "999999999")], b"{}"), b"")
        with patch.object(ghread, "run_gh", return_value=result):
            with self.assertRaises(ghread.ResponseTooLarge):
                ghread.invoke(["gh", "api", "x"], max_bytes=1024)


class RequestConstructionTests(unittest.TestCase):
    def test_rest_read_is_an_explicit_get_with_bounded_output(self):
        result = ghread.RawResult(0, block(b"200 OK", [("content-type", "application/json")],
                                           b'{"ok":true}'), b"")
        recorded = {}

        def fake_run(argv, **kwargs):
            recorded["argv"] = argv
            recorded["kwargs"] = kwargs
            return result

        with patch.object(ghread, "run_gh", side_effect=fake_run):
            response = ghread.read("https://api.github.com/repos/o/r/pulls/1",
                                   accept="application/vnd.github.diff", budget=ghread.Budget(5))
        argv = recorded["argv"]
        self.assertEqual(argv[:4], ["gh", "api", "--include", "--method"])
        self.assertEqual(argv[4], "GET")
        self.assertIn("https://api.github.com/repos/o/r/pulls/1", argv)
        self.assertLess(argv.index("https://api.github.com/repos/o/r/pulls/1"),
                        argv.index("accept: application/vnd.github.diff"))
        self.assertEqual(argv[argv.index("accept: application/vnd.github.diff") - 1], "-H")
        self.assertNotIn("--input", argv)
        joined = " ".join(argv).lower()
        self.assertNotIn("authorization", joined)
        self.assertNotIn("token", joined)
        self.assertEqual(response.body, b'{"ok":true}')
        self.assertEqual(response.status, 200)

    def test_redirects_are_followed_only_within_the_allowed_origin(self):
        moved = ghread.RawResult(0, block(b"302 Found",
                                          [("location", "https://api.github.com/repos/o/r/pulls/2")],
                                          b""), b"")
        final = ghread.RawResult(0, block(b"200 OK", [("content-type", "application/json")], b"[]"), b"")
        calls = []

        def fake_run(argv, **kwargs):
            calls.append(argv[-1])
            return moved if len(calls) == 1 else final

        with patch.object(ghread, "run_gh", side_effect=fake_run):
            response = ghread.read("https://api.github.com/repos/o/r/pulls/1", budget=ghread.Budget(5))
        self.assertEqual(len(calls), 2)
        self.assertEqual(response.url, "https://api.github.com/repos/o/r/pulls/2")
        with patch.object(ghread, "run_gh", return_value=ghread.RawResult(
                0, block(b"302 Found", [("location", "https://evil.example/x")], b""), b"")):
            with self.assertRaises(ghread.ReadError):
                ghread.read("https://api.github.com/repos/o/r/pulls/1", budget=ghread.Budget(5))

    def test_every_attempt_is_charged_and_failures_are_counted(self):
        budget = ghread.Budget(2)
        with patch.object(ghread, "run_gh", return_value=ghread.RawResult(
                1, block(b"500 Server Error", [], b""), b"boom")):
            with self.assertRaises(ghread.ReadError):
                ghread.read("https://api.github.com/repos/o/r/pulls/1", budget=budget)
        self.assertEqual(budget.used, 1)
        self.assertEqual(budget.failures, 1)
        with patch.object(ghread, "run_gh", return_value=ghread.RawResult(
                0, block(b"404 Not Found", [], b"{}"), b"")) as run:
            with self.assertRaisesRegex(ghread.ReadError, "404"):
                ghread.read("https://api.github.com/repos/o/r/pulls/1", budget=budget)
        self.assertEqual(budget.used, 2)
        with self.assertRaises(ghread.BudgetExhausted):
            ghread.read("https://api.github.com/repos/o/r/pulls/1", budget=budget)
        self.assertEqual(run.call_count, 1)

    def test_graphql_is_a_query_only_and_never_carries_the_document_in_argv(self):
        result = ghread.RawResult(0, block(b"200 OK", [("content-type", "application/json")],
                                           b'{"data":{"ok":true}}'), b"")
        recorded = {}

        def fake_run(argv, **kwargs):
            recorded["argv"] = argv
            recorded["stdin"] = kwargs.get("stdin_data")
            return result

        with patch.object(ghread, "run_gh", side_effect=fake_run):
            response = ghread.graphql(
                "query($owner:String!){ repository(owner:$owner){ id } }",
                {"owner": "o"}, budget=ghread.Budget(4))
        self.assertEqual(recorded["argv"][-2:], ["--input", "-"])
        self.assertEqual(recorded["argv"][recorded["argv"].index("--method") + 1], "POST")
        payload = json.loads(recorded["stdin"])
        self.assertIn("query", payload["query"])
        self.assertEqual(payload["variables"], {"owner": "o"})
        self.assertEqual(response.json()["data"], {"ok": True})
        for document in ("mutation { deleteThing }", "subscription { x }",
                         "not a query", "", None):
            with self.subTest(document=document), self.assertRaises(ghread.ReadError):
                ghread.graphql(document)

    def test_diagnostics_are_short_and_credential_free(self):
        detail = ghread._diagnostic(b"gh: Not Found (HTTP 404)\nfatal: token=ghp_secretvalue\n")
        self.assertNotIn("ghp_secretvalue", detail)
        self.assertIn("REDACTED", detail)


class SubprocessBoundTests(unittest.TestCase):
    """The one place a real child process runs: output must be genuinely bounded."""

    def test_oversized_stdout_is_truncated_and_the_child_is_stopped(self):
        writer = "import sys; sys.stdout.write('x' * 5_000_000)"
        result = ghread.run_gh([sys.executable, "-c", writer], timeout=30, max_bytes=4096)
        self.assertTrue(result.truncated)
        self.assertEqual(len(result.stdout), 4096)
        self.assertNotEqual(result.returncode, 0)  # killed, not merely ignored

    def test_normal_output_is_returned_intact_with_stderr_captured(self):
        script = "import sys; sys.stdout.write('{\"a\":1}'); sys.stderr.write('note')"
        result = ghread.run_gh([sys.executable, "-c", script], timeout=30, max_bytes=4096)
        self.assertFalse(result.truncated)
        self.assertEqual(result.stdout, b'{"a":1}')
        self.assertEqual(result.stderr, b"note")
        self.assertEqual(result.returncode, 0)

    def test_a_hanging_child_is_killed_within_the_timeout(self):
        script = "import time; time.sleep(300)"
        result = ghread.run_gh([sys.executable, "-c", script], timeout=1, max_bytes=4096)
        self.assertTrue(result.timed_out)
        self.assertEqual(result.returncode, -9)

    def test_stdin_payload_reaches_the_child(self):
        script = "import sys; sys.stdout.buffer.write(sys.stdin.buffer.read())"
        result = ghread.run_gh([sys.executable, "-c", script], timeout=30,
                               max_bytes=4096, stdin_data=b'{"query":"query{x}"}')
        self.assertEqual(result.stdout, b'{"query":"query{x}"}')
        self.assertEqual(result.returncode, 0)


class InvokeIntegrationTests(unittest.TestCase):
    def test_invoke_parses_recorded_gh_output_without_running_gh(self):
        raw = block(b"200 OK", [("content-type", "application/json; charset=utf-8")], b"[1,2,3]")
        with patch.object(ghread, "run_gh", return_value=ghread.RawResult(0, raw, b"")):
            status, headers, body, truncated, hops = ghread.invoke(["gh", "api", "x"])
        self.assertEqual((status, body, truncated, hops),
                         (200, b"[1,2,3]", False, 1))
        self.assertEqual(headers["content-type"], "application/json; charset=utf-8")
        self.assertIsInstance(hops, int)

    def test_gh_failure_without_output_becomes_a_read_error(self):
        with patch.object(ghread, "run_gh", return_value=ghread.RawResult(
                1, b"", b"gh: authentication required\n")):
            with self.assertRaisesRegex(ghread.ReadError, "authentication required"):
                ghread.invoke(["gh", "api", "x"])


if __name__ == "__main__":
    unittest.main()


class MutationRefusalTests(unittest.TestCase):
    """The GraphQL seam must refuse anything that is not a read-only query."""

    def test_mutations_and_subscriptions_are_refused_before_any_request(self):
        recorded = []

        def fake_run(argv, **kwargs):
            recorded.append(argv)
            return ghread.RawResult(0, b'HTTP/2.0 200 OK\r\n\r\n{"data":{}}', b"")

        for document in ("mutation { deleteIssue(input:{}) { clientMutationId } }",
                         "query { repository(owner:\"o\",name:\"r\") { id } } "
                         "mutation { deleteThing }",
                         "subscription { issues { number } }",
                         "{ repository(owner:\"o\",name:\"r\") { id } }"):
            with self.subTest(document=document[:24]):
                with patch.object(ghread, "run_gh", side_effect=fake_run):
                    if "mutation" in document or "subscription" in document:
                        with self.assertRaises(ghread.ReadError):
                            ghread.graphql(document)
                    else:
                        ghread.graphql(document)
        # Exactly the one legal query reached a subprocess; every mutation,
        # subscription and mixed document was refused before any request.
        self.assertEqual(len(recorded), 1)
