"""Parity tests for the extracted bound-report seam.

`report_loader.load()` is the single authority for reading a bound report; the
MCP server and the evidence CLI both consume it. These tests pin that they
cannot drift: the same mutations must be refused by both, the served identity
must be the loader's identity, and the loader must carry no MCP response limit.
"""
import json
import sys
import unittest
from pathlib import Path

import report_loader
import tranche
from tests import test_tranche as fixtures

pr = fixtures.pr


class ReportLoaderTests(unittest.TestCase):
    setUp = fixtures.WorkflowTests.setUp
    inputs = fixtures.WorkflowTests.inputs
    judgments = fixtures.WorkflowTests.judgments
    pairs = fixtures.WorkflowTests.pairs
    cluster = fixtures.WorkflowTests.cluster

    def core(self):
        import mcp_server
        return mcp_server

    def reports(self, *, batches=True, parked=True):
        prs = self.inputs([pr(1), pr(2, draft=True), pr(3)])
        self.judgments(prs)
        self.pairs(prs, [])
        self.cluster()
        dupes = json.loads((self.out / "dupes.json").read_text())
        judgments = tranche.current_judgments(prs)
        if batches:
            tranche.atomic_json(self.out / "batches.json", tranche.merge_batches(
                dupes, judgments, prs, tranche.digest(dupes)))
        if parked:
            tranche.atomic_json(self.out / "parked.json", tranche.parked_payload(
                tranche.park_state(dupes, judgments, prs), prs, judgments, tranche.digest(dupes)))
        return prs

    def test_the_loader_and_the_mcp_server_refuse_the_same_mutations(self):
        core = self.core()
        for target, field, value in (
            ("summary.json", "allow_unbound", True),
            ("summary.json", "format_version", 1),
            ("summary.json", "repo", "foreign/repo"),
            ("summary.json", "report_binding", "wrong"),
            ("clusters.json", "tampered", True),
            ("dupes.json", "tampered", True),
            ("batches.json", "batch_size", 99),
            ("parked.json", "parked", 999),
        ):
            with self.subTest(target=target, field=field):
                self.reports()
                path = self.out / target
                data = json.loads(path.read_text())
                data[field] = value
                path.write_text(json.dumps(data))
                with self.assertRaises(report_loader.ReportError):
                    report_loader.load()
                with self.assertRaises(core.ReportError):
                    core.Reports().surface()
        # A moved head and a changed judgment are refused by both, identically.
        self.reports()
        self.inputs([pr(1, head={"sha": "moved"}), pr(2, draft=True), pr(3)])
        with self.assertRaises(report_loader.ReportError):
            report_loader.load()
        with self.assertRaises(core.ReportError):
            core.Reports().surface()

    def test_the_loader_refuses_a_park_record_that_is_missing_or_drifted(self):
        self.reports()
        parked = json.loads((self.out / "parked.json").read_text())
        parked["members"][0]["reasons"] = ["mutated"]
        (self.out / "parked.json").write_text(json.dumps(parked))
        with self.assertRaises(report_loader.ReportError):
            report_loader.load()
        # Batches that parked PRs without their park record are incomplete evidence.
        self.reports()
        (self.out / "parked.json").unlink()
        with self.assertRaises(report_loader.ReportError):
            report_loader.load()

    def test_the_served_identity_is_the_loaders_identity(self):
        self.reports()
        core = self.core()
        served = core.Reports().digests()["digests"]
        loaded = report_loader.load().identity
        for key, value in loaded.items():
            self.assertEqual(served[key], value, key)
        # input_bytes is added by the server's own concurrent-read guard only.
        self.assertNotIn("input_bytes", loaded)
        self.assertIn("input_bytes", served)

    def test_the_loader_carries_no_mcp_response_limit(self):
        """A large report must load even though no MCP response could carry it."""
        self.reports()
        core = self.core()
        loaded = report_loader.load()
        self.assertEqual(sorted(loaded.prs), [1, 2, 3])
        self.assertEqual(loaded.summary["prs_in_corpus"], 3)
        self.assertIsNotNone(loaded.batches)
        # The MCP layer keeps its own response bound, distinct from the file bound.
        self.assertEqual(core.MAX_RESULT_BYTES, 1024 * 1024)
        self.assertEqual((core.MAX_FILE_BYTES, core.MAX_TOTAL_BYTES, core.MAX_INPUT_FILES),
                         (report_loader.DEFAULT_LIMITS.max_file_bytes,
                          report_loader.DEFAULT_LIMITS.max_total_bytes,
                          report_loader.DEFAULT_LIMITS.max_input_files))
        with self.assertRaises(report_loader.ReportError):
            report_loader.input_digests(report_loader.Limits(max_file_bytes=1))

    def test_a_report_without_batches_or_park_still_loads(self):
        self.reports(batches=False, parked=False)
        loaded = report_loader.load()
        self.assertIsNone(loaded.batches)
        self.assertIsNone(loaded.parked)
        self.assertIsNone(loaded.identity["batches.json"])
        self.assertIsNone(loaded.identity["parked.json"])
        self.assertFalse(self.core().Reports().surface()["batches_available"])

    def test_the_seam_has_no_circular_import(self):
        """`report_loader` is the bottom of the stack: it must not import consumers."""
        source = Path(report_loader.__file__).read_text()
        self.assertNotIn("import mcp_server", source)
        self.assertNotIn("import evidence", source)
        self.assertIn("import tranche", source)
        for name in ("report_loader", "mcp_server"):
            module = sys.modules.get(name)
            self.assertIsNotNone(module, name)

    def test_the_concurrent_read_fingerprint_now_covers_the_park_record(self):
        """The served park record gates batches, so it must be fingerprinted too.

        Before this seam existed the fingerprint list stopped at batches.json, so
        parked.json could change under a read without the guard noticing.
        """
        self.reports()
        digests = report_loader.input_digests()
        parked = str(self.out / "parked.json")
        batches = str(self.out / "batches.json")
        self.assertIn(parked, digests)
        self.assertIn(batches, digests)
        self.assertIsNotNone(digests[parked])
        fingerprint = digests[parked]
        (self.out / "parked.json").write_text("{}")
        self.assertNotEqual(report_loader.input_digests()[parked], fingerprint)


if __name__ == "__main__":
    unittest.main()
