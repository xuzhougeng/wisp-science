"""Throttled, retrying GET client for the OpenAlex REST API.

Politeness: one client instance enforces a minimum interval between requests
(default 0.5 s -> <= 2 req/s) and identifies itself for OpenAlex's polite
pool via the ``mailto`` parameter on every request. Budget discipline (MCP
transport allows < 60 s per tool call): request timeout 20 s, at most ONE
retry on 429/5xx/transport errors with a short fixed back-off.
"""
from __future__ import annotations

import json
import os
import time

import requests

from mcp_servers_common.ratelimit import retry_after_seconds
from mcp_servers_common.ua import contact_email, product_ua

BASE_URL = "https://api.openalex.org"
# Per-install operator identification (legal Y12). The polite-pool mailto is
# now the user-consented address (or None — the polite pool is retired
# anyway; the api_key wire is M4, separate).
MAILTO = contact_email()
USER_AGENT = product_ua("openalex-works")
# Per-request authentication (#115): OpenAlex's free anonymous quota is now
# tight; a user-supplied key is injected by the host app as OPENALEX_API_KEY.
API_KEY = os.environ.get("OPENALEX_API_KEY") or None


class OpenAlexApiError(RuntimeError):
    """Unrecoverable API or transport error."""


class NotFound(OpenAlexApiError):
    """The API returned 404 for the requested entity."""


class OpenAlexClient:
    """GET client returning parsed JSON bodies."""

    RETRY_STATUSES = {429, 500, 502, 503, 504}

    def __init__(self, base_url: str = BASE_URL, mailto: str | None = MAILTO,
                 min_interval_s: float = 0.5, timeout_s: float = 20.0,
                 max_attempts: int = 2,
                 session: requests.Session | None = None,
                 api_key: str | None = API_KEY):
        self.base_url = base_url.rstrip("/")
        self.mailto = mailto
        self.api_key = api_key
        self.min_interval_s = min_interval_s
        self.timeout_s = timeout_s
        self.max_attempts = max_attempts
        self.session = session or requests.Session()
        self.session.headers.update({"User-Agent": USER_AGENT})
        self._last_request_t = 0.0

    def _throttle(self) -> None:
        dt = time.monotonic() - self._last_request_t
        if dt < self.min_interval_s:
            time.sleep(self.min_interval_s - dt)

    def get(self, path: str, params: dict | None = None) -> dict:
        """GET ``base_url + path``; return the parsed JSON dict.

        Raises NotFound on 404, OpenAlexApiError on other HTTP errors or
        after exhausting the single retry on 429/5xx/transport failures.
        """
        q = dict(params or {})
        if self.mailto:
            q["mailto"] = self.mailto
        if self.api_key:
            q["api_key"] = self.api_key
        url = f"{self.base_url}{path}"
        last_err: Exception | None = None
        for attempt in range(self.max_attempts):
            self._throttle()
            try:
                resp = self.session.get(url, params=q, timeout=self.timeout_s)
            except requests.RequestException as exc:
                self._last_request_t = time.monotonic()
                last_err = exc
                if attempt < self.max_attempts - 1:  # no dead sleep on the final attempt (#2875 review 3386234809)
                    time.sleep(2.0)
                continue
            self._last_request_t = time.monotonic()
            if resp.status_code == 404:
                raise NotFound(f"not found: {path}")
            if resp.status_code in self.RETRY_STATUSES:
                last_err = OpenAlexApiError(
                    f"HTTP {resp.status_code}: {resp.text[:200]}")
                retry_after = resp.headers.get("Retry-After", "")
                # Bounded back-off: the whole tool call must stay < 50 s.
                delay = retry_after_seconds(
                    retry_after, 5.0 if resp.status_code == 429 else 2.0,
                    cap=10.0)
                if attempt < self.max_attempts - 1:  # no dead sleep on the final attempt (#2875 review 3386234809)
                    time.sleep(delay)
                continue
            if resp.status_code != 200:
                raise OpenAlexApiError(
                    f"HTTP {resp.status_code}: {resp.text[:300]}")
            try:
                return resp.json()
            except json.JSONDecodeError as exc:
                last_err = exc
                if attempt < self.max_attempts - 1:  # no dead sleep on the final attempt (#2875 review 3386234809)
                    time.sleep(2.0)
                continue
        raise OpenAlexApiError(
            f"giving up after {self.max_attempts} attempts: {last_err!r}")
