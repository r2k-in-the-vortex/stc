# Feature Development Workflow

Standard workflow for delivering a feature or wishlist target (a `WL-` item) on this
repository. It is derived from the WL-14 delivery (PR #27) and is written for the agent that
picks up the next item.

The owner is the decision authority. An agent may propose, implement, and verify; only the owner
approves plans, resolves open questions, and merges pull requests. Phase acceptance follows the
automatable-gate rule in section 4.

## 1. Start a new branch

- Branch from the parent planning branch, not `main`. For v3.0.0 work the parent is
  `v3.0.0-planning`.
- Use the item id as the branch name, for example `WL-14`.
- Start from a clean working tree. Do not carry uncommitted changes from another item.
- Confirm the base is current before branching (`git fetch origin`).

## 2. Write the phased plan and get owner approval

- Create a temporary working document for the item, for example `WL-14-Plan.md` at the
  repository root. Do not put the working plan in the durable specification tree.
- Split the work into phases. Each phase states its goal, the work, the exit criteria, and the
  evidence it will produce.
- Include the locked scope and the explicit exclusions, so the boundary is testable.
- Present the plan to the owner and wait for approval. Do not start implementation before the
  plan is approved.
- Keep the working plan updated as phases complete; it is deleted at closeout.

## 3. Close owner questions in Phase 0

- Make Phase 0 the question-closing phase: list every open question about scope, semantics,
  contracts, compatibility, or exclusions.
- Ask the owner and record the answers.
- Durable answers belong in the decisions register (`specification/Decisions.md`) with the next
  `D-` number, or as an amendment to an existing decision.
- Amend an existing decision when it is extended or narrowed: keep the original text and add the
  amendment. Mark a decision superseded rather than deleting or rewriting it, and keep it in the
  register with an explicit `Superseded by D-xxx` note.
- Phase 0 is complete when no open question blocks implementation.
- If a question appears later, stop and ask; do not guess on owner-reserved decisions.

## 4. Hand each phase to a subagent

- Hand one phase at a time to a subagent. Give it the plan, the phase goal, the locked scope,
  the constraints, and the definition of done.
- Require the subagent to return the changed files, the commands run, the test results, and any
  remaining questions.
- Review the result before accepting the phase. Record the acceptance in the plan with the date,
  the commit hash, and the CI link.
- Acceptance rule:
  - If every exit gate can be checked automatically (tests, CI, static checks), a green CI run
    plus the phase evidence is sufficient to accept the phase; the agent records the acceptance
    and continues.
  - If the phase includes UI interactions or anything that cannot be fully verified
    automatically, the owner must manually accept the phase before the next phase starts.
  - The owner may request manual acceptance of any phase regardless of the gates.
- Do not start the next phase until the previous phase is accepted and its CI is green.

## 5. Complete each phase with CI

- Every phase ends with pushed commits and a green CI run that covers the changed surface.
- Update the plan status with every phase completion: phase accepted, commit hash, CI run link.
- Do not write new code while CI is still running. Wait for the result before starting the next
  change.
- Before pushing, run the local checks that apply: `python tools/ci/check_docs.py`,
  `markdownlint`, `cargo xtask traceability --check`, `cargo xtask host-audit --check`, and
  `git diff --check`.
- Use `gh workflow run ci.yml --ref <branch>` when a branch needs CI and the push did not
  trigger it.

## 6. Commit hygiene

- Make small, logical commits; keep one concern per commit.
- Use conventional prefixes: `feat:`, `fix:`, `ci:`, `docs:`, `test:`, `refactor:`.
- Explain what changed and why in the commit body when the subject is not enough.
- Never amend or rewrite pushed history. Supersede a commit with a new correcting commit.
- Do not revert an accepted change just to correct its documentation; add a superseding commit
  instead (the WL-14 documentation correction is the precedent).
- Keep the working tree clean before handing off or starting the next phase.

## 7. Pause and ask the owner

Stop and ask the owner when any of these happens:

- a question arises that changes scope, semantics, contracts, decisions, or user-visible
  behavior;
- two consecutive CI runs fail (do not keep retrying);
- a test or CI job fails intermittently, or a failure is dismissed as unrelated to the current
  change. A rerun may distinguish a flake from a deterministic failure, but the flaky result is
  still recorded and reported to the owner for follow-up; it is not ignored because it is
  outside the current item's scope;
- a phase cannot meet its exit criteria as written;
- a base-branch conflict requires a judgment about which side is authoritative.

Do not silently widen scope, delete user work, or rewrite pushed history to make a problem go
away.

## 8. Manual verification before the PR

- Before opening the PR, run the manual walkthrough for the user-visible surface: build from the
  branch, exercise the feature, and capture the evidence (commands, output, logs, screenshots,
  or package checks as applicable).
- Present the evidence to the owner and get approval.
- This gate is always required before the PR, even when every phase gate was automated.
  Phase-level manual acceptance is only required by section 4 for UI interactions or other
  non-automatable work.
- Do not open the PR or mark the item complete before this approval.

## 9. Open the pull request against the parent branch

- Open the PR against the parent planning branch (`v3.0.0-planning`), never against `main`
  unless the owner says otherwise.
- Keep the PR body concrete: summary, what changed, verification evidence, CI links, and any
  exclusions or known limitations.
- If the base advanced, merge the base into the feature branch and resolve conflicts in favor of
  the base's authoritative items; keep the feature's completion records.
- Get the PR CI green and report the run link.
- Leave merging to the owner. Do not merge the PR, delete the branch, or mark the item complete
  before the owner's approval.

## 10. Close out: documentation and branch cleanup

- After the owner approves the manual check, update the planning documents: the target row, the
  target status, the retirement checklist, and the change log in `v3.0.0-plan.md` and
  `FutureWishlist.md`.
- Delete the temporary phase plan from the tree.
- Record the PR number and merge hash in the planning documents.
- After the owner merges, delete the local and remote feature branches and prune stale
  remote-tracking refs.
- Verify the parent branch with a full CI run after the merge record is pushed.
- Leave the working tree clean.

## 11. Coverage expectations

- New code should target 95% line coverage so the change does not run into the CI coverage
  gate.
- The project coverage gate is currently 85%. Do not push if project coverage is below that
  limit.
- Check coverage locally when the change is large enough to move the project number.

## 12. General rules

- No timing threshold or CI timing gate is added unless the owner explicitly approves it.
- Do not dismiss a failing test or CI job as flaky or unrelated and continue silently. Record
  the failure and report it to the owner for follow-up, even when it is outside the current
  item's scope. A rerun may distinguish a flake from a deterministic failure, but a flaky
  result is still reported.
- The target set and scope are locked; adding, removing, or redefining an item requires an
  explicit owner re-plan.
- Investigate unfamiliar files and changes before deleting or overwriting them; they may be
  user work.
- Keep commits, plans, and evidence links accurate; do not claim a gate passed without the run
  to show it.

## Phase checklist

Use this checklist for every phase:

- [ ] The phase goal and exit criteria are in the working plan.
- [ ] The subagent has the plan, scope, constraints, and definition of done.
- [ ] The change is implemented and the relevant tests are added or updated.
- [ ] Local checks pass (docs, lint, traceability, host audit, diff check).
- [ ] The phase commit is pushed and CI is green.
- [ ] The plan status is updated with the acceptance date, commit, and CI link.
- [ ] No new code was written while CI was running.
- [ ] Owner acceptance is obtained when the phase has UI interactions or other non-automatable
      gates.
- [ ] Any owner question is recorded and answered before the next phase starts.
