# 18 Final design judgment

Design revision 0.1 — 2026-10-05. Proposed architecture; implementation and production qualification are pending. Numbered research references resolve in [Sources](sources.md).

The project is feasible if portability is defined as shared semantics and certified host contracts. The strongest part of the original idea is the pure explicit-state reducer. The hard part is reliable coordination around effects, not converting a graph to Wasm. Concentrating that work in a small auditable kernel and semantic store contract gives the project a defensible scope.

The result can support both ordinary application workflows and agent-driven work without treating every node as an LLM call. WIT is useful for stable interfaces and community implementations. It should sit alongside executable behavioral specifications and fault tests, because types alone cannot guarantee recovery, authorization, or progress.
