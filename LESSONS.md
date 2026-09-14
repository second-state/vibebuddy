# 经验教训

## EIM 激活脚本不要从自动化脚本中直接 source

EIM 0.19.0 生成的 `activate_idf_v5.5.3.sh` 会读取调用者的 `$0`、`$1` 和 shell 变量来判断是否被 source。它从启用 `set -u` 的 Bash 脚本调用时，可能先因未定义的 `ZSH_VERSION` 失败；即使临时关闭 nounset，也会把调用脚本名误判为“直接执行”并 `exit 1`。

自动化脚本应执行激活脚本的 `-e` 模式读取环境变量，再用固定 Python 解释器调用 `$IDF_PATH/tools/idf.py`。交互式 shell 仍可正常 source 激活脚本。
