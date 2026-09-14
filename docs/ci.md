# CI 状态回流

## 为什么值得做

CI 出结果的时候，用户通常早就切走做别的事了。笔记本屏幕上的那个页签要主动去看才有用，而这个盒子一直在视野边缘——这是设备真正比屏幕有用的场景之一。

CI 与 Agent 共用同一套任务卡、同一只小灯灵和同一组播报，只是来源不同：Agent 由 Hook 推送，CI 由 `beacond` 主动轮询 GitHub Actions。

## 映射

| GitHub Actions run | AgentBeacon 状态 |
| --- | --- |
| `queued` / `in_progress` | 工作中，标题 `CI:<仓库名>` |
| `completed` + `success` | 完成，播放一次“任务完成” |
| `completed` + `failure` / `timed_out` / 其他 | 失败，播放一次“任务遇到问题” |
| `completed` + `cancelled` / `skipped` / `neutral` | 安静收起卡片，不播报 |

每个仓库最多占一张卡，取该仓库最近一次 run。轮询间隔 30 秒；CI 以分钟计，30 秒既够用，也不至于把 API 配额花在这上面。

**只报告亲眼见过在跑的 run。** 某次 run 已经结束、而 `beacond` 从没见过它处于运行中，就什么都不做。没有这条规则，daemon 每次重启都会把每个仓库最近一次历史结果重新宣告一遍，包括昨天那次失败。代价是：一次 run 若在两次轮询之间开始并结束，它不会被播报。

## 配置

监听哪些仓库写在 `~/.config/agentbeacon/ci-repos`，一行一个 `owner/repo`，`#` 之后是注释：

```text
# 每次轮询都会重新读这个文件，加仓库不用重启 daemon
longzhi/agent-beacon
someone/another-repo
```

文件不存在或为空时，整个功能静默关闭，不发任何请求。`BEACON_CI_REPOS` 环境变量（逗号分隔）可以覆盖它，便于测试。

## `gh` 的路径

`beacond` 通过 `gh run list` 读取状态，沿用用户已有的 GitHub 登录，不自己保存 token。

LaunchAgent 启动的进程只有一个很短的 `PATH`（`/usr/bin:/bin:/usr/sbin:/sbin`），`gh` 通常不在里面。如果日志里出现 `无法执行 gh`，在 plist 里补上路径：

```xml
<key>EnvironmentVariables</key>
<dict>
  <key>PATH</key>
  <string>/Users/dragon/bin:/opt/homebrew/bin:/usr/bin:/bin:/usr/sbin:/sbin</string>
</dict>
```

也可以用 `BEACON_GH` 直接指定可执行文件的绝对路径。

## 边界

只请求 run 的 `databaseId`、`status` 和 `conclusion`。不读取日志、不读取 diff、不读取 commit message；设备上只出现仓库名和状态。

一个仓库连续读取失败时只记录第一次，恢复后记录一次恢复。每 30 秒一条告警一天就是几千行，会把真正有用的日志淹掉。
