//! HD 圖的調色盤換色。原版換 DAC 會連已畫好的像素一起變色，HD 畫布是 RGBA，要自己換。
//!
//! 方法接近 `tools/hd_variants.py`：32×32×32 查表，每格依最近的 3 個來源色加權，把來源色到目的色的差量帶過去。
//! 這裡保留低 3 位元（輸出 = 輸入 + 差量），漸層不會變成 32 階；權重比較陡，只有幾個色號改色時不會染到別的顏色。

/// 16 色表，8 位元 RGB。
pub type Colors = [[u8; 3]; 16];

/// 兩張色表有幾個色號不同。
pub fn changed(a: &Colors, b: &Colors) -> usize {
    a.iter().zip(b).filter(|(x, y)| x != y).count()
}

/// 差量查表。
pub struct Lut {
    delta: Vec<[i16; 3]>,
}

impl Lut {
    pub fn build(from: &Colors, to: &Colors) -> Lut {
        let mut delta = Vec::with_capacity(32 * 32 * 32);
        let node = |i: usize| (4 + i * 8) as f32;
        for r in 0..32 {
            for g in 0..32 {
                for b in 0..32 {
                    let c = [node(r), node(g), node(b)];
                    // 最近的 3 個來源色。
                    let mut best = [(f32::MAX, 0usize); 3];
                    for (i, p) in from.iter().enumerate() {
                        let d = (0..3).map(|k| (c[k] - p[k] as f32).powi(2)).sum::<f32>().sqrt() + 1e-3;
                        if d < best[2].0 {
                            best[2] = (d, i);
                            if best[2].0 < best[1].0 {
                                best.swap(1, 2);
                            }
                            if best[1].0 < best[0].0 {
                                best.swap(0, 1);
                            }
                        }
                    }
                    let (mut sum, mut out) = (0f32, [0f32; 3]);
                    for (d, i) in best {
                        // 四次方：沒有改色的色號附近不會被改色的色號拖走。
                        let w = 1.0 / (d * d * d * d);
                        sum += w;
                        for k in 0..3 {
                            out[k] += w * (to[i][k] as f32 - from[i][k] as f32);
                        }
                    }
                    delta.push([(out[0] / sum) as i16, (out[1] / sum) as i16, (out[2] / sum) as i16]);
                }
            }
        }
        Lut { delta }
    }

    /// 就地換色。透明像素不動。
    pub fn apply_rgba(&self, px: &mut [u8]) {
        for p in px.chunks_exact_mut(4) {
            if p[3] == 0 {
                continue;
            }
            let i = ((p[0] as usize >> 3) << 10) | ((p[1] as usize >> 3) << 5) | (p[2] as usize >> 3);
            let d = self.delta[i];
            for k in 0..3 {
                p[k] = (p[k] as i16 + d[k]).clamp(0, 255) as u8;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn colors(f: impl Fn(usize) -> [u8; 3]) -> Colors {
        std::array::from_fn(f)
    }

    #[test]
    fn palette_entries_map_to_their_targets() {
        let a = colors(|i| [(i * 16) as u8, 255 - (i * 16) as u8, 40]);
        let b = colors(|i| [40, (i * 16) as u8, 255 - (i * 16) as u8]);
        let lut = Lut::build(&a, &b);
        for i in 0..16 {
            let mut p = [a[i][0], a[i][1], a[i][2], 255];
            lut.apply_rgba(&mut p);
            for k in 0..3 {
                assert!((p[k] as i32 - b[i][k] as i32).abs() <= 24, "色 {i} 通道 {k}：{} 與 {}", p[k], b[i][k]);
            }
        }
    }

    #[test]
    fn identical_palettes_change_nothing() {
        let a = colors(|i| [(i * 16) as u8, 0, 255 - (i * 16) as u8]);
        let lut = Lut::build(&a, &a);
        let mut p = [17, 99, 201, 255];
        lut.apply_rgba(&mut p);
        assert_eq!(p, [17, 99, 201, 255]);
    }

    #[test]
    fn transparent_pixels_stay() {
        let a = colors(|i| [i as u8 * 10, 0, 0]);
        let b = colors(|i| [0, i as u8 * 10, 0]);
        let mut p = [50, 60, 70, 0];
        Lut::build(&a, &b).apply_rgba(&mut p);
        assert_eq!(p, [50, 60, 70, 0]);
    }
}
