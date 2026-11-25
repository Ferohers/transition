import { c_ as portraitPlacement, de as recolorBody } from "./ui-CyUfM8WT.js";
import "./index-ChdtDDPz.js";
function agentAvatar(body, pictures) {
  return (body.headshot ? pictures?.get(body.headshot) : void 0) ?? (body.avatar ? pictures?.get(body.avatar) : void 0) ?? null;
}
const columns = /* @__PURE__ */ new Map();
const MAX = 200;
function lookColumn(looks, body, colors) {
  const key = [looks.themeId, body.id, body.column, colors.skin ?? "", colors.shirt ?? "", colors.hair ?? ""].join("|");
  const hit = columns.get(key);
  if (hit) {
    columns.delete(key);
    columns.set(key, hit);
    return hit;
  }
  const fw = looks.sprite.frameWidth;
  const px = recolorBody(looks.sheet, body, colors, { x0: body.column * fw, fw, fh: looks.sprite.frameHeight }, looks.builder ?? {});
  const cv = document.createElement("canvas");
  cv.width = px.width;
  cv.height = px.height;
  cv.getContext("2d").putImageData(new ImageData(new Uint8ClampedArray(px.data), px.width, px.height), 0, 0);
  columns.set(key, cv);
  if (columns.size > MAX) columns.delete(columns.keys().next().value);
  return cv;
}
function paintPackPortrait(ctx, looks, plan) {
  const avatar = agentAvatar(plan.body, looks.avatars);
  if (avatar) {
    ctx.imageSmoothingEnabled = !!(plan.body.headshot && looks.avatars?.has(plan.body.headshot));
    const cv = document.createElement("canvas");
    cv.width = avatar.width;
    cv.height = avatar.height;
    cv.getContext("2d").putImageData(new ImageData(new Uint8ClampedArray(avatar.data), avatar.width, avatar.height), 0, 0);
    const { dx: dx2, dy: dy2, dw: dw2, dh: dh2 } = portraitPlacement([0, 0, avatar.width, avatar.height], ctx.canvas.width, ctx.canvas.height);
    ctx.drawImage(cv, dx2, dy2, dw2, dh2);
    return;
  }
  const col = lookColumn(looks, plan.body, plan.colors);
  const [cx, cy, cw, ch] = plan.crop;
  const { dx, dy, dw, dh } = portraitPlacement(plan.crop, ctx.canvas.width, ctx.canvas.height);
  ctx.imageSmoothingEnabled = false;
  ctx.drawImage(col, cx, plan.sy + cy, cw, ch, dx, dy, dw, dh);
}
export {
  lookColumn,
  paintPackPortrait
};
