# Vibe Buddy

Vibe Buddy 把本地 AI Agent 的状态变成实体宠物的画面与声音。本文只定义领域词汇，不记录实现决定；实现边界见 [`docs/architecture.md`](docs/architecture.md)。

## Language

### 角色与组件

**Vibe Buddy**：
产品名，2026-09-15 起用，此前叫 AgentBeacon。2026-09-16 起内部名也全部跟着改：仓库 `vibe-buddy`、daemon `vibebuddyd`、Hook `vibebuddy-hook`、CLI `vibebuddy`、Vibe Buddy Protocol、bundle id `com.vibebuddy.app`、本机目录 `VibeBuddy`。
_Avoid_: AgentBeacon、beacond、beacon-hook（旧名，只在讲历史时出现）、氛围助手（那是解释，不是名字）

**氛围小助手**：
Vibe Buddy 的原创像素角色，设备上唯一的拟人化主体，英文也叫 Vibe Buddy。2026-09-15 前叫小灯灵，代码里叫 `buddy`。
_Avoid_: 小灯灵（旧名）、宠物（泛指时可用，指代本角色时不可）、Pet、Codex 宠物、助手（单独用时指 Agent，见下）

**Agent**：
被 Vibe Buddy 观测的本地 AI 助手，例如 Codex 或 Claude Code。Agent 是观测对象，不是 Vibe Buddy 的组成部分。
_Avoid_: 客户端、AI、助手；也不指 Claude Code 内部的 subagent

**Adapter**：
把某个 Agent 的事件翻译成 Vibe Buddy 领域概念的那一层，包含本机隐私过滤脚本和 daemon 内的事件映射。每个 Agent 一个 Adapter，聚合逻辑不属于 Adapter。
_Avoid_: 集成、插件、connector

**App**：
Vibe Buddy 在 Mac 上的图形界面：菜单栏图标加设置窗，负责首次引导、设置、音色与固件，并看管 daemon。它不是第二个提醒出口，Agent 的事都由氛围小助手来讲。
_Avoid_: 客户端、面板、控制台、伴侣应用

**链路**：
Mac 与设备之间的连接。链路断开时氛围小助手闭眼变灰，链路异常是 App 唯一会自己出声提醒的事。
_Avoid_: 连接、USB、串口（那是链路的一种实现）

**接入**：
某个 Agent 与 Vibe Buddy 之间的 Hook 连接及其状态：装没装、最近一次收到事件是什么时候。
_Avoid_: 集成、安装、配置

**运行处**：
Agent 进程实际待的地方：自己的桌面应用、别的应用（终端、编辑器的集成终端），或者没有宿主（SSH、后台进程）。它只决定 K2 把人送回哪里，不参与活动身份、可见状态与播报。判定在 Hook 里做，那是唯一看得见进程环境的地方。
_Avoid_: 环境、入口、entrypoint、surface、终端（终端只是运行处的一种）

### 声音

**播报音色**：
氛围小助手说话用的那一个声音，全部五句共用，同一时刻只有一个。候选音色各自成套归档，用户在 App 里挑选并写入设备。
_Avoid_: 声音、语音（语音指那几句台词本身）、voice、TTS

**语音包**：
一个播报音色的五句成品打成的一包，写进设备里独立于程序的位置；设备没有语音包时用出厂内置的那一套。换音色就是换语音包，不换固件。
_Avoid_: 音色包、资产包、语音资产（那是仓库里的源文件）

**静音**：
设备扬声器的总开关，只在设备上长按 K2 切换：关上后播报和番茄钟的铃声都不出声，画面照常。它是设备自己的事，Mac 端不记、不显示、不代切。
_Avoid_: 免打扰、勿扰、mute、关声音

**音量**：
设备扬声器的响度档位，20 到 100，存在设备上、重启不丢；播报和番茄钟的铃共用一个。App 的滑块只是遥控，显示的永远是设备报回来的值。下限不到零：关声音只能靠静音。
_Avoid_: 声音大小、volume、音量设置（那是 App 里的一处界面，不是这个概念）

### 模式

**模式**：
氛围小助手此刻在替用户做的事，同一时刻只有一个，每个模式拥有整块画面。三个：值班、番茄钟、休闲。模式决定画面归谁，不决定语音——扬声器不分模式。
_Avoid_: 场景、页面、视图、状态（那是值班里 Agent 的词）

**值班**：
默认模式：盯着 Agent，有事叫你。任务卡、氛围小助手的表情、当日战绩都在这里。Agent 一有动静就回到值班。
_Avoid_: Agent 模式、助手模式（“助手”已被 Agent 一词避免）、工作模式（用户在番茄钟里也在工作）

**番茄钟**：
设备本地的专注计时器：专注 25 分钟、休息 5 分钟，两个阶段都由用户按键开始。它的状态只在固件里，Mac 端不参与；它不是 Activity，也不产生任务卡。
_Avoid_: 计时器、Timer、番茄、专注模式（会与专注阶段相撞）

**阶段**：
番茄钟当前所处的一段：专注或休息，各自有待开始、运行中、暂停三种运行状态。阶段结束是只消费一次的边沿，与播报同一性质。
_Avoid_: session

**休闲**：
值班时空闲够久之后氛围小助手自己去玩的模式。它由无聊度驱动而不是由用户切换；Agent 有动静、按键、链路断开都让它立刻回到值班。
_Avoid_: 娱乐模式、屏保、空闲模式（空闲是值班里的一个状态）

**无聊度**：
连续空闲的时长，按状态而非按消息累计：Agent 无事可做、番茄钟没在走、链路正常、没有按键。分三档：待命、无聊、困倦。
_Avoid_: 空闲时间（会与值班的空闲状态混）

**剧目**：
休闲模式里随机演出的一段几秒到十几秒的动画，例如巡逻、踢球、看书。
_Avoid_: 动画、小动作（那是值班空闲时的眨眼伸懒腰）

### 英文界面用语

App 英文界面与英文日志里的固定说法。写英文文案时用这些词，不另造同义词。

| 中文 | English | 说明 |
|---|---|---|
| 盒子 | box | 指设备本身；界面里小写，句首大写 |
| 氛围小助手 | the buddy | 角色；产品名仍是 Vibe Buddy |
| 链路 | link | 设置页「Link」一栏，断开时说 disconnected |
| 接入 / 修复 / 移除 | Connect / Repair / Remove | Hook 的三个操作；设置页标签页叫 Agents，未接入叫 Not set up |
| 播报音色 | announcement voice | 选音色的地方；单说音色时用 voice |
| 内置音色 | built-in voice | 固件自带的湾湾小何 |
| 语音包 | voice pack | |
| 音量 / 静音 | volume / mute | |
| 模式 | mode | |
| 值班 / 番茄钟 / 休闲 | On duty / Pomodoro / Leisure | 界面用首字母大写；代码与注释里是 duty / pomodoro / leisure |
| 剧目 | skit | 休闲模式里的小表演 |
| 当日战绩 | today's stats | 菜单里写作 Today: done · asks · busy |
| 需要确认 | needs input | 语音台词是 Need your input.，战绩里计作 asks |
| 任务卡 | task card | |
| 固件 / 刷入 | firmware / flash | |
| UART 桥 / 原生 USB | UART bridge / native USB | |

### 活动生命周期

**Session**：
一个 Agent 进程中的一次完整会话。由 Agent 提供标识，Vibe Buddy 不自行生成。
_Avoid_: 连接、会话实例

**Turn**：
从用户提交一次输入到助手停止生成之间的工作单元。Codex 以 `turn_id` 标识，Claude Code 以 `prompt_id` 标识；两者语义对齐，因此 Turn 是跨 Agent 的通用概念，不是某个 Agent 的专有词汇。
_Avoid_: 轮次、prompt、请求

**Activity**：
Vibe Buddy 正在追踪的一次工作及其当前状态，是值得单独播报的最小单位。对话式 Agent 的 Activity 由 Session 与 Turn 共同确定身份；训练任务、CI 等没有 Turn 的生产者自行提供标识。Activity 在结束或过期时消失。
_Avoid_: 任务、Job、Task（Task 专指设备上的呈现，见下）

**任务卡**：
Activity 在设备屏幕上的呈现形式，最多同时显示 3 张，最近活动的在最上。Vibe Buddy Protocol 中承载它的字段名为 `tasks`，属于 v1 的历史命名，不改变 Activity 才是领域对象这一事实。
_Avoid_: 卡片、条目、Task item

### 状态与通知

**可见状态**：
当前应当显示在设备上的那一个状态快照，由所有 Activity 聚合得出。可见状态是持续的、可去重的：相同的快照不重复下发。
_Avoid_: 当前状态、全局状态、快照

**播报**：
只应消费一次的边沿通知，对应一次短语音。播报不改变可见状态，也不参与去重；同一个 Turn 的播报不得触发第二次。
_Avoid_: 通知、提醒、语音事件

**当日战绩**：
按本地自然日累计的完成数、需要确认次数和忙碌时长。它描述的是这一天发生过什么，不是当前状态，因此只在空闲时显示，也不参与去重。
_Avoid_: 统计、metrics、历史

**需要确认**：
某个 Activity 正在等待用户回应时的状态。它在全局优先级上高于工作中，因为它回答的是"我现在最需要你做什么"。
_Avoid_: 阻塞、等待中、pending
