---
status: accepted
---

# Claude Adapter 不解析 transcript，接受 `Stop` 的语义损失

Codex 的 `Stop` 提供 `last_assistant_message`，因此 Codex Adapter 能在本机判断助手是在提问还是已完成；Claude Code 的 `Stop` 只提供 `transcript_path`，要得到同样的信息必须读取会话文件。我们决定不读：Claude Adapter 把 `Stop` 一律映射为完成，"需要确认"完全依赖 `PermissionRequest` 这个显式信号。

## 与关键决定第 7 条的关系

架构第 7 条禁止解析不稳定的 transcript，其依据是 Codex 官方声明 transcript 格式不是 Hook 的稳定接口。Claude Code 的 `transcript_path` 是 Hook 载荷主动提供的字段，并无同类声明，因此第 7 条并不直接禁止这条路径。本决定不是受第 7 条约束的结果，而是独立的取舍。

## 被拒绝的方案

读取 `transcript_path` 的最后一条助手消息，复用 `codex-hook.py` 的文本规则。它能让两个 Adapter 行为一致，且解析失败可降级。拒绝的理由是它会把 issue #1 的猜测面积扩大一倍，并额外承担一份文件格式风险，而此时没有任何数据表明这种猜测是必要的。

## 后果

`PermissionRequest` 覆盖了 Claude Code 中绝大多数需要用户介入的场景，但纯对话式提问会被播报为"任务完成"。**这是静默失败**：用户以为工作结束，设备不会再次提醒。看到这个现象的人容易把它当作缺陷去"修复"，它是有意接受的代价。

由此两个 Adapter 的可靠性并不对称：Claude 侧的"需要确认"来自显式信号，比 Codex 侧的文本猜测更可靠；Claude 侧的"完成"则比 Codex 侧更容易误报。

重新评估的触发条件是实际误报造成困扰。届时应连同 `Notification` 的 `idle_prompt` 一起评估，并用真实样本检验文本规则。
