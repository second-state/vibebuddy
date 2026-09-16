# Issue tracker：GitHub

本仓库的 issue 与规格都以 GitHub issue 的形式存在，所有操作用 `gh` CLI。

## 约定

- **建 issue**：`gh issue create --title "..." --body "..."`，多行正文用 heredoc。
- **读 issue**：`gh issue view <number> --comments`，用 `jq` 过滤评论并顺带取标签。
- **列 issue**：`gh issue list --state open --json number,title,body,labels,comments --jq '[.[] | {number, title, body, labels: [.labels[].name], comments: [.comments[].body]}]'`，按需加 `--label` 与 `--state`。
- **评论**：`gh issue comment <number> --body "..."`
- **加 / 去标签**：`gh issue edit <number> --add-label "..."` / `--remove-label "..."`
- **关闭**：`gh issue close <number> --comment "..."`

仓库从 `git remote -v` 推断；在检出目录里运行时 `gh` 会自动识别。

## PR 作为 triage 入口

**PR 作为需求入口：否。** _（若本仓库把外部 PR 当作功能请求处理，改为「是」；`/triage` 会读这个开关。）_

设为「是」时，PR 走与 issue 相同的标签与状态，用 `gh pr` 的对应命令：

- **读 PR**：`gh pr view <number> --comments`，diff 用 `gh pr diff <number>`。
- **列待 triage 的外部 PR**：`gh pr list --state open --json number,title,body,labels,author,authorAssociation,comments`，只保留 `authorAssociation` 为 `CONTRIBUTOR`、`FIRST_TIME_CONTRIBUTOR` 或 `NONE` 的（去掉 `OWNER`、`MEMBER`、`COLLABORATOR`）。
- **评论 / 标签 / 关闭**：`gh pr comment`、`gh pr edit --add-label`/`--remove-label`、`gh pr close`。

GitHub 的 issue 与 PR 共用一个编号空间，裸写的 `#42` 可能是任一种：先 `gh pr view 42`，不行再 `gh issue view 42`。

## 技能说「发布到 issue tracker」时

建一个 GitHub issue。

## 技能说「取相关 ticket」时

运行 `gh issue view <number> --comments`。

## Wayfinding 操作

供 `/wayfinder` 使用。**地图**是一个 issue，**子项**是作为 ticket 的子 issue。

- **地图**：一个打了 `wayfinder:map` 标签的 issue，正文放 Notes / Decisions-so-far / Fog。`gh issue create --label wayfinder:map`。
- **子 ticket**：以 GitHub sub-issue 的方式挂在地图下（走 `gh api` 的 sub-issues 端点）。sub-issue 不可用时，把子项加进地图正文的任务列表，并在子项正文顶部写 `Part of #<map>`。标签：`wayfinder:<type>`（`research`/`prototype`/`grilling`/`task`）。认领后把 ticket 指派给负责的开发者。
- **阻塞**：用 GitHub **原生 issue 依赖**，这是界面可见的规范表示。加一条边：`gh api --method POST repos/<owner>/<repo>/issues/<child>/dependencies/blocked_by -F issue_id=<blocker-db-id>`，其中 `<blocker-db-id>` 是阻塞方的数字**数据库 id**（`gh api repos/<owner>/<repo>/issues/<n> --jq .id`，_不是_ `#number` 或 `node_id`）。GitHub 通过 `issue_dependencies_summary.blocked_by` 报告仍开着的阻塞方，那就是实时的门。依赖不可用时，退回到子项正文顶部一行 `Blocked by: #<n>, #<n>`。所有阻塞方都关闭时 ticket 才解除阻塞。
- **前沿查询**：列出地图的开放子项（`gh issue list --state open`，限定在地图的 sub-issue / 任务列表内），去掉仍有开放阻塞方（`issue_dependencies_summary.blocked_by > 0`，或 `Blocked by` 行里有开放 issue）或已有指派人的，按地图顺序取第一个。
- **认领**：`gh issue edit <n> --add-assignee @me`，是会话的第一次写入。
- **解决**：`gh issue comment <n> --body "<answer>"`，然后 `gh issue close <n>`，再把上下文指针（gist 加链接）追加到地图的 Decisions-so-far。
