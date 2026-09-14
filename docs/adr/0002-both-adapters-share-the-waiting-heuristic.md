---
status: accepted
---

# 两个 Adapter 使用同一套等待判定

2026-09-14 的实测表明，Claude Code 的 `Stop` 事件提供 `last_assistant_message`，与 Codex 一致；`SubagentStop` 同样提供。因此 Claude Adapter 与 Codex Adapter 使用同一套本机文本规则判断助手是否在等待回答，并在此之上额外使用 `PermissionRequest` 这个显式信号。

## 为何取代 ADR-0001

ADR-0001 的前提是 Claude Code 的 `Stop` 只提供 `transcript_path`。该前提来自对官方文档缺失部分的推断，实测证明不成立。载荷里既然已有 `last_assistant_message`，就既不需要读取 transcript，也不必接受"提问被播报为完成"的静默失败。

## 后果

两个 Adapter 的可见行为一致，不存在可靠性不对称。Claude 侧的信息反而更全：权限请求有显式事件，对话式提问有文本判断。

代价是 issue #1 的文本规则从单一 Adapter 的局部问题变成两个 Adapter 的共同依赖，其误判会同时影响两条链路，重要性相应上升。

## 教训

对外部接口的事实必须实测，不能采信对文档的二手推断。文档没有写，不等于该字段不存在。
