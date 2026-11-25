# Binary Patches Preserved from `Munder Difflin.app` (`0.5.10`)

- **Binary Path:** `/Users/altan/Documents/transition/Munder Difflin.app/Contents/Resources/app.asar`
- **SHA-256:** `355750e44c6b662dd4b79afd136f2f728e46841b8078ee2b5e8e053fc10944c7`
- **Patched File:** `out/renderer/assets/index--nBMM_aP.js` (61 changed blocks vs. pre-patch extraction)

## Patch 1: Pro View Compact Density (60 Blocks)

All Pro sidebar/rail UI rows (`ProSidebar`, `AgentRow`, `ProjectRows`, `SectionLabel`, `Count`, `TeammateRow`, `Strip`, Cloud/Sandbox status rows) were scaled down by `1 / 1.4`:

| Metric | Unpatched (`px`) | Patched (`px`) |
| --- | ---: | ---: |
| Primary agent / teammate name | `18.2` | `13` |
| Project / input text | `17.5` | `12.5` |
| Action row / search / task title | `16.8` | `12` |
| Empty / cloud status note | `16.1` | `11.5` |
| Strip / status / badge / count | `15.4` | `11` |
| Metadata / ticket ID / section label | `14.7` | `10.5` |
| Status chip font | `14` | `10` |
| Sandbox / small chip font | `13.3` | `9.5` |
| Orchestrator chip font | `12.6` | `9` |
| Row / chip heights | `25` / `24` / `22` / `21` | `18` / `17` / `16` / `15` |

## Patch 2: Enlarged Main Office Area (`homeBoxOf`)

In `homeBoxOf(i8)` when `i8.kind === "floor"`:

- **Before (unpatched):**
  ```js
  const w2 = most;
  const h2 = w2 / aspect;
  const x2 = (homeW - w2) / 2, y2 = Math.max(0, (stageH - h2) / 2);
  ```
  where `most = Math.max(0, Math.min(homeW * HOME_INPUT_SHARE, room * FLOOR_ASPECT_MIN))` (`HOME_INPUT_SHARE = 0.9`, reserving bottom receipt/gap space).
- **After (patched):**
  ```js
  const w2 = Math.floor(Math.max(0, Math.min(homeW, stageH * aspect)) * 100) / 100;
  const h2 = Math.floor(Math.max(0, Math.min(stageH, w2 / aspect)) * 100) / 100;
  const x2 = Math.max(0, (homeW - w2) / 2), y2 = Math.max(0, (stageH - h2) / 2);
  ```
  Expands the office floor canvas to the full available `homeW` × `stageH` viewport while preserving `boxAspectFor(i8.inner)`.
