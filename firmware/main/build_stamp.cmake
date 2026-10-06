# Rewrites the current time on every build so DISPLAY READY BUILD reports this firmware's build stamp.
string(TIMESTAMP now "%Y-%m-%d %H:%M")
file(WRITE "${OUTPUT_FILE}" "#pragma once\n#define AGENT_BUILD_STAMP \"${now}\"\n")
