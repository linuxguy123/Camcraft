#!/usr/bin/env python3
"""Archive a public GitHub repository's issues and pull request discussions.

Requires an authenticated `gh` CLI. Writes public API responses to a local
directory; it does not alter either GitHub repository.
"""

from __future__ import annotations

import argparse
import csv
import json
import subprocess
import sys
import tempfile
import time
from datetime import datetime, timezone
from pathlib import Path


def call(args: list[str], *, cwd: Path | None = None) -> str:
    for attempt in range(3):
        result = subprocess.run(args, text=True, capture_output=True, check=False, cwd=cwd)
        if result.returncode == 0:
            return result.stdout
        if attempt == 2:
            raise RuntimeError(f"{' '.join(args)}: {result.stderr.strip()}")
        time.sleep(2**attempt)
    raise AssertionError("unreachable")


def api(endpoint: str) -> object:
    return json.loads(call(["gh", "api", endpoint]))


def api_list(endpoint: str) -> list[dict]:
    pages = json.loads(call(["gh", "api", "--paginate", "--slurp", endpoint]))
    return [item for page in pages for item in page]


def save_json(path: Path, value: object) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, indent=2, ensure_ascii=False) + "\n")


def git_patch(pull: dict, folder: Path) -> None:
    """Recover a PR patch when GitHub's diff endpoint hits its 300-file limit."""
    base_sha = pull["base"]["sha"]
    head_sha = pull["head"]["sha"]
    base_url = pull["base"]["repo"]["clone_url"]
    head_repo = pull["head"].get("repo")
    head_url = head_repo["clone_url"] if head_repo else base_url
    with tempfile.TemporaryDirectory(prefix="camcraft-pr-patch-") as temp:
        directory = Path(temp)
        call(["git", "init", "--quiet"], cwd=directory)
        call(["git", "fetch", "--no-tags", base_url, base_sha], cwd=directory)
        try:
            call(["git", "fetch", "--no-tags", head_url, head_sha], cwd=directory)
        except RuntimeError:
            call(["git", "fetch", "--no-tags", base_url,
                  f"refs/pull/{pull['number']}/head"], cwd=directory)
            head_sha = call(["git", "rev-parse", "FETCH_HEAD"], cwd=directory).strip()
        comparison = f"{base_sha}...{head_sha}"
        patch = call(["git", "diff", "--binary", comparison], cwd=directory)
        if not patch:
            raise RuntimeError("Git returned an empty PR patch")
        (folder / "changes.patch").write_text(patch)
        (folder / "files_all.tsv").write_text(
            call(["git", "diff", "--name-status", comparison], cwd=directory)
        )
        save_json(folder / "patch_source.json", {
            "method": "git diff --binary",
            "base_sha": base_sha,
            "head_sha": head_sha,
            "head_source": head_url,
        })


def csv_index(path: Path, records: list[dict]) -> None:
    with path.open("w", newline="") as stream:
        writer = csv.writer(stream)
        writer.writerow(["number", "state", "title", "author", "created_at", "updated_at", "url"])
        for item in records:
            writer.writerow([
                item["number"], item["state"], item["title"],
                (item.get("user") or {}).get("login", ""),
                item.get("created_at", ""), item.get("updated_at", ""),
                item["html_url"],
            ])


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("repository", help="owner/name")
    parser.add_argument("output", type=Path)
    parser.add_argument(
        "--baseline", type=Path, action="append", default=[],
        help="archive directory whose issue and PR numbers should be skipped (repeatable)",
    )
    parser.add_argument(
        "--baseline-root", type=Path,
        help="include every completed archive directly under this directory",
    )
    parser.add_argument(
        "--include-updates", action="store_true",
        help="also capture older records updated since the latest archive began",
    )
    parser.add_argument(
        "--skip-empty", action="store_true",
        help="leave no output directory when no issues or PRs changed",
    )
    args = parser.parse_args()
    repo = args.repository
    out = args.output
    baselines = list(args.baseline)
    if args.baseline_root:
        baselines.extend(
            path for path in sorted(args.baseline_root.iterdir())
            if path.is_dir() and (path / "manifest.json").is_file()
            and not json.loads((path / "manifest.json").read_text()).get("errors")
        )
    baselines = list(dict.fromkeys(baselines))
    if args.include_updates and not baselines:
        parser.error("--include-updates requires at least one baseline archive")
    latest_started = max(
        (datetime.fromisoformat(json.loads((path / "manifest.json").read_text())["started_utc"])
         for path in baselines),
        default=None,
    )

    errors: list[str] = []
    started = datetime.now(timezone.utc).isoformat()
    meta = api(f"repos/{repo}")
    issues = [item for item in api_list(f"repos/{repo}/issues?state=all&per_page=100") if "pull_request" not in item]
    pulls = api_list(f"repos/{repo}/pulls?state=all&per_page=100")
    if baselines:
        old_issues = {
            item["number"] for baseline in baselines
            for item in json.loads((baseline / "issues.json").read_text())
        }
        old_pulls = {
            item["number"] for baseline in baselines
            for item in json.loads((baseline / "pulls.json").read_text())
        }
        issues = [
            item for item in issues
            if item["number"] not in old_issues
            or (args.include_updates and
                datetime.fromisoformat(item["updated_at"].replace("Z", "+00:00")) >= latest_started)
        ]
        pulls = [
            item for item in pulls
            if item["number"] not in old_pulls
            or (args.include_updates and
                datetime.fromisoformat(item["updated_at"].replace("Z", "+00:00")) >= latest_started)
        ]
    if args.skip_empty and not issues and not pulls:
        print("No new or updated issues or pull requests", flush=True)
        return 0
    out.mkdir(parents=True, exist_ok=True)
    labels = api_list(f"repos/{repo}/labels?per_page=100")
    milestones = api_list(f"repos/{repo}/milestones?state=all&per_page=100")
    save_json(out / "repository.json", meta)
    save_json(out / "labels.json", labels)
    save_json(out / "milestones.json", milestones)
    save_json(out / "issues.json", issues)
    save_json(out / "pulls.json", pulls)
    csv_index(out / "issues.csv", issues)
    csv_index(out / "pulls.csv", pulls)
    print(f"Found {len(issues)} issues and {len(pulls)} pull requests", flush=True)

    for index, issue in enumerate(issues, 1):
        number = issue["number"]
        folder = out / "issues" / str(number)
        save_json(folder / "issue.json", issue)
        if issue.get("comments", 0) and not (folder / "comments.json").exists():
            try:
                save_json(folder / "comments.json", api_list(f"repos/{repo}/issues/{number}/comments?per_page=100"))
            except Exception as error:
                errors.append(f"issue {number} comments: {error}")
        if index % 25 == 0:
            print(f"Issues: {index}/{len(issues)}", flush=True)

    for index, pull_summary in enumerate(pulls, 1):
        number = pull_summary["number"]
        folder = out / "pulls" / str(number)
        try:
            pull = api(f"repos/{repo}/pulls/{number}")
        except Exception as error:
            errors.append(f"PR {number} detail: {error}")
            pull = pull_summary
        save_json(folder / "pull.json", pull)
        if pull.get("comments", 0) and not (folder / "issue_comments.json").exists():
            try:
                save_json(folder / "issue_comments.json", api_list(f"repos/{repo}/issues/{number}/comments?per_page=100"))
            except Exception as error:
                errors.append(f"PR {number} discussion: {error}")
        if not (folder / "reviews.json").exists():
            try:
                save_json(folder / "reviews.json", api_list(f"repos/{repo}/pulls/{number}/reviews?per_page=100"))
            except Exception as error:
                errors.append(f"PR {number} reviews: {error}")
        if pull.get("review_comments", 0) and not (folder / "review_comments.json").exists():
            try:
                save_json(folder / "review_comments.json", api_list(f"repos/{repo}/pulls/{number}/comments?per_page=100"))
            except Exception as error:
                errors.append(f"PR {number} review comments: {error}")
        if not (folder / "changes.patch").exists():
            try:
                save_json(folder / "commits.json", api_list(f"repos/{repo}/pulls/{number}/commits?per_page=100"))
            except Exception as error:
                errors.append(f"PR {number} commits: {error}")
            try:
                save_json(folder / "files.json", api_list(f"repos/{repo}/pulls/{number}/files?per_page=100"))
            except Exception as error:
                errors.append(f"PR {number} file list: {error}")
            try:
                if pull.get("changed_files", 0) > 300:
                    git_patch(pull, folder)
                else:
                    try:
                        (folder / "changes.patch").write_text(
                            call(["gh", "pr", "diff", str(number), "-R", repo, "--patch"])
                        )
                    except RuntimeError:
                        git_patch(pull, folder)
            except Exception as error:
                errors.append(f"PR {number} patch: {error}")
        if index % 25 == 0:
            print(f"Pull requests: {index}/{len(pulls)}", flush=True)

    summary = {
        "source": repo,
        "source_url": meta["html_url"],
        "baselines": [str(path) for path in baselines],
        "latest_baseline_started_utc": latest_started.isoformat() if latest_started else None,
        "includes_updates": args.include_updates,
        "started_utc": started,
        "finished_utc": datetime.now(timezone.utc).isoformat(),
        "issues": len(issues),
        "open_issues": sum(item["state"] == "open" for item in issues),
        "pull_requests": len(pulls),
        "open_pull_requests": sum(item["state"] == "open" for item in pulls),
        "errors": errors,
    }
    save_json(out / "manifest.json", summary)
    baseline_note = (
        "This incremental archive contains records absent from the baseline "
        "archives listed in `manifest.json`, and records updated since the latest "
        "baseline began. Earlier versions remain in those archives.\n\n"
        if baselines and args.include_updates else
        "This incremental archive contains records absent from the baseline "
        "archives listed in `manifest.json`. Earlier records remain there.\n\n"
        if baselines else ""
    )
    (out / "README.md").write_text(
        f"# {repo} GitHub metadata archive\n\n"
        f"Captured {summary['finished_utc']} from GitHub's public API.\n\n"
        f"{baseline_note}"
        f"- {summary['issues']} issues ({summary['open_issues']} open)\n"
        f"- {summary['pull_requests']} pull requests ({summary['open_pull_requests']} open)\n"
        f"- {len(errors)} export errors (details in `manifest.json`)\n\n"
        "`issues.csv` and `pulls.csv` are indexes. The JSON files retain the API's "
        "original authors, timestamps, labels, links, and text. PR folders "
        "also contain commit and file lists and a patch. Attached files remain "
        "external links; this archive does not download them. This is a snapshot, "
        "not a live mirror.\n"
    )
    print(json.dumps(summary, indent=2), flush=True)
    return 0 if not errors else 1


if __name__ == "__main__":
    sys.exit(main())
