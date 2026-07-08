# Task 5 Report: Render The Settings Lifecycle UI

## Summary

Implemented the fallback model lifecycle UI in Settings and routed the setup-opened experience through the same typed `FallbackModelView` state.

## What Changed

- Replaced the old `Model files` and `Storage` system rows with one compact `Local fallback` lifecycle row.
- Added a shared lifecycle projection in `desktop/src/components/panels/app-sheets.tsx` to drive:
  - status text
  - primary action
  - secondary actions
  - live/pending disablement
- Wired the full action matrix:
  - `missing`: Install, Open folder
  - `downloading`: Cancel, Open folder
  - `verifying`: Open folder
  - `ready`: Reinstall, Verify, Disable, Remove, Open folder
  - `corrupted`: Repair, Remove, Open folder
  - `disabled`: Enable, Remove, Open folder
  - `error`: Retry, Remove, Open folder
- Added `AlertDialog` confirmation for Remove with the required warning that local live fallback will be unavailable until reinstalled.
- Kept `Open folder` available across lifecycle states.
- Rewired `App.tsx` to pass real lifecycle handlers instead of placeholder `void ...` references.
- Moved setup auto-open behavior to typed fallback lifecycle application so the setup prompt follows `FallbackModelView` instead of the older coarse setup snapshot.
- Updated the setup-opened settings flow to land on the `System` section whenever fallback setup is still needed.

## Files Changed

- `desktop/src/components/panels/app-sheets.tsx`
- `desktop/src/App.tsx`
- `desktop/tests/unit/app-types.test.ts`

## Verification

Ran:

```powershell
cd .\desktop
pnpm test -- app-types
pnpm build
```

Results:

- `pnpm test -- app-types`: passed
- `pnpm build`: passed

## Commit

- `8ef7590` `Render fallback settings lifecycle UI`

## Notes

- I did not modify `desktop/src/components/ui/alert-dialog.tsx`; the existing component was sufficient.
- There was an unrelated pre-existing modified file in the worktree: `.superpowers/sdd/task-2-report.md`. I left it untouched.
