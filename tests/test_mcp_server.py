"""Offline MCP core tests; importing the core never requires the SDK."""
import importlib.util
import json
from pathlib import Path
import sys
import unittest

import hostpython
import tranche
from tests import test_tranche as fixtures

pr = fixtures.pr
answers = fixtures.answers


class MCPTests(unittest.TestCase):
    setUp = fixtures.WorkflowTests.setUp
    inputs = fixtures.WorkflowTests.inputs
    judgments = fixtures.WorkflowTests.judgments
    pairs = fixtures.WorkflowTests.pairs
    cluster = fixtures.WorkflowTests.cluster

    def reports(self):
        prs = self.inputs([pr(1), pr(2)])
        self.judgments(prs)
        self.pairs(prs, [])
        self.cluster()
        dupes = json.loads((self.out / "dupes.json").read_text())
        judgments = tranche.current_judgments(prs)
        tranche.atomic_json(self.out / "batches.json", tranche.merge_batches(
            dupes, judgments, prs, tranche.digest(dupes)))
        tranche.atomic_json(self.out / "parked.json", tranche.parked_payload(
            tranche.park_state(dupes, judgments, prs), prs, judgments, tranche.digest(dupes)))
        return prs

    def core(self):
        self.assertIsNotNone(importlib.util.find_spec("mcp_server"), "MCP core missing")
        import mcp_server
        return mcp_server

    def test_surface_exposes_bound_read_only_observation(self):
        self.reports()
        result = self.core().Reports().surface()
        self.assertEqual(result["repo"], tranche.REPO)
        self.assertEqual(result["summary"]["prs_in_corpus"], 2)
        self.assertEqual(result["summary"]["judged"], 2)
        self.assertEqual(result["activity"]["head_moved"], 0)
        self.assertEqual(result["activity"]["idle_since_known"], 2)
        self.assertIn("idle_30d", result["activity"])
        summary_path = self.out / "summary.json"
        summary = json.loads(summary_path.read_text())
        summary.update(prs_in_corpus=999999, judged=999999)
        summary_path.write_text(json.dumps(summary))
        computed = self.core().Reports().surface()["summary"]
        self.assertEqual(computed["prs_in_corpus"], 2)
        self.assertEqual(computed["judged"], 2)
        self.assertIn("not", result["disclaimer"])
        self.assertEqual(result["digests"]["batches.json"],
                         tranche.digest(json.loads((self.out / "batches.json").read_text())))
        self.assertEqual(result["digests"]["report_binding"],
                         json.loads((self.out / "summary.json").read_text())["report_binding"])

    def test_every_call_refuses_changed_bound_inputs(self):
        core = self.core()
        for target, field, value in (
            ("summary.json", "allow_unbound", True),
            ("summary.json", "format_version", 1),
            ("summary.json", "repo", "foreign/repo"),
            ("summary.json", "report_binding", "wrong"),
            ("clusters.json", "tampered", True),
            ("dupes.json", "tampered", True),
            ("batches.json", "batch_size", 99),
        ):
            with self.subTest(target=target, field=field):
                self.reports()
                server = core.Reports()
                server.surface()
                path = self.out / target
                data = json.loads(path.read_text())
                data[field] = value
                path.write_text(json.dumps(data))
                with self.assertRaises(core.ReportError):
                    server.surface()
        self.reports()
        self.inputs([pr(1, head={"sha": "changed"}), pr(2)])
        with self.assertRaises(core.ReportError):
            core.Reports().surface()
        self.reports()
        self.judgments(tranche.load_prs(), custom=dict(fixtures.answers(), risk={"score": 4}))
        with self.assertRaises(core.ReportError):
            core.Reports().surface()
        prs = self.reports()
        self.pairs(prs, [(1, 2, "same_change", 0.9)])
        with self.assertRaises(core.ReportError):
            core.Reports().surface()

    def test_read_refuses_concurrent_changes_malformed_or_oversized_files(self):
        from unittest.mock import patch
        core = self.core()
        self.reports()
        original = tranche.current_pairs

        def change(*args, **kwargs):
            result = original(*args, **kwargs)
            with tranche.JUDGMENTS_PATH.open("a") as stream:
                stream.write("\n")
            return result

        with patch.object(tranche, "current_pairs", side_effect=change):
            with self.assertRaisesRegex(core.ReportError, "changed during"):
                core.Reports().surface()
        self.reports()
        (self.out / "summary.json").write_text("{")
        with self.assertRaises(core.ReportError):
            core.Reports().surface()
        self.reports()
        with patch.object(core, "MAX_FILE_BYTES", 1):
            with self.assertRaisesRegex(core.ReportError, "limit"):
                core.Reports().surface()

    def test_user_experience_category_is_derived_and_filterable(self):
        self.reports()
        records = [json.loads(line) for line in tranche.JUDGMENTS_PATH.read_text().splitlines()]
        records[0]["answers"]["category"] = {"choice": "user-experience"}
        tranche.JUDGMENTS_PATH.write_text("".join(json.dumps(r) + "\n" for r in records))
        self.cluster()
        (self.out / "batches.json").unlink()
        server = self.core().Reports()
        surface = server.surface()
        self.assertEqual(surface["category_counts"],
                         {"user-experience": 1, fixtures.answers()["category"]["choice"]: 1})
        self.assertIn("user-experience", surface["filters"]["categories"])
        result = server.query(category="user-experience")
        self.assertEqual(result["total"], 1)
        self.assertEqual(result["items"][0]["number"], 1)

    def test_query_paginates_security_first_preserving_unknowns_and_provenance(self):
        self.reports()
        prs = tranche.load_prs()
        records = [json.loads(line) for line in tranche.JUDGMENTS_PATH.read_text().splitlines()]
        records[1]["answers"]["security_flag"] = {"noul": 0.9}
        records[1]["answers"]["risk"] = {"score": None}
        tranche.JUDGMENTS_PATH.write_text("".join(json.dumps(r) + "\n" for r in records))
        self.cluster()
        (self.out / "batches.json").unlink()
        server = self.core().Reports()
        surface = server.surface()
        self.assertEqual(surface["category_counts"], {fixtures.answers()["category"]["choice"]: 2})
        for queue, descriptor in surface["queues"].items():
            result = server.query(queue=queue)
            self.assertEqual(descriptor["count"], result["total"])
            self.assertEqual(descriptor["members"], [item["number"] for item in result["items"]])
        result = server.query(limit=1)
        self.assertEqual(result["total"], 2)
        self.assertEqual(result["next_offset"], 1)
        row = result["items"][0]
        self.assertEqual(row["number"], 2)
        self.assertIsNone(row["risk"])
        self.assertEqual(row["head_sha"], prs[2]["head_sha"])
        self.assertEqual(row["source_digest"], prs[2]["source_digest"])
        self.assertIsNone(row["answers"]["dupe_signal"]["noul"])
        self.assertEqual(server.query(offset=1)["items"][0]["number"], 1)
        self.assertEqual(server.query(queue="security")["total"], 1)
        self.assertEqual(server.query(category="security-review")["total"], 1)
        self.assertEqual(server.query(risk_band="unknown")["total"], 1)
        self.assertEqual(server.query(security=False)["items"][0]["number"], 1)
        self.assertEqual(server.query(finished_form=2)["total"], 2)
        self.assertEqual(server.query(text="@fixture #1")["total"], 1)
        self.assertEqual(server.query(queue="candidates")["total"], 1)
        self.assertEqual(server.query(queue="senior")["total"], 1)
        self.assertEqual(server.query(queue="followup")["total"], 0)
        self.assertEqual(server.query(queue="related")["total"], 0)

    def test_query_refuses_malformed_and_unbounded_arguments(self):
        self.reports()
        core = self.core()
        for kwargs in ({"offset": -1}, {"offset": True}, {"offset": 100001},
                       {"limit": 0}, {"limit": 101}, {"limit": "1"},
                       {"category": "fake"}, {"risk_band": "fake"}, {"queue": "fake"},
                       {"security": 1}, {"finished_form": float("nan")},
                       {"finished_form": True}, {"finished_form": 4},
                       {"text": None}, {"text": "x" * 513},
                       {"batch": "../batches.json"}):
            with self.subTest(kwargs=kwargs), self.assertRaises(core.ReportError):
                core.Reports().query(**kwargs)

    def test_pick_and_next_prompt_match_batches_byte_for_byte(self):
        self.reports()
        core = self.core()
        server = core.Reports()
        expected = json.loads((self.out / "batches.json").read_text())["batches"][0]
        picked = server.pick("B001")
        self.assertEqual(picked["batch"], expected)
        self.assertEqual({p["number"] for p in picked["prs"]}, set(expected["members"]))
        self.assertEqual(set(picked["activity"]), {str(n) for n in expected["members"]})
        self.assertEqual(server.next_prompt()["activity"], picked["activity"])
        self.assertEqual(server.query(batch="B001")["total"], expected["count"])
        self.assertEqual(server.next_prompt()["batch"], expected)
        self.assertIsNone(server.next_prompt(after=1)["batch"])
        self.assertIsNone(server.next_prompt(after="B001")["batch"])
        self.assertEqual(server.next_prompt(after=0)["batch"]["review_prompt"].encode(),
                         expected["review_prompt"].encode())
        for value in ("../B001", "B999", None, 1):
            with self.subTest(value=value), self.assertRaises(core.ReportError):
                server.pick(value)
        for value in (True, -1, 1000000, "B999", "1", 1.5):
            with self.subTest(value=value), self.assertRaises(core.ReportError):
                server.next_prompt(after=value)
        (self.out / "batches.json").unlink()
        self.assertFalse(server.surface()["batches_available"])
        with self.assertRaises(core.ReportError):
            server.pick("B001")
        with self.assertRaises(core.ReportError):
            server.next_prompt()

    def test_related_exposes_conflicts_without_survivors(self):
        prs = self.reports()
        self.pairs(prs, [(1, 2, "related_but_different", 0.8)])
        self.cluster()
        (self.out / "batches.json").unlink(missing_ok=True)
        (self.out / "parked.json").unlink(missing_ok=True)
        core = self.core()
        result = core.Reports().related(1)
        pair = result["items"][0]
        self.assertEqual(pair["kind"], "pair")
        self.assertEqual(pair["classification"], "contradictory")
        self.assertEqual(pair["binding"], tranche.pair_binding(prs, 1, 2))
        self.assertEqual(pair["sources"]["2"]["source_digest"], prs[2]["source_digest"])
        self.assertNotIn("survivor", json.dumps(result))
        for value in (0, True, "1", 999):
            with self.subTest(value=value), self.assertRaises(core.ReportError):
                core.Reports().related(value)
        self.assertEqual(core.Reports().digests()["digests"]["report_binding"],
                         result["digests"]["report_binding"])

    def test_server_runs_on_a_single_resolved_interpreter(self):
        """Every launch path resolves the same absolute interpreter."""
        import subprocess
        core = self.core()
        resolved = hostpython.host_interpreter()
        self.assertIsNotNone(resolved, "No qualifying Python interpreter on this host")
        self.assertTrue(Path(resolved or "").is_absolute(), resolved)
        self.assertGreaterEqual(sys.version_info[:2], hostpython.MINIMUM)
        request = ('{"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": '
                   '{"protocolVersion": "2025-11-25", "capabilities": {}, '
                   '"clientInfo": {"name": "interpreter-check", "version": "1"}}}\n')
        result = subprocess.run(
            [resolved, str(Path(core.__file__)), "--root", str(Path(core.__file__).parent)],
            input=request, capture_output=True, text=True, timeout=30)
        self.assertEqual(result.stderr, "", result.stderr)
        self.assertEqual(json.loads(result.stdout)["result"]["serverInfo"]["name"], "tranche")

    def test_start_without_sdk_explains_optional_dependency_on_stderr(self):
        import subprocess
        import sys
        script = ("import builtins; original=builtins.__import__; "
                  "builtins.__import__=lambda name,*a,**k: "
                  "(_ for _ in ()).throw(ModuleNotFoundError(name)) "
                  "if name.startswith('mcp.') else original(name,*a,**k); "
                  "import mcp_server; mcp_server.main([])")
        result = subprocess.run([sys.executable, "-c", script], capture_output=True,
                                text=True, timeout=10, input="")
        self.assertEqual(result.returncode, 0)
        self.assertEqual(result.stdout, "")

    def test_protocol_surface_is_standard_sdk_free_mcp(self):
        import subprocess
        import sys
        core = self.core()
        script = ("import builtins; original=builtins.__import__; "
                  "builtins.__import__=lambda name,*a,**k: "
                  "(_ for _ in ()).throw(ModuleNotFoundError(name)) "
                  "if name == 'mcp' or name.startswith('mcp.') else original(name,*a,**k); "
                  "import json, sys; sys.path.insert(0, sys.argv[1]); "
                  "import mcp_server; print(json.dumps(mcp_server.TOOLS))")
        listing = json.loads(subprocess.run(
            [sys.executable, "-c", script, str(Path(core.__file__).parent)],
            capture_output=True, text=True, timeout=10).stdout)
        self.assertEqual({tool["name"] for tool in listing},
                         {"surface", "query", "pick", "next_prompt", "related", "digests"})
        for tool in listing:
            self.assertEqual(tool["annotations"], {"readOnlyHint": True, "destructiveHint": False,
                                                   "idempotentHint": True, "openWorldHint": False})
            self.assertEqual(tool["inputSchema"]["additionalProperties"], False)
            self.assertEqual(tool["inputSchema"]["type"], "object")
        self.assertEqual(core.dispatch({"jsonrpc": "2.0", "id": 1, "method": "tools/list"})["result"]["tools"],
                         listing)
        unknown = core.dispatch({"jsonrpc": "2.0", "id": 2, "method": "tools/call",
                                 "params": {"name": "nope", "arguments": {}}})
        self.assertEqual(unknown["error"]["code"], -32602)
        self.assertEqual(core.dispatch({"jsonrpc": "2.0", "method": "notifications/initialized"}), None)

    def test_every_tool_enforces_its_advertised_schema(self):
        """The advertised inputSchema is the contract, and the server enforces it.

        A client that skips its own validation must get the same refusal as one
        that does it, described in plain terms — never a Python call-signature
        error leaking class and method names.
        """
        self.reports()
        core = self.core()
        valid = {"surface": {}, "query": {}, "pick": {"batch_id": "B001"},
                 "next_prompt": {}, "related": {"number": 1}, "digests": {}}
        internals = ("Traceback", "unexpected keyword argument", "positional argument",
                     "Reports", "mcp_server", "self.", "TypeError")
        for tool in core.TOOLS:
            name, schema = tool["name"], tool["inputSchema"]
            cases = [("additional property", {**valid[name], "__nope__": 1})]
            if schema["required"]:
                cases.append(("missing required property", {}))
            for prop, spec in schema["properties"].items():
                if prop in schema.get("required", []) or prop in valid[name]:
                    continue
                if "enum" in spec:
                    cases.append((f"{prop} outside enum", {**valid[name], prop: "__not_in_enum__"}))
                elif "minimum" in spec:
                    cases.append((f"{prop} below minimum", {**valid[name], prop: spec["minimum"] - 1}))
                elif spec.get("type") in ("integer", "number"):
                    cases.append((f"{prop} not numeric", {**valid[name], prop: "not-a-number"}))
                elif spec.get("type") == "boolean":
                    cases.append((f"{prop} not boolean", {**valid[name], prop: "yes"}))
            for label, arguments in cases:
                with self.subTest(tool=name, case=label):
                    response = core.dispatch({"jsonrpc": "2.0", "id": 1, "method": "tools/call",
                                              "params": {"name": name, "arguments": arguments}})
                    result = response["result"]
                    self.assertTrue(result["isError"], f"{name}: accepted {label}")
                    message = result["content"][0]["text"]
                    for leak in internals:
                        self.assertNotIn(leak, message, f"{name}: leaked internals for {label}: {message}")
            accepted = core.dispatch({"jsonrpc": "2.0", "id": 2, "method": "tools/call",
                                      "params": {"name": name, "arguments": valid[name]}})["result"]
            self.assertFalse(accepted["isError"], f"{name}: rejected valid arguments")

    def test_schema_types_reject_booleans_where_numbers_are_advertised(self):
        """JSON booleans are not JSON numbers, however Python's isinstance sees them."""
        self.reports()
        core = self.core()
        for name, arguments in (("query", {"limit": True}), ("query", {"offset": False}),
                                ("related", {"number": True}), ("query", {"finished_form": True})):
            with self.subTest(tool=name, arguments=arguments):
                result = core.dispatch({"jsonrpc": "2.0", "id": 1, "method": "tools/call",
                                        "params": {"name": name, "arguments": arguments}})["result"]
                self.assertTrue(result["isError"], f"{name}: accepted {arguments}")

    def test_modified_batch_types_and_response_size_are_refused(self):
        from unittest.mock import patch
        self.reports()
        core = self.core()
        path = self.out / "batches.json"
        data = json.loads(path.read_text())
        data["batches"][0]["ordinal"] = True
        path.write_text(json.dumps(data))
        with self.assertRaises(core.ReportError):
            core.Reports().pick("B001")
        self.reports()
        with patch.object(core, "MAX_RESULT_BYTES", 10):
            with self.assertRaisesRegex(core.ReportError, "Response"):
                core.Reports().query()

    def test_recluster_excludes_stale_cache_without_serving_its_claims(self):
        core = self.core()
        self.reports()
        records = [json.loads(line) for line in tranche.JUDGMENTS_PATH.read_text().splitlines()]
        records[0]["binding"] = "stale"
        tranche.JUDGMENTS_PATH.write_text("".join(json.dumps(r) + "\n" for r in records))
        self.cluster()
        (self.out / "batches.json").unlink(missing_ok=True)
        (self.out / "parked.json").unlink(missing_ok=True)
        result = core.Reports().query(text="#1")
        self.assertEqual(result["items"][0]["freshness"], "unjudged")
        self.assertIsNone(result["items"][0]["risk"])
        self.assertFalse(result["items"][0]["candidate"])
        prs = self.reports()
        self.pairs(prs, [(1, 2, "same_change", 0.9)])
        record = json.loads(tranche.PAIRS_PATH.read_text())
        record["binding"] = "stale"
        tranche.PAIRS_PATH.write_text(json.dumps(record) + "\n")
        self.cluster()
        (self.out / "batches.json").unlink(missing_ok=True)
        (self.out / "parked.json").unlink(missing_ok=True)
        self.assertEqual(core.Reports().related(1)["items"], [])

    def test_legacy_self_pairs_are_ignored_like_cli_policy(self):
        self.reports()
        with tranche.PAIRS_PATH.open("a") as stream:
            stream.write(json.dumps({"a": 1, "b": 1, "verdict": "same_change"}) + "\n")
        self.assertEqual(self.core().Reports().surface()["summary"]["judged"], 2)

    def test_malformed_cache_json_and_huge_input_numbers_fail_closed(self):
        self.reports()
        core = self.core()
        with tranche.JUDGMENTS_PATH.open("a") as stream:
            stream.write("{broken\n")
        with self.assertRaises(core.ReportError):
            core.Reports().surface()
        self.reports()
        with self.assertRaises(core.ReportError):
            core.Reports().query(finished_form=10 ** 1000)

    def test_park_serves_from_the_producer_predicate_and_refuses_drift(self):
        prs = self.inputs([pr(1), pr(2, draft=True), pr(3)])
        tranche.JUDGMENTS_PATH.write_text("".join(
            json.dumps({"number": n, "answers": answers(),
                        "binding": tranche.judgment_binding(prs[n])}) + "\n"
            for n in sorted(prs)))
        self.pairs(prs, [])
        self.cluster()
        dupes = json.loads((self.out / "dupes.json").read_text())
        judgments = tranche.current_judgments(prs)
        tranche.atomic_json(self.out / "batches.json", tranche.merge_batches(
            dupes, judgments, prs, tranche.digest(dupes)))
        tranche.atomic_json(self.out / "parked.json", tranche.parked_payload(
            tranche.park_state(dupes, judgments, prs), prs, judgments, tranche.digest(dupes)))
        core = self.core()
        server = core.Reports()
        surface = server.surface()
        self.assertEqual(surface["queues"]["parked"]["members"], [2])
        self.assertEqual(surface["parked"]["count"], 1)
        self.assertEqual(surface["parked"]["members"], [2])
        self.assertIn("parked", surface["filters"]["queues"])
        result = server.query(queue="parked")
        self.assertEqual(result["total"], 1)
        self.assertEqual(result["items"][0]["number"], 2)
        by_number = {row["number"]: row for row in server.query()["items"]}
        self.assertEqual(by_number[2]["parked"], ["draft"])
        self.assertEqual(by_number[1]["parked"], [])
        # A parked.json that no longer matches the corpus refuses every read...
        parked = json.loads((self.out / "parked.json").read_text())
        parked["members"][0]["reasons"] = ["mutated"]
        (self.out / "parked.json").write_text(json.dumps(parked))
        with self.assertRaises(core.ReportError):
            core.Reports().surface()
        with self.assertRaises(core.ReportError):
            core.Reports().query(queue="parked")
        # ...and a batches run without its park record is incomplete evidence.
        tranche.atomic_json(self.out / "parked.json", tranche.parked_payload(
            tranche.park_state(dupes, judgments, prs), prs, judgments, tranche.digest(dupes)))
        (self.out / "parked.json").unlink()
        with self.assertRaises(core.ReportError):
            core.Reports().surface()
        # The park record participates in the served identity digests.
        self.reports()
        digests = self.core().Reports().digests()["digests"]
        self.assertEqual(digests["parked.json"],
                         tranche.digest(json.loads((self.out / "parked.json").read_text())))

    def test_surface_lists_filter_values_and_batch_overview(self):
        self.reports()
        result = self.core().Reports().surface()
        self.assertIn("security-review", result["filters"]["categories"])
        self.assertIn("unknown", result["filters"]["risk_bands"])
        self.assertIn("related", result["filters"]["queues"])
        self.assertEqual(result["batches"][0]["id"], "B001")
        self.assertNotIn("review_prompt", result["batches"][0])


if __name__ == "__main__":
    unittest.main()
