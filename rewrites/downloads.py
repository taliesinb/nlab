#!/usr/bin/env python3
"""Download the files nLab pages reference, governed by a JSON rules config.

Every run:
  1. scan     every pages/**/content.md for URLs (absolute links, relative
              /nlab/files/ refs, [[Name:file]] links, imagefromfile names)
              into the `downloads` / `download_refs` tables of build/state.db
  2. evaluate match each URL against the config's rules (first match wins)
              and (re)queue what the current config allows
  3. fetch    a thread pool downloads the queue, per-host limited
              (max_concurrent, min_interval, robots.txt incl. Crawl-delay,
              per-host back-off on 429/5xx/timeouts) - resumable: Ctrl-C and
              rerun; nothing is marked done until it is on disk
  4. export   build/files.tsv: canonical URL, store path, kind - for every
              downloaded file the CURRENT config still allows (so switching
              to a narrower preset also narrows what gets published)

Files are stored content-addressed as build/files/<sha256[:32]>.<kind>,
where kind is sniffed from the bytes (png/jpg/gif/webp/svg/pdf), never
taken from the URL or Content-Type: ncatlab.org answers a missing upload
with 200 + an HTML page.

rewrites/bulk_pages.py keys each page's conversion on the files.tsv
entries of the images it references, so a changed export re-converts
exactly the affected pages.

Config: a JSON file {"extends", "global", "rules", "note"}; see
downloads/*.json and README.md. `extends` (path or list, relative to the
file) loads parents first; their rules come first. --config may be given
several times; files are concatenated in order (each loaded once).

Usage: downloads.py [--config FILE ...] [--status] [--limit N]
                    [--rule NAME ...] [--retry] [--recheck] [--no-fetch]
"""

import argparse
import collections
import concurrent.futures
import dataclasses
import email.utils
import fcntl
import fnmatch
import hashlib
import http.client
import json
import pathlib
import re
import shutil
import signal
import sqlite3
import sys
import threading
import time
import urllib.error
import urllib.parse
import urllib.request
import urllib.robotparser

ROOT = pathlib.Path(__file__).resolve().parent.parent
PAGES = ROOT / "build/nlab-content/pages"
STATE_DB = ROOT / "build/state.db"
STORE = ROOT / "build/files"  # published (as dist/html/files/) via files.tsv
MAP_TSV = ROOT / "build/files.tsv"  # read by nlab-typ (NLAB_FILES_MAP)
TMP = ROOT / "build/.downloads-tmp"  # same filesystem as STORE: atomic rename
DEFAULT_CONFIG = ROOT / "downloads/ncatlab.json"

KIND_GROUPS = {
    "image": {"png", "jpg", "gif", "webp", "svg"},
    "pdf": {"pdf"},
}
ALL_KINDS = set().union(*KIND_GROUPS.values())

# statuses
QUEUED, RETRY, DONE = "queued", "retry", "done"
TERMINAL = ("missing", "refused", "rejected", "too-large", "blocked", "failed")


# ------------------------------------------------------------- config

GLOBAL_DEFAULTS = {
    "max_concurrent": 8,  # downloads in flight across all hosts
    "user_agent": "ncatlab-dash/1.0 (+https://github.com/taliesinb/ncatlab-dash)",
    "max_attempts": 4,  # transient failures before a URL is 'failed'
    "max_wait": 300,  # seconds; a host backed off longer is left for the next run
}
RULE_DEFAULTS = {
    "host": "*",
    "path": "*",
    "disable": False,
    "accept": ["image", "pdf"],
    "rewrite": None,
    "max_concurrent": 2,  # per host
    "min_interval": 1.0,  # seconds between request starts, per host
    "max_bytes": 50 * 1024 * 1024,
    "max_total_bytes": None,  # budget for everything this rule downloads
    "timeout": 60,
    "robots": True,  # obey robots.txt (Disallow and Crawl-delay)
    "backoff_initial": 60,
    "backoff_max": 3600,
}
TOP_KEYS = {"extends", "global", "rules", "note"}


@dataclasses.dataclass
class Rule:
    name: str
    hosts: list
    paths: list
    disable: bool
    accept: set
    rewrite: tuple | None
    max_concurrent: int
    min_interval: float
    max_bytes: int
    max_total_bytes: int | None
    timeout: float
    robots: bool
    backoff_initial: float
    backoff_max: float
    host_re: re.Pattern = None
    path_re: re.Pattern = None

    def matches(self, host: str, path: str) -> bool:
        return bool(self.host_re.match(host) and self.path_re.match(path))

    def fetch_url(self, url: str) -> str:
        if self.rewrite:
            return self.rewrite[0].sub(self.rewrite[1], url, count=1)
        return url


def _globs(v, what: str, where: str) -> list:
    v = [v] if isinstance(v, str) else v
    if not isinstance(v, list) or not v or not all(isinstance(x, str) for x in v):
        raise SystemExit(f"config {where}: '{what}' must be a glob or list of globs")
    return [x.lower() for x in v]


def _any_glob(globs: list) -> re.Pattern:
    return re.compile("|".join(f"(?:{fnmatch.translate(g)})" for g in globs))


def parse_rule(raw: dict, where: str) -> Rule:
    if not isinstance(raw, dict):
        raise SystemExit(f"config {where}: a rule must be an object")
    unknown = set(raw) - set(RULE_DEFAULTS) - {"name", "note"}
    if unknown:
        raise SystemExit(f"config {where}: unknown rule keys {sorted(unknown)}")
    if not isinstance(raw.get("name"), str) or not raw["name"]:
        raise SystemExit(f"config {where}: every rule needs a 'name'")
    r = {**RULE_DEFAULTS, **raw}
    accept = set()
    for a in ([r["accept"]] if isinstance(r["accept"], str) else r["accept"]):
        if a in KIND_GROUPS:
            accept |= KIND_GROUPS[a]
        elif a in ALL_KINDS:
            accept.add(a)
        else:
            raise SystemExit(f"config {where}: unknown accept kind {a!r} "
                             f"(use {sorted(KIND_GROUPS) + sorted(ALL_KINDS)})")
    rewrite = None
    if r["rewrite"] is not None:
        if not (isinstance(r["rewrite"], list) and len(r["rewrite"]) == 2):
            raise SystemExit(f"config {where}: 'rewrite' is [regex, replacement]")
        try:
            rewrite = (re.compile(r["rewrite"][0]), r["rewrite"][1])
        except re.error as e:
            raise SystemExit(f"config {where}: bad rewrite regex: {e}")
    for k in ("max_concurrent", "min_interval", "max_bytes", "timeout",
              "backoff_initial", "backoff_max"):
        if not isinstance(r[k], (int, float)) or isinstance(r[k], bool) or r[k] < 0:
            raise SystemExit(f"config {where}: '{k}' must be a non-negative number")
    if r["max_concurrent"] < 1:
        raise SystemExit(f"config {where}: 'max_concurrent' must be >= 1")
    hosts = _globs(r["host"], "host", where)
    paths = _globs(r["path"], "path", where)
    return Rule(
        name=r["name"], hosts=hosts, paths=paths, disable=bool(r["disable"]),
        accept=accept, rewrite=rewrite, max_concurrent=int(r["max_concurrent"]),
        min_interval=float(r["min_interval"]), max_bytes=int(r["max_bytes"]),
        max_total_bytes=r["max_total_bytes"], timeout=float(r["timeout"]),
        robots=bool(r["robots"]), backoff_initial=float(r["backoff_initial"]),
        backoff_max=float(r["backoff_max"]),
        host_re=_any_glob(hosts), path_re=_any_glob(paths),
    )


class Config:
    def __init__(self, paths: list[pathlib.Path]):
        self.glob = dict(GLOBAL_DEFAULTS)
        self.rules: list[Rule] = []
        self.files: list[pathlib.Path] = []
        for p in paths:
            self._load(pathlib.Path(p).resolve(), ())
        names = [r.name for r in self.rules]
        dupes = {n for n in names if names.count(n) > 1}
        if dupes:
            raise SystemExit(f"config: duplicate rule names {sorted(dupes)}")
        self.by_name = {r.name: r for r in self.rules}

    def _load(self, p: pathlib.Path, stack: tuple):
        if p in stack:
            raise SystemExit(f"config: extends cycle through {p}")
        if p in self.files:
            return  # diamond / passed twice: load once
        try:
            data = json.loads(p.read_text(encoding="utf-8"))
        except (OSError, json.JSONDecodeError) as e:
            raise SystemExit(f"config {p}: {e}")
        if not isinstance(data, dict) or set(data) - TOP_KEYS:
            raise SystemExit(f"config {p}: top level must be an object with "
                             f"keys from {sorted(TOP_KEYS)}")
        ext = data.get("extends", [])
        for e in [ext] if isinstance(ext, str) else ext:
            self._load((p.parent / e).resolve(), stack + (p,))
        g = data.get("global", {})
        unknown = set(g) - set(GLOBAL_DEFAULTS) - {"note"}
        if unknown:
            raise SystemExit(f"config {p}: unknown global keys {sorted(unknown)}")
        self.glob.update({k: v for k, v in g.items() if k != "note"})
        for i, raw in enumerate(data.get("rules", [])):
            self.rules.append(parse_rule(raw, f"{p.name} rule {i}"))
        self.files.append(p)

    def match(self, url: str) -> Rule | None:
        try:
            u = urllib.parse.urlsplit(url)
        except ValueError:
            return None
        if u.scheme not in ("http", "https") or not u.hostname:
            return None
        host, path = u.hostname.lower(), (u.path or "/").lower()
        for r in self.rules:
            if r.matches(host, path):
                return r
        return None


# --------------------------------------------------------------- scan

URL_RE = re.compile(r"https?://[^\s<>\"'\]\)\}|\\`]+")
REL_RE = re.compile(r"(?<![\w/.:])/nlab/files/[A-Za-z0-9_.%+\-]+")
WIKIFILE_RE = re.compile(r"\[\[([^\[\]|]+?):file(?:\|[^\[\]]*)?\]\]")
IMGFILE_RE = re.compile(r'"file_name"\s*:\s*"([^"]+)"')
NLAB_FILES = "https://ncatlab.org/nlab/files/"


def canonical_url(url: str) -> str:
    """Map key; must match canonical_file_url in crates/nlab-typ/src/prose.rs.
    nLab uploads (absolute, relative, http or https) become
    https://ncatlab.org/nlab/files/<name>; other URLs stay as written."""
    u = url.strip()
    if "/nlab/files/" in u:
        name = re.split(r"[?#]", u.split("/nlab/files/", 1)[1], maxsplit=1)[0]
        return NLAB_FILES + name
    return u


def urls_in(text: str) -> set[str]:
    out = set()
    for u in URL_RE.findall(text):
        u = u.rstrip(".,;:!?*'")
        if u.count("(") < u.count(")"):
            u = u.rstrip(")")
        out.add(canonical_url(u))
    out |= {canonical_url(u) for u in REL_RE.findall(text)}
    # same URL forms nlab-typ emits for these (spaces become '+')
    out |= {NLAB_FILES + n.strip().replace(" ", "+")
            for n in WIKIFILE_RE.findall(text)}
    out |= {NLAB_FILES + n.strip().replace(" ", "+")
            for n in IMGFILE_RE.findall(text)}
    return {u for u in out if not u.endswith("/nlab/files/")}


def scan(con: sqlite3.Connection):
    """Rebuild download_refs from the mirror; insert unseen URLs."""
    if not PAGES.is_dir():
        raise SystemExit(f"error: {PAGES} missing (fetch the nlab-content mirror)")
    refs = []
    for md in PAGES.glob("*/*/*/*/*/content.md"):
        pid = md.parent.name
        for u in urls_in(md.read_text(encoding="utf-8", errors="replace")):
            refs.append((u, pid))
    now = time.time()
    con.execute("DELETE FROM download_refs")
    con.executemany("INSERT OR IGNORE INTO download_refs(url, page) VALUES (?, ?)", refs)
    con.executemany(
        "INSERT OR IGNORE INTO downloads(url, status, first_seen) VALUES (?, 'new', ?)",
        ((u,) + (now,) for u in {u for u, _ in refs}))
    con.commit()


# ----------------------------------------------------------- database

SCHEMA = """
CREATE TABLE IF NOT EXISTS downloads(
  url TEXT PRIMARY KEY,        -- canonical URL as referenced by pages
  fetch_url TEXT,              -- after the matching rule's rewrite
  host TEXT,
  rule TEXT,                   -- name of the rule that matched last
  status TEXT NOT NULL,        -- new queued retry done | missing refused
                               -- rejected too-large blocked failed |
                               -- disabled unmatched
  attempts INTEGER NOT NULL DEFAULT 0,
  next_attempt REAL,
  http_status INTEGER,
  error TEXT,
  content_type TEXT,           -- server's claim, for diagnosis only
  final_url TEXT,              -- after redirects
  kind TEXT,                   -- sniffed: png jpg gif webp svg pdf (html ...)
  sha256 TEXT,
  size INTEGER,
  etag TEXT,
  last_modified TEXT,
  first_seen REAL,
  checked REAL
);
CREATE TABLE IF NOT EXISTS download_refs(
  url TEXT NOT NULL, page TEXT NOT NULL, PRIMARY KEY(url, page)
) WITHOUT ROWID;
CREATE INDEX IF NOT EXISTS download_refs_page ON download_refs(page);
CREATE TABLE IF NOT EXISTS download_hosts(
  host TEXT PRIMARY KEY,
  backoff_until REAL NOT NULL DEFAULT 0,
  failures INTEGER NOT NULL DEFAULT 0,
  robots TEXT,                 -- robots.txt body ('' = none / allow all)
  robots_fetched REAL
);
"""


def connect() -> sqlite3.Connection:
    STATE_DB.parent.mkdir(parents=True, exist_ok=True)
    con = sqlite3.connect(STATE_DB)
    con.row_factory = sqlite3.Row
    con.execute("PRAGMA journal_mode=WAL")
    con.executescript(SCHEMA)
    return con


def evaluate(con: sqlite3.Connection, cfg: Config, retry: bool):
    """Assign rule/fetch_url/host to every referenced URL and set statuses
    the current config implies. Done rows keep their file."""
    updates = []
    for row in con.execute(
            "SELECT url, status, kind, size, fetch_url FROM downloads d "
            "WHERE EXISTS (SELECT 1 FROM download_refs r WHERE r.url = d.url)"):
        url, status = row["url"], row["status"]
        rule = cfg.match(url)
        fetch = rule.fetch_url(url) if rule else None
        host = (urllib.parse.urlsplit(fetch).hostname or "").lower() if fetch else None
        new = status
        if status != DONE:
            if rule is None:
                new = "unmatched"
            elif rule.disable:
                new = "disabled"
            elif status in ("new", "unmatched", "disabled"):
                new = QUEUED
            elif status == "rejected" and row["kind"] in rule.accept:
                new = QUEUED  # config now accepts what we found
            elif status == "too-large" and (row["size"] or 0) <= rule.max_bytes:
                new = QUEUED
            elif retry and status in TERMINAL:
                new = QUEUED
        # done rows stay done (recheck selects them in load_jobs)
        updates.append((fetch, host, rule.name if rule else None, new,
                        0 if new == QUEUED and status in TERMINAL else None, url))
    con.executemany(
        "UPDATE downloads SET fetch_url = ?, host = ?, rule = ?, status = ?, "
        "attempts = COALESCE(?, attempts) WHERE url = ?", updates)
    con.commit()


# ------------------------------------------------------------ network

def sniff(head: bytes) -> str | None:
    if b"%PDF-" in head[:1024]:
        return "pdf"
    if head.startswith(b"\x89PNG\r\n\x1a\n"):
        return "png"
    if head[:3] == b"\xff\xd8\xff":
        return "jpg"
    if head[:6] in (b"GIF87a", b"GIF89a"):
        return "gif"
    if head[:4] == b"RIFF" and head[8:12] == b"WEBP":
        return "webp"
    low = head[:4096].lower()
    if b"<html" in low or low.lstrip().startswith(b"<!doctype html"):
        return "html"
    if b"<svg" in low:
        return "svg"
    return None


def retry_after(value: str | None) -> float | None:
    if not value:
        return None
    try:
        return max(0.0, float(value))
    except ValueError:
        pass
    try:
        return max(0.0, email.utils.parsedate_to_datetime(value).timestamp() - time.time())
    except (TypeError, ValueError):
        return None


class NoRedirect(urllib.request.HTTPRedirectHandler):
    """Redirects are followed by hand, so every hop is re-checked against
    the rules and rate-limited for the host it actually contacts."""

    def redirect_request(self, *args, **kwargs):
        return None


OPENER = urllib.request.build_opener(NoRedirect)


@dataclasses.dataclass
class HostState:
    active: int = 0
    next_start: float = 0.0
    backoff_until: float = 0.0
    failures: int = 0


class Robots:
    """robots.txt per host, cached in download_hosts (refetched after a week)."""

    TTL = 7 * 86400

    def __init__(self, con, user_agent):
        self.ua = user_agent
        self.lock = threading.Lock()
        self.host_locks = collections.defaultdict(threading.Lock)
        self.parsers: dict[str, urllib.robotparser.RobotFileParser] = {}
        self.dirty: dict[str, str] = {}
        now = time.time()
        for row in con.execute(
                "SELECT host, robots, robots_fetched FROM download_hosts "
                "WHERE robots IS NOT NULL"):
            if now - (row["robots_fetched"] or 0) < self.TTL:
                self.parsers[row["host"]] = self._parse(row["robots"])

    def _parse(self, text: str):
        rp = urllib.robotparser.RobotFileParser()
        rp.parse(text.splitlines())
        return rp

    def known(self, host: str):
        with self.lock:
            return self.parsers.get(host)

    def get(self, scheme: str, host: str, timeout: float):
        """Parser for host, fetching robots.txt if needed (caller holds a
        host slot, so this request is rate-limited like any other)."""
        with self.host_locks[host]:
            rp = self.known(host)
            if rp is not None:
                return rp, False
            text = ""
            try:
                req = urllib.request.Request(
                    f"{scheme}://{host}/robots.txt", headers={"User-Agent": self.ua})
                with urllib.request.urlopen(req, timeout=timeout) as r:
                    if r.status == 200:
                        text = r.read(512 * 1024).decode("utf-8", "replace")
            except urllib.error.HTTPError as e:
                if e.code in (401, 403):
                    text = "User-agent: *\nDisallow: /\n"
            except Exception:
                pass  # unreachable robots.txt: treat as allow-all
            rp = self._parse(text)
            with self.lock:
                self.parsers[host] = rp
                self.dirty[host] = text
            return rp, True

    def crawl_delay(self, host: str) -> float:
        rp = self.known(host)
        if rp is None:
            return 0.0
        try:
            return float(rp.crawl_delay(self.ua) or 0)
        except (TypeError, ValueError):
            return 0.0

    def flush(self, con):
        with self.lock:
            dirty, self.dirty = self.dirty, {}
        now = time.time()
        con.executemany(
            "INSERT INTO download_hosts(host, robots, robots_fetched) VALUES (?, ?, ?) "
            "ON CONFLICT(host) DO UPDATE SET robots = excluded.robots, "
            "robots_fetched = excluded.robots_fetched",
            ((h, t, now) for h, t in dirty.items()))


class Limiter:
    """Per-host concurrency + pacing + back-off, shared by the dispatcher
    and by workers following redirects."""

    def __init__(self, con, robots: Robots):
        self.cv = threading.Condition()
        self.robots = robots
        self.hosts: dict[str, HostState] = collections.defaultdict(HostState)
        for row in con.execute("SELECT host, backoff_until, failures FROM download_hosts"):
            s = self.hosts[row["host"]]
            s.backoff_until, s.failures = row["backoff_until"], row["failures"]
        self.dirty: set[str] = set()

    def interval(self, host: str, rule: Rule) -> float:
        d = self.robots.crawl_delay(host) if rule.robots else 0.0
        return max(rule.min_interval, d)

    def try_acquire(self, host: str, rule: Rule) -> bool:  # cv held
        s, now = self.hosts[host], time.time()
        if s.active >= rule.max_concurrent or now < s.next_start or now < s.backoff_until:
            return False
        s.active += 1
        s.next_start = now + self.interval(host, rule)
        return True

    def acquire(self, host: str, rule: Rule, deadline: float, stop) -> bool:
        with self.cv:
            while not self.try_acquire(host, rule):
                if time.time() > deadline or stop.is_set():
                    return False
                self.cv.wait(0.25)
            return True

    def release(self, host: str):
        with self.cv:
            self.hosts[host].active -= 1
            self.cv.notify_all()

    def success(self, host: str):
        with self.cv:
            s = self.hosts[host]
            if s.failures or s.backoff_until:
                s.failures, s.backoff_until = 0, 0.0
                self.dirty.add(host)

    def failure(self, host: str, rule: Rule, after: float | None) -> float:
        with self.cv:
            s = self.hosts[host]
            s.failures += 1
            delay = after if after is not None else min(
                rule.backoff_max, rule.backoff_initial * 2 ** (s.failures - 1))
            s.backoff_until = max(s.backoff_until, time.time() + delay)
            self.dirty.add(host)
            return s.backoff_until

    def flush(self, con):
        with self.cv:
            rows = [(h, self.hosts[h].backoff_until, self.hosts[h].failures)
                    for h in self.dirty]
            self.dirty.clear()
        con.executemany(
            "INSERT INTO download_hosts(host, backoff_until, failures) VALUES (?, ?, ?) "
            "ON CONFLICT(host) DO UPDATE SET backoff_until = excluded.backoff_until, "
            "failures = excluded.failures", rows)


@dataclasses.dataclass
class Job:
    fetch_url: str
    host: str
    rule: Rule
    urls: list
    etag: str | None = None
    last_modified: str | None = None
    old_sha: str | None = None
    not_before: float = 0.0


@dataclasses.dataclass
class Result:
    status: str  # done not-modified missing refused rejected too-large
    # blocked transient deferred
    host: str = ""
    http_status: int | None = None
    error: str | None = None
    content_type: str | None = None
    final_url: str | None = None
    kind: str | None = None
    sha256: str | None = None
    size: int | None = None
    etag: str | None = None
    last_modified: str | None = None
    retry_after: float | None = None


def fetch(job: Job, cfg: Config, limiter: Limiter, robots: Robots, stop) -> Result:
    """Runs in a worker; the dispatcher already holds a slot for job.host."""
    url, host, rule = job.fetch_url, job.host, job.rule
    ua = cfg.glob["user_agent"]
    hops = 0
    tmp = None
    try:
        while True:
            parts = urllib.parse.urlsplit(url)
            if rule.robots:
                rp, fetched = robots.get(parts.scheme, host, rule.timeout)
                if fetched:  # that was a request too: keep the pacing
                    time.sleep(limiter.interval(host, rule))
                if not rp.can_fetch(ua, url):
                    return Result("blocked", host, error="robots.txt disallows")
            headers = {"User-Agent": ua, "Accept": "*/*"}
            if hops == 0 and job.etag:
                headers["If-None-Match"] = job.etag
            if hops == 0 and job.last_modified:
                headers["If-Modified-Since"] = job.last_modified
            try:
                resp = OPENER.open(urllib.request.Request(url, headers=headers),
                                   timeout=rule.timeout)
            except urllib.error.HTTPError as e:
                code = e.code
                if code in (301, 302, 303, 307, 308):
                    loc = e.headers.get("Location")
                    e.close()
                    hops += 1
                    if not loc or hops > 5:
                        return Result("failed", host, code, "bad or too many redirects")
                    new = urllib.parse.urljoin(url, loc)
                    nparts = urllib.parse.urlsplit(new)
                    if nparts.scheme not in ("http", "https") or not nparts.hostname:
                        return Result("blocked", host, code, f"redirect to {new[:200]}")
                    nrule = cfg.match(new)
                    if nrule is not None and nrule.disable:
                        return Result("blocked", host, code,
                                      f"redirect into disabled rule {nrule.name}: {new[:200]}")
                    nhost = nparts.hostname.lower()
                    if nhost != host:
                        limiter.release(host)
                        host = None
                        lim_rule = nrule or rule
                        if not limiter.acquire(nhost, lim_rule,
                                               time.time() + cfg.glob["max_wait"], stop):
                            return Result("deferred", nhost, code, "redirect host busy")
                        host = nhost
                    url = new
                    continue
                if code == 304:
                    e.close()
                    return Result("not-modified", host, code)
                ra = retry_after(e.headers.get("Retry-After"))
                e.close()
                if code in (404, 410):
                    return Result("missing", host, code, f"HTTP {code}")
                if code == 429 or code >= 500:
                    return Result("transient", host, code, f"HTTP {code}", retry_after=ra)
                return Result("refused", host, code, f"HTTP {code}")
            except (urllib.error.URLError, OSError, ValueError,
                    http.client.HTTPException) as e:
                return Result("transient", host, error=f"{type(e).__name__}: {e}"[:300])

            with resp:
                ctype = resp.headers.get("Content-Type")
                clen = resp.headers.get("Content-Length")
                base = Result("", host, resp.status, content_type=ctype, final_url=url)
                if clen and clen.isdigit() and int(clen) > rule.max_bytes:
                    return dataclasses.replace(base, status="too-large", size=int(clen))
                try:
                    head = b""
                    while len(head) < 4096:
                        chunk = resp.read(4096 - len(head))
                        if not chunk:
                            break
                        head += chunk
                    kind = sniff(head)
                    if kind not in rule.accept:
                        return dataclasses.replace(
                            base, status="rejected", kind=kind or "other",
                            error=f"content is {kind or 'unrecognized'}")
                    TMP.mkdir(parents=True, exist_ok=True)
                    tmp = TMP / f"{threading.get_ident()}-{time.time_ns()}.part"
                    h = hashlib.sha256(head)
                    size = len(head)
                    with open(tmp, "wb") as f:
                        f.write(head)
                        while True:
                            if stop.is_set():
                                return dataclasses.replace(base, status="deferred",
                                                           error="interrupted")
                            chunk = resp.read(256 * 1024)
                            if not chunk:
                                break
                            size += len(chunk)
                            if size > rule.max_bytes:
                                return dataclasses.replace(base, status="too-large", size=size)
                            h.update(chunk)
                            f.write(chunk)
                except (OSError, ValueError, http.client.HTTPException) as e:
                    # connection dropped mid-body
                    return Result("transient", host, error=f"{type(e).__name__}: {e}"[:300])
                sha = h.hexdigest()
                dest = STORE / f"{sha[:32]}.{kind}"
                STORE.mkdir(parents=True, exist_ok=True)
                if dest.exists():
                    tmp.unlink()  # same content already stored (dedup)
                else:
                    tmp.replace(dest)
                tmp = None
                return dataclasses.replace(
                    base, status="done", kind=kind, sha256=sha, size=size,
                    etag=resp.headers.get("ETag"),
                    last_modified=resp.headers.get("Last-Modified"))
    finally:
        if tmp is not None:
            tmp.unlink(missing_ok=True)
        if host is not None:
            limiter.release(host)


# ----------------------------------------------------------- dispatch

def load_jobs(con, cfg: Config, rules_only: set, recheck: bool) -> list[Job]:
    statuses = [QUEUED, RETRY] + ([DONE] if recheck else [])
    q = ("SELECT url, fetch_url, host, rule, status, etag, last_modified, sha256, "
         "next_attempt FROM downloads d WHERE status IN (%s) AND fetch_url IS NOT NULL "
         "AND EXISTS (SELECT 1 FROM download_refs r WHERE r.url = d.url)"
         % ",".join("?" * len(statuses)))
    jobs: dict[str, Job] = {}
    for row in con.execute(q, statuses):
        rule = cfg.by_name.get(row["rule"])
        if rule is None or rule.disable or (rules_only and rule.name not in rules_only):
            continue
        if row["status"] == DONE and not (row["etag"] or row["last_modified"]):
            continue  # nothing to revalidate with
        j = jobs.get(row["fetch_url"])
        if j is None:
            j = jobs[row["fetch_url"]] = Job(row["fetch_url"], row["host"], rule, [])
            if row["status"] == DONE:
                j.etag, j.last_modified, j.old_sha = (
                    row["etag"], row["last_modified"], row["sha256"])
            j.not_before = row["next_attempt"] or 0.0
        j.urls.append(row["url"])
    return list(jobs.values())


def record(con, job: Job, res: Result, cfg: Config, limiter: Limiter,
           requeue) -> str:
    """Write a result to every URL row of the job; returns a stats key."""
    now = time.time()
    marks = ",".join("?" * len(job.urls))
    if res.status in ("done", "not-modified"):
        limiter.success(res.host)
    if res.status == "done":
        con.execute(
            f"UPDATE downloads SET status = 'done', attempts = 0, next_attempt = NULL, "
            f"http_status = ?, error = NULL, content_type = ?, final_url = ?, kind = ?, "
            f"sha256 = ?, size = ?, etag = ?, last_modified = ?, checked = ? "
            f"WHERE url IN ({marks})",
            (res.http_status, res.content_type, res.final_url, res.kind, res.sha256,
             res.size, res.etag, res.last_modified, now, *job.urls))
        return "done"
    if res.status == "not-modified":
        con.execute(f"UPDATE downloads SET checked = ? WHERE url IN ({marks})",
                    (now, *job.urls))
        return "not-modified"
    if res.status == "deferred":
        return "deferred"  # untouched: still queued for the next run
    if res.status == "transient":
        until = limiter.failure(res.host, job.rule, res.retry_after)
        row = con.execute(f"SELECT max(attempts) FROM downloads WHERE url IN ({marks})",
                          job.urls).fetchone()
        attempts = (row[0] or 0) + 1
        final = attempts >= cfg.glob["max_attempts"]
        con.execute(
            f"UPDATE downloads SET status = ?, attempts = ?, next_attempt = ?, "
            f"http_status = ?, error = ?, checked = ? WHERE url IN ({marks})",
            ("failed" if final else RETRY, attempts, until, res.http_status,
             res.error, now, *job.urls))
        if not final:
            job.not_before = until
            requeue(job)
        return "failed" if final else "retry"
    if job.old_sha is not None and res.status in ("missing", "refused"):
        # a recheck found the file gone: keep the copy we have, note why
        con.execute(f"UPDATE downloads SET error = ?, checked = ? WHERE url IN ({marks})",
                    (res.error, now, *job.urls))
        return "gone-upstream"
    con.execute(
        f"UPDATE downloads SET status = ?, http_status = ?, error = ?, content_type = ?, "
        f"final_url = ?, kind = COALESCE(?, kind), size = COALESCE(?, size), checked = ? "
        f"WHERE url IN ({marks})",
        (res.status, res.http_status, res.error, res.content_type, res.final_url,
         res.kind, res.size, now, *job.urls))
    return res.status


def run(con, cfg: Config, jobs: list[Job], limit: int | None):
    robots = Robots(con, cfg.glob["user_agent"])
    limiter = Limiter(con, robots)
    stop = threading.Event()
    pending: collections.OrderedDict[str, collections.deque] = collections.OrderedDict()

    def requeue(job: Job):
        pending.setdefault(job.host, collections.deque()).append(job)

    for j in sorted(jobs, key=lambda j: j.fetch_url):
        requeue(j)
    used = collections.Counter()  # bytes per rule, for max_total_bytes
    for row in con.execute("SELECT rule, sum(size) FROM downloads "
                           "WHERE status = 'done' GROUP BY rule"):
        used[row[0]] = row[1] or 0

    total = len(jobs)
    stats = collections.Counter()
    started = 0
    running: dict[concurrent.futures.Future, Job] = {}
    t0 = last_report = last_commit = time.time()
    G = cfg.glob["max_concurrent"]
    print(f"{total} downloads queued on {len(pending)} hosts "
          f"(at most {G} at a time; Ctrl-C to stop, rerun to resume)", flush=True)

    ex = concurrent.futures.ThreadPoolExecutor(G)
    try:
        while (pending or running) and not stop.is_set():
            now = time.time()
            with limiter.cv:
                for host in list(pending):
                    if len(running) >= G or (limit is not None and started >= limit):
                        break
                    q = pending[host]
                    job = q[0]
                    s = limiter.hosts[host]
                    wait = max(job.not_before, s.backoff_until) - now
                    if wait > cfg.glob["max_wait"]:
                        stats["deferred"] += len(q)
                        del pending[host]
                        continue
                    budget = job.rule.max_total_bytes
                    if budget is not None and used[job.rule.name] >= budget:
                        stats["over-budget"] += 1
                        q.popleft()
                        if not q:
                            del pending[host]
                        continue
                    if wait > 0 or not limiter.try_acquire(host, job.rule):
                        continue
                    q.popleft()
                    if not q:
                        del pending[host]
                    started += 1
                    running[ex.submit(fetch, job, cfg, limiter, robots, stop)] = job
            if pending:
                pending.move_to_end(next(iter(pending)))  # round-robin hosts
            if limit is not None and started >= limit and not running:
                break
            if not running:
                time.sleep(0.2)  # every host is pacing / backing off
            done, _ = concurrent.futures.wait(
                running, timeout=0.2, return_when=concurrent.futures.FIRST_COMPLETED)
            for fut in done:
                job = running.pop(fut)
                try:
                    res = fut.result()
                except Exception as e:  # a bug, not a network condition
                    res = Result("transient", job.host, error=f"internal: {e!r}"[:300])
                key = record(con, job, res, cfg, limiter, requeue)
                stats[key] += 1
                if key == "done":
                    used[job.rule.name] += res.size or 0
            now = time.time()
            if now - last_commit > 5:
                robots.flush(con)
                limiter.flush(con)
                con.commit()
                last_commit = now
            if now - last_report > 15:
                el = now - t0
                tally = ", ".join(f"{k} {v}" for k, v in stats.most_common())
                print(f"  {sum(stats.values())}/{total} handled in {el:.0f}s"
                      + (f" ({tally})" if tally else "")
                      + f"; {sum(len(q) for q in pending.values())} pending, "
                      f"{len(running)} in flight", flush=True)
                last_report = now
    except KeyboardInterrupt:
        print("\ninterrupted: finishing in-flight downloads' bookkeeping ...", flush=True)
        stop.set()
    finally:
        stop.set()
        for fut, job in list(running.items()):
            try:
                res = fut.result()
                stats[record(con, job, res, cfg, limiter, lambda j: None)] += 1
            except Exception:
                pass
        ex.shutdown(wait=True, cancel_futures=True)
        robots.flush(con)
        limiter.flush(con)
        con.commit()
    print(f"fetch: {started} started; " + ", ".join(
        f"{k} {v}" for k, v in stats.most_common()), flush=True)


# ------------------------------------------------------------- export

def read_map(path: pathlib.Path) -> dict[str, tuple[str, str]]:
    out = {}
    if path.exists():
        for line in path.read_text(encoding="utf-8").splitlines():
            cols = line.split("\t")
            if len(cols) >= 3:
                out[cols[0]] = (cols[1], cols[2])
    return out


def export(con, cfg: Config):
    old = read_map(MAP_TSV)
    new = {}
    for row in con.execute(
            "SELECT url, kind, sha256 FROM downloads d WHERE status = 'done' "
            "AND EXISTS (SELECT 1 FROM download_refs r WHERE r.url = d.url)"):
        rule = cfg.match(row["url"])
        if rule is None or rule.disable or row["kind"] not in rule.accept:
            continue  # downloaded under a broader config: not published now
        if "\t" in row["url"] or "\n" in row["url"]:
            continue
        stored = f"files/{row['sha256'][:32]}.{row['kind']}"
        if (ROOT / "build" / stored).exists():
            new[row["url"]] = (stored, row["kind"])
    text = "".join(f"{u}\t{p}\t{k}\n" for u, (p, k) in sorted(new.items()))
    if not MAP_TSV.exists() or MAP_TSV.read_text(encoding="utf-8") != text:
        tmp = MAP_TSV.with_suffix(".tsv.part")
        tmp.write_text(text, encoding="utf-8")
        tmp.replace(MAP_TSV)
    # images are embedded at conversion time; bulk_pages.py's cache key
    # covers each page's entries, so changed images re-convert their pages
    changed = sum(1 for u in set(old) | set(new) if old.get(u) != new.get(u))
    kinds = collections.Counter(k for _, k in new.values())
    print(f"export: {len(new)} files -> {MAP_TSV.relative_to(ROOT)} ("
          + ", ".join(f"{k} {v}" for k, v in sorted(kinds.items())) + ")"
          + (f"; {changed} entries changed" if changed else ""))


def status_report(con, cfg: Config):
    print("rule / status (referenced URLs):")
    rows = con.execute(
        "SELECT COALESCE(rule, '-') AS rule, status, count(*) AS n, "
        "sum(CASE WHEN status = 'done' THEN size ELSE 0 END) AS bytes "
        "FROM downloads d WHERE EXISTS (SELECT 1 FROM download_refs r "
        "WHERE r.url = d.url) GROUP BY 1, 2 ORDER BY 1, 3 DESC").fetchall()
    order = {r.name: i for i, r in enumerate(cfg.rules)}
    for row in sorted(rows, key=lambda r: (order.get(r["rule"], 1e9), -r["n"])):
        extra = f"  {row['bytes'] / 1e6:.1f} MB" if row["bytes"] else ""
        print(f"  {row['rule']:24} {row['status']:12} {row['n']:7}{extra}")


# --------------------------------------------------------------- main

def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("--config", action="append", type=pathlib.Path,
                    help=f"rules file (repeatable; default {DEFAULT_CONFIG.relative_to(ROOT)})")
    ap.add_argument("--status", action="store_true",
                    help="scan + evaluate, print counts per rule/status, no network")
    ap.add_argument("--no-fetch", action="store_true",
                    help="scan, evaluate and export only")
    ap.add_argument("--limit", type=int, help="start at most N downloads this run")
    ap.add_argument("--rule", action="append", default=[],
                    help="only download for these rule names (repeatable)")
    ap.add_argument("--retry", action="store_true",
                    help="requeue missing/refused/rejected/too-large/blocked/failed")
    ap.add_argument("--recheck", action="store_true",
                    help="revalidate downloaded files (If-None-Match/If-Modified-Since)")
    args = ap.parse_args()

    cfg = Config(args.config or [DEFAULT_CONFIG])
    unknown = set(args.rule) - set(cfg.by_name)
    if unknown:
        raise SystemExit(f"error: no rules named {sorted(unknown)}")

    STATE_DB.parent.mkdir(parents=True, exist_ok=True)
    lock = open(STATE_DB.with_suffix(".db.lock"), "w")
    try:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
    except BlockingIOError:
        raise SystemExit("error: another downloads.py is running")
    shutil.rmtree(TMP, ignore_errors=True)

    def on_term(signum, frame):  # `kill` stops as gracefully as Ctrl-C
        raise KeyboardInterrupt
    signal.signal(signal.SIGTERM, on_term)

    con = connect()
    print("config: " + " + ".join(str(p.relative_to(ROOT)) if p.is_relative_to(ROOT)
                                  else str(p) for p in cfg.files)
          + f" ({len(cfg.rules)} rules)")
    scan(con)
    evaluate(con, cfg, args.retry)
    if args.status:
        status_report(con, cfg)
        return
    if not args.no_fetch:
        jobs = load_jobs(con, cfg, set(args.rule), args.recheck)
        if jobs:
            run(con, cfg, jobs, args.limit)
        else:
            print("nothing to download")
    export(con, cfg)


if __name__ == "__main__":
    sys.exit(main())
