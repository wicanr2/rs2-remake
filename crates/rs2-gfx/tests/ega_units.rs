//! EGA 色表：6 位元 rgbRGB 的位元 2、1、0 是紅、綠、藍（2/3 亮度），位元 5、4、3 是紅、綠、藍（1/3 亮度），
//! 每個通道 `2 × 主 + 副` 個 55h。標準 16 色是開機時 `sub_1007D` 載入的 `00～07,38h～3Fh`。

use rs2_core::DrawCmd;
use rs2_gfx::{ATTR_REGS, Assets, VgaSink, ega_rgb16};

#[test]
fn standard_sixteen_colors() {
    let c = ega_rgb16(&ATTR_REGS[..16].try_into().unwrap());
    let want: [[u8; 3]; 16] = [
        [0, 0, 0],
        [0, 0, 170],
        [0, 170, 0],
        [0, 170, 170],
        [170, 0, 0],
        [170, 0, 170],
        [170, 170, 0],
        [170, 170, 170],
        [85, 85, 85],
        [85, 85, 255],
        [85, 255, 85],
        [85, 255, 255],
        [255, 85, 85],
        [255, 85, 255],
        [255, 255, 85],
        [255, 255, 255],
    ];
    // 屬性值 06 是 EGA 的「棕」之前的暗黃（170,170,0）；VGA 的 BIOS 把色號 6 改成 14h（棕）。
    assert_eq!(c, want);
    // 06h 的 rgbRGB 是紅加綠的主色，沒有副色，所以是暗黃；14h 是紅主色加綠副色，是棕（170,85,0）。
    assert_eq!(ega_rgb16(&[0x14; 16])[0], [170, 85, 0]);
    // 這個遊戲第一關的屬性表第 1 項 2Ch：紅主色加紅副色得 255，藍副色得 85。
    assert_eq!(ega_rgb16(&[0x2C; 16])[0], [255, 0, 85]);
}

#[test]
fn sink_starts_with_the_standard_palette_and_follows_ega_attr() {
    let mut s = VgaSink::new(Assets::empty());
    assert_eq!(s.ega_regs(), <[u8; 16]>::try_from(&ATTR_REGS[..16]).unwrap());
    let mut regs = [0u8; 17];
    regs[3] = 0x13;
    s.apply(&DrawCmd::EgaAttr { regs });
    assert_eq!(s.ega_regs()[3], 0x13);
    // 色號 3 在平面 0、1 的位元是 1：畫面第一個像素用 EGA 色表。
    let mut ega = vec![0u8; 320 * 200 * 3];
    let mut vga = vec![0u8; 320 * 200 * 3];
    s.render_page_ega(0, &mut ega);
    s.render_page(0, &mut vga);
    assert_eq!(&ega[..3], &ega_rgb16(&s.ega_regs())[0], "色號 0 的像素");
    assert_eq!(&vga[..3], &[0, 0, 0], "VGA 的 DAC 全 0，不受 EgaAttr 影響");
}
