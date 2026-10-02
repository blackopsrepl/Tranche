"""Regressions for defects found while landing the native evidence CLI."""
import os
import time

import evidence
from tests.test_evidence import EvidenceTests, FakeGh, BASE_NAME, metadata_payload


class LandingRegressions(EvidenceTests):
    def test_ci_pagination_counts_all_pages(self):
        self.build_report()
        gh = FakeGh()
        member = self.routes(gh)
        endpoint = f"/repos/{BASE_NAME}/commits/{member['head_sha']}/check-runs"
        gh.on(endpoint, None, pages={
            1: {"payload": {"total_count": 2, "check_runs": [{"head_sha": member['head_sha']}]},
                "link": f'<https://api.github.com{endpoint}?page=2>; rel="next"'},
            2: {"payload": {"total_count": 2, "check_runs": [{"head_sha": member['head_sha']}]}}
        })
        _, manifest, _ = self.capture(gh)
        state = evidence.group_entry(manifest, member['number'], 'checks', 'check_runs')
        self.assertEqual(state['items_observed'], 2)
        self.assertEqual(state['status'], 'complete')

    def test_live_writer_is_not_reclaimed_after_ttl(self):
        first = evidence.Lock('1' * 32, ttl=1)
        first.acquire()
        self.addCleanup(first.release)
        os.utime(first.path, (time.time() - 10, time.time() - 10))
        second = evidence.Lock('1' * 32, ttl=1)
        self.addCleanup(second.release)
        with self.assertRaisesRegex(evidence.EvidenceError, 'another writer'):
            second.acquire()

    def test_body_reads_refuse_symlinks_even_with_matching_digest(self):
        import hashlib
        self.build_report()
        data = b'outside evidence root'
        sha = hashlib.sha256(data).hexdigest()
        outside = self.out / 'outside.bin'
        outside.write_bytes(data)
        path = evidence.body_path(sha)
        path.parent.mkdir(parents=True, exist_ok=True)
        path.symlink_to(outside)
        with self.assertRaisesRegex(evidence.EvidenceError, 'symlink'):
            evidence.read_body(sha)

    def test_invalidated_capture_cannot_resume_even_if_revision_returns(self):
        self.build_report()
        gh = FakeGh()
        self.routes(gh)
        capture_id, manifest, _ = self.capture(gh)
        manifest['capture']['stop_reason'] = 'revision_drift'
        evidence.write_manifest(capture_id, manifest)
        with self.assertRaisesRegex(evidence.SelectionError, 'invalidated'):
            self.capture(gh, capture_id=capture_id)

    def test_revision_drift_after_acquisition_invalidates_capture(self):
        self.build_report()
        gh = FakeGh()
        member = self.routes(gh)
        reads = []

        def moving_metadata(call):
            reads.append(call)
            return metadata_payload(member, head_sha='c' * 40 if len(reads) > 2 else None)

        gh.on(f"/repos/{BASE_NAME}/pulls/{member['number']}", moving_metadata)
        with self.assertRaises(evidence.SelectionError):
            self.capture(gh)
