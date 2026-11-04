# IDAPython：對解包後 S.EXE 匯出函式、字串 xref、中斷呼叫與全檔反組譯。輸出路徑由第一個參數指定。
import json, os, sys, hashlib
import idautils, idc, ida_auto, ida_funcs, ida_pro, ida_segment, ida_bytes, ida_ua, ida_nalt, ida_loader

out_dir = idc.ARGV[1] if len(idc.ARGV) > 1 else "/work"
ida_auto.auto_wait()

# 從原始 entry 開始補做函式：near call 目標全部建函式
for seg in idautils.Segments():
    ea = idc.get_segm_start(seg)
    end = idc.get_segm_end(seg)
    while ea < end:
        if ida_bytes.is_code(ida_bytes.get_flags(ea)):
            insn = ida_ua.insn_t()
            if ida_ua.decode_insn(insn, ea) and idc.print_insn_mnem(ea) == "call":
                t = idc.get_operand_value(ea, 0)
                if t and ida_bytes.is_code(ida_bytes.get_flags(t)) and not ida_funcs.get_func(t):
                    ida_funcs.add_func(t)
        ea = idc.next_head(ea, end)
ida_auto.auto_wait()

funcs = []
for f in idautils.Functions():
    fo = ida_funcs.get_func(f)
    funcs.append({
        "ea": f, "seg": idc.get_segm_name(f), "name": ida_funcs.get_func_name(f),
        "size": fo.end_ea - fo.start_ea,
        "callers": sorted(set(ida_funcs.get_func(x).start_ea if ida_funcs.get_func(x) else x for x in idautils.CodeRefsTo(f, 0))),
    })
segs = [{"name": idc.get_segm_name(s), "start": s, "end": idc.get_segm_end(s),
         "sel": ida_segment.getseg(s).sel, "para": ida_segment.get_segm_para(ida_segment.getseg(s))} for s in idautils.Segments()]
ints = []
for seg in idautils.Segments():
    ea = idc.get_segm_start(seg); end = idc.get_segm_end(seg)
    while ea < end:
        if ida_bytes.is_code(ida_bytes.get_flags(ea)) and idc.print_insn_mnem(ea) == "int":
            ints.append({"ea": ea, "text": idc.generate_disasm_line(ea, 0), "func": ida_funcs.get_func_name(ea)})
        ea = idc.next_head(ea, end)
inp = ida_nalt.get_input_file_path()
res = {"input": inp, "entry": idc.get_inf_attr(idc.INF_START_EA), "segments": segs, "functions": funcs, "ints": ints}
with open(os.path.join(out_dir, "s_export.json"), "w") as fh:
    json.dump(res, fh)
# 全檔反組譯（位址 + 文字）
with open(os.path.join(out_dir, "s_disasm.txt"), "w") as fh:
    for seg in idautils.Segments():
        ea = idc.get_segm_start(seg); end = idc.get_segm_end(seg)
        fh.write(f"; SEG {idc.get_segm_name(seg)} {seg:#x}-{end:#x}\n")
        while ea < end:
            fn = ida_funcs.get_func(ea)
            if fn and fn.start_ea == ea:
                fh.write(f"\n{ea:05X} {ida_funcs.get_func_name(ea)} proc\n")
            lab = idc.get_name(ea)
            if lab and not (fn and fn.start_ea == ea):
                fh.write(f"{ea:05X} {lab}:\n")
            fh.write(f"{ea:05X}  {idc.generate_disasm_line(ea, 0)}\n")
            nxt = idc.next_head(ea, end)
            if nxt == idc.BADADDR or nxt <= ea:
                break
            ea = nxt
ida_loader.save_database(ida_loader.get_path(ida_loader.PATH_TYPE_IDB), 0)
ida_pro.qexit(0)
