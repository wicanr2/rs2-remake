/* 用 Nuked OPL3 把暫存器序列合成成參考 PCM，只當黑箱比對。
 * 編譯時由 tools/audio/nuked_ref.sh 引入 DOSBox-X 原始碼裡的 nukedopl.cpp，那份程式碼不進這個 repo。
 *
 * 輸入（stdin，文字）：每行「取樣序 暫存器 值」，十進位，依序排好。第一行是總取樣數。
 * 輸出（stdout）：單聲道 16 位元小端 PCM，49716 Hz。OPL3 輸出是立體聲，OPL2 模式左右相同，只取左聲道。
 */
#include <stdio.h>
#include <stdint.h>
#include <stdlib.h>
#include NUKED_C
int main(void) {
    static opl3_chip chip;
    unsigned long total, at, cur = 0;
    int reg, val;
    int16_t buf[4];
    OPL3_Reset(&chip, 49716);
    if (scanf("%lu", &total) != 1) return 2;
    int have = scanf("%lu %d %d", &at, &reg, &val) == 3;
    for (cur = 0; cur < total; cur++) {
        while (have && at <= cur) {
            OPL3_WriteReg(&chip, (uint16_t)reg, (uint8_t)val);
            have = scanf("%lu %d %d", &at, &reg, &val) == 3;
        }
        OPL3_Generate(&chip, buf);
        fwrite(buf, 2, 1, stdout);
    }
    return 0;
}
