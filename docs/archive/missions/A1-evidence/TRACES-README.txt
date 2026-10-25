The two raw lifecycle traces are stored gzipped:

  pre-b21-trace.jsonl.gz    (22.6 MB raw)
  post-b21-trace.jsonl.gz   (22.5 MB raw)

Gunzip before running analyze_lifecycle.py, which expects the .jsonl names.
Compressed by the coordinator at merge: 48 MB -> 8 MB for the packet. The
findings doc references them by their uncompressed names.

Not included: A1-evidence/handoff/ (the shadow-repo bundle and patches Sol
produced because codex cannot lock a linked worktree's git index). Its two
commits are in main's history directly, so the bundle is redundant.
