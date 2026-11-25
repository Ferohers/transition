import { n as reactExports, l as jsxRuntimeExports, t as useTranslation, i as instance, a as createRoot } from "./index-ChdtDDPz.js";
import { n as subscribeIdeUnsaved, g as ideUnsaved, I as IDE_Z_INDEX, q as useDirectionSync, e as applyOfficeFloorTheme, b as IdePanel, c as adoptAppTheme, l as startWindowControls } from "./global-DYy7uepk.js";
import { aj as Notice, e as Btn, eb as useStore, bK as floorOfficeTheme, b8 as adoptAppSkin } from "./ui-CyUfM8WT.js";
const EDITOR_OFFICE_START = { owner: null, closeHeld: false };
function editorOfficeStep(s, e) {
  switch (e.type) {
    case "owner":
      return { owner: e.owner, closeHeld: e.owner?.same === true ? false : s.closeHeld };
    case "saveFirst":
      return { ...s, closeHeld: true };
    case "unsaved":
      return e.files === 0 && s.closeHeld ? { ...s, closeHeld: false } : s;
    case "closeAnswer":
      return e.ok ? s : { ...s, closeHeld: false };
  }
}
function editorOfficeNotices(s) {
  const back = s.owner?.same === true;
  return {
    ownerSwitched: s.owner !== null && s.owner.same === false && s.owner.drawnFor === null,
    // The main process holds a close only while the owner shows another office, so the hold itself says so, also before the first answer of `ideOwnerOffice`.
    notClosed: s.closeHeld && !back
  };
}
const FOOT = "--ide-foot";
function EditorOfficeNoticesView({ shown, onCloseWithoutSaving }) {
  const { t } = useTranslation();
  const strip = reactExports.useRef(null);
  const any = shown.ownerSwitched || shown.notClosed;
  reactExports.useEffect(() => {
    const el = strip.current;
    const root2 = document.documentElement;
    if (!el) {
      root2.style.removeProperty(FOOT);
      return;
    }
    const set = () => root2.style.setProperty(FOOT, `${el.offsetHeight}px`);
    set();
    const ro = new ResizeObserver(set);
    ro.observe(el);
    return () => {
      ro.disconnect();
      root2.style.removeProperty(FOOT);
    };
  }, [any]);
  if (!any) return null;
  return /* @__PURE__ */ jsxRuntimeExports.jsxs("div", { ref: strip, "data-editor-office-notices": true, style: { position: "fixed", insetInline: 0, bottom: 0, zIndex: IDE_Z_INDEX, boxSizing: "border-box", display: "flex", flexDirection: "column", gap: 6, padding: 8, borderTop: "1px solid var(--cth-ink-300)", background: "var(--cth-paper-100)", fontFamily: "var(--cth-font-ui)" }, children: [
    shown.notClosed && /* @__PURE__ */ jsxRuntimeExports.jsx("div", { "data-editor-office-notice": "notClosed", children: /* @__PURE__ */ jsxRuntimeExports.jsx(Notice, { tone: "waiting", word: t("pro.office.editor.closeUnsaved.head"), action: /* @__PURE__ */ jsxRuntimeExports.jsx(Btn, { size: "sm", onClick: onCloseWithoutSaving, dataAttrs: { "data-editor-close-without-saving": "" }, children: t("pro.office.editor.closeUnsaved.button") }), children: t("pro.office.editor.closeUnsaved.body") }) }),
    shown.ownerSwitched && /* @__PURE__ */ jsxRuntimeExports.jsx("div", { "data-editor-office-notice": "ownerSwitched", children: /* @__PURE__ */ jsxRuntimeExports.jsx(Notice, { tone: "thinking", word: t("pro.office.editor.ownerSwitched.head", { office: t("pro.office.main") }), children: t("pro.office.editor.ownerSwitched.body") }) })
  ] });
}
function EditorOfficeNotices() {
  const [state, step] = reactExports.useReducer(editorOfficeStep, EDITOR_OFFICE_START);
  reactExports.useEffect(() => {
    let alive = true;
    let pushed = false;
    const offOwner = window.cth.onIdeOwnerOffice?.((owner) => {
      pushed = true;
      step({ type: "owner", owner });
    });
    const offSave = window.cth.onIdeSaveFirst?.(() => step({ type: "saveFirst" }));
    void Promise.resolve(window.cth.ideOwnerOffice?.()).then((owner) => {
      if (alive && !pushed && owner !== void 0) step({ type: "owner", owner });
    }, () => void 0);
    const offUnsaved = subscribeIdeUnsaved(() => step({ type: "unsaved", files: ideUnsaved() }));
    return () => {
      alive = false;
      offOwner?.();
      offSave?.();
      offUnsaved();
    };
  }, []);
  const closeWithoutSaving = () => {
    void Promise.resolve(window.cth.ideCloseWithoutSaving?.()).then((r) => {
      if (r) step({ type: "closeAnswer", ok: r.ok === true });
    }, () => void 0);
  };
  return /* @__PURE__ */ jsxRuntimeExports.jsx(EditorOfficeNoticesView, { shown: editorOfficeNotices(state), onCloseWithoutSaving: closeWithoutSaving });
}
function IdeWindowRoot() {
  useDirectionSync();
  const [ready, setReady] = reactExports.useState(false);
  reactExports.useEffect(() => {
    let alive = true;
    const s = useStore.getState();
    s.setIdeDetached(true);
    void window.cth.ideHandoff().then(({ handoff, agents }) => {
      if (!alive) return;
      const st = useStore.getState();
      if (agents.length > 0) st.mirrorAgents(agents);
      st.setIdeHandoff(handoff);
      const byRoot = handoff?.root ? useStore.getState().agents.find((a) => a.cwd === handoff.root)?.id ?? null : null;
      st.setIdeOpen(true, handoff?.agentId ?? byRoot);
      setReady(true);
    }, () => {
      if (!alive) return;
      useStore.getState().setIdeOpen(true);
      setReady(true);
    });
    return () => {
      alive = false;
    };
  }, []);
  reactExports.useEffect(() => window.cth.onIdeOpen((req) => {
    const st = useStore.getState();
    if (req.path) st.openFileInIde(req.path, req.location, req.root ?? void 0);
    else if (req.root) st.openProjectInIde(req.root);
    else if (req.agentId) st.setIdeOpen(true, req.agentId);
  }), []);
  reactExports.useEffect(() => window.cth.onIdeAgents((list) => useStore.getState().mirrorAgents(list)), []);
  reactExports.useEffect(() => {
    void window.cth.getConfig().then((c) => {
      useStore.getState().setAvatars(c.avatars ?? []);
      if (!applyOfficeFloorTheme()) useStore.getState().setOfficeTheme(floorOfficeTheme(c, c.harnessHome));
    });
    return window.cth.onConfigChanged((c) => {
      useStore.getState().setAvatars(c.avatars ?? []);
      if (!applyOfficeFloorTheme()) useStore.getState().setOfficeTheme(floorOfficeTheme(c, c.harnessHome));
    });
  }, []);
  reactExports.useEffect(() => {
    const onStorage = (e) => {
      if (e.key === "cth.theme" && (e.newValue === "light" || e.newValue === "dark")) adoptAppTheme(e.newValue);
      else if (e.key === "cth.skin" && (e.newValue === "office" || e.newValue === "professional")) adoptAppSkin(e.newValue);
      else if (e.key === "cth.language" && e.newValue) void instance.changeLanguage(e.newValue);
    };
    window.addEventListener("storage", onStorage);
    return () => window.removeEventListener("storage", onStorage);
  }, []);
  if (!ready) return /* @__PURE__ */ jsxRuntimeExports.jsx("div", { style: { width: "100vw", height: "100vh", background: "var(--cth-paper-100)" } });
  return /* @__PURE__ */ jsxRuntimeExports.jsxs(jsxRuntimeExports.Fragment, { children: [
    /* @__PURE__ */ jsxRuntimeExports.jsx(IdePanel, {}),
    /* @__PURE__ */ jsxRuntimeExports.jsx(EditorOfficeNotices, {})
  ] });
}
const root = document.getElementById("root");
if (!root) throw new Error("No root element");
startWindowControls();
createRoot(root).render(
  /* @__PURE__ */ jsxRuntimeExports.jsx(reactExports.StrictMode, { children: /* @__PURE__ */ jsxRuntimeExports.jsx(IdeWindowRoot, {}) })
);
