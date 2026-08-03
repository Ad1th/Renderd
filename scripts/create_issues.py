#!/usr/bin/env python3
"""
Script to parse ISSUES-0001-milestones.md and create 100 GitHub issues on Ad1th/renderd.

Usage:
  # Dry-run mode (parses and previews issues without posting):
  python3 scripts/create_issues.py --dry-run

  # Post issues using GitHub CLI (gh):
  python3 scripts/create_issues.py --use-gh

  # Post issues using GitHub REST API token:
  python3 scripts/create_issues.py --token YOUR_GITHUB_TOKEN
"""

import sys
import os
import re
import json
import time
import argparse
import subprocess
import urllib.request
import urllib.error

REPO = "Ad1th/renderd"
DOC_PATH = os.path.join(os.path.dirname(__file__), "..", "docs", "ISSUES-0001-milestones.md")

def parse_issues(doc_path):
    with open(doc_path, "r", encoding="utf-8") as f:
        content = f.read()

    milestone_blocks = re.split(r'## (Milestone \d+: [^\n]+)', content)
    
    issues = []
    
    for i in range(1, len(milestone_blocks), 2):
        milestone_title = milestone_blocks[i].strip()
        block_text = milestone_blocks[i+1]
        
        issue_pattern = re.compile(
            r'### Issue #(\d+):\s*([^\n]+)\n'
            r'(.*?)(?=(?:### Issue #|\Z))',
            re.DOTALL
        )

        for match in issue_pattern.finditer(block_text):
            num = match.group(1).strip()
            title = match.group(2).strip()
            body_text = match.group(3).strip()

            rationale_m = re.search(r'-\s*\*\*Rationale:\*\*\s*(.*?)(?=\n-\s*\*\*|\Z)', body_text, re.DOTALL)
            deps_m = re.search(r'-\s*\*\*Dependencies:\*\*\s*(.*?)(?=\n-\s*\*\*|\Z)', body_text, re.DOTALL)
            criteria_m = re.search(r'-\s*\*\*Acceptance Criteria:\*\*\s*(.*?)(?=\n-\s*\*\*|\Z)', body_text, re.DOTALL)
            testing_m = re.search(r'-\s*\*\*Testing:\*\*\s*(.*?)(?=\n-\s*\*\*|\Z)', body_text, re.DOTALL)
            effort_m = re.search(r'-\s*\*\*Estimated Effort:\*\*\s*(.*?)(?=\n-\s*\*\*|\Z)', body_text, re.DOTALL)

            rationale = rationale_m.group(1).strip() if rationale_m else ""
            deps = deps_m.group(1).strip() if deps_m else "None"
            criteria = criteria_m.group(1).strip() if criteria_m else ""
            testing = testing_m.group(1).strip() if testing_m else ""
            effort = effort_m.group(1).strip() if effort_m else "1 day"

            formatted_title = f"[{milestone_title.split(':')[0]}] Issue #{num}: {title}"

            formatted_body = f"""## Issue #{num}: {title}

### Rationale
{rationale}

### Dependencies
{deps}

### Acceptance Criteria
{criteria}

### Testing
{testing}

---
**Milestone:** {milestone_title}  
**Estimated Effort:** {effort}
"""

            issues.append({
                "number": int(num),
                "title": formatted_title,
                "milestone": milestone_title,
                "body": formatted_body,
                "effort": effort,
            })

    return issues

def create_issue_gh(issue):
    cmd = [
        "gh", "issue", "create",
        "--repo", REPO,
        "--title", issue["title"],
        "--body", issue["body"],
    ]
    try:
        res = subprocess.run(cmd, capture_output=True, text=True, check=True)
        print(f"✅ Created #{issue['number']}: {res.stdout.strip()}")
        return True
    except subprocess.CalledProcessError as e:
        print(f"❌ Failed to create #{issue['number']}: {e.stderr.strip()}")
        return False

def create_issue_api(issue, token):
    url = f"https://api.github.com/repos/{REPO}/issues"
    headers = {
        "Authorization": f"Bearer {token}",
        "Accept": "application/vnd.github+json",
        "User-Agent": "Renderd-Issue-Creator",
    }
    payload = {
        "title": issue["title"],
        "body": issue["body"],
    }
    data = json.dumps(payload).encode("utf-8")
    req = urllib.request.Request(url, data=data, headers=headers, method="POST")
    try:
        with urllib.request.urlopen(req) as resp:
            res_json = json.loads(resp.read().decode("utf-8"))
            print(f"✅ Created #{issue['number']}: {res_json.get('html_url')}")
            return True
    except urllib.error.HTTPError as e:
        err_body = e.read().decode("utf-8")
        print(f"❌ Failed to create #{issue['number']}: HTTP {e.code} - {err_body}")
        return False

def main():
    parser = argparse.ArgumentParser(description="Create GitHub issues for Renderd milestone roadmap.")
    parser.add_argument("--dry-run", action="store_true", help="Parse and preview issues without sending API requests.")
    parser.add_argument("--use-gh", action="store_true", help="Use `gh` CLI tool to create issues.")
    parser.add_argument("--token", type=str, help="GitHub Personal Access Token for REST API.")
    args = parser.parse_args()

    issues = parse_issues(DOC_PATH)
    print(f"Parsed {len(issues)} issues from {DOC_PATH}\n")

    use_gh = getattr(args, "use_gh", False)
    dry_run = getattr(args, "dry_run", False)

    if dry_run or (not use_gh and not args.token):
        print("=== DRY RUN MODE ===")
        print(f"Sample Issue #1:\nTitle: {issues[0]['title']}\nMilestone: {issues[0]['milestone']}\nBody:\n{issues[0]['body']}\n")
        print(f"Sample Issue #100:\nTitle: {issues[-1]['title']}\nMilestone: {issues[-1]['milestone']}\nBody:\n{issues[-1]['body']}\n")
        print("Run with `--use-gh` or `--token YOUR_TOKEN` to post to GitHub.")
        return

    print(f"Creating {len(issues)} issues on {REPO}...")
    success_count = 0

    for issue in issues:
        if use_gh:
            ok = create_issue_gh(issue)
        elif args.token:
            ok = create_issue_api(issue, args.token)
        
        if ok:
            success_count += 1
        time.sleep(1.5)  # Pause to avoid GitHub secondary rate limits

    print(f"\nFinished! Successfully created {success_count}/{len(issues)} issues.")

if __name__ == "__main__":
    main()
