//! Geometric algorithms for SolidCraft Wood panels (Free Panel 1 -> 2 -> 3 solver, face normal classification,
//! 2D sketch line intersection, inward line offsetting, and quad insetting).

use serde_json::{Value, json};
use crate::WoodHost;

/// Classifies the closest face in `faces_info` to `point` and returns `(normal, axis_name, sign)`.
/// Falls back to `([0.0, 0.0, 1.0], "Z", 1.0)` when no matching face normal is found.
pub fn classify_face_normal(faces_info: &[Value], point: [f64; 3]) -> ([f64; 3], &'static str, f64) {
    let mut best_face: Option<&Value> = None;
    let mut best_dist = f64::INFINITY;

    for f in faces_info {
        let coords = f.get("centroid").or_else(|| f.get("point")).and_then(Value::as_array);
        let Some(c) = coords else { continue };
        let (Some(cx), Some(cy), Some(cz)) = (
            c.first().and_then(Value::as_f64),
            c.get(1).and_then(Value::as_f64),
            c.get(2).and_then(Value::as_f64),
        ) else {
            continue;
        };
        let dx = cx - point[0];
        let dy = cy - point[1];
        let dz = cz - point[2];
        let dist = dx * dx + dy * dy + dz * dz;
        if dist < best_dist {
            best_dist = dist;
            best_face = Some(f);
        }
    }

    if let Some(face) = best_face {
        let n_arr = face.get("plane_normal").or_else(|| face.get("normal")).and_then(Value::as_array);
        if let Some(n_arr) = n_arr
            && let (Some(nx), Some(ny), Some(nz)) = (
                n_arr.first().and_then(Value::as_f64),
                n_arr.get(1).and_then(Value::as_f64),
                n_arr.get(2).and_then(Value::as_f64),
            )
        {
            let normal = [nx, ny, nz];
            let ax_x = nx.abs();
            let ax_y = ny.abs();
            let ax_z = nz.abs();
            if ax_x >= ax_y && ax_x >= ax_z && ax_x > 0.05 {
                let sign = if nx >= 0.0 { 1.0 } else { -1.0 };
                return (normal, "X", sign);
            } else if ax_y >= ax_x && ax_y >= ax_z && ax_y > 0.05 {
                let sign = if ny >= 0.0 { 1.0 } else { -1.0 };
                return (normal, "Y", sign);
            } else if ax_z > 0.05 {
                let sign = if nz >= 0.0 { 1.0 } else { -1.0 };
                return (normal, "Z", sign);
            }
        }
    }

    ([0.0, 0.0, 1.0], "Z", 1.0)
}

/// Fully queries `model.faces` or inspects bounding box to determine the exact outward normal of a picked face.
pub fn resolve_face_normal(
    body_name: &str,
    face_index: u64,
    pick_point: [f64; 3],
    host: &mut dyn WoodHost,
) -> ([f64; 3], &'static str, f64) {
    // 1. Spróbuj pobrać dokładną geometrię ścian przez model.faces
    if let Ok(faces_val) = host.execute("model.faces", json!({"body": body_name})) {
        if let Some(faces_arr) = faces_val.get("faces").and_then(Value::as_array) {
            let matched_face = faces_arr.iter().find(|f| f.get("index").and_then(Value::as_u64) == Some(face_index));
            if let Some(f) = matched_face {
                let norm = f.get("plane_normal").or_else(|| f.get("normal")).and_then(Value::as_array);
                if let Some(n) = norm
                    && let (Some(nx), Some(ny), Some(nz)) = (
                        n.first().and_then(Value::as_f64),
                        n.get(1).and_then(Value::as_f64),
                        n.get(2).and_then(Value::as_f64),
                    )
                {
                    let ax_x = nx.abs();
                    let ax_y = ny.abs();
                    let ax_z = nz.abs();
                    if ax_x >= ax_y && ax_x >= ax_z && ax_x > 0.05 {
                        let sign = if nx >= 0.0 { 1.0 } else { -1.0 };
                        return ([nx, ny, nz], "X", sign);
                    } else if ax_y >= ax_x && ax_y >= ax_z && ax_y > 0.05 {
                        let sign = if ny >= 0.0 { 1.0 } else { -1.0 };
                        return ([nx, ny, nz], "Y", sign);
                    } else if ax_z > 0.05 {
                        let sign = if nz >= 0.0 { 1.0 } else { -1.0 };
                        return ([nx, ny, nz], "Z", sign);
                    }
                }
            }

            // Dopasuj po najbliższym punkcie
            let (n, axis, sign) = classify_face_normal(faces_arr, pick_point);
            if n[0].abs() > 0.05 || n[1].abs() > 0.05 || n[2].abs() > 0.05 {
                return (n, axis, sign);
            }
        }
    }

    // 2. Fallback: sprawdź bounding box bryły przez document.inspect
    if let Ok(doc_info) = host.inspect_document(true) {
        if let Some(bodies) = doc_info.get("bodies").and_then(Value::as_array) {
            if let Some(b) = bodies.iter().find(|b| b.get("name").and_then(Value::as_str) == Some(body_name)) {
                let bbox = b.get("bbox").or_else(|| b.get("bounds"));
                if let Some(bb) = bbox {
                    let min = bb.get("min").and_then(Value::as_array);
                    let max = bb.get("max").and_then(Value::as_array);
                    if let (Some(mn), Some(mx)) = (min, max)
                        && let (Some(x0), Some(y0), Some(z0), Some(x1), Some(y1), Some(z1)) = (
                            mn.first().and_then(Value::as_f64),
                            mn.get(1).and_then(Value::as_f64),
                            mn.get(2).and_then(Value::as_f64),
                            mx.first().and_then(Value::as_f64),
                            mx.get(1).and_then(Value::as_f64),
                            mx.get(2).and_then(Value::as_f64),
                        )
                    {
                        let d_x0 = (pick_point[0] - x0).abs();
                        let d_x1 = (pick_point[0] - x1).abs();
                        let d_y0 = (pick_point[1] - y0).abs();
                        let d_y1 = (pick_point[1] - y1).abs();
                        let d_z0 = (pick_point[2] - z0).abs();
                        let d_z1 = (pick_point[2] - z1).abs();

                        let min_d = d_x0.min(d_x1).min(d_y0).min(d_y1).min(d_z0).min(d_z1);
                        if (min_d - d_z0).abs() < 1e-3 {
                            return ([0.0, 0.0, -1.0], "Z", -1.0);
                        } else if (min_d - d_z1).abs() < 1e-3 {
                            return ([0.0, 0.0, 1.0], "Z", 1.0);
                        } else if (min_d - d_y0).abs() < 1e-3 {
                            return ([0.0, -1.0, 0.0], "Y", -1.0);
                        } else if (min_d - d_y1).abs() < 1e-3 {
                            return ([0.0, 1.0, 0.0], "Y", 1.0);
                        } else if (min_d - d_x0).abs() < 1e-3 {
                            return ([-1.0, 0.0, 0.0], "X", -1.0);
                        } else {
                            return ([1.0, 0.0, 0.0], "X", 1.0);
                        }
                    }
                }
            }
        }
    }

    // 3. Fallback bezpośrednio ze współrzędnych punktu w przestrzeni
    if pick_point[2].abs() < 2.0 {
        ([0.0, 0.0, -1.0], "Z", -1.0)
    } else if pick_point[1].abs() < 2.0 {
        ([0.0, -1.0, 0.0], "Y", -1.0)
    } else if pick_point[0].abs() < 2.0 {
        ([-1.0, 0.0, 0.0], "X", -1.0)
    } else {
        ([0.0, 0.0, 1.0], "Z", 1.0)
    }
}

/// Computes the 2D intersection point of the infinite line through `(p1, p2)` and the infinite line through `(p3, p4)`.
/// Returns `None` if the lines are parallel or nearly parallel.
pub fn line_intersection_2d(p1: [f64; 2], p2: [f64; 2], p3: [f64; 2], p4: [f64; 2]) -> Option<[f64; 2]> {
    let (x1, y1) = (p1[0], p1[1]);
    let (x2, y2) = (p2[0], p2[1]);
    let (x3, y3) = (p3[0], p3[1]);
    let (x4, y4) = (p4[0], p4[1]);

    let denom = (x1 - x2) * (y3 - y4) - (y1 - y2) * (x3 - x4);
    if denom.abs() < 1e-9 {
        return None;
    }

    let det12 = x1 * y2 - y1 * x2;
    let det34 = x3 * y4 - y3 * x4;
    let px = (det12 * (x3 - x4) - (x1 - x2) * det34) / denom;
    let py = (det12 * (y3 - y4) - (y1 - y2) * det34) / denom;
    if px.is_finite() && py.is_finite() {
        Some([px, py])
    } else {
        None
    }
}

/// Offsets the 2D segment `(p1, p2)` perpendicular to its direction toward `toward_pt` by `dist`.
pub fn offset_line_2d(p1: [f64; 2], p2: [f64; 2], toward_pt: [f64; 2], dist: f64) -> ([f64; 2], [f64; 2]) {
    let dx = p2[0] - p1[0];
    let dy = p2[1] - p1[1];
    let length = dx.hypot(dy);
    if length < 1e-9 {
        return (p1, p2);
    }

    let mut nx = -dy / length;
    let mut ny = dx / length;
    let vx = toward_pt[0] - p1[0];
    let vy = toward_pt[1] - p1[1];
    if vx * nx + vy * ny < 0.0 {
        nx = -nx;
        ny = -ny;
    }

    (
        [p1[0] + nx * dist, p1[1] + ny * dist],
        [p2[0] + nx * dist, p2[1] + ny * dist],
    )
}

/// Insets a 4-corner polygon toward its centroid by `ratio` (e.g. `0.15`), rounding to 3 decimals
/// so that subsequent `collinear` / `parallel` constraints snap cleanly without degenerate initial coincidence.
pub fn inset_quad(corners: &[[f64; 2]; 4], ratio: f64) -> [[f64; 2]; 4] {
    let cx = (corners[0][0] + corners[1][0] + corners[2][0] + corners[3][0]) * 0.25;
    let cy = (corners[0][1] + corners[1][1] + corners[2][1] + corners[3][1]) * 0.25;
    let round3 = |v: f64| (v * 1000.0).round() / 1000.0;
    [
        [
            round3(corners[0][0] + (cx - corners[0][0]) * ratio),
            round3(corners[0][1] + (cy - corners[0][1]) * ratio),
        ],
        [
            round3(corners[1][0] + (cx - corners[1][0]) * ratio),
            round3(corners[1][1] + (cy - corners[1][1]) * ratio),
        ],
        [
            round3(corners[2][0] + (cx - corners[2][0]) * ratio),
            round3(corners[2][1] + (cy - corners[2][1]) * ratio),
        ],
        [
            round3(corners[3][0] + (cx - corners[3][0]) * ratio),
            round3(corners[3][1] + (cy - corners[3][1]) * ratio),
        ],
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn line_intersection_finds_orthogonal_crossing() {
        let pt = line_intersection_2d([18.0, 0.0], [18.0, 560.0], [0.0, 20.0], [800.0, 20.0]);
        assert_eq!(pt, Some([18.0, 20.0]));
    }

    #[test]
    fn offset_line_moves_toward_interior_point() {
        let (a, b) = offset_line_2d([0.0, 0.0], [800.0, 0.0], [400.0, 280.0], 100.0);
        assert!((a[1] - 100.0).abs() < 1e-6);
        assert!((b[1] - 100.0).abs() < 1e-6);
    }

    #[test]
    fn classify_normal_picks_nearest_face() {
        let faces = vec![
            json!({"centroid": [400.0, 280.0, 0.0], "normal": [0.0, 0.0, -1.0]}),
            json!({"centroid": [400.0, 280.0, 720.0], "normal": [0.0, 0.0, 1.0]}),
        ];
        let (_, axis, sign) = classify_face_normal(&faces, [400.0, 280.0, 719.0]);
        assert_eq!(axis, "Z");
        assert_eq!(sign, 1.0);
    }
}
