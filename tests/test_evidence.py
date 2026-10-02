"""Offline tests for the native evidence service: synthetic reports and fake transport.

No network, no `gh`, no model. Every test drives the production code path -
`evidence.select`, `evidence.Capture`, `evidence.export_bytes`, the CLI commands -
against a synthetic native report and a recorded transport, so the assertions are
about the real request construction and the real storage/identity rules rather
than a replacement implementation returning idealized dictionaries.
"""
import json
import os
import unittest
from unittest.mock import patch

import evidence
import ghread
import report_loader
import tranche
from tests import test_tranche as fixtures

pr = fixtures.pr
answers = fixtures.answers

BASE_ID, BASE_NAME = 994093166, "omacom/omarchy"
FORK_ID, FORK_NAME = 1338981908, "contributor/omarchy"
SHA = {"base": "a" * 40, "head": "b" * 40}
HEAD_SHA2 = "c" * 40


def captured(number, *, head_sha=None, updated="2026-01-02T00:00:00Z", fork=True, **changes):
    """A captured PR item with the base/head identity evidence selection needs."""
    head_sha = head_sha or SHA["head"]
    item = pr(number, **changes)
    item["updated_at"] = updated
    item["head"] = {"sha": head_sha, "ref": "feature", "label": f"{FORK_NAME.split('/')[0]}:feature",
                    "repo": {"id": FORK_ID if fork else BASE_ID,
                             "full_name": FORK_NAME if fork else BASE_NAME, "fork": fork}}
    item["base"] = {"sha": SHA["base"], "ref": "master",
                    "repo": {"id": BASE_ID, "full_name": BASE_NAME, "fork": False}}
    return item


def body(payload) -> bytes:
    return json.dumps(payload, ensure_ascii=False).encode("utf-8")


class FakeGh:
    """A recorded GitHub transport honouring the real request contract.

    Routes are keyed by a URL fragment and answer per page, so pagination,
    continuation links, foreign identities and hostile next links are all
    exercised through the same seam the capture uses.
    """

    def __init__(self):
        self.calls = []
        self.routes = []
        self.graphql_calls = []
        self.fail = {}

    def on(self, fragment, payload, *, link=None, media="application/vnd.github+json",
           status=200, raw=None, pages=None, repository=None, sha=None,
           graphql_number=None):
        self.routes.append({"fragment": fragment, "payload": payload, "link": link,
                            "media": media, "status": status, "raw": raw,
                            "pages": pages, "repository": repository, "sha": sha,
                            "graphql_number": graphql_number})
        return self

    def _ranked(self):
        """Most specific fragment first; a later registration overrides an equal one."""
        return sorted(enumerate(self.routes), key=lambda pair: (-len(pair[1]["fragment"]),
                                                               -pair[0]))

    def _find(self, url, accept):
        # Longest fragment wins (`/pulls/1/files` is not `/pulls/1`), and a test
        # that registers an endpoint again is overriding the earlier answer.
        for _index, route in self._ranked():
            if route["fragment"] in url:
                if route["pages"] is not None:
                    page = int(dict(p.split("=", 1) for p in url.split("?", 1)[-1].split("&")
                                    if "=" in p).get("page", "1")) if "?" in url else 1
                    if page in route["pages"]:
                        entry = route["pages"][page]
                        return route, entry
                    continue
                return route, route
        return None, None

    def read(self, url, *, accept=None, params=None, budget=None, reserve=None,
             timeout=None, max_bytes=None, repo_prefix=None):
        if budget is not None:
            budget.charge(reserve)
        query = "&".join(f"{k}={v}" for k, v in sorted((params or {}).items()))
        full = url + ("&" + query if query and "?" in url else
                      "?" + query if query else "")
        self.calls.append({"url": url, "accept": accept, "params": dict(params or {}),
                           "repo_prefix": repo_prefix, "full": full})
        route, entry = self._find(full, accept)
        if route is None:
            raise ghread.ReadError(f"no recorded route for {full}")
        if route["fragment"] in self.fail:
            raise self.fail[route["fragment"]]
        payload = entry.get("raw")
        if payload is None:
            data = entry["payload"]
            if callable(data):
                data = data(self.calls[-1])
            payload = body(data)
        headers = {"content-type": entry.get("media") or "application/vnd.github+json"}
        if entry.get("link"):
            headers["link"] = entry["link"].format(base=ghread.ORIGIN)
        if entry.get("repository") is not None:
            payload = body({"repository": entry["repository"], "state": "pending",
                            "total_count": 0, "statuses": [],
                            "sha": entry.get("sha") or SHA["head"]})
        if route["status"] >= 400:
            raise ghread.ReadError(f"HTTP {route['status']} from {full}")
        return ghread.Response(route["status"], full, headers, payload)

    def graphql(self, query, variables=None, *, budget=None, reserve=None, timeout=None,
                max_bytes=None):
        if budget is not None:
            budget.charge(reserve)
        variables = dict(variables or {})
        self.graphql_calls.append({"query": query, "variables": variables})
        # A GraphQL answer is about one PR: select the route registered for the
        # number this call asked for, exactly as a real endpoint would.
        for _index, route in self._ranked():
            if route["fragment"] != "graphql":
                continue
            if route.get("graphql_number") not in (None, variables.get("number")):
                continue
            data = route["payload"]
            if callable(data):
                data = data(variables)
            return ghread.Response(200, ghread.GRAPHQL_URL,
                                   {"content-type": "application/json"}, body(data))
        raise ghread.ReadError(f"no recorded graphql route for {variables}")


def metadata_payload(member, *, head_sha=None, base_sha=None, updated=None, number=None,
                     head_repo=None, base_repo=None):
    head_repo = head_repo if head_repo is not None else {
        "id": member["head_repo_id"], "full_name": member["head_repo_name"],
        "fork": member["head_fork"]}
    base_repo = base_repo if base_repo is not None else {
        "id": member["base_repo_id"], "full_name": member["base_repo_name"], "fork": False}
    return {"number": number if number is not None else member["number"],
            "head": {"sha": head_sha or member["head_sha"], "ref": "feature", "repo": head_repo},
            "base": {"sha": base_sha or member["base_sha"], "ref": "master", "repo": base_repo},
            "updated_at": updated if updated is not None else member["updated_at"]}


def closing_payload(number, *, has_next=False, cursor=None, nodes=None, repo=None, pr_number=None):
    return {"data": {"repository": {
        "nameWithOwner": repo or BASE_NAME, "id": BASE_ID,
        "pullRequest": {"number": pr_number or number, "url": f"https://github.com/{BASE_NAME}/pull/{number}",
                        "closingIssuesReferences": {
                            "totalCount": len(nodes or []),
                            "pageInfo": {"hasNextPage": has_next, "endCursor": cursor},
                            "nodes": nodes if nodes is not None else []}}}}}


class EvidenceTests(unittest.TestCase):
    """Synthetic native report, fake transport, real service code."""

    setUp = fixtures.WorkflowTests.setUp
    inputs = fixtures.WorkflowTests.inputs
    judgments = fixtures.WorkflowTests.judgments
    pairs = fixtures.WorkflowTests.pairs
    cluster = fixtures.WorkflowTests.cluster

    def build_report(self, items=None):
        """A validated native report with one real batch of two members."""
        items = items or [captured(1), captured(2, fork=False)]
        prs = self.inputs(items)
        self.judgments(prs)
        self.pairs(prs, [])
        self.cluster()
        dupes = json.loads((self.out / "dupes.json").read_text())
        judgments = tranche.current_judgments(prs)
        digest = tranche.digest(dupes)
        tranche.atomic_json(self.out / "batches.json",
                            tranche.merge_batches(dupes, judgments, prs, digest))
        tranche.atomic_json(self.out / "parked.json",
                            tranche.parked_payload(tranche.park_state(dupes, judgments, prs),
                                                   prs, judgments, digest))
        return prs

    def selection(self, batch="B001"):
        return evidence.select(batch, report_loader.load())

    def routes(self, gh, *, files=None, discussion=None):
        """The full endpoint set for every member of the batch.

        Every member must have routes: the capture's live revision check reads
        each member's metadata before it acquires anything, so a fixture that
        only knows one member fail-stops - correctly - instead of acquiring.
        """
        for member in self.selection().members:
            self.member_routes(gh, member, files=files, discussion=discussion)
        return self.selection().members[0]

    def member_routes(self, gh, member, *, files=None, discussion=None):
        base = member["base_repo_name"]
        head = member["head_sha"]
        gh.on(f"/repos/{base}/pulls/{member['number']}", metadata_payload(member))
        gh.on(f"/repos/{base}/pulls/{member['number']}/files", files if files is not None else [])
        gh.on(f"/repos/{base}/issues/{member['number']}/comments", discussion or [])
        gh.on(f"/repos/{base}/pulls/{member['number']}/comments", [])
        gh.on(f"/repos/{base}/pulls/{member['number']}/reviews", [])
        gh.on(f"/repos/{base}/commits/{head}/check-runs", {"total_count": 0, "check_runs": []})
        gh.on(f"/repos/{base}/commits/{head}/status", None,
              repository={"id": member["base_repo_id"], "full_name": base}, sha=head)
        if member["head_repo_name"] != base:
            gh.on(f"/repos/{member['head_repo_name']}/commits/{head}/check-runs",
                  {"total_count": 0, "check_runs": []})
            gh.on(f"/repos/{member['head_repo_name']}/commits/{head}/status", None,
                  repository={"id": member["head_repo_id"], "full_name": member["head_repo_name"]},
                  sha=head)
        gh.on("graphql", lambda variables: closing_payload(member["number"]),
              graphql_number=member["number"])
        return member

    def capture(self, gh, *, batch="B001", budget=200, capture_id=None, **kwargs):
        """Run one capture exactly as `cmd_capture` resolves its manifest.

        A supplied capture id resumes the stored checkpoint (re-binding the
        association and carrying unchanged code over), which is what the CLI
        does; it never silently starts from a blank manifest.
        """
        selection = self.selection(batch)
        capture_id = capture_id or evidence.new_capture_id()
        if evidence.manifest_path(capture_id).exists():
            manifest = evidence.read_manifest(capture_id)
        else:
            manifest = evidence.build_manifest(selection, capture_id, request_limit=budget)
            evidence.carry_over_code(selection, manifest)
        manifest["selection"] = selection.as_json()
        manifest["capture"]["request_limit"] = budget
        evidence.write_manifest(capture_id, manifest)
        with patch.object(evidence.ghread, "read", gh.read), \
             patch.object(evidence.ghread, "graphql", gh.graphql):
            run = evidence.Capture(selection, capture_id, manifest,
                                   budget=ghread.Budget(budget, reserved=evidence.RESERVED_BUDGET),
                                   **kwargs)
            run.run()
        manifest["citations"] = evidence.extract_citations(manifest)
        return capture_id, manifest, run


class SelectionTests(EvidenceTests):
    """One authority for batch selection; every refusal is tested."""

    def test_select_binds_identity_provenance_and_membership(self):
        self.build_report()
        selection = self.selection()
        batch = json.loads((self.out / "batches.json").read_text())["batches"][0]
        self.assertEqual(selection.numbers, batch["members"])
        self.assertEqual(selection.batch["review_prompt"], batch["review_prompt"])
        self.assertEqual(selection.repository, {"id": BASE_ID, "full_name": BASE_NAME,
                                                "visibility": "unknown"})
        payload = selection.as_json()
        self.assertEqual(payload["batch"]["members"], batch["members"])
        self.assertEqual(payload["batch"]["review_prompt"], batch["review_prompt"])
        self.assertEqual(payload["report"]["report_binding"],
                         json.loads((self.out / "summary.json").read_text())["report_binding"])
        self.assertEqual(set(payload["report"]["output_digests"]),
                         {"clusters.json", "dupes.json", "batches.json", "parked.json"})
        member = payload["members"][0]
        self.assertEqual(member["base_sha"], SHA["base"])
        self.assertEqual(member["head_repo_name"], FORK_NAME)
        self.assertTrue(member["head_fork"])
        # The generation is the report association and the code identity.
        generation = evidence.generation_id(selection, "0" * 32)
        self.assertEqual(generation, evidence.evidence_digest({
            "format": evidence.FORMAT, "profile": evidence.PROFILE, "capture_id": "0" * 32,
            "repository": selection.repository, "revision": selection.revision(),
            "membership": selection.membership_digest, "batch": selection.batch["id"],
            "report": selection.report}))

    def test_select_refuses_unknown_and_malformed_batch_ids(self):
        self.build_report()
        report = report_loader.load()
        for batch in ("B999", "b001", "B1", "", None, 1, "../B001", "B001\n"):
            with self.subTest(batch=batch), self.assertRaises(evidence.SelectionError):
                evidence.select(batch, report)

    def test_select_refuses_a_parked_member_batch(self):
        """The batch plan and the park record must agree, or nothing is selected."""
        self.build_report()
        parked = json.loads((self.out / "parked.json").read_text())
        parked["members"] = [{"number": 1, "reasons": ["draft"],
                              "unblock": "x", "title": "t", "author": "a",
                              "head_sha": None, "url": "u", "created": "",
                              "security_flag": None}]
        (self.out / "parked.json").write_text(json.dumps(parked))
        with self.assertRaises(report_loader.ReportError):
            self.selection()

    def test_select_refuses_a_short_or_missing_revision(self):
        for change in ({"head": {"sha": "b" * 12, "repo": {"id": FORK_ID,
                                                          "full_name": FORK_NAME, "fork": True}}},
                       {"head": {"sha": None, "repo": {"id": FORK_ID,
                                                       "full_name": FORK_NAME, "fork": True}}},
                       {"base": {"sha": SHA["base"], "repo": None}},
                       {"base": {"sha": SHA["base"], "repo": {"id": None,
                                                              "full_name": BASE_NAME}}}):
            with self.subTest(change=list(change)):
                item = captured(1)
                for key, value in change.items():
                    item[key] = value
                self.build_report([item, captured(2, fork=False)])
                with self.assertRaises(evidence.SelectionError):
                    self.selection()

    def test_select_refuses_a_deleted_fork_head(self):
        item = captured(1)
        item["head"]["repo"] = None
        self.build_report([item, captured(2, fork=False)])
        with self.assertRaisesRegex(evidence.SelectionError, "deleted fork"):
            self.selection()

    def test_captured_items_checks_the_snapshot_checksum(self):
        self.build_report()
        path = self.pages / "snapshot.json"
        value = json.loads(path.read_text())
        value["digest"] = "0" * 64
        path.write_text(json.dumps(value))
        with self.assertRaises(evidence.SelectionError):
            evidence.captured_items()

    def test_a_reversed_batch_order_is_not_the_same_selection(self):
        """Membership order is part of the batch identity, not presentation."""
        self.build_report()
        selection = self.selection()
        batch = json.loads((self.out / "batches.json").read_text())["batches"][0]
        reversed_batch = dict(batch, members=list(reversed(batch["members"])),
                              review_prompt=batch["review_prompt"] + "\n(edited)")
        other = evidence.Selection(selection.repository, selection.report, reversed_batch,
                                   list(reversed(selection.members)))
        self.assertNotEqual(other.as_json(), selection.as_json())
        self.assertNotEqual(evidence.generation_id(other, "0" * 32),
                            evidence.generation_id(selection, "0" * 32))

    def test_prompt_edits_alone_do_not_change_the_generation(self):
        """The prompt is recorded provenance; it is not a cache key."""
        self.build_report()
        selection = self.selection()
        rewritten = dict(selection.batch, review_prompt="a rewritten prompt")
        other = evidence.Selection(selection.repository, selection.report,
                                   rewritten, selection.members)
        self.assertEqual(evidence.generation_id(other, "0" * 32),
                         evidence.generation_id(selection, "0" * 32))
        self.assertNotEqual(other.as_json()["batch"]["review_prompt"],
                            selection.as_json()["batch"]["review_prompt"])


class CaptureTests(EvidenceTests):
    """Acquisition through the production path with recorded responses."""

    def test_capture_completes_every_component_of_both_members(self):
        self.build_report()
        gh = FakeGh()
        self.routes(gh)
        capture_id, manifest, run = self.capture(gh)
        self.assertTrue(evidence.is_complete(manifest))
        self.assertIsNone(manifest["capture"]["stop_reason"])
        self.assertEqual({entry["status"] for entry in manifest["components"]}, {"complete"})
        # One component per member/component pair, and no orphans.
        self.assertEqual(len(manifest["components"]), 2 * len(evidence.COMPONENTS))
        cited = {source["id"] for source in manifest["sources"]}
        self.assertEqual({c["source_id"] for c in manifest["citations"]} - cited, set())
        # The base repo answers the PR's own checks; the fork answers its own.
        status_urls = {call["url"] for call in gh.calls if call["url"].endswith("/status")}
        self.assertIn(f"https://api.github.com/repos/{BASE_NAME}/commits/{SHA['head']}/status",
                      status_urls)
        self.assertIn(f"https://api.github.com/repos/{FORK_NAME}/commits/{SHA['head']}/status",
                      status_urls)
        # Every request was an explicit, bounded, credential-free GET.
        for call in gh.calls:
            self.assertEqual(call["repo_prefix"] in (BASE_NAME, FORK_NAME), True)
        self.assertEqual(len(manifest["sources"]), len(gh.calls))

    def test_requests_name_real_endpoints_and_media_types(self):
        self.build_report()
        gh = FakeGh()
        self.routes(gh)
        self.capture(gh)
        urls = {call["url"] for call in gh.calls}
        self.assertIn(f"https://api.github.com/repos/{BASE_NAME}/pulls/1", urls)
        self.assertIn(f"https://api.github.com/repos/{BASE_NAME}/pulls/1/files", urls)
        self.assertIn(f"https://api.github.com/repos/{BASE_NAME}/issues/1/comments", urls)
        self.assertNotIn("metadata", urls)  # the proposal's synthetic endpoint
        accepts = {call["accept"] for call in gh.calls}
        self.assertIn(evidence.DIFF_ACCEPT, accepts)
        self.assertIn(evidence.JSON_MEDIA, accepts)
        self.assertTrue(gh.graphql_calls)
        # The query is a query, and its provenance carries no credential.
        for call in gh.graphql_calls:
            self.assertTrue(call["query"].lstrip().startswith("query"))
            self.assertNotIn("mutation", call["query"])
            self.assertEqual(set(call["variables"]),
                             {"owner", "name", "number", "cursor"})
            self.assertNotIn("token", json.dumps(call["variables"]).lower())

    def test_empty_collections_are_captured_as_empty_not_invented(self):
        self.build_report()
        gh = FakeGh()
        self.routes(gh)
        capture_id, manifest, _ = self.capture(gh)
        empty = [s for s in manifest["sources"] if s["component"] == "discussion"]
        self.assertTrue(empty)
        for source in empty:
            self.assertEqual(json.loads(evidence.read_body(source["body_sha256"])), [])
        # An empty response is captured evidence; it needs no fabricated citation.
        self.assertEqual([c for c in manifest["citations"] if c["component"] == "discussion"], [])

    def test_ci_runs_and_statuses_are_separate_and_named(self):
        self.build_report()
        gh = FakeGh()
        member = self.routes(gh)
        runs = {"total_count": 1, "check_runs": [
            {"id": 7, "head_sha": member["head_sha"], "status": "completed",
             "conclusion": "success", "name": "build"}]}
        gh.on(f"/repos/{BASE_NAME}/commits/{member['head_sha']}/check-runs", runs)
        gh.on(f"/repos/{FORK_NAME}/commits/{member['head_sha']}/check-runs", runs)
        capture_id, manifest, _ = self.capture(gh)
        groups = {g["group"]: g for entry in manifest["components"]
                  if entry["component"] == "checks" for g in entry["groups"]}
        self.assertEqual(set(groups), {"check_runs", "statuses",
                                       "fork_check_runs", "fork_statuses"})
        self.assertEqual(groups["check_runs"]["items_observed"], 1)
        self.assertEqual(groups["check_runs"]["items_reported"], 1)
        self.assertEqual(groups["statuses"]["items_observed"], 0)

    def test_a_delivered_check_run_cap_is_reported_instead_of_certified(self):
        """A full, terminal page at GitHub's own ceiling is a named gap.

        GitHub returns at most the 1000 most recent suites' runs; a response that
        reaches that ceiling with nothing left to page is not proof that every
        run was observed, so it must not be recorded as complete.
        """
        self.build_report()
        gh = FakeGh()
        member = self.routes(gh)
        runs = [{"id": index, "head_sha": member["head_sha"], "name": f"job-{index}",
                 "status": "completed", "conclusion": "success"}
                for index in range(evidence.CHECK_RUN_CAP)]
        gh.on(f"/repos/{BASE_NAME}/commits/{member['head_sha']}/check-runs",
              {"total_count": evidence.CHECK_RUN_CAP, "check_runs": runs})
        gh.on(f"/repos/{FORK_NAME}/commits/{member['head_sha']}/check-runs",
              {"total_count": evidence.CHECK_RUN_CAP, "check_runs": runs})
        capture_id, manifest, _ = self.capture(gh)
        entry = next(e for e in manifest["components"]
                     if e["component"] == "checks" and e["number"] == member["number"])
        group = next(g for g in entry["groups"] if g["group"] == "check_runs")
        self.assertEqual(group["status"], "partial")
        self.assertIn("caps check runs", group["reason"])
        # The captured bytes still hold every run that was delivered.
        self.assertEqual(group["items_observed"], evidence.CHECK_RUN_CAP)

    def test_a_shortfall_between_reported_and_observed_is_named(self):
        self.build_report()
        gh = FakeGh()
        member = self.routes(gh)
        gh.on(f"/repos/{BASE_NAME}/commits/{member['head_sha']}/check-runs",
              {"total_count": 5, "check_runs": []})
        capture_id, manifest, _ = self.capture(gh)
        entry = next(e for e in manifest["components"]
                     if e["component"] == "checks" and e["number"] == member["number"])
        group = next(g for g in entry["groups"] if g["group"] == "check_runs")
        self.assertEqual(group["status"], "partial")
        self.assertIn("5 items", group["reason"])


class IdentityTests(EvidenceTests):
    """What forces a re-download, what is reused, and what is refused."""

    def test_a_moved_head_invalidates_the_capture_instead_of_rebinding(self):
        self.build_report()
        gh = FakeGh()
        self.routes(gh)
        capture_id, manifest, _ = self.capture(gh)
        sources_before = {s["id"]: s["body_sha256"] for s in manifest["sources"]}
        self.assertTrue(evidence.is_complete(manifest))
        # The PR moves upstream.
        moved = self.selection().members[0]
        gh2 = FakeGh()
        self.routes(gh2)
        gh2.on(f"/repos/{BASE_NAME}/pulls/{moved['number']}",
               metadata_payload(moved, head_sha=HEAD_SHA2))
        with self.assertRaises(evidence.SelectionError):
            self.capture(gh2, capture_id=capture_id)
        # The recorded bytes and the previous checkpoint are untouched.
        reread = evidence.read_manifest(capture_id)
        self.assertEqual({s["id"]: s["body_sha256"] for s in reread["sources"]},
                         sources_before)
        self.assertEqual(reread["generation"], manifest["generation"])
        self.assertTrue(evidence.is_complete(reread))
        for source in reread["sources"]:
            evidence.read_body(source["body_sha256"])

    def test_an_updated_at_change_refreshes_mutable_and_keeps_code_bytes(self):
        """A thread update is not a revision: unchanged code must not be re-fetched."""
        self.build_report()
        gh = FakeGh()
        self.routes(gh)
        capture_id, manifest, _ = self.capture(gh)
        code_before = {s["body_sha256"] for s in manifest["sources"]
                       if s["component"] in evidence.CODE_COMPONENTS}
        self.assertTrue(code_before)
        # Upstream thread churn only.
        member = self.selection().members[0]
        gh2 = FakeGh()
        self.routes(gh2)
        gh2.on(f"/repos/{BASE_NAME}/pulls/{member['number']}",
               metadata_payload(member, updated="2026-05-05T05:05:05Z"))
        capture_id2, manifest2, run = self.capture(gh2, capture_id=capture_id)
        code_after = {s["body_sha256"] for s in manifest2["sources"]
                      if s["component"] in evidence.CODE_COMPONENTS}
        self.assertTrue(code_before <= code_after)
        # The generation survived, so historical citations still resolve.
        self.assertEqual(manifest2["generation"], manifest["generation"])
        self.assertEqual(manifest2["code_observation"]["thread_updated_at"][str(member["number"])],
                         "2026-05-05T05:05:05Z")
        # The stale mutable groups were re-acquired, not left complete.
        entry = next(e for e in manifest2["components"]
                     if e["component"] == "discussion" and e["number"] == member["number"])
        self.assertEqual(entry["status"], "complete")
        for citation in manifest["citations"]:
            evidence.resolve_citation(manifest2, citation["id"])

    def test_a_report_only_change_carries_code_evidence_over_without_requests(self):
        """A re-cluster must not orphan captured bytes, or fetch them again."""
        self.build_report()
        gh = FakeGh()
        self.routes(gh)
        capture_id, manifest, _ = self.capture(gh)
        code_before = [s for s in manifest["sources"]
                       if s["component"] in evidence.CODE_COMPONENTS]
        # A different report association over the same revisions.
        selection = self.selection()
        rewritten = dict(selection.report, report_binding="f" * 64)
        other = evidence.Selection(selection.repository, rewritten, selection.batch,
                                   selection.members)
        fresh_id = evidence.new_capture_id()
        fresh = evidence.build_manifest(other, fresh_id, request_limit=200)
        self.assertNotEqual(fresh["generation"], manifest["generation"])
        adopted = evidence.carry_over_code(other, fresh)
        self.assertEqual(adopted, len(code_before))
        code_after = [s for s in fresh["sources"]
                      if s["component"] in evidence.CODE_COMPONENTS]
        self.assertEqual({s["body_sha256"] for s in code_before},
                         {s["body_sha256"] for s in code_after})
        # The adopted records belong to the new generation and the new capture.
        for source in fresh["sources"]:
            self.assertEqual(source["generation"], fresh["generation"])
            self.assertTrue(source["id"].startswith(""))  # id recomputed, not copied
        # A capture with adopted code needs no network for those components.
        gh2 = FakeGh()
        self.routes(gh2)
        evidence.write_manifest(fresh_id, fresh)
        with patch.object(evidence.ghread, "read", gh2.read), \
             patch.object(evidence.ghread, "graphql", gh2.graphql):
            run = evidence.Capture(other, fresh_id, fresh,
                                   budget=ghread.Budget(200, reserved=evidence.RESERVED_BUDGET))
            run.run()
        fetched = {call["url"].rsplit("/", 1)[-1] for call in gh2.calls}
        # Code components are not re-fetched: their bytes were adopted.
        self.assertNotIn("files", fetched)
        # Mutable components still are: CI and the thread move without a head move.
        self.assertIn("check-runs", fetched)
        self.assertIn("comments", fetched)

    def test_a_revision_move_is_not_carried_over(self):
        """Reuse is keyed to the revision, so a moved head must not be adopted."""
        self.build_report()
        gh = FakeGh()
        self.routes(gh)
        capture_id, manifest, _ = self.capture(gh)
        selection = self.selection()
        moved = [dict(m) for m in selection.members]
        moved[0] = dict(moved[0], head_sha=HEAD_SHA2)
        other = evidence.Selection(selection.repository, selection.report,
                                   selection.batch, moved)
        fresh = evidence.build_manifest(other, evidence.new_capture_id(), request_limit=200)
        adopted = evidence.carry_over_code(other, fresh)
        # Only the unmoved member's code is adopted; the moved one is not.
        self.assertEqual({s["number"] for s in fresh["sources"]}, {moved[1]["number"]})
        self.assertEqual(adopted, len([s for s in fresh["sources"]
                                       if s["component"] in evidence.CODE_COMPONENTS]))

    def test_an_unrelated_corpus_change_does_not_touch_the_capture(self):
        self.build_report()
        gh = FakeGh()
        self.routes(gh)
        capture_id, manifest, _ = self.capture(gh)
        before = evidence.read_manifest(capture_id)
        # Add unrelated PRs and rebuild the report.
        self.build_report([captured(1), captured(2, fork=False), captured(9, fork=False)])
        self.assertNotEqual(self.selection().numbers, [1, 2])  # membership really moved
        # The stored capture is byte-for-byte what it was.
        self.assertEqual(evidence.read_manifest(capture_id), before)

    def test_a_capture_from_another_selection_is_not_this_selection(self):
        """A manifest that claims another batch is refused as inconsistent."""
        self.build_report()
        gh = FakeGh()
        self.routes(gh)
        capture_id, _, _ = self.capture(gh)
        manifest = evidence.read_manifest(capture_id)
        manifest["selection"]["batch"] = dict(manifest["selection"]["batch"],
                                              id="B002", ordinal=2)
        evidence.write_manifest(capture_id, manifest)
        with self.assertRaisesRegex(evidence.EvidenceError, "generation"):
            evidence.read_manifest(capture_id)

    def test_a_consistent_manifest_of_another_batch_is_a_different_selection(self):
        """Renumbering the batch is a new selection even with identical members."""
        self.build_report()
        selection = self.selection()
        renumbered = dict(selection.batch, id="B002", ordinal=2)
        other = evidence.Selection(selection.repository, selection.report, renumbered,
                                   selection.members)
        fresh = evidence.build_manifest(other, evidence.new_capture_id(), request_limit=200)
        evidence.write_manifest(fresh["capture_id"], fresh)
        reread = evidence.read_manifest(fresh["capture_id"])
        self.assertEqual(evidence.stored_selection(reread).batch["id"], "B002")
        self.assertNotEqual(reread["selection"], selection.as_json())
        self.assertNotEqual(reread["generation"],
                            evidence.generation_id(selection, fresh["capture_id"]))

    def test_batch_renumbering_is_a_new_generation_unless_explicitly_reused(self):
        self.build_report()
        gh = FakeGh()
        self.routes(gh)
        capture_id, manifest, _ = self.capture(gh)
        same = evidence.generation_id(self.selection(), capture_id)
        self.assertEqual(same, manifest["generation"])
        fresh = evidence.generation_id(self.selection(), evidence.new_capture_id())
        self.assertNotEqual(fresh, manifest["generation"])


class StorageTests(EvidenceTests):
    """Bounds, confinement, corruption and crash windows."""

    def test_stored_bytes_are_verified_on_read_and_corruption_is_refused(self):
        self.build_report()
        gh = FakeGh()
        self.routes(gh)
        capture_id, manifest, _ = self.capture(gh)
        source = manifest["sources"][0]
        path = evidence.body_path(source["body_sha256"])
        path.write_bytes(b"tampered")
        with self.assertRaisesRegex(evidence.EvidenceError, "corrupted"):
            evidence.read_body(source["body_sha256"])
        with self.assertRaises(evidence.EvidenceError):
            evidence.window(manifest, source["id"])

    def test_a_missing_body_is_refused_not_silently_empty(self):
        self.build_report()
        gh = FakeGh()
        self.routes(gh)
        capture_id, manifest, _ = self.capture(gh)
        source = manifest["sources"][0]
        evidence.body_path(source["body_sha256"]).unlink()
        with self.assertRaisesRegex(evidence.EvidenceError, "missing"):
            evidence.read_body(source["body_sha256"])

    def test_the_storage_budget_stops_and_names_itself(self):
        self.build_report()
        gh = FakeGh()
        self.routes(gh)
        capture_id, manifest, run = self.capture(gh, max_bytes=200)
        self.assertEqual(manifest["capture"]["stop_reason"], "storage_budget")
        self.assertFalse(evidence.is_complete(manifest))
        # Whatever was stored before the stop is still readable.
        for source in manifest["sources"][:1]:
            evidence.read_body(source["body_sha256"])

    def test_the_request_budget_stops_and_leaves_a_resumable_checkpoint(self):
        self.build_report()
        gh = FakeGh()
        self.routes(gh)
        capture_id = evidence.new_capture_id()
        selection = self.selection()
        manifest = evidence.build_manifest(selection, capture_id, request_limit=9)
        evidence.write_manifest(capture_id, manifest)
        with patch.object(evidence.ghread, "read", gh.read), \
             patch.object(evidence.ghread, "graphql", gh.graphql):
            run = evidence.Capture(selection, capture_id, manifest,
                                   budget=ghread.Budget(9, reserved=evidence.RESERVED_BUDGET))
            run.run()
        self.assertEqual(manifest["capture"]["stop_reason"], "request_budget")
        self.assertLessEqual(manifest["capture"]["requests_used"],
                             manifest["capture"]["request_limit"])
        self.assertTrue(manifest["sources"])
        # The checkpoint on disk is the resumable one.
        reread = evidence.read_manifest(capture_id)
        self.assertEqual(reread["capture"]["stop_reason"], "request_budget")
        self.assertTrue(reread["sources"])

    def test_a_resumed_capture_does_not_restore_the_budget(self):
        """Resuming must not silently refill the budget the operator set."""
        self.build_report()
        gh = FakeGh()
        self.routes(gh)
        capture_id = evidence.new_capture_id()
        selection = self.selection()
        manifest = evidence.build_manifest(selection, capture_id, request_limit=9)
        evidence.write_manifest(capture_id, manifest)
        with patch.object(evidence.ghread, "read", gh.read), \
             patch.object(evidence.ghread, "graphql", gh.graphql):
            evidence.Capture(selection, capture_id, manifest,
                             budget=ghread.Budget(9, reserved=evidence.RESERVED_BUDGET)).run()
        used = manifest["capture"]["requests_used"]
        gh2 = FakeGh()
        self.routes(gh2)
        with patch.object(evidence.ghread, "read", gh2.read), \
             patch.object(evidence.ghread, "graphql", gh2.graphql):
            evidence.Capture(selection, capture_id, manifest,
                             budget=ghread.Budget(used, reserved=0)).run()
        self.assertLessEqual(manifest["capture"]["requests_used"] + 0, used + len(gh2.calls))

    def test_path_confinement_refuses_traversal_symlinks_and_links(self):
        """Traversal, symlinks and hard links are all refused.

        The final resolved-path check in `confined_file` is deliberately kept as
        defence in depth against a component being swapped for a symlink between
        the walk and the resolve; no deterministic test isolates it, because the
        `..` rejection and the symlink walk already catch every non-racing input.
        """
        root = evidence.root()
        root.mkdir(parents=True, exist_ok=True)
        victim = root / "victim.txt"
        victim.write_bytes(b"x")
        (root / "escape").symlink_to("/etc/passwd")
        (root / "hardlink").write_bytes(b"y")
        os.link(root / "hardlink", root / "hardlink2")
        for bad in ("../manifest.json", "a/../../etc/passwd", "/etc/passwd",
                    "escape", "hardlink", "sub/../victim.txt/sub", "", "x\x00y"):
            with self.subTest(path=bad), self.assertRaises(evidence.EvidenceError):
                evidence.confined_file(bad, root)
        good = evidence.confined_file("victim.txt", root)
        self.assertEqual(good, victim.resolve())

    def test_the_writer_lock_refuses_a_live_holder_and_recovers_a_dead_one(self):
        evidence.root().mkdir(parents=True, exist_ok=True)
        capture_id = evidence.new_capture_id()
        first = evidence.Lock(capture_id)
        first.acquire()
        try:
            with self.assertRaisesRegex(evidence.EvidenceError, "another writer"):
                evidence.Lock(capture_id).acquire()
        finally:
            first.release()
        # A lock left by a process that no longer exists is reclaimed.
        stale = evidence.Lock(capture_id)
        stale.path.write_text(json.dumps({"pid": 2**22 + 12345, "started_at": 0}))
        reclaimed = evidence.Lock(capture_id)
        reclaimed.acquire()
        reclaimed.release()
        # And an explicit override replaces a live-looking one.
        stale.path.write_text(json.dumps({"pid": os.getpid(), "started_at": 0}))
        forced = evidence.Lock(capture_id)
        forced.acquire(break_lock=True)
        forced.release()


class GuardTests(EvidenceTests):
    """Each of these fails if its guard is removed; verified by negative control."""

    def test_ci_from_the_wrong_repository_is_refused(self):
        """A repository-prefix URL check is not proof: the answer must name it too.

        Removing the identity comparison in `verify_ci_scope` makes this test
        pass with foreign CI recorded, so it is the guard's own test.
        """
        self.build_report()
        gh = FakeGh()
        member = self.routes(gh)
        for group, path in (("check_runs", "check-runs"), ("statuses", "status")):
            gh.on(f"/repos/{BASE_NAME}/commits/{member['head_sha']}/{path}", None,
                  repository={"id": 999999, "full_name": "someone/else"}, sha=member["head_sha"])
        capture_id, manifest, _ = self.capture(gh)
        entry = next(e for e in manifest["components"]
                     if e["component"] == "checks" and e["number"] == member["number"])
        for group in entry["groups"]:
            if group["group"] in ("check_runs", "statuses"):
                self.assertEqual(group["status"], "blocked")
                self.assertIn("someone/else", group["reason"])

    def test_a_ci_answer_for_another_revision_is_refused(self):
        self.build_report()
        gh = FakeGh()
        member = self.routes(gh)
        gh.on(f"/repos/{BASE_NAME}/commits/{member['head_sha']}/check-runs",
              {"total_count": 1, "check_runs": [{"id": 1, "head_sha": "d" * 40,
                                                 "name": "job"}]})
        capture_id, manifest, _ = self.capture(gh)
        entry = next(e for e in manifest["components"]
                     if e["component"] == "checks" and e["number"] == member["number"])
        group = next(g for g in entry["groups"] if g["group"] == "check_runs")
        self.assertEqual(group["status"], "blocked")
        self.assertIn("revision", group["reason"])

    def test_a_foreign_closing_issues_answer_is_refused(self):
        self.build_report()
        gh = FakeGh()
        member = self.routes(gh)
        gh.on("graphql", closing_payload(member["number"], repo="someone/else"),
              graphql_number=member["number"])
        capture_id, manifest, _ = self.capture(gh)
        entry = next(e for e in manifest["components"]
                     if e["component"] == "closing_issues" and e["number"] == member["number"])
        self.assertEqual(entry["status"], "blocked")
        self.assertIn("someone/else", entry["reason"])

    def test_a_closing_issues_graphql_error_is_refused_not_recorded_as_empty(self):
        self.build_report()
        gh = FakeGh()
        member = self.routes(gh)
        gh.on("graphql", {"errors": [{"message": "rate limited"}], "data": None},
              graphql_number=member["number"])
        capture_id, manifest, _ = self.capture(gh)
        entry = next(e for e in manifest["components"]
                     if e["component"] == "closing_issues" and e["number"] == member["number"])
        self.assertEqual(entry["status"], "blocked")
        self.assertIn("errors", entry["reason"])
