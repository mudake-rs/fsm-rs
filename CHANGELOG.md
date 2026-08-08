# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

### Added

- Hierarchical (nested) states: composite states with mandatory `*` initial
  children, composite states as transition sources (expand to all descendant
  leaves) and targets (resolve to the initial leaf), child-first dispatch via
  specificity ordering, and UML LCA entry/exit ordering.
  See the "Hierarchical states" section of the README.

### Changed

- **Breaking:** within a transition, the action now runs *after* the source
  exit hooks and before the target entry hooks (UML order). Previously the
  action ran before the exit hook. Only machines that use both an action and
  a source `exit` hook on the same transition are affected.

## [0.1.0] - 2026-08-08

### Added

- `state_machine!` table DSL: guards, actions, `entry`/`exit` hooks,
  `on_transition`, `unhandled` policies, wildcards and `|` patterns.
- Compile-time exhaustiveness over (state × event) pairs, plus compile errors
  for unknown states/events, unreachable rows and multiple initial states.
- Async callbacks via per-callback `async` markers (mixed sync/async allowed).
- Optional `serde` persistence (`serde: true` + the `serde` feature).
