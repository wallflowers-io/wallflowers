#!/usr/bin/env python3
"""no-mess — every branch and worktree in the workspace is an open PR, or it does not exist.

Sixty-three worktrees and 78 unmerged branches were left across product, website and
egregore by 26 Oct 2026: sessions branched, worked, and ended. This makes it a check
that ends turns, not a rule a session can forget.

  no-mess.py              every repository's mess, listed
  no-mess.py tidy [repo]  remove what is finished: clean worktrees and branches whose PR
                          merged or closed (merged into the default branch, off GitHub)
  no-mess.py start        SessionStart hook: the mess, into the session's context
  no-mess.py stop         Stop hook: refuses to end a turn on mess this session made

A session made an item if one of its commands or edits names the item's path or branch.
Mess left by others is reported, never blocked on: it may be another session's work in
progress. The Stop hook blocks once; if the mess is still there, it says so to Ralph.
Any failure of the check itself lets the turn end.
"""
import json, os, re, subprocess, sys
from pathlib import Path

WS = Path.home() / "wallflowers"
WT = WS / "wt"
PUBLIC = WS / "wallflowers_public"


def git(repo, *args):
    r = subprocess.run(["git", "-C", str(repo), *args], capture_output=True, text=True, timeout=20)
    return r.stdout.strip() if r.returncode == 0 else None


def repos():
    found = []
    for root, dirs, _ in os.walk(WS):
        p = Path(root)
        if (p / ".git").is_dir():
            found.append(p)
        dirs[:] = [d for d in dirs if d not in (".git", "node_modules", "target", "wt", ".venv")
                   and len(p.relative_to(WS).parts) < 3]
    return found


def github_slug(repo):
    url = git(repo, "remote", "get-url", "origin") or ""
    m = re.search(r"github\.com[:/](.+?)(?:\.git)?$", url)
    return m.group(1) if m else None


def prs(slug):
    r = subprocess.run(["gh", "pr", "list", "-R", slug, "--state", "all", "--limit", "300",
                        "--json", "headRefName,state,number"], capture_output=True, text=True, timeout=30)
    if r.returncode != 0:
        raise RuntimeError("gh: " + r.stderr.strip())
    by = {}
    for p in json.loads(r.stdout):  # newest first; an open PR wins over older ones
        cur = by.get(p["headRefName"])
        if cur is None or (p["state"] == "OPEN" and cur["state"] != "OPEN"):
            by[p["headRefName"]] = p
    return by


def worktrees(repo):
    out, cur = [], {}
    for line in (git(repo, "worktree", "list", "--porcelain") or "").splitlines() + [""]:
        if not line:
            if cur:
                out.append(cur)
            cur = {}
            continue
        k, _, v = line.partition(" ")
        cur[k] = v or True
    return out


def survey(repo):
    """[(path or None, branch or None, problem)] for one repository."""
    default = (git(repo, "symbolic-ref", "--short", "refs/remotes/origin/HEAD") or "origin/main").split("/", 1)[1]
    slug = github_slug(repo)
    pulls = prs(slug) if slug else {}
    items, held = [], set()
    wts = worktrees(repo)
    for i, w in enumerate(wts):
        path = w["worktree"]
        branch = w["branch"].removeprefix("refs/heads/") if "branch" in w else None
        held.add(branch)
        if i == 0:
            if branch != default:
                items.append((path, branch, f"main checkout not on {default}"))
            elif git(path, "status", "--porcelain"):
                items.append((path, branch, "main checkout has uncommitted changes"))
            continue
        if "prunable" in w:
            items.append((path, branch, "worktree's folder is gone: git worktree prune"))
            continue
        if not Path(path).resolve().is_relative_to(WT.resolve()):
            items.append((path, branch, f"worktree outside {WT}"))
        if branch is None:
            items.append((path, None, "detached HEAD"))
        if git(path, "status", "--porcelain"):
            items.append((path, branch, "uncommitted changes"))
    for branch in (git(repo, "for-each-ref", "--format=%(refname:short)", "refs/heads") or "").splitlines():
        if branch == default:
            continue
        path = next((w["worktree"] for w in wts[1:] if w.get("branch") == "refs/heads/" + branch), None)
        p = branch_problem(repo, branch, default, slug, pulls)
        if p:
            items.append((path, branch, p))
    return items


def branch_problem(repo, branch, default, slug, pulls):
    if slug:
        pr = pulls.get(branch)
        if pr is None:
            return "no PR: open one (a draft will do), or delete the branch"
        if pr["state"] != "OPEN":
            return f"PR #{pr['number']} {pr['state'].lower()}: remove worktree and branch"
        if git(repo, "rev-parse", branch) != git(repo, "rev-parse", f"origin/{branch}"):
            return f"PR #{pr['number']}: local commits not pushed"
        return None
    if git(repo, "merge-base", "--is-ancestor", branch, default) is not None:
        return f"merged into {default}: remove worktree and branch"
    return f"not merged into {default}"


def everything():
    return {r: survey(r) for r in repos()}


def name(repo):
    return repo.relative_to(WS) if repo.is_relative_to(WS) else repo


def fmt(repo, items):
    rel = name(repo)
    return [f"  {rel}: {b or '(detached)'}{'  ' + p if p else ''} — {why}" for p, b, why in items]


def report(mess, detail=lambda r: True):
    lines = []
    for repo, items in mess.items():
        if not items:
            continue
        if detail(repo):
            lines += fmt(repo, items)
        else:
            lines.append(f"  {name(repo)}: {len(items)} item{'s' * (len(items) != 1)} (tools/no-mess.py lists them)")
    return lines


def session_text(transcript):
    """Every command this session ran, as one string; every file it wrote."""
    parts, writes = [], []
    try:
        with open(transcript) as f:
            for line in f:
                try:
                    d = json.loads(line)
                except ValueError:
                    continue
                if d.get("type") != "assistant":
                    continue
                for c in d.get("message", {}).get("content", []):
                    if isinstance(c, dict) and c.get("type") == "tool_use":
                        inp = c.get("input", {})
                        parts.append(str(inp.get("command", "")))
                        writes += [str(inp[k]) for k in ("file_path", "notebook_path") if k in inp]
    except OSError:
        pass
    return "\n".join(parts + writes), writes


def made_here(seen, path, branch, main_checkout):
    text, writes = seen
    if main_checkout:  # shared: only a session that wrote there made it dirty
        return any(Path(w).is_relative_to(path) and not Path(w).is_relative_to(WT) for w in writes)
    if path and path in text:
        return True
    return bool(branch) and re.search(r"(?<![\w/.-])" + re.escape(branch) + r"(?![\w/.-])", text) is not None


def tidy(repo):
    default = (git(repo, "symbolic-ref", "--short", "refs/remotes/origin/HEAD") or "origin/main").split("/", 1)[1]
    git(repo, "fetch", "-q", "--prune", "origin")
    git(repo, "worktree", "prune")
    for path, branch, why in survey(repo):
        if not branch or not ("remove worktree and branch" in why):
            continue
        if path and git(path, "status", "--porcelain"):
            print(f"  kept {path}: uncommitted changes")
            continue
        if path and git(repo, "worktree", "remove", path) is None:
            print(f"  kept {path}: git worktree remove refused")
            continue
        if git(repo, "branch", "-D", branch) is not None:
            print(f"  removed {branch}{'  ' + path if path else ''}")
    left = survey(repo)
    print("\n".join(fmt(repo, left)) if left else f"  {name(repo)}: clean")


def main():
    cmd = sys.argv[1] if len(sys.argv) > 1 else "list"
    if cmd == "list":
        lines = report(everything())
        print("\n".join(lines) if lines else "  no mess")
        return 1 if lines else 0
    if cmd == "tidy":
        given = sys.argv[2] if len(sys.argv) > 2 else PUBLIC
        tidy(Path(git(given, "rev-parse", "--path-format=absolute", "--git-common-dir")).parent)
        return 0
    hook = json.load(sys.stdin)
    if cmd == "start":
        lines = report(everything(), detail=lambda r: r == PUBLIC)
        if lines:
            print("Unfinished branches and worktrees (tools/no-mess.py; an open PR is finished enough):")
            print("\n".join(lines))
        return 0
    if cmd == "stop":
        seen = session_text(hook.get("transcript_path", ""))
        mine = {r: [i for i in items if made_here(seen, i[0], i[1], i[2].startswith("main checkout"))]
                for r, items in everything().items()}
        lines = report(mine)
        if not lines:
            return 0
        if hook.get("stop_hook_active"):
            print(json.dumps({"systemMessage": "no-mess: this session leaves\n" + "\n".join(lines)}))
            return 0
        print("This session made branches or worktrees that are not open PRs. Finish them before "
              "ending the turn (~/.claude/CLAUDE.md, PR HYGINE):\n" + "\n".join(lines), file=sys.stderr)
        return 2
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except Exception as e:  # the check failing never traps a session
        print(f"no-mess: check failed, not enforced: {e}", file=sys.stderr)
        sys.exit(0)
