#!/usr/bin/env python3
"""
Prefetch all upstream source data using urllib (Python's stdlib) and write
to a cache directory in the exact layout expected by ipbr-rank's --offline
mode. Used as a workaround when the Rust binary's outbound network is
blocked but Python/curl work.

Reads AA_API_KEY, OPENROUTER_API_KEY, HF_TOKEN from the environment
(typically loaded from .env).
"""
import json
import os
import subprocess
import sys
import time
import urllib.request
import urllib.error
from pathlib import Path

CACHE_DIR = Path(sys.argv[1]) if len(sys.argv) > 1 else Path("cache")
CACHE_DIR.mkdir(parents=True, exist_ok=True)

UA = "Mozilla/5.0 (compatible; ipbr-rank-prefetch/1.0)"


def get_json(url, headers=None, max_retries=6):
    delay = 2.0
    for attempt in range(max_retries):
        req = urllib.request.Request(url, headers={"User-Agent": UA, **(headers or {})})
        try:
            with urllib.request.urlopen(req, timeout=60) as r:
                return json.loads(r.read())
        except urllib.error.HTTPError as e:
            if e.code == 429 and attempt < max_retries - 1:
                ra = e.headers.get("Retry-After")
                wait = float(ra) if ra and ra.isdigit() else delay
                print(f"  429 on {url[:80]}... sleeping {wait}s")
                time.sleep(wait)
                delay *= 2
                continue
            raise


def get_json_curl(url, headers=None, max_retries=8):
    """Fallback: use system curl when python urllib gets 429s."""
    delay = 10.0
    for attempt in range(max_retries):
        cmd = ["/usr/bin/curl", "-sS", "-A", UA, "-w", "\n%{http_code}"]
        for k, v in (headers or {}).items():
            cmd += ["-H", f"{k}: {v}"]
        cmd.append(url)
        result = subprocess.run(cmd, capture_output=True, text=True)
        if result.returncode != 0:
            raise RuntimeError(f"curl failed: {result.stderr.strip()}")
        body, _, status_text = result.stdout.rpartition("\n")
        status = int(status_text) if status_text.isdigit() else 0
        if 200 <= status < 300:
            return json.loads(body)
        if status == 429 and attempt < max_retries - 1:
            print(f"  curl 429, sleeping {delay}s")
            time.sleep(delay)
            delay *= 2
            continue
        if status == 501:
            raise RuntimeError(f"curl failed HTTP {status}: {body.strip()}")
        if status >= 500 and attempt < max_retries - 1:
            print(f"  curl HTTP {status}, sleeping {delay}s")
            time.sleep(delay)
            delay *= 2
            continue
        raise RuntimeError(f"curl failed HTTP {status}: {body.strip()}")
    raise RuntimeError("curl: max retries exhausted")


def is_locked_dataset_error(err):
    message = str(err)
    return (
        "LockedDatasetTimeoutError" in message
        or "dataset is currently locked" in message
    )


def get_text(url, headers=None):
    req = urllib.request.Request(url, headers={"User-Agent": UA, **(headers or {})})
    with urllib.request.urlopen(req, timeout=60) as r:
        return r.read().decode("utf-8", errors="replace")


def write_json(key, payload):
    p = CACHE_DIR / f"{key}.json"
    p.write_text(json.dumps(payload, indent=2))
    print(f"  wrote {p} ({p.stat().st_size} bytes)")


def write_html(key, html):
    p = CACHE_DIR / f"{key}.html"
    p.write_text(html)
    print(f"  wrote {p} ({p.stat().st_size} bytes)")


def fetch_swebench():
    print("[swebench]")
    payload = get_json("https://raw.githubusercontent.com/swe-bench/swe-bench.github.io/master/data/leaderboards.json")
    write_json("swebench_leaderboards", payload)


def fetch_openrouter():
    print("[openrouter]")
    headers = {}
    key = os.environ.get("OPENROUTER_API_KEY")
    if key:
        headers["Authorization"] = f"Bearer {key}"
    payload = get_json("https://openrouter.ai/api/v1/models", headers)
    write_json("openrouter_models", payload)


def fetch_artificial_analysis():
    print("[artificial_analysis]")
    key = os.environ.get("AA_API_KEY")
    if not key:
        print("  SKIP: AA_API_KEY not set")
        return
    payload = get_json(
        "https://artificialanalysis.ai/api/v2/data/llms/models",
        headers={"x-api-key": key},
    )
    write_json("artificial_analysis_llms", payload)


def fetch_aistupidlevel():
    print("[aistupidlevel]")
    candidates = [
        "https://aistupidlevel.info/api/dashboard/cached",
        "https://aistupidlevel.info/dashboard/cached",
        "https://aistupidlevel.info/api/dashboard",
    ]
    last_err = None
    for url in candidates:
        try:
            payload = get_json(url)
            print(f"  using {url}")
            write_json("aistupidlevel_dashboard", payload)
            return
        except Exception as e:
            last_err = e
            continue
    raise RuntimeError(f"all aistupidlevel endpoints failed: {last_err}")


def fetch_lmarena():
    print("[lmarena]")
    dataset = "lmarena-ai/leaderboard-dataset"
    configs = ["text", "webdev", "search", "document"]
    headers = {}
    hf = os.environ.get("HF_TOKEN")
    if hf:
        headers["Authorization"] = f"Bearer {hf}"
    out_configs = {}
    for cfg in configs:
        pages = []
        offset = 0
        while True:
            url = f"https://datasets-server.huggingface.co/rows?dataset={dataset}&config={cfg}&split=latest&offset={offset}&length=100"
            try:
                page = get_json_curl(url, headers)
            except Exception as e:
                if offset == 0 and (is_locked_dataset_error(e) or "HTTP 501" in str(e)):
                    fallback_url = f"https://datasets-server.huggingface.co/first-rows?dataset={dataset}&config={cfg}&split=latest"
                    page = get_json_curl(fallback_url, headers)
                    pages.append(page)
                    break
                raise
            time.sleep(5.0)
            rows = page.get("rows") or []
            pages.append(page)
            total = page.get("num_rows_total") or page.get("num_rows") or len(rows)
            if not rows:
                break
            offset += len(rows)
            if offset >= total:
                break
        print(f"  config={cfg} pages={len(pages)}")
        out_configs[cfg] = pages
    payload = {
        "dataset": dataset,
        "split": "latest",
        "configs": out_configs,
    }
    write_json("lmarena_overall", payload)


def fetch_openevals():
    print("[openevals]")
    dataset = "OpenEvals/leaderboard-data"
    config = "default"
    split = "train"
    headers = {}
    hf = os.environ.get("HF_TOKEN")
    if hf:
        headers["Authorization"] = f"Bearer {hf}"
    pages = []
    offset = 0
    while True:
        url = f"https://datasets-server.huggingface.co/rows?dataset={dataset}&config={config}&split={split}&offset={offset}&length=100"
        page = get_json(url, headers)
        rows = page.get("rows") or []
        pages.append(page)
        total = page.get("num_rows_total") or page.get("num_rows") or len(rows)
        if not rows:
            break
        offset += len(rows)
        if offset >= total:
            break
    print(f"  pages={len(pages)}")
    payload = {
        "dataset": dataset,
        "config": config,
        "split": split,
        "pages": pages,
    }
    write_json("openevals_leaderboard", payload)


def fetch_html(key, url):
    print(f"[{key}]")
    write_html(key, get_text(url))


FETCHERS = {
    "swebench": fetch_swebench,
    "openrouter": fetch_openrouter,
    "artificial_analysis": fetch_artificial_analysis,
    "aistupidlevel": fetch_aistupidlevel,
    "lmarena": fetch_lmarena,
    "openevals": fetch_openevals,
    "bfcl": lambda: fetch_html("bfcl", "https://gorilla.cs.berkeley.edu/leaderboard.html"),
    "terminal_bench": lambda: fetch_html("terminal_bench", "https://www.tbench.ai/leaderboard/terminal-bench/2.0"),
    "livecodebench": lambda: fetch_html("livecodebench", "https://livecodebench.github.io/leaderboard.html"),
    "aider_polyglot": lambda: fetch_html("aider_polyglot", "https://aider.chat/docs/leaderboards/"),
}


def main():
    only = os.environ.get("PREFETCH_ONLY")
    failures = []
    for name, fn in FETCHERS.items():
        if only and name not in only.split(","):
            continue
        try:
            fn()
        except Exception as e:
            print(f"  FAIL: {e}")
            failures.append((name, str(e)))
    print()
    print(f"cache dir: {CACHE_DIR.resolve()}")
    if failures:
        print(f"failures: {failures}")
        sys.exit(1)


if __name__ == "__main__":
    main()
