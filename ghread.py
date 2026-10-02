#!/usr/bin/env python3
"""Bounded, GET-only read transport for GitHub evidence acquisition.

`tranche.fetch_page` returns a parsed page and throws away the HTTP status,
headers and raw bytes; evidence needs all three. This module is the narrow
extension: it keeps the existing credential ownership (`gh` holds the token, the
process never reads it and never puts it in an argv) while returning exact
response bytes, the status, the headers and the continuation link.

Everything here is read-only. REST requests are explicit GET. GraphQL requests
are queries only and are refused if the document is not a query. Redirects and
continuation links are validated against the allowed origin and repository scope
before they are followed, and every attempted request is charged to a budget.
"""
from __future__ import annotations

import json
import os
import re
import select
import subprocess
import time
from urllib.parse import parse_qsl, urlsplit

ORIGIN = "https://api.github.com"
GRAPHQL_URL = f"{ORIGIN}/graphql"
MAX_RESPONSE_BYTES = 4 * 1024 * 1024
MAX_STDERR_BYTES = 64 * 1024
MAX_GRAPHQL_BODY_BYTES = 256 * 1024
MAX_REDIRECTS = 3
CHUNK_BYTES = 64 * 1024
GH_TIMEOUT = 60

CREDENTIAL_KEYS = frozenset({
    "token", "access_token", "authorization", "api_key", "client_secret",
    "private_token", "password", "key", "auth", "credentials",
})
REDIRECT_STATUSES = (301, 302, 303, 307, 308)


class ReadError(RuntimeError):
    """The transport could not produce a usable response."""


class BudgetExhausted(ReadError):
    """No request budget is left for an attempt (or for a reserved one)."""


class ResponseTooLarge(ReadError):
    """The response body exceeded the configured bound."""


class Budget:
    """Attempt accounting: every invocation counts, failures included.

    `reserved` capacity is held back from ordinary requests so work that must not
    be starved - the final identity checks a capture performs before it certifies
    anything - can still run. A caller that is spending that reserved capacity
    says so explicitly with ``reserve=0``.
    """

    def __init__(self, limit: int, reserved: int = 0):
        if type(limit) is not int or type(limit) is bool or limit < 0:
            raise ReadError("request budget must be a nonnegative integer")
        if type(reserved) is not int or type(reserved) is bool or reserved < 0 or reserved > limit:
            raise ReadError("reserved budget must be within the request budget")
        self.limit = limit
        self.reserved = reserved
        self.used = 0
        self.failures = 0
        self.retries = 0
        self.identity_checks = 0

    @property
    def remaining(self) -> int:
        return max(0, self.limit - self.used)

    def can(self, reserve: int | None = None) -> bool:
        """True when one more request fits while keeping `reserve` (default: the floor)."""
        keep = self.reserved if reserve is None else reserve
        return self.used + 1 + keep <= self.limit

    def charge(self, reserve: int | None = None) -> None:
        """Consume one attempt, keeping `reserve` requests available for later work."""
        keep = self.reserved if reserve is None else reserve
        if not self.can(keep):
            raise BudgetExhausted(
                f"request budget exhausted ({self.used}/{self.limit}, "
                f"{keep} held back)")
        self.used += 1

    def summary(self) -> dict:
        return {"limit": self.limit, "used": self.used, "reserved": self.reserved,
                "failures": self.failures, "retries": self.retries,
                "identity_checks": self.identity_checks}


class Response:
    """One HTTP response: exact decoded bytes plus the metadata it arrived with."""

    __slots__ = ("status", "url", "headers", "body", "truncated", "attempts")

    def __init__(self, status, url, headers, body, truncated=False, attempts=1):
        self.status = status
        self.url = url
        self.headers = headers
        self.body = body
        self.truncated = truncated
        self.attempts = attempts

    @property
    def media_type(self) -> str:
        return (self.headers.get("content-type") or "").split(";")[0].strip()

    @property
    def next_url(self) -> str | None:
        return parse_next_link(self.headers.get("link"))

    def json(self):
        """Parse the body as JSON. Truncated bodies are never parsed silently."""
        if self.truncated:
            raise ResponseTooLarge("truncated response body cannot be parsed as JSON")
        try:
            return json.loads(self.body.decode("utf-8"))
        except (UnicodeDecodeError, json.JSONDecodeError) as exc:
            raise ReadError(f"response body is not UTF-8 JSON ({exc})") from exc


# ---------------------------------------------------------------------------
# URL policy
# ---------------------------------------------------------------------------

def validate_url(url: str, repo_prefix: str | None = None) -> str:
    """A URL we are willing to request: allowed origin, no credentials, no fragment.

    `repo_prefix` is `owner/repo` when the request must stay inside one
    repository's REST namespace. It is a scope check on the URL, never proof that
    a response belongs to the resource we asked for - that is verified against
    the returned payload at acquisition time.
    """
    if not isinstance(url, str) or not url:
        raise ReadError("refusing empty or non-string URL")
    if any(character in url for character in "\r\n\t "):
        raise ReadError("refusing URL containing whitespace or control characters")
    parts = urlsplit(url)
    if parts.scheme != "https" or parts.netloc != "api.github.com":
        raise ReadError(f"refusing URL outside {ORIGIN}: {url}")
    if parts.fragment:
        raise ReadError("refusing URL with a fragment")
    if url[: len(ORIGIN)] != ORIGIN:
        raise ReadError(f"refusing URL that only resembles {ORIGIN}: {url}")
    keys = {key.lower() for key, _ in parse_qsl(parts.query, keep_blank_values=True)}
    if keys & CREDENTIAL_KEYS:
        raise ReadError("refusing URL carrying credential-shaped query parameters")
    if repo_prefix is not None:
        prefix = f"/repos/{repo_prefix}"
        if parts.path != prefix and not parts.path.startswith(f"{prefix}/"):
            raise ReadError(f"refusing URL outside repository {repo_prefix}: {url}")
    return url


def parse_next_link(link_header: str | None) -> str | None:
    """The `rel="next"` continuation URL, validated; other relations are ignored."""
    if not link_header:
        return None
    for entry in re.split(r",(?=\s*<)", link_header):
        match = re.match(r"\s*<([^>]*)>\s*(.*)", entry)
        if not match:
            continue
        relations = match.group(2)
        if not re.search(r"rel\s*=\s*\"?next\"?", relations):
            continue
        return validate_url(match.group(1))
    return None


def page_param(url: str) -> int:
    values = dict(parse_qsl(urlsplit(url).query, keep_blank_values=True))
    try:
        return int(values.get("page", "1"))
    except (TypeError, ValueError) as exc:
        raise ReadError(f"unreadable page parameter in {url}") from exc


# ---------------------------------------------------------------------------
# Process invocation
# ---------------------------------------------------------------------------

class RawResult:
    __slots__ = ("returncode", "stdout", "stderr", "truncated", "timed_out")

    def __init__(self, returncode, stdout, stderr, truncated=False, timed_out=False):
        self.returncode = returncode
        self.stdout = stdout
        self.stderr = stderr
        self.truncated = truncated
        self.timed_out = timed_out


def run_gh(argv, *, timeout=GH_TIMEOUT, max_bytes=MAX_RESPONSE_BYTES, stdin_data=None):
    """Run `gh` with bounded, streamed output.

    stdout and stderr are read in chunks on non-blocking pipes and the child is
    killed the moment stdout exceeds the bound, so an unbounded response can
    never be buffered whole. No shell is involved and no credential ever enters
    the argv.
    """
    process = subprocess.Popen(
        argv,
        stdin=subprocess.PIPE if stdin_data is not None else subprocess.DEVNULL,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )
    if stdin_data is not None:
        try:
            process.stdin.write(stdin_data)
        finally:
            process.stdin.close()
    out_pipe, err_pipe = process.stdout, process.stderr
    if out_pipe is None or err_pipe is None:  # unreachable with PIPE, keeps types honest
        raise ReadError("gh pipes are unavailable")
    out_fd, err_fd = out_pipe.fileno(), err_pipe.fileno()
    deadline = time.monotonic() + timeout if timeout else None
    streams = {
        out_fd: {"data": b"", "limit": max_bytes, "truncated": False, "done": False},
        err_fd: {"data": b"", "limit": MAX_STDERR_BYTES, "truncated": False, "done": False},
    }
    for fd in (out_fd, err_fd):
        os.set_blocking(fd, False)
    handles = {out_fd: out_pipe, err_fd: err_pipe}
    timed_out = False
    try:
        while not all(stream["done"] for stream in streams.values()):
            remaining = None if deadline is None else deadline - time.monotonic()
            if remaining is not None and remaining <= 0:
                timed_out = True
                process.kill()
                break
            watch = [fd for fd, stream in streams.items() if not stream["done"]]
            try:
                ready, _, _ = select.select(watch, [], [], remaining)
            except InterruptedError:
                continue
            if not ready:
                timed_out = True
                process.kill()
                break
            for fd in ready:
                stream = streams[fd]
                try:
                    chunk = os.read(fd, CHUNK_BYTES)
                except (BlockingIOError, InterruptedError):
                    continue
                except OSError:
                    stream["done"] = True
                    continue
                if not chunk:
                    stream["done"] = True
                    continue
                room = stream["limit"] - len(stream["data"])
                if room > 0:
                    stream["data"] += chunk[:room]
                if len(chunk) > room:
                    stream["truncated"] = True
                    if fd == out_fd:
                        process.kill()
                        stream["done"] = True
                    # stderr overflow is dropped, never fatal.
        try:
            process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            timed_out = True
            process.kill()
            process.wait(timeout=5)
    finally:
        for handle in handles.values():
            try:
                handle.close()
            except OSError:
                pass
    return RawResult(process.returncode if process.returncode is not None else -1,
                     streams[out_fd]["data"], streams[err_fd]["data"],
                     streams[out_fd]["truncated"], timed_out)


def _split_response(raw: bytes):
    """Split `gh api --include` output into header blocks and the body.

    `gh` emits one header block per HTTP hop, CRLF-terminated, followed by the
    body. Redirects therefore appear as several blocks; the last one is the
    response we act on.
    """
    blocks, index = [], 0
    while True:
        match = re.search(rb"\r?\n\r?\n", raw[index:])
        if not match:
            break
        end = index + match.start()
        block = raw[index:end]
        if not block.lstrip().upper().startswith(b"HTTP/"):
            break
        blocks.append(block)
        index = index + match.end()
    if not blocks:
        raise ReadError("gh api --include produced no HTTP status line")
    return blocks, raw[index:]


def _headers(block: bytes) -> dict:
    lines = block.decode("utf-8", errors="replace").splitlines()
    headers: dict[str, str] = {}
    for line in lines[1:]:
        if ":" in line:
            key, value = line.split(":", 1)
            headers[key.strip().lower()] = value.strip()
    return headers


def _status(block: bytes) -> int:
    match = re.match(rb"HTTP/[\d.]+\s+(\d{3})", block.strip())
    if not match:
        raise ReadError("unreadable HTTP status line from gh")
    return int(match.group(1))


def invoke(argv, *, timeout=GH_TIMEOUT, max_bytes=MAX_RESPONSE_BYTES, stdin_data=None):
    """The single place a subprocess is spawned; tests replace this seam.

    Returns `(status, headers, body, truncated, hops)` for the final response.
    """
    result = run_gh(argv, timeout=timeout, max_bytes=max_bytes, stdin_data=stdin_data)
    if result.timed_out:
        raise ReadError(f"gh timed out after {timeout}s")
    if result.returncode != 0 and not result.stdout.strip():
        detail = _diagnostic(result.stderr)
        raise ReadError(f"gh failed ({detail})")
    blocks, body = _split_response(result.stdout)
    # gh writes the body for every hop it follows itself; our own redirect
    # handling only sees the ones it does not follow.
    headers = _headers(blocks[-1])
    if "content-length" in headers and not result.truncated:
        try:
            if int(headers["content-length"]) > max_bytes:
                raise ResponseTooLarge(
                    f"response declares {headers['content-length']} bytes, over the "
                    f"{max_bytes} byte bound")
        except ValueError:
            pass
    if result.returncode != 0:
        raise ReadError(f"gh failed with HTTP {_status(blocks[-1])} ({_diagnostic(result.stderr)})")
    return _status(blocks[-1]), headers, body, result.truncated, len(blocks)


def _diagnostic(stderr: bytes) -> str:
    """A short, credential-free diagnostic line for the operator."""
    text = stderr.decode("utf-8", errors="replace").strip()
    lines = [line.strip() for line in text.splitlines() if line.strip()]
    detail = lines[-1] if lines else "no detail"
    detail = re.sub(r"(?i)(token|authorization|password)[=:\s]+\S+", r"\1=REDACTED", detail)
    return detail[:200]


def read(url: str, *, accept: str | None = None, params: dict | None = None,
         budget: Budget | None = None, reserve: int | None = None,
         timeout: int = GH_TIMEOUT,
         max_bytes: int = MAX_RESPONSE_BYTES, repo_prefix: str | None = None) -> Response:
    """One bounded GET, following validated redirects, charging every attempt."""
    attempt_url = validate_url(url, repo_prefix)
    hops = 0
    attempts = 0
    while True:
        if budget is not None:
            budget.charge(reserve)
        attempts += 1
        argv = ["gh", "api", "--include", "--method", "GET", attempt_url]
        if accept:
            argv += ["-H", f"accept: {accept}"]
        for key, value in (params or {}).items():
            argv += ["-f", f"{key}={value}"]
        try:
            status, headers, body, truncated, blocks = invoke(
                argv, timeout=timeout, max_bytes=max_bytes)
        except ReadError:
            if budget is not None:
                budget.failures += 1
            raise
        hops += blocks - 1
        if status in REDIRECT_STATUSES and headers.get("location"):
            if hops >= MAX_REDIRECTS:
                raise ReadError(f"too many redirects from {attempt_url}")
            attempt_url = validate_url(_absolute(headers["location"], attempt_url), repo_prefix)
            continue
        if status >= 400:
            if budget is not None:
                budget.failures += 1
            raise ReadError(f"HTTP {status} from {attempt_url}")
        if truncated:
            raise ResponseTooLarge(
                f"response from {attempt_url} exceeded the {max_bytes} byte bound")
        return Response(status, attempt_url, headers, body, truncated=False, attempts=attempts)


def _absolute(location: str, base: str) -> str:
    if location.startswith("https://"):
        return location
    if not location.startswith("/"):
        raise ReadError(f"refusing relative redirect target {location!r}")
    return f"{ORIGIN}{location}"


QUERY_DOCUMENT = re.compile(r"^\s*(query\b|\{)", re.IGNORECASE)
MUTATION_DOCUMENT = re.compile(r"\b(mutation|subscription)\b", re.IGNORECASE)


def graphql(query: str, variables: dict | None = None, *, budget: Budget | None = None,
            reserve: int | None = None, timeout: int = GH_TIMEOUT,
            max_bytes: int = MAX_RESPONSE_BYTES) -> Response:
    """One GraphQL query. Mutations and subscriptions are refused by construction.

    The document is sent on stdin rather than in the argv, so the query text
    never appears in `ps` output and no credential-shaped value can be mistaken
    for an argument. The recorded provenance keeps the query text and the
    variables, never the transport.
    """
    if not isinstance(query, str) or not query.strip() or not QUERY_DOCUMENT.match(query):
        raise ReadError("refusing a GraphQL document that is not a query")
    if MUTATION_DOCUMENT.search(query):
        raise ReadError("refusing a GraphQL document mentioning mutation/subscription")
    body = json.dumps({"query": query, "variables": variables or {}},
                      ensure_ascii=False, allow_nan=False).encode("utf-8")
    if len(body) > MAX_GRAPHQL_BODY_BYTES:
        raise ReadError("GraphQL request body exceeds the bound")
    argv = ["gh", "api", "--include", "--method", "POST", GRAPHQL_URL, "--input", "-"]
    try:
        if budget is not None:
            budget.charge(reserve)
        status, headers, payload, truncated, _hops = invoke(
            argv, timeout=timeout, max_bytes=max_bytes, stdin_data=body)
    except ReadError:
        if budget is not None:
            budget.failures += 1
        raise
    if status >= 400:
        raise ReadError(f"HTTP {status} from {GRAPHQL_URL}")
    if truncated:
        raise ResponseTooLarge(f"GraphQL response exceeded the {max_bytes} byte bound")
    return Response(status, GRAPHQL_URL, headers, payload)
