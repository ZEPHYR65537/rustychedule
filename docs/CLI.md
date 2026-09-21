# CLI 指令清单 · v0.3

schedule、tc 完全等价，tongchou 为旧兼容入口。以下统一写 tc。所有子命令支持 --help；全局 --data PATH、--json 可放在子命令前或后。默认无子命令等于 dashboard。帮助为中文，数据枚举使用英文稳定值。

尖括号表示必填参数，方括号表示可选参数，不需要实际输入括号。逗号列表应写作 a,b；偏序边含 >，务必用引号。

## 总览、规划、备份

| 命令 | 作用/参数 |
|---|---|
| tc init [--demo] | 初始化空状态；--demo 仅适用于空数据 |
| tc dashboard / tc | 任务、额度条、卡、续期和今日分钟数 |
| tc matrix | 四象限视图 |
| tc plan [--minutes 240] [--reserve-percent 10] [--project NAME] [--rebalance] [--commit] | 预览排程；commit 保存预留；rebalance 释放所选项目范围内未用预算再排 |
| tc report [--days 7] | 按模型和额度来源展示原始 token、每日趋势 |
| tc doctor | 数据校验、版本与目录 |
| tc export <PATH> [--force] | 导出规划账本（不含 agent 正文）；覆盖已存在文件需 force |
| tc import <PATH> [--replace] | 校验后导入；非空状态需要 replace |

plan 只使用当前额度，不预测未来重置。分钟数是本次连续可工作时间，不是日历范围。任务放不下时显示暂缓原因，不会留下部分新预留。

## 任务

- tc task add <TITLE>
- tc task edit <ID>
- tc task list [--all] [--status STATUS] [--project NAME] [--tag TAG] [--search TEXT]
- tc task show <ID>
- tc task status <ID> <todo|doing|blocked|done|cancelled> [--note TEXT]

add 的选项：

| 选项 | 默认与语义 |
|---|---|
| -i, --importance 1..5 | 默认 3 |
| -u, --urgency 1..5 | 默认 3；截止时间可提高有效紧迫性 |
| --due DATE | YYYY-MM-DD（本地当天 23:59:59）或 RFC3339 |
| --minutes N | 默认 30，本次剩余工作分钟 |
| --input N / --output N | 默认 0，全任务基准 token 估计 |
| --progress P | 默认 0，绝对完成百分比 0..100 |
| --capability 1..5 | 默认 1，模型最低能力筛选 |
| --project NAME / --tags a,b | 项目标识和标签 |
| --depends 1,2 | 前置任务，拒绝循环 |
| --models a,b | 允许列表；省略为全体模型 |
| --prefer 'a>b' | 可重复；任务偏序整体覆盖全局偏序 |
| --factor a=1.5 | 可重复；该模型完成相同工作相对基准的 token 倍率 |
| --no-split | 禁止跨模型分段；同模型可用订阅+API |
| --note TEXT | 任务上下文 |

edit 支持同名长选项，含 --title；不传的字段不变。清空选项为 --clear-due、--clear-depends、--all-models、--clear-preferences、--clear-factors。edit 使用 --splittable true|false，不使用 --no-split。--prefer、--factor、--tags、--depends、--models 一旦传入便替换该集合。清空任务偏序后恢复使用全局偏序。结束状态会释放任务预算。

## 模型、订阅、API

| 命令 | 作用 |
|---|---|
| tc model add <NAME> [--provider custom] [--capability 3] [--unit token] [--input-weight 1] [--output-weight 1] | 建立独立模型；未配置任何窗口/账户的旧模式模型按无限额处理并提示，实际使用前请配订阅或 API |
| tc model edit <NAME> [--provider TEXT] [--capability N] [--enabled true\|false] [--input-weight X] [--output-weight X] | 停用保留历史；标准订阅模型权重必须为 1 |
| tc model list | 模型列表 |
| tc subscription set <MODEL> --weekly-tokens N [--plan NAME] [--renewal-day 1..31] [--next-reset RFC3339] [--note TEXT] | 建立/更新 7 天固定 week 窗口及订阅信息，启用订阅；容量为估计 |
| tc subscription estimate <MODEL> <N> [--note TEXT] | 只更新周 token 容量估计，保留原始百分比观测 |
| tc subscription active <MODEL> <true\|false> | 仅启停订阅，不停用模型自身 API |
| tc subscription list | 订阅状态、估计容量、月续期日 |
| tc api set <MODEL> --limit <N> | 累计 API token 使用上限；0 禁用新分配并释放此模型 API 预留 |
| tc api top-up <MODEL> <N> | 上限增加 N；本地记账，不实际购买 |
| tc api list | 各模型 API 上限、累计实际用量、未用预留、余额 |

subscription set 是完整订阅配置；重配时需重新提供希望保留的 plan、renewal-day、note。不提供 next-reset 时保留既有 week 锚点，新窗口从当前时刻起算。renewal-day 可省略，且仅作提示，月续期不会额外清零周账本。

上限是独立、累计、不自然重置的 token 上界。缓存输入包含于 input，API 扣 input+output，不重复加 cached。实际用量允许如实记录为超限，规划拒绝新增超配。订阅重置不影响 API。

### 高级额度窗口

- tc quota add <MODEL> <NAME> --limit N [--kind fixed|rolling|manual] [--period 5h|7d] [--next-reset RFC3339]
- tc quota limit <MODEL> <WINDOW> <LIMIT>
- tc quota list [--model NAME]

fixed/rolling 需要周期；manual 不需要。不同窗口共同约束同一订阅用量，不相加。例如周窗口与五小时窗口都必须满足。自定义单位/权重的旧模型不能被 subscription set 静默转换为原始 token；继续使用 quota，或另建模型。

## 偏好与预算

- tc policy models --prefer 'a>b' --prefer 'a>c'
- tc policy models --clear
- tc policy funding <subscription-first|model-first|subscription-only>
- tc policy show
- tc budget set <TASK> <MODEL> [--source subscription|api] [--input N] [--output N] [--force]
- tc budget list [--task ID]
- tc budget release <TASK> [--model NAME]

默认 subscription-first；model-first 表示优先保持用户指定的模型质量。两者都不对模型名称内置排名。修改策略不自动重写已有预算，用 plan --rebalance 预览。

budget set 替换指定任务/模型/来源的**剩余**预算。--force 可显式登记超配预留，规划会指出超配并暂缓，不应常规使用。release 同时释放所选任务/模型的订阅和 API 预留。

## 用量与复盘

- tc usage log <MODEL> [--source subscription|api] [--task ID] [--input N] [--output N] [--cached N] [--units X] [--at RFC3339] [--progress P] [--note TEXT]
- tc usage reconcile <MODEL> <WINDOW> --percent P [--note TEXT]
- tc usage reconcile <MODEL> <WINDOW> --used X [--note TEXT]
- tc usage list [--model NAME] [--last 20]
- tc usage void <ID>
- tc work log <TASK> [--minutes N] [--model NAME] [--source subscription|api] [--input N] [--output N] [--cached N] [--at RFC3339] [--done TEXT] [--learned TEXT] [--next TEXT] [--progress P]
- tc work list [--task ID]
- tc work summary <TASK>
- tc work void <ID>

默认 source=subscription。API 不接受 --units，始终按原始 token；--units 仅校正订阅这笔额度扣量。--progress 要求关联任务，是用户明确填入的绝对百分比，不由消耗推断。

work log 可只记时间/完成内容；记 token 时必须提供 model。带模型时创建一条关联用量事件，不要再为同一次使用另记 usage log。work void 撤销复盘和它创建的用量，不回退手动进度；usage void 只撤销用量/校准，不删除复盘的完成内容和时间。所有撤销保留原记录。回填时间不能位于未来。

## 重置

- tc credit add [--kind card|manual] [--count 1] [--models a,b] [--windows week] [--expires DATE] [--note TEXT]
- tc credit list
- tc reset --models a,b [--windows week] --credit <ID> [--restart-clock] [--note TEXT]
- tc reset --models a,b [--windows week] --source <tibo|manual> [--restart-clock] [--note TEXT]

不填卡适用模型/窗口表示不限该维度。一次合法 reset 操作扣一次卡库存，即使作用于多个允许模型；模型账本仍各自刷新。过期、范围不匹配、次数不足时整次失败。--windows 省略时为选定模型全部订阅窗口。默认保留自然周期相位；restart-clock 才改为现在+周期。自然重置无需命令。以上操作不作用于 API。

## Agent 项目、文件夹和聊天

与规划任务独立。名称支持中文，歧义时使用完整 ID 或至少 10 字符的唯一 ID 前缀。命令里的 NAME/ID 均可用可唯一解析的名称。

| 命令 | 作用 |
|---|---|
| tc agent create NAME [--provider TOOL] [--source URL_OR_ID] | 建立容器身份，不创建平台服务端项目 |
| tc agent list [--all] | 默认活动项目，all 含归档 |
| tc agent tree [AGENT] / show AGENT | 树状结构 / 项目元信息 |
| tc agent bind AGENT --provider TOOL --source URL_OR_ID | 增加平台来源绑定 |
| tc agent folder AGENT NAME [--parent NODE] | 空文件夹 |
| tc agent import AGENT PATH [--name NAME] [--parent NODE] [--exclude PATTERN] [--apply] | 默认预览；纳管正文，嵌套 Git 仓库转索引；exclude 可重复 |
| tc agent collect NODE [--apply] [--prune] | 根据本机来源基准收集；prune 需 apply，才传播安全的来源删除 |
| tc agent export AGENT --to NEWPATH [--bind] | 恢复目录树和仓库清单；bind 将顶层文件夹设为本机后续收集来源 |
| tc agent path NODE | 中央直属正文位置 |
| tc agent find QUERY [--agent AGENT] | 本地 UTF-8 正文搜索，单文件至多 2 MiB；报告未检出的节点 |
| tc agent rename AGENT NAME / rename-node NODE NAME | 更名，身份不变 |
| tc agent move NODE [--agent AGENT] [--parent NODE] | 移动子树；无 parent 为项目根；省略 agent 时由 parent 或原归属推定 |
| tc agent split NODE --name NAME | 子树成为新容器的根文件夹 |
| tc agent merge FROM INTO [--apply] | 默认预览；保留来源分组和旧项目去向 |
| tc agent archive AGENT [--restore] | 归档/恢复，正文保留；合并的撤销使用 Git |
| tc agent remove NODE [--apply] | 默认预览；删除中央子树，apply 前须提交；源文件、仓库索引、逻辑任务不动 |
| tc agent link TARGET --task ID [--task ID] [--remove] | 项目或节点到逻辑任务的多对多关联 |
| tc agent chat AGENT TITLE --source URL_OR_ID [--provider manual] [--parent NODE] [--summary FILE] | 聊天来源及交接摘要 |
| tc agent attach AGENT REPO [--parent NODE] [--name NAME] | 引用已有仓库 |

## 机器与代码仓库

| 命令 | 作用 |
|---|---|
| tc machine alias NAME | 设置本机别名；稳定 ID 不变，可在连接 hub 前设置 |
| tc machine list | 共享机器列表及本机身份 |
| tc repo add NAME [PATH] [--remote URL] | 路径/remote 至少一个；本地注册检测 origin，记录机器位置 |
| tc repo bind REPO PATH | 绑定本机已有仓库；有 origin 时核对远程身份 |
| tc repo remote REPO URL | 更新索引 remote，不改原仓库配置 |
| tc repo status [REPO] / show REPO | 本机 Git 改动、已有上游落后/领先数 / 完整登记 |
| tc repo checkout REPO --to NEWPATH | 从原远程克隆；旧 bundle 可恢复；仅本地路径则提示来源机器 |

## Sync 仓库

| 命令 | 作用 |
|---|---|
| tc hub init PATH [--remote URL\|--github OWNER] [--repo schedule-workspace] | 新建格式 2 的专用工作空间；GitHub 仓库需预先创建 |
| tc hub clone REMOTE NEWPATH [--metadata-only] | 克隆；metadata-only 请求 partial clone 并只检出目录信息，传输节省取决于服务端 |
| tc hub remote URL | 设置 origin |
| tc hub status / diff / check | 离线状态 / 差异统计 / 目录与内容校验 |
| tc hub commit [-m MESSAGE] | 仅提交受管文件，不联网；未知已暂存文件会阻止操作 |
| tc hub select [AGENT_OR_NODE ...] [--all] | 选择项目/文件夹正文；无目标为仅元信息；all 恢复全部 |
| tc hub push [--agents-only] | 提交并推送当前分支；agents-only 不导出本机规划账本 |
| tc hub pull [--agents-only\|--replace] | 先验证 Git 合并；agents-only 不导入本机账本；replace 明确接受中央账本，不替你解决 Git 分叉 |
| tc hub migrate [--apply] | v0.2 迁移预览/执行；旧文件保留在 legacy/v02 |

GitHub 账号只用于生成地址；Git 负责认证。不要在 URL 拼接密码或 token。hub 也支持本地 bare remote，便于离线验证。agents-only 仍推拉当前分支的 Git 历史，不能过滤已提交的其他内容。

旧 `project register/bind/checkout/status/diff/note/snapshot/show` 仅用于未迁移的旧 hub；旧实现行为见 IMPLEMENTATION_V2.md。新格式不再默认制作代码快照。