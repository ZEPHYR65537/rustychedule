# Rustychedule · Schedule

Rust 编写的任务与多模型用量管理 CLI。主命令 **schedule**，短命令 **tc**；两者完全等价。当前版本 **0.2.0**。

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

## 项目与跨机器同步

已有代码仓库继续保存代码；工作空间仓库集中保存任务、用量、上下文、长期记忆和原仓库列表。本机目录绑定独立存放。以下 my-project 是注册项目的稳定名称：

~~~sh
tc project register my-project ./my-existing-repo
tc project note my-project --context "当前目标与约束" --memory "长期约定与关键决策"
tc task edit 1 --project my-project
tc project status
tc project diff my-project
~~~

先在自己的 GitHub 账号下创建一个**空的私有** schedule-workspace 仓库，再连接：

~~~sh
tc hub init ../schedule-hub --github YOUR_ACCOUNT
tc hub status
tc hub push
~~~

--github 只根据账号生成地址，不创建 GitHub 仓库、不处理登录。账号名不是认证凭据；认证复用 Git。日常查看离线，只有明确的同步/检出操作联网。

另一台机器：

~~~sh
tc hub clone https://github.com/YOUR_ACCOUNT/schedule-workspace.git ../schedule-hub
tc hub pull
tc project bind my-project ./existing-checkout
# 或从原仓库克隆到新目录：
tc project checkout my-project --to ./new-checkout
~~~

没有原远程仓库的项目默认 snapshot 模式：先在原项目提交代码，再 tc project snapshot NAME，随后 hub push；恢复使用 project checkout。快照包含 HEAD 可达的提交历史，限 90 MiB，不含未提交/忽略文件、其他分支、子模块内容或 LFS 实体。大项目适合 reference 模式。

采用“离开设备前 push，到新设备先 pull”的工作流。双端都改了状态时停止同步，避免静默覆盖；目前不自动合并离线编辑。--replace 明确接受远端状态前，先 export 留下独立备份。

## 文档与实现

- [落地设计与完整工作流](docs/IMPLEMENTATION_V2.md)
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

未来会扩展周容量学习、任务用量预测、自动采集与多设备实体级合并；本版保留观测和复盘资料，尚不实现预测。
