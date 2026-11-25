# Code theme libraries

The libraries a code theme can load from `mdtheme://lib/<name>` inside the locked box
(code themes plan version 2, sections 1 and 6). Main serves only the names listed in
`src/main/codeTheme/themeLibs.ts`, and checks each file's SHA-256 against that list. Until the
box's `mdtheme://lib/` host reads that list, it serves this folder through `boxFileFor`
(src/main/codeTheme/box.ts, boxRules.ts): only the file types a theme may hold, nothing outside it.
Adding a library is a reviewed PR: the file, its licence beside it, and a row in that list.

| Served as | File | From | Licence |
|---|---|---|---|
| `mdtheme://lib/three.module.js` | `three.module.js` | npm `three@0.170.0`, `build/three.module.min.js`, renamed | MIT, `three.LICENSE.txt` |
| `mdtheme://lib/three.LICENSE.txt` | `three.LICENSE.txt` | npm `three@0.170.0`, `LICENSE` | |

three@0.170.0 was taken from `https://registry.npmjs.org/three/-/three-0.170.0.tgz`, whose
sha512 matched the registry's integrity
(`FQK+LEpYc0fBD+J8g6oSEyyNzjp+Q7Ks1C568WWaoMRLW+TkNNWmenWeGgJjV105Gd+p/2ql1ZcjYvNiPZBhuQ==`).
The module build is one file with no imports.

No art of the app's ever goes in this folder: the box serves what is here to any theme,
including one a friend sent.
