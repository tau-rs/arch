# Recorded GitHub responses

Bodies the tests replay through `Recorded` (no network). Recorded with `gh api` on 2026-10-04
unless marked *crafted*.

| file | source |
|---|---|
| `repo.json` | `GET /repos/tau-rs/arch` |
| `pulls-merged.json` | `GET /repos/tau-rs/arch/pulls?head=tau-rs:LEBOCQTitouan/facts-key-59-recompute&state=all` (PR #70, merged) |
| `pulls-none.json` | the same call for a branch with no PR |
| `pull-created.json` | `GET /repos/tau-rs/arch/pulls/72` (a draft PR: the body `POST /pulls` answers with) |
| `check-runs-passed.json` | `GET /repos/tau-rs/arch/commits/9232ce4/check-runs` |
| `check-runs-failed.json` | the same for `9069e5d` (`check` failed) |
| `check-runs-pending.json` | *crafted*: `check-runs-passed.json` with its first run `in_progress` |
| `status-none.json` | `GET /repos/tau-rs/arch/commits/9232ce4/status`: no statuses, yet `state` is `pending` |
| `status-failed.json` | *crafted*: `status-none.json` with one failed status from an external CI |
| `requested-reviewers.json`, `reviews.json` | rust-lang/cargo PR 17245: one reviewer re-requested after commenting |
| `requested-reviewers-none.json`, `reviews-mixed.json` | rust-lang/rust-analyzer PR 23252: approved, changes requested, commented |
| `merge-ok.json` | *crafted* from the GitHub REST docs (`PUT /pulls/{n}/merge`, 200) |
| `merge-405.json` | *crafted* from the GitHub REST docs (405, the strategy is not allowed) |
