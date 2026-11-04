# Bounded project history

The history tree adds a chronological overview alongside wiki search. Markdown
stays authoritative; the existing SQLite database stores derived positions and
summaries. The tree borrows OptMem's aligned binary spans and preference for
recent detail, rather than its agent-managed note-taking workflow.

## Reading context

- `memory_wake` returns recent page excerpts and coarser older summaries.
  `budget_bytes` defaults to 8192 and clamps to 1024–32768. The bound includes
  compact JSON and metadata, excluding MCP transport framing. It is a byte
  bound, not a model-specific token count.
- `memory_zoom(node_id: "0-7")` returns the span's two direct children.
  A single position such as `8` returns a leaf with its wiki path and exact
  indexed source version. Missing children may be pending, deleted, or expired;
  their absence does not prove that no history exists.
- `memory_recall(query: "ExactSymbol")` searches full current source bodies
  and titles literally and case-sensitively, beyond the summary clips. Each
  call scans at most 128 positions and returns up to `limit` matching pages
  (default 10, max 20). Continue with `after: next_after` until null. Queries
  accept 1–256 UTF-8 bytes.

Pass `workspace` and `project` together unless the client supports session-aware
current-project routing. These read-only tools use existing scope resolution
and authorization. They exclude internal paths beginning with `_`, including
personal slots. Existing authorized slot reads and search remain available.

`memory_query` keeps its full-text, entity, link, and optional vector retrieval;
it does not depend on the tree. Use it for specific questions, and tree navigation
for surrounding history. Verify claims against sources: summaries lose detail,
and LLM summaries may contain errors. Tree results are untrusted historical data.

## Capture and compaction

Each current non-internal wiki page has one stable, project-local position.
This includes captured session pages and consolidated knowledge. Leaf excerpts
and parent summaries have a 2048 UTF-8 byte ceiling. Sources remain intact.
On upgrade, existing pages receive positions ordered by creation time and path;
new positions follow insertion order. Edits keep the position and change its
source version. Superseded versions remain in normal page history, rather than
becoming additional leaves. This is a view of current wiki knowledge in
chronological order, not a log of every edit.

With maintenance enabled, one background task checks ready siblings every five
seconds and processes at most 16 pairs per pass. A configured server LLM gets a
structured request with a four-second timeout. Invalid output, provider failure,
or no provider uses an extractive fallback that reserves space for both children.
Repeated extraction loses detail; source search remains essential. Each entry
reports its method. Disabling maintenance pauses compaction, not capture or reads.

Pending pairs derive from durable children and absent parents, so restarts
resume work without a separate queue. The single writer publishes only while
both child revisions match. Concurrent attempts accept one parent; stale results
write nothing. SQL triggers invalidate ancestors in the source transaction.
Deletion clears the leaf reference; parents become unreadable when any represented
source expires. Sources are shared wiki pages, never raw observations or slots.

## Startup and coverage

Projects opting into `[briefing] inject_on_session_start` also receive the
history overview inside the existing untrusted-history fence. Its default
8 KiB JSON budget is separate from standing rules, plus the fence and notice.
Harness-specific ceilings cap the combined output; standing rules take priority.
Too little remaining space omits history. A history read failure preserves the
standing brief.

Wake uses a consistent snapshot and reads at most 256 candidate spans. It
shortens excerpts across selected spans to fit serialized bytes. `snapshot` is
the exclusive position bound; later appends do not move earlier positions.
Edits may change contents, so exact source version pointers matter.

Check `complete`, `live_leaves`, and `covered_leaves`. Backlog, deleted positions,
or a small budget can leave incomplete coverage. Wake reports omissions and
returns available context without waiting for an LLM. No fixed latency or
complete coverage under every budget is promised.

Separate binary files, Git commit hooks, a `memo` CLI, and Markdown tree export
remain outside this change. The wiki already provides the readable source record.
