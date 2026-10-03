"""Exercise a packaged binary from an empty deployment, using local stubs only.

Run: python tests/smoke_standalone.py /absolute/path/to/tranche
"""
import http.server
import json
import os
import pathlib
import re
import shutil
import subprocess
import sys
import tempfile
import threading
import zipfile


class Model(http.server.BaseHTTPRequestHandler):
    calls = 0

    def do_POST(self):
        body = self.rfile.read(int(self.headers["Content-Length"]))
        type(self).calls += 1
        if b'"sameness"' in body:
            answers = {"sameness": {"choice": "related_but_different", "probabilities": {"same_change": 0.2}}}
        else:
            answers = {
                "category": {"choice": "fix"}, "risk": {"score": 1},
                "is_fix": {"noul": 0.9}, "dupe_signal": {"noul": 0.1},
                "finished_form": {"score": 2}, "review_effort": {"score": 1},
                "security_flag": {"noul": 0.0},
            }
        payload = json.dumps({"answers": answers, "model": "local-test-model"}).encode()
        self.send_response(200)
        self.send_header("Content-Length", str(len(payload)))
        self.end_headers()
        self.wfile.write(payload)

    def log_message(self, format, *args):
        pass


def smoke(binary):
    scratch = os.environ.get("TMPDIR")
    with tempfile.TemporaryDirectory(prefix="tranche-package-", dir=scratch) as directory:
        root = pathlib.Path(directory)
        executable = root / "tranche"
        shutil.copy2(binary, executable)
        deployment = root / "deployment"
        deployment.mkdir()
        commands = root / "commands"
        commands.mkdir()
        items = [
            {"number": n, "title": "Fix widget startup crash", "body": "Tested fix.",
             "head": {"sha": str(n) * 40}, "user": {"login": "tester"}}
            for n in (1, 2)
        ]
        fixture = commands / "membership.json"
        fixture.write_text(json.dumps(items))
        fake = commands / "gh"
        # No real GitHub access. Validate the deployment selected the intended repository.
        fake.write_text('#!/bin/sh\ncase "$2" in\n*repos/sample/widgets/pulls*) ;;\n*) exit 9 ;;\nesac\nexec /bin/cat "' + str(fixture) + '"\n')
        fake.chmod(0o755)
        with http.server.ThreadingHTTPServer(("127.0.0.1", 0), Model) as server:
            thread = threading.Thread(target=server.serve_forever, daemon=True)
            thread.start()
            env = dict(os.environ, TYPESAFE_API_KEY="local-test-key-not-real",
                       TRANCHE_DEV_API_URL=f"http://127.0.0.1:{server.server_port}",
                       PATH=f"{commands}:{os.environ.get('PATH', '')}")

            def run(*args):
                result = subprocess.run([str(executable), "--root", str(deployment), *args],
                                        cwd=root, env=env, capture_output=True, text=True, timeout=30)
                assert result.returncode == 0, result.stdout + result.stderr
                return result

            try:
                run("init", "sample/widgets")
                run("refresh")
                run("page", "--no-html", "--export-json", "--export-xlsx")
                calls = Model.calls
                assert calls == 3, f"expected two judgments and one comparison; got {calls}"
                run("refresh")
                assert Model.calls == calls, "unchanged refresh must reuse model work"
                summary = json.loads(run("info", "--json").stdout)
                assert summary["repo"] == "sample/widgets" and summary["judged"] == 2
                docs = deployment / "docs"
                html = (docs / "index.html").read_text()
                for asset in re.findall(r'(?:src|href|srcset)="(assets/[^" ]+)"', html):
                    assert (docs / asset).is_file(), asset
                assert "assets/omarchy" not in html
                data = json.loads((docs / "data/workbench.json").read_text())
                assert data["repository"] == "sample/widgets" and len(data["prs"]) == 2
                with zipfile.ZipFile(docs / "data/report.xlsx") as workbook:
                    assert "xl/workbook.xml" in workbook.namelist()
                assert json.loads((docs / "data/report.json").read_text())["repository"] == "sample/widgets"
                print("PASS: isolated init, refresh, embedded assets, JSON/XLSX, verified info and zero-call resume")
            finally:
                server.shutdown()
                thread.join()


if __name__ == "__main__":
    smoke(pathlib.Path(sys.argv[1]).resolve())
