# Rustychedule · Schedule

Rust 编写的任务与多模型用量管理 CLI。主命令 **schedule**，短命令 **tc**；两者完全等价。当前版本 **0.3.0**。

按重要性、紧迫性、截止时间和依赖安排任务；按用户指定的模型偏序分配工作。每个模型分别维护订阅额度、API token 上限、已用量和任务预留。只管理用量，不记录金额、单价或费用，也不调用模型或替用户购买额度。

## 安装与体验

需要 Rust 1.85+。项目同步另需 Git，并提前配置 Git 的 HTTPS 凭据或 SSH 认证。

~~~sh
git clone https://github.com/ZEPHYR65537/rustychedule.git
cd rustychedule
cargo install --path . --locked

tc --data .demo init --demo
tc --data .demo
tc --data .demo matrix
tc --data .demo plan --minutes 240
~~~

也可以使用 cargo run -- --help。旧命令 tongchou 保留兼容；Windows、Linux、macOS 均配置持续集成。UTF-8 终端显示中文表格最佳；任何命令加 --json 可取得完整机器可读数据。

## 十分钟建立自己的计划

下面的模型名、能力和数字仅为用户配置示例，不代表任何厂商套餐或官方模型排名。

~~~sh
tc init
tc model add strong --capability 5
tc model add fast --capability 3
tc subscription set strong --weekly-tokens 1000000 --renewal-day 15
tc subscription set fast --weekly-tokens 500000
tc api set strong --limit 50000
tc api set fast --limit 0
tc policy models --prefer 'strong>fast'
tc policy funding model-first

tc task add "完成项目方案" -i 5 -u 4 --minutes 90 --input 40000 --output 10000 --factor fast=1.5
tc plan --minutes 180
tc plan --minutes 180 --commit
~~~

第一条任务通常是 #1，但应以命令返回的 ID 为准；任务、预算、流水、卡、复盘共用 ID 序列。

**API 的 --limit 是累计允许使用的 token 上限，不是剩余余额，也不是每次规划可重复领取的额度。** 例如 strong 上限 50000、实际已用 12000、已预留 8000，最多还能新分配 30000。上限为 0 直接禁用 API；订阅不足时降级。fast 的额度不能补给 strong。默认规划额外保留 10% 安全余量；--reserve-percent 0 才可分配到上限。

| 策略 | 用量选择顺序 |
|---|---|
| subscription-first（默认） | 优先可用订阅；其间按模型偏序选择，订阅候选耗尽后才用各模型自己的 API |
| model-first | 优先偏好的模型：本模型订阅 → 本模型 API 剩余额度 → 下一可用模型 |
| subscription-only | 只用订阅，API 配置保留但不参与规划 |

对于禁止跨模型分段的任务，只在能够完整覆盖剩余工作的模型中选择；同模型订阅和 API 可以共同承担。模型能力底线和允许列表始终是硬约束。

## 每天怎么用

~~~sh
tc                         # 任务、模型额度条、续期提示、卡、今日工作分钟
tc matrix                  # 重要性 × 紧迫性四象限
tc usage reconcile strong week --percent 70
tc work log 1 --minutes 45 --model strong --source subscription --input 18000 --output 2000 --done "完成需求拆分" --learned "发现两项接口约束" --next "验证方案" --progress 40
tc task edit 1 --minutes 60
tc work summary 1
tc plan --rebalance --commit
tc report --days 7
~~~

面板的百分比只是一条观测。订阅周 token 总量是可调整的估计；tc subscription estimate strong 1200000 更新估计，保留原始百分比。工作复盘和实际用量关联，只扣一次；实际耗费不自动等于进度。--minutes 是剩余工作时间，需要自己调整。

~~~sh
tc credit add --models strong --windows week --count 1 --expires 2026-12-31
tc credit list
tc reset --models strong --windows week --credit 7
tc reset --models strong --windows week --source tibo
~~~

卡 ID 7 只是示例，先查看实际 ID。reset 仅登记外部已经发生的刷新；卡扣一次库存，Tibo 不扣卡。自然周重置自动按时间计算。所有订阅重置都不补充 API、不删除历史或未用预留。不能把尚未发生的 Tibo 计入计划。

## Agent 项目与跨设备同步

**逻辑任务负责规划，agent 项目负责工作容器和资料。** 两者可多对多关联，也可独立使用。一个 agent 项目 A 可以有 X/Y/Z 多个文件夹；文件夹内还可以有子目录、聊天摘要和多个代码仓库。

先在自己的 GitHub 下创建空的私有 `schedule-workspace` 仓库，配置 Git 认证，再执行：

~~~sh
tc hub init ../schedule-hub --github YOUR_ACCOUNT
tc machine alias 台式机
tc agent create A --provider codex
tc agent import A ./X                  # 预览
tc agent import A ./X --apply
tc agent import A ./Y --apply
tc agent folder A Z
tc agent tree A
tc hub check
tc hub diff
tc hub push
~~~

无独立 Git 仓库的工作文件由 CLI 托管，不必先判断哪些是记忆。遇到代码仓库时只保存索引：有 GitHub/可携带远程就记录地址，否则记录机器别名和本地路径。**本地路径索引不是代码备份。** 代码仓库内部未推送的资料不会随中央索引上传。

~~~sh
tc repo add code ./existing-repo
tc agent attach A code --parent X
tc repo status
tc agent collect X --apply              # 来源/中央双边修改会停止
tc agent link A --task 1                # 可选关联，不改变任务或用量
~~~

另一台设备：

~~~sh
tc hub clone https://github.com/YOUR_ACCOUNT/schedule-workspace.git ../schedule-hub --metadata-only
tc hub pull --agents-only               # 不导入本机规划账本
tc hub select A                        # 或选择某个文件夹
tc machine alias 笔记本
tc agent export A --to ./A --bind       # 恢复原目录结构；代码只列入口
tc repo checkout code --to ./A/X/code   # 按需从原仓库克隆
# 如果还需要逻辑任务与用量，再执行 tc hub pull
~~~

支持重命名、移动、拆分、保留来源分区的合并、归档与节点删除；结构与正文使用普通 Git 文件维护。`hub commit` 本地保存，`hub push/pull` 同步当前分支。分支合并先在临时工作树检查，文本或结构冲突不会覆盖原工作树。规划账本保持独立冲突保护，不自动拼接用量记录。

旧版工作空间先用 `tc hub migrate` 预览、`tc hub migrate --apply` 升级，再提交推送。旧文件和 bundle 保存在 `legacy/v02`，其他旧设备直接 pull 跟随中央身份。新格式使用 `agent/repo`，旧 `project` 命令仅供旧格式兼容。

完整工作流、排除规则、机器定位、按需同步和迁移见 [Agent 工作空间使用指南](docs/AGENT_WORKSPACES.md)。

## 文档与实现

- [v0.3 落地设计与验收约束](docs/IMPLEMENTATION_V3.md)
- [Agent 工作空间使用指南](docs/AGENT_WORKSPACES.md)
- [CLI 指令清单](docs/CLI.md)
- [维护的数据结构与存储布局](docs/DATA_MODEL.md)
- [数学定义、资源约束与降级算例](docs/MATHEMATICS.md)
- [长期设计决策](docs/DECISIONS.md)
- [模块架构](docs/ARCHITECTURE.md)、[验证记录](docs/VALIDATION.md)、[后续计划](docs/ROADMAP.md)

规划是确定性的优先级贪心，默认仅预览，不声称全局最优；--commit 才保存预算。--rebalance 会释放所选范围的未用预留再规划，历史消耗保持原模型/来源。

数据目录：Windows 为 %APPDATA%/schedule；其他平台优先 $XDG_DATA_HOME/schedule，再使用 ~/.local/share/schedule。--data 或 SCHEDULE_DATA 可覆盖；自动兼容旧 tongchou 数据目录和 TONGCHOU_DATA。v1 数据读取时迁移，下一次保存保留旧文件为 state.json.bak。备份含个人工作内容，请使用个人位置保存。

~~~sh
tc doctor
tc export ./schedule-backup.json
tc import ./schedule-backup.json --replace
cargo fmt --check
cargo test --locked
cargo clippy --locked --all-targets -- -D warnings
~~~

export/import 只备份规划账本；agent 元信息与正文由独立 sync Git 仓库备份。未来扩展周容量学习、任务用量预测和自动采集，本版尚不实现预测。
