# AgentBeacon

AgentBeacon 把本地 AI Agent 的状态变成实体宠物的画面与声音。本文只定义领域词汇，不记录实现决定；实现边界见 [`docs/architecture.md`](docs/architecture.md)。

## Language

### 角色与组件

**小灯灵**：
AgentBeacon 的原创像素角色，设备上唯一的拟人化主体。英文写作 Beaconling。
_Avoid_: 宠物（泛指时可用，指代本角色时不可）、Pet、Codex 宠物

**Agent**：
被 AgentBeacon 观测的本地 AI 助手，例如 Codex 或 Claude Code。Agent 是观测对象，不是 AgentBeacon 的组成部分。
_Avoid_: 客户端、AI、助手；也不指 Claude Code 内部的 subagent

**Adapter**：
把某个 Agent 的事件翻译成 AgentBeacon 领域概念的那一层，包含本机隐私过滤脚本和 daemon 内的事件映射。每个 Agent 一个 Adapter，聚合逻辑不属于 Adapter。
_Avoid_: 集成、插件、connector

### 活动生命周期

**Session**：
一个 Agent 进程中的一次完整会话。由 Agent 提供标识，AgentBeacon 不自行生成。
_Avoid_: 连接、会话实例

**Turn**：
从用户提交一次输入到助手停止生成之间的工作单元。Codex 以 `turn_id` 标识，Claude Code 以 `prompt_id` 标识；两者语义对齐，因此 Turn 是跨 Agent 的通用概念，不是某个 Agent 的专有词汇。
_Avoid_: 轮次、prompt、请求

**Activity**：
AgentBeacon 正在追踪的一次工作及其当前状态，是值得单独播报的最小单位。对话式 Agent 的 Activity 由 Session 与 Turn 共同确定身份；训练任务、CI 等没有 Turn 的生产者自行提供标识。Activity 在结束或过期时消失。
_Avoid_: 任务、Job、Task（Task 专指设备上的呈现，见下）

**任务卡**：
Activity 在设备屏幕上的呈现形式，最多同时显示 3 张，最近活动的在最上。Beacon Protocol 中承载它的字段名为 `tasks`，属于 v1 的历史命名，不改变 Activity 才是领域对象这一事实。
_Avoid_: 卡片、条目、Task item

### 状态与通知

**可见状态**：
当前应当显示在设备上的那一个状态快照，由所有 Activity 聚合得出。可见状态是持续的、可去重的：相同的快照不重复下发。
_Avoid_: 当前状态、全局状态、快照

**播报**：
只应消费一次的边沿通知，对应一次短语音。播报不改变可见状态，也不参与去重；同一个 Turn 的播报不得触发第二次。
_Avoid_: 通知、提醒、语音事件

**需要确认**：
某个 Activity 正在等待用户回应时的状态。它在全局优先级上高于工作中，因为它回答的是"我现在最需要你做什么"。
_Avoid_: 阻塞、等待中、pending
