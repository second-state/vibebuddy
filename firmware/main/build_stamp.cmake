# 每次构建都重新写一遍当前时刻，供页脚显示本机固件的构建标识。
string(TIMESTAMP now "%Y-%m-%d %H:%M")
file(WRITE "${OUTPUT_FILE}" "#pragma once\n#define AGENT_BUILD_STAMP \"${now}\"\n")
