# Active Payroll Period Uniqueness (#578)

At most one payroll period is "active" at a time, and while a period is
active the payroll flow must target it. This document describes the
active-period lifecycle in `contracts/payroll/src/lib.rs` — what it
guarantees, how it is enforced, and how it interacts with the period
freeze guard and the duplicate-period guard.

## Overview

Period labels are free-form symbols supplied by callers, so nothing
prevented two consecutive "open" calls from leaving it ambiguous which
period current payroll work belonged to. A freeze applied to one period
would silently not cover the other, and `get_active_period` would have no
single answer.

The contract now keeps exactly one active period, stored under the
`ActivePeriodKey::Active` storage key:

| Field | Meaning |
|-------|---------|
| *(the stored symbol)* | The active period label, or **absent** when no period is active |

Absence is the pre-#578 default, which is what makes this backward
compatible: see [Backward compatibility](#backward-compatibility).

> **Why not `DataKey`?** `DataKey` is at the Soroban contract-spec ceiling
> of 50 union cases, so it cannot take a 51st variant without dropping a
> storage key that deployed contracts already persist. `ActivePeriodKey` is
> a separate one-variant enum for exactly that reason, which also keeps
> this feature self-contained.

## Lifecycle

| Entrypoint | Effect |
|------------|--------|
| `open_payroll_period(admin, period_label)` | Makes `period_label` the active period. Emits `payroll_period_opened`. |
| `close_payroll_period(admin)` | Clears the active period. Emits `payroll_period_closed`. |
| `get_active_period()` | Returns the active period label, or `None`. |

Both mutations require the company admin, run `require_not_paused`, and
call `admin.require_auth()`.

### Uniqueness validation

`open_payroll_period` rejects rather than silently re-pointing the active
period. Each rejection names the fix, so a caller is never left guessing
which period is active:

| Situation | Panic message |
|-----------|---------------|
| The period is already the active one | `Payroll period is already the active period` |
| A different period is active | `An active payroll period already exists: close it before opening a new one` |
| Closing with nothing active | `No active payroll period to close` |

## Enforcement in the payroll flow

`create_run_draft` checks the active period: while one is open, a draft
whose `period_label` is a different label is rejected with
`A different payroll period is active; close it or draft against the active period`.

This is the only place the active period gates an existing entrypoint.
Amending, describing, finalizing and submitting a draft are untouched —
they already operate on a draft that was created under a known label.

## Backward compatibility

The enforcement is conditional: when **no** period is active, `create_run_draft`
behaves exactly as it did before #578 and accepts any label. The vast
majority of deployments never open a period, so the default path is
unchanged and no migration is required. Adopting the guard is opt-in —
open a period when you want the contract to pin payroll work to it.

## Interaction with existing period features

| Feature | Relationship |
|---------|--------------|
| Period freeze guard (#471) | Independent. Freezing blocks *edits* to a period; the active period is a separate slot. A frozen period can be opened, and closing a period does not unfreeze it. |
| Duplicate-period draft guard (#398) | Independent and additive. `ActiveDraftForPeriod` still limits one *pending draft* per label. Closing the active period does not clear that slot, so re-opening a label resumes against the draft that is still pending. |

Closing clears **only** the active-period slot. Per-period state — drafts,
freezes — is keyed by period label and is deliberately left untouched, so a
later re-open of the same label resumes against the state it already has.

## Event surface

| Event | Topics | Data |
|-------|--------|------|
| `payroll_period_opened` | `("payroll", "payroll_period_opened")` | `(period_label, opened_by)` |
| `payroll_period_closed` | `("payroll", "payroll_period_closed")` | `(period_label, closed_by)` |

Both carry only the period label and an address: **no salary values,
commitments, or employee lists**, consistent with the rest of the
privacy-safe event surface.

## QA coverage

`contracts/payroll/tests/active_period_uniqueness.rs` (15 tests) covers:

- **Success** — open/close roundtrip, close-then-open a different period,
  drafting against the active period, drafting again after the close.
- **Failure** — second period rejected, re-opening the active period
  rejected, closing with nothing active, closing twice, drafting for a
  different period while one is active.
- **Authorization** — open and close each reject a non-admin.
- **Edge cases** — a closed period keeps its existing draft (and its #398
  slot); a frozen period can still be opened and closed; with no period
  active, drafts stay unrestricted (the backward-compatibility guard).

```bash
cargo test -p payroll --test active_period_uniqueness
```
