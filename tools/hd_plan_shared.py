"""hd_plan、hd_ref、hd_cut 共用：角色描述與輪廓重建。"""
import numpy as np

ROLE_DESC = {
    "enemy": "enemy craft or creature sprites; pieces in one group are animation frames or growth stages of the same subject",
    "boss_part": "parts of a large biomechanical boss machine",
    "bullet": "weapon projectiles and energy shots",
    "explosion": "explosion and fireball animation frames",
    "player": "the player's small futuristic fighter craft and its attachments",
    "item": "glowing power-up capsules and carrier objects",
    "ui": "interface illustration",
    "text": "stylized lettering",
    "cutscene": "cinematic illustration",
    "background": "scrolling background scenery for a side-scrolling shooter",
    "effect": "a vertical lightning energy effect",
    "unused": "leftover sprites from the same game",
    "misc": "small assorted sprites from one stage of the game",
}


def silhouette_for(S, p):
    """依 plan 的 piece 欄位重建輪廓：True = 不透明。"""
    px = S.indexed(p["lib"], p["index"])
    if p["opaque"] or p.get("free"):
        return np.ones(px.shape, bool)
    sil = px != 0
    m = p.get("mask")
    if m and m.startswith("item:"):
        mp = S.indexed(p["lib"], int(m[5:]))
        h = min(px.shape[0], mp.shape[0])
        sil[:h] |= mp[:h] == 0
    elif m and m.startswith("mask:"):
        mb = S.mask_bits(p["lib"], int(m[5:]))
        for y in range(px.shape[0]):
            my = y + p.get("dy", 0)
            if 0 <= my < mb.shape[0]:
                sil[y] |= mb[my] == 0
    return sil
