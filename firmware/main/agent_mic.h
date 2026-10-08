#pragma once

// 按命令录两秒，导出 16 kHz / 单声道 / PCM16；平时不采集。
// 仅面包板板型编译。回调接收完整的一行（不含换行）。
void agent_mic_capture(void (*write_line)(const char *));
