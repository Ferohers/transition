import { n as reactExports, l as jsxRuntimeExports } from "./index-ChdtDDPz.js";
import { U as StationFloor } from "./StationFloor-C8oG0qID.js";
import { r as ownPackOf } from "./themeCatalog-CVKmJPh3.js";
import { s as previewDraws } from "./index--nBMM_aP.js";
import "./ui-CyUfM8WT.js";
import "./lookArt-BobBg7Xf.js";
import "./global-DYy7uepk.js";
import "./glyph-CD_xZXfA.js";
import "./puck-ac2UtXrw.js";
import "./portraitArt-B3eZ2mvW.js";
function WorkbenchPreview({ jobId, rev, onDrawn }) {
  const [pack, setPack] = reactExports.useState(null);
  reactExports.useEffect(() => {
    let live = true;
    void (async () => {
      const answer = await window.cth.themeWorkbenchPreview(jobId);
      if (!live || !previewDraws(answer) || !answer.ok || answer.empty === true) return;
      const { pack: next } = await ownPackOf(`workbench:${jobId}`, answer);
      if (live && next) setPack(next);
    })().catch(() => {
    });
    return () => {
      live = false;
    };
  }, [jobId, rev]);
  reactExports.useEffect(() => {
    onDrawn(!!pack);
  }, [pack, onDrawn]);
  return pack ? /* @__PURE__ */ jsxRuntimeExports.jsx(StationFloor, { pack }) : null;
}
export {
  WorkbenchPreview as default
};
