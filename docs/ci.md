# CI 状态回流

## 为什么值得做

CI 出结果的时候，用户通常早就切走做别的事了。笔记本屏幕上的那个页签要主动去看才有用，而这个盒子一直在视野边缘——这是设备真正比屏幕有用的场景之一。

CI 与 Agent 共用同一套任务卡、同一只氛围小助手和同一组播报，只是来源不同：Agent 由 Hook 推送，CI 由 `vibebuddyd` 主动轮询 GitHub Actions。

## 映射

| GitHub Actions run | Vibe Buddy 状态 |
| --- | --- |
| `queued` / `in_progress` | 工作中，标题 `CI:<仓库名>` |
| `completed` + `success` | 完成，播放一次“任务完成” |
| `completed` + `failure` / `timed_out` / 其他 | 失败，播放一次“任务遇到问题” |
| `completed` + `cancelled` / `skipped` / `neutral` | 安静收起卡片，不播报 |

每个仓库最多占一张卡，取该仓库最近一次 run。轮询间隔 30 秒；CI 以分钟计，30 秒既够用，也不至于把 API 配额花在这上面。

**只报告亲眼见过在跑的 run。** 某次 run 已经结束、而 `vibebuddyd` 从没见过它处于运行中，就什么都不做。没有这条规则，daemon 每次重启都会把每个仓库最近一次历史结果重新宣告一遍，包括昨天那次失败。代价是：一次 run 若在两次轮询之间开始并结束，它不会被播报。

## 关注哪些仓库

没有配置文件，也不需要你列清单。`vibebuddyd` 关注的就是 **Agent 最近一小时工作过的 GitHub 仓库**：每个 Hook 都带 `cwd`，适配器为了生成任务卡标题本来就要把它解析成 git 项目根，CI 复用同一个事实，再从 `.git/config` 的 `origin` 远端读出 `owner/repo`。

这个推导只读本地文件，不调用 git 也不调用网络。子目录和 worktree 都会归到主仓库。没有 GitHub 远端的项目、以及一小时内没有 Agent 活动的项目，都不会被轮询；没有任何项目在跟踪时，整个功能不发一次请求。

一小时这个窗口对应的是「我正在这个仓库上干活，所以我关心它的 CI」。CI 通常在推送后几分钟内出结果，而推送前总会有 Agent 活动。

`VIBEBUDDY_CI_REPOS`（逗号分隔）可以覆盖自动推导，用于观察本机没有检出的仓库。

## `gh` 的位置

`vibebuddyd` 通过 `gh run list` 读取状态，沿用你已有的 GitHub 登录，不自己保存 token。

launchd 启动的进程只有一个很短的 `PATH`（`/usr/bin:/bin:/usr/sbin:/sbin`），`gh` 通常不在里面，所以 `vibebuddyd` 会依次尝试 `~/bin`、`/opt/homebrew/bin`、`/usr/local/bin`、`/usr/bin`，都找不到才回退到 `PATH`。装在别处可以用 `VIBEBUDDY_GH` 指定绝对路径。不需要改 plist。

## 边界

只请求 run 的 `databaseId`、`status` 和 `conclusion`。不读取日志、不读取 diff、不读取 commit message；设备上只出现仓库名和状态。

一个仓库连续读取失败时只记录第一次，恢复后记录一次恢复。每 30 秒一条告警一天就是几千行，会把真正有用的日志淹掉。
