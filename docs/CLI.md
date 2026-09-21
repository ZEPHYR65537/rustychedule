# CLI 指令清单 · v0.2

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
| tc export <PATH> [--force] | 导出便携状态；覆盖已存在文件需 force |
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

## 项目与同步

| 命令 | 作用 |
|---|---|
| tc project register <NAME> <PATH> [--mode reference\|snapshot] [--remote URL] [--description TEXT] | 注册 Git 仓库；自动检测 origin，有可携带远程时 reference，否则 snapshot |
| tc project bind <NAME> <PATH> | 绑定本机已有 Git 仓库，不修改代码 |
| tc project checkout <NAME> --to <NEWPATH> | 引用模式从原仓库克隆，快照模式从 bundle 恢复；只写新目录 |
| tc project status | 所有注册仓库分支、提交、改动数、关联任务及未绑定提示 |
| tc project diff <NAME> | 本机跟踪文件 diff 统计及包含未跟踪文件的状态清单 |
| tc project note <NAME> [--context TEXT\|--context-file PATH] [--memory TEXT\|--memory-file PATH] | 替换指定上下文/长期记忆 |
| tc project snapshot <NAME> | snapshot 模式的已提交 HEAD 历史打包；原代码不变，之后 push 才上传 |
| tc project show <NAME> | 项目说明、记忆和任务列表 |
| tc hub init <PATH> [--remote URL\|--github OWNER] [--repo schedule-workspace] | 初始化空的专用镜像；github 根据账号生成 URL，私有仓库需预先创建 |
| tc hub clone <REMOTE> <NEWPATH> | 克隆既有工作空间；随后 pull 导入任务数据 |
| tc hub remote <URL> | 设置/更换镜像 origin |
| tc hub status | 本机状态是否已变、镜像 Git 更改；不联网 |
| tc hub push | 检查远端、生成索引、提交并推送 main；不推送引用项目代码 |
| tc hub pull [--replace] | 获取并快进镜像，校验、导入状态；双端修改时停止，replace 明确接受远端 |

项目名称使用 1–64 位小写 ASCII 字母、数字、-、_，拒绝路径穿越和 Windows 设备名。引用项目远程支持不带凭据的 HTTPS、git@host:path；本机文件路径仅留在 local.json。hub 支持普通 Git URL 和本地 bare 仓库路径；不要把密码/token 拼进远程 URL，应使用 Git 凭据管理器/SSH。

v0.2 采用单写端交接，不自动合并离线编辑，也不提供自动创建私有仓库、后台同步、模型调用或自动学习。
