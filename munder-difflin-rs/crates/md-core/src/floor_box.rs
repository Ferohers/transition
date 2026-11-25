//! Pure geometry port of the patched `homeBoxOf` / `boxAspectFor` calculations
//! preserved from `Munder Difflin.app` (`0.5.10`).

use serde::{Deserialize, Serialize};

pub const OFFICE_ART_W: f64 = 560.0;
pub const OFFICE_ART_H: f64 = 364.0;
pub const OFFICE_FLOOR_W: f64 = 512.0;
pub const OFFICE_FLOOR_H: f64 = 324.0;
pub const OFFICE_EDGE_LINE: f64 = 2.0;
pub const OFFICE_EDGE_BAND: f64 = 6.0;
pub const HOME_INPUT_SHARE: f64 = 0.9;
pub const HOME_FLOOR_GAP: f64 = 16.0;
pub const HOME_RECEIPT_ROW: f64 = 50.0;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum HomeStageKind {
    Floor,
    Minimal,
    Standard,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HomeBoxInput {
    pub kind: HomeStageKind,
    pub home_w: f64,
    pub stage_h: f64,
    pub inner: f64,
    pub receipt: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct HomeBox {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
    pub input_w: f64,
}

fn round2(n: f64) -> f64 {
    (n * 100.0).round() / 100.0
}

fn floor2(n: f64) -> f64 {
    (n * 100.0).floor() / 100.0
}

pub fn box_aspect_for(inner: f64) -> f64 {
    let edge = 2.0 * OFFICE_EDGE_LINE + OFFICE_EDGE_BAND;
    let inner_wall_ratio = (OFFICE_ART_W - 2.0 * edge) / (OFFICE_ART_H - 2.0 * edge);
    if inner <= inner_wall_ratio {
        OFFICE_ART_W / ((OFFICE_ART_W - 2.0 * edge) / inner + 2.0 * edge)
    } else {
        ((OFFICE_ART_H - 2.0 * edge) * inner + 2.0 * edge) / OFFICE_ART_H
    }
}

/// Computes the home stage rectangle.
///
/// For `HomeStageKind::Floor`, this matches Patch 2 from the shipped binary:
/// expanding the office floor to fill the available `home_w` x `stage_h`
/// viewport while preserving `box_aspect_for(inner)`.
pub fn home_box_of(input: HomeBoxInput) -> HomeBox {
    let home_w = input.home_w.max(0.0);
    let stage_h = input.stage_h.max(0.0);
    let room = (stage_h
        + if input.receipt { HOME_RECEIPT_ROW } else { 0.0 }
        - HOME_RECEIPT_ROW
        - 2.0 * HOME_FLOOR_GAP)
        .max(0.0);
    let floor_aspect_min = box_aspect_for(OFFICE_FLOOR_W / OFFICE_FLOOR_H);
    let most = (home_w * HOME_INPUT_SHARE)
        .min(room * floor_aspect_min)
        .max(0.0);

    match input.kind {
        HomeStageKind::Floor => {
            let aspect = box_aspect_for(input.inner);
            let w2 = floor2((home_w.min(stage_h * aspect)).max(0.0));
            let h2 = floor2((stage_h.min(if aspect > 0.0 { w2 / aspect } else { 0.0 })).max(0.0));
            let x2 = ((home_w - w2) / 2.0).max(0.0);
            let y2 = ((stage_h - h2) / 2.0).max(0.0);
            HomeBox {
                x: round2(x2),
                y: round2(y2),
                w: round2(w2),
                h: round2(h2),
                input_w: round2(w2),
            }
        }
        HomeStageKind::Minimal => {
            let side = 16.0;
            let top = 6.0;
            let bottom = 4.0;
            let x2 = (home_w - most) / 2.0;
            let h2 = (stage_h - top - bottom).max(0.0);
            HomeBox {
                x: round2(x2 - side),
                y: top,
                w: round2(most + 2.0 * side),
                h: round2(h2),
                input_w: round2(most),
            }
        }
        HomeStageKind::Standard => {
            let x = (home_w - most) / 2.0;
            let h = (stage_h - 2.0 * HOME_FLOOR_GAP).max(0.0);
            HomeBox {
                x: round2(x),
                y: HOME_FLOOR_GAP,
                w: round2(most),
                h: round2(h),
                input_w: round2(most),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn floor_fills_viewport_preserving_aspect() {
        let inner = OFFICE_FLOOR_W / OFFICE_FLOOR_H;
        let out = home_box_of(HomeBoxInput {
            kind: HomeStageKind::Floor,
            home_w: 1400.0,
            stage_h: 900.0,
            inner,
            receipt: false,
        });
        assert!((out.h - 900.0).abs() <= 0.02);
        assert!(out.w > 1300.0 && out.w <= 1400.0);
        assert!(out.x >= 0.0);
        assert_eq!(out.y, 0.0);
    }
}
