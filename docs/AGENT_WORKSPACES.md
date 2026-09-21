# Agent 工作空间使用指南 · v0.3

`tc task` 规划你的逻辑任务；`tc agent` 整理模型的工作容器。两者互不要求存在，通过 `agent link` 进行多对多关联。`task --project` 保留为原来的规划分组标签，不是 agent 项目身份。

## 第一次使用

先在自己的 GitHub 账号下建立空的私有 `schedule-workspace` 仓库，并配置 Git 认证。下面的命令不会为你创建 GitHub 仓库。程序源码仓库 rustychedule 与你自己的 sync 仓库是两回事。

```sh
tc hub init ../schedule-hub --github YOUR_ACCOUNT
tc machine alias 台式机
tc agent create A --provider codex
tc agent import A ./X                 # 预览文件数、大小、仓库入口与跳过项
tc agent import A ./X --apply
tc agent import A ./Y --apply
tc agent folder A Z                   # 暂时没有来源文件的空文件夹
tc agent tree A
tc hub check
tc hub diff
tc hub commit -m "登记 A 的 X/Y/Z 工作目录"
tc hub push
```

树状视图是：

```text
A
├── X
├── Y
└── Z
```

X/Y/Z 内部可以包含文件、子文件夹、多个代码仓库。普通文件正文进入中央，独立 Git 仓库只登记入口。不要为了让 CLI 同步普通笔记先执行 `git init`，否则它会被识别为独立仓库而只保存索引。

项目、文件夹名支持中文和空格（有空格需引号）。名称不唯一时使用命令返回的完整 ID，或至少 10 字符的唯一 ID 前缀；名称可改，身份不变。树同时展示可复制 ID。

## 代码仓库和机器

```sh
tc repo add schedule ./existing-code
tc repo add library --remote https://github.com/OWNER/library.git
tc agent attach A schedule --parent X
tc repo status
tc repo show schedule
tc machine alias 实验机
tc machine list
```

已有 origin 时保存无凭据远程地址；没有远程时保存机器 ID、别名映射及本地绝对路径。机器位置是有意共享的索引，并非代码备份。另一台设备无法仅凭路径取得本地-only 代码。`repo status` 只检查本机路径，显示原代码未提交改动以及已有上游跟踪引用的落后/领先数；不联网刷新上游。

```sh
tc repo remote schedule https://github.com/OWNER/schedule.git
tc repo bind schedule ./another-local-checkout
tc repo checkout library --to ./new-library
```

更新 remote 只更新中央索引，不修改原仓库 Git 配置。同一仓库可被多个 agent 项目引用。已有登记应 bind/attach 复用；系统不会仅凭相同 remote 自动合并两条独立登记。

## 日常文件更新

```sh
tc agent collect X                    # 比较来源、上次基准和中央正文
tc agent collect X --apply
tc agent path X                      # 查看中央直属正文位置
tc agent find "重置规则" --agent A
tc hub diff
tc hub push --agents-only
```

导入后原目录保留且不自动修改。可以继续在来源目录工作，然后 collect；也可编辑中央正文。两边对同一文件有不同修改时停止，手工统一后再次 collect。来源缺失时不会删除中央正文；来源删文件默认也不传播，确需传播时用 `collect --apply --prune`。删除与中央修改冲突仍停止。

`.gitignore`、`.tcignore` 使用 Git 的匹配规则；`import --exclude PATTERN` 可重复，保存到本机来源绑定。常见缓存/构建目录、`.env*`、`*.key`、`*.pem`、符号链接/Windows 目录联接默认排除。单文件小于 90 MiB，单次收集不超过 512 MiB，目录深度不超过 64。文件正文按字节保存；不迁移 ACL、扩展属性或特殊文件。

`AGENTS.md`、`GOAL.md` 没有特殊上传资格：只要属于纳管范围就同步。中央按节点 ID 存放直属文件；完整目录层级由元信息保存，子目录用 folder/import 登记。不要在 `content/<节点>/` 下直接创建未登记子目录。给 agent 一个保持原指令作用域的工作环境时，使用 export 恢复原目录树。

## 换设备与按需恢复

```sh
tc hub clone https://github.com/YOUR_ACCOUNT/schedule-workspace.git ../schedule-hub --metadata-only
tc hub pull --agents-only            # 暂不导入逻辑任务和额度
tc agent list
tc hub select A                      # 只检出 A 的正文
# 也可选择单独文件夹：tc hub select X
tc machine alias 笔记本
tc agent export A --to ./A --bind
tc repo checkout library --to ./A/Y/library
```

导出只能使用新目录，恢复 X/Y/Z 层级与全部选中项目正文；仓库代码不会隐式克隆，根目录 `.tc-export.json` 保存仓库入口清单、来源和项目身份。`--bind` 将导出后的顶层文件夹作为本机 collect 来源；同一节点在本机只维护一个活动来源绑定。默认 export 只是副本。

默认 clone 获取整个 Git 仓库。`--metadata-only` 请求 partial clone 并仅检出目录信息；服务器不支持过滤时可能仍传输全部对象。`hub select` 是工作树选择，不是权限隔离，也不会清除已下载的历史对象。Git 合并有冲突时可能需要额外正文对象。未选择正文不会被当成删除；搜索会报告未搜索的节点。`hub select` 不传目标只保留元信息，`hub select --all` 恢复全部正文。

如需继续规划，另行 `tc hub pull` 导入完整逻辑任务和用量。各模型预算始终按全量规划数据计算，不因正文选择而少算其他任务的预留。

## 聊天与逻辑任务关联

```sh
tc agent bind A --provider chatgpt --source PROJECT_URL_OR_ID
tc agent chat A 文献讨论 --provider chatgpt --source CHAT_URL_OR_ID --summary ./handoff.md
tc agent link A --task 1 --task 2
tc agent link 文献讨论 --task 1 --task 3
tc agent link A --task 2 --remove
```

这些操作登记来源、明确摘要和关联，不读写平台隐藏记忆，也不在平台创建/合并/删除项目。整理 agent 项目不更改逻辑任务进度或账本。关联 ID 来自 `task list`，不用同名判断同一逻辑任务。

## 整理结构与 Git 工作流

```sh
tc agent rename A 研究工作室
tc agent rename-node X 论文资料
tc agent move 论文资料 --agent B       # 移到 B 根目录，或者 --parent 目标文件夹
tc agent split Y --name 实验工作室
tc agent merge 实验工作室 B           # 先预览
tc agent merge 实验工作室 B --apply
tc agent archive B
tc agent archive B --restore
tc agent remove Z                     # 先预览
tc hub commit -m "删除前保存"
tc agent remove Z --apply
```

合并在目标里增加来源分组，保留来源项目的归档记录与合并去向；不拼接文档。拆分/移动保持节点身份，原平台容器来源不变。`remove` 删除中央节点子树和正文，不删除原目录、仓库索引或源代码；要求先提交当前版本，方便 Git 恢复。项目删除采用 archive 隐藏，正文保留；已合并项目的整体撤销使用 Git 恢复对应变更，不能用 archive --restore 假装内容已搬回。

可直接在 hub 中使用 Git 分支、diff、log、restore、revert。完成 Git 手工操作后运行 `tc hub check`。CLI push/pull 使用当前分支及 origin 上的同名分支，不强制写 main、不 force push。`hub pull` 首先在临时工作树验证合并结果，文本或结构冲突不修改原工作树；失败时可在 hub 中手工 merge 后解决，再 check。

如果 Git 分支双方分别修改了规划账本，不做自动文本合并：需明确保留一份正确账本或手工对账。`--agents-only` 不读写本机规划账本，但仍同步当前 Git 分支历史，无法过滤掉你已经提交的规划文件变更。

## v0.2 升级

如果只登记过旧 project、还没有旧 hub，第一次 `hub init` 会把旧登记及记忆导入待整理容器，并保留原规划数据，不需要先建立旧格式 hub。

在主设备把旧数据 push/pull 对齐后：

```sh
tc export ./planning-before-v3.json
tc hub migrate                       # 预览
tc hub migrate --apply
tc hub check
tc agent list
tc agent tree "v0.2 待整理资料"
tc hub commit -m "迁移 agent 工作空间"
tc hub push
```

旧任务、预算、用量、复盘完全保留；旧文件及 bundle 保存在 `legacy/v02`。旧仓库转为仓库索引，旧 CONTEXT/MEMORY 转为待整理容器内的正文；不猜测旧项目对应哪个平台容器或逻辑任务。旧 snapshot 仍可通过 `repo checkout` 恢复，新本地-only 仓库不再默认 bundle。

其他旧设备直接 `hub pull` 跟随中央迁移，使用同一批 ID；如果旧分支已分叉，先保留原目录，在新目录 clone，不能各自再生成一套身份。原有 `project` 命令仅为旧格式兼容，新格式应使用 agent/repo。新 hub 格式号为 2，规划 State schema 仍为 2，程序版本为 0.3.0；三者不是同一个版本号。

`tc export/import` 备份恢复的是规划账本，不包括新 agent 正文。完整 agent 数据由 sync Git 仓库备份，导出的工作目录是便于使用的副本，不含完整 Git 历史。
