# Rustychedule · 统筹

一个用 Rust 编写的离线任务管理 CLI。命令名为 **`tongchou`**。

同时回答三个问题：**现在先做什么？把剩余工作交给哪个模型？各模型还剩多少可以分配的额度？**

每个模型有自己的额度，绝不把不同模型的余额相加。任务可以声明模型偏序；高优先级任务先分配偏好的模型，额度不足时分段降级。任务进度与 token 消耗独立记录。工具不会调用模型 API，也不会实际兑换服务商的重置卡。

## 安装与快速体验

需要 Rust 1.85+；本次在 Windows / Rust 1.94 上构建和测试。CI 同时配置 Windows、Linux、macOS。

```sh
git clone https://github.com/ZEPHYR65537/rustychedule.git
cd rustychedule
cargo install --path . --locked

# 用独立目录体验；所有示例价格和额度都是虚构的
tongchou --data .demo init --demo
tongchou --data .demo
tongchou --data .demo matrix
tongchou --data .demo plan --minutes 240
tongchou --data .demo plan --minutes 240 --commit
tongchou --data .demo report
```

也可以不安装，直接 `cargo run -- --help`；编译好的 Windows 程序在 `target/release/tongchou.exe`。Windows Terminal 等支持 UTF-8 的终端显示最佳；用 `--json` 可获取完整、未经表格截断的内容。

## 核心能力

| 需求 | 实现 |
|---|---|
| 重要性、紧迫性 | 1–5 级、截止日期自动提升紧迫性、四象限、可解释优先分 |
| 任务统筹 | 项目、标签、搜索、状态、依赖 DAG、工作分钟数、截止时间提示 |
| 多模型偏好 | 每个任务独立的偏序 DAG、传递关系、互不可比模型、允许列表、能力底线 |
| 强模型用完后降级 | 同一任务分段分配；模型专属 token 倍率；可禁止分段 |
| 额度 | 每模型独立；同一模型可以同时有短周期、周、滚动或手动窗口 |
| 预算 | 按任务/模型预留；实际用量抵扣；完成/取消释放；可重规划剩余预算 |
| 手动用量 | 输入、输出、缓存输入、费用、折算额度；用已用百分比校准 |
| 重置 | 自然重置按时间计算；Tibo 记录随机刷新；reset 卡管理库存、范围、有效期 |
| 可视化 | CLI 四象限、额度进度条、工作时间表、每模型每日趋势 |
| 数据 | 本地 JSON、文件锁、原子替换、上一个版本备份、完整导入导出、撤销错误流水 |

## 建立正式数据

省略 `--data` 时使用系统用户数据目录：Windows 为 `%APPDATA%/tongchou`；其他系统使用 `$XDG_DATA_HOME/tongchou` 或 `~/.local/share/tongchou`。也可设置环境变量 `TONGCHOU_DATA`。`doctor` 显示实际位置。

下面的额度和能力等级只是演示，不代表任何服务商的真实套餐：

```sh
tongchou init --currency CNY
tongchou model add chatgpt --provider OpenAI --capability 5
tongchou model add fallback --provider custom --capability 3
tongchou model add local --provider local --capability 2

tongchou quota add chatgpt short --limit 100000 --period 5h
tongchou quota add chatgpt week --limit 500000 --period 7d
tongchou quota add fallback daily --limit 200000 --period 1d
tongchou quota add local rolling --limit 300000 --kind rolling --period 24h
```

固定周期默认从添加窗口时起算。知道真实重置时间时，在 `quota add` 中指定 `--next-reset '2026-10-01T18:00:00+08:00'`。`--kind manual` 不带 `--period`，仅在手动登记重置时刷新。周期是固定秒数，`1d` 指 24 小时，**不是**会随夏令时变化的当地日历日。

**未配置窗口的模型按不受限处理**，规划中会显示提示。各模型的定价、能力等级和 token 权重均由用户配置；不存在内置“某品牌永远更强”的判断。

## 模型偏序与降级

```sh
tongchou task add '完成重要研究报告' --importance 5 --urgency 4 --minutes 90 --input 24000 --output 8000 --capability 2 --models chatgpt,fallback,local --prefer 'chatgpt>fallback' --prefer 'chatgpt>local' --factor fallback=1.5 --factor local=2
```

这表示：

- `chatgpt` 优于另外两个模型；`fallback` 与 `local` **互不可比**。不会凭空推导二者的优劣。
- `--capability 2` 是最低能力要求。模型即使便宜、有额度，也不能突破这个硬约束。
- `--input/--output` 是**整个任务的基准 token 估计**；`fallback=1.5` 表示同样剩余工作估计需要 1.5 倍 token。未指定的模型倍率为 1。
- 默认允许分段；例如先让强模型完成一部分，再让较弱模型接手。加 `--no-split` 要求剩余工作由一个模型承担。
- 偏序最高层有多个可用模型时，依次按估算费用、能力等级升序、名称选择，并在输出中解释。希望固定质量顺序时，请显式补充偏序边。

偏好形成环、依赖形成环、引用不存在的模型或任务时，整个修改会被拒绝，原数据不变。

```sh
# 查看/更改任务；ID 使用添加命令返回的编号
tongchou task list --project research
tongchou task show 1
tongchou task edit 1 --progress 40 --minutes 50
tongchou task edit 1 --prefer 'chatgpt>fallback' --prefer 'fallback>local'
tongchou task status 1 doing
tongchou task status 1 blocked --note '等待实验结果'
tongchou task status 1 done
```

`task edit --prefer` 替换全部偏序边，`--factor` 替换全部倍率。清空选项有 `--clear-preferences`、`--clear-factors`、`--clear-depends`、`--clear-due`，取消模型限制用 `--all-models`。取消任务用 `task status ID cancelled`，保留历史；重新打开用 `todo` 或 `doing`。完成会把进度设为 100%，重新打开时按需重新设置进度和剩余分钟数。

## 每日协作循环

```sh
# 1. 校准模型面板上看到的已用量，百分比指“已用”
tongchou usage reconcile chatgpt short --percent 80

# 2. 预览并提交今天/这次工作的计划
tongchou plan --minutes 180 --reserve-percent 10
tongchou plan --minutes 180 --reserve-percent 10 --commit

# 3. 工作后分别登记真实消耗和实际进度
tongchou usage log chatgpt --task 1 --input 5000 --output 1200 --cached 2000 --progress 25
tongchou task edit 1 --minutes 70

# 4. 强模型额度变化，重新分配剩余工作
tongchou usage reconcile chatgpt short --percent 100
tongchou plan --minutes 180 --rebalance
tongchou plan --minutes 180 --rebalance --commit
```

**用掉 25% 的 token 不等于完成 25% 的任务。** `--progress` 是手动确认的绝对完成百分比，而非本次增量；如果未提供，记用量不会改变进度。`--minutes` 是剩余工作时间，不会随进度自动缩短。实际费用与原始 token 统计不会因为重规划而消失。

默认规划保留已有预留，只补足不足部分。`--rebalance` 在工作副本中释放所选范围内所有未用预算，再重新安排；仅有 `--commit` 才保存。**被暂缓或阻塞的范围内任务也会释放旧预留。** `--project` 可以缩小范围，范围外预算仍占用额度。

任务按优先级、依赖、时间和额度进行贪心排程；只有剩余任务能够完整安排时才接受该任务。试分配后仍不足会全部回滚该任务的新增预算，再尝试后续任务。因此一个尚不能完整安排的大任务不会吞掉所有额度。它不是全局最优求解器，也不预支未来的自然重置或偶然刷新。

## 预算和用量

```sh
tongchou budget set 1 chatgpt --input 12000 --output 3000
tongchou budget list --task 1
tongchou budget release 1 --model chatgpt
tongchou usage list --model chatgpt --last 30
tongchou usage void 12
tongchou report --days 14
```

`budget set` 设置的是**现在起的剩余预留**，同一任务/模型的旧预算会被替换。超配默认拒绝，只有显式 `--force` 才保留超配记录。超配会在总览警示，默认规划要求先调整或重规划。

缓存输入包含在 `--input` 内，不能重复计入总输入。手填费用使用 `--cost`，模型价格使用 `model add/edit --input-price/--output-price/--cached-price`，单位是当前统一货币/百万 token。不提供价格则按 0 估算；程序不做汇率换算。

供应商额度不是 token 时，可配置本模型的额度换算：

```sh
tongchou model add point-model --unit points --input-weight 0.001 --output-weight 0.003
tongchou quota add point-model week --limit 1000 --period 7d
tongchou usage log point-model --input 1000 --output 500 --units 4.7
tongchou usage reconcile point-model week --used 630
```

权重必须由用户估计；不能从面板百分比推导真实 token 兑换率。`--units` 覆盖这条流水消耗的本模型额度；原始 token 和费用仍独立保留。

校准是当前窗口的权威快照，覆盖此前的额度估计，不抹去历史 token/费用。只校准一个窗口时，其他窗口不变。滚动窗口无法从总数知道每笔到期时间，快照按整批在一个周期后到期做保守估计并标 `~`。

历史补记可用 `usage log --at '2026-09-21T09:30:00+08:00'`；拒绝未来用量。补记时间早于快照时，快照继续作为额度基线；早于预算创建时间的用量不抵扣该预算。`usage void` 保留审计记录、重新计算统计和预算，但不会撤回手动修改的进度，也不会改写之后的权威快照。刷新流水不可撤销，可用新校准纠正。

## 自然重置、Tibo、reset 卡

| 类型 | 如何处理 |
|---|---|
| 自然固定周期 | 根据周期边界自动计算，不需要后台进程；不累计没用完的额度 |
| 滚动窗口 | 每条用量分别在一个周期后释放 |
| Tibo | 不可预测的外部自动刷新，观察到后登记；不扣卡，也不提前当成可用额度 |
| reset 卡 | 手动登记库存，可限定模型、窗口和有效期；登记一次兑换消耗一次机会 |

```sh
# 观察到 Tibo 刷新，仅刷新发生变化的模型/窗口
tongchou reset --models chatgpt --windows short --source tibo --note '面板已刷新'

# 登记卡；使用返回的 ID 兑换，下面假设返回 15
tongchou credit add --count 2 --models chatgpt --expires '2026-12-31' --note '获赠 reset 卡'
tongchou credit list
tongchou reset --models chatgpt --credit 15
```

不填 `--windows` 会清零所选模型的所有窗口；可用逗号选择多个模型，但它们仍独立清零、独立计费。卡的范围必须覆盖全部目标，一次命令消耗一张卡。默认保留原自然重置时间；只有确知时钟重新起算时才加 `--restart-clock`。刷新不会释放任务预留，不会清零历史累计用量或费用。

## 可视化与脚本接口

```text
模型/窗口       用量 / 预留           已用%  已用+预留 / 限额
chatgpt/short   [██████████░░░·····]  56%    56k+18k / 100k token
fallback/day   [██░░░░░···········]  10%    20k+60k / 200k token
```

`matrix` 为四象限；`plan` 显示时间安排、各段模型和降级原因；`report` 每个模型分别显示近期统计和每日字符趋势。所有命令接受全局 `--json`。正常运行退出码为 0，参数或操作失败为 2，操作错误 JSON 写入标准错误；命令行解析错误保留 clap 的标准错误格式。

```sh
tongchou --json task list
tongchou --json quota list
tongchou export backup.json
tongchou import backup.json --replace
tongchou doctor
```

修改会先校验、再原子替换 `state.json`，并保留上一次可用文件 `state.json.bak`。文件损坏时拒绝默默覆盖，可用 `import BACKUP --replace` 恢复；恢复时损坏原件保存为 `state.json.corrupt`，不会破坏上一份可用备份。备份包含任务和备注，按个人数据保管。执行过程中持有目录锁，第二个写入/读取进程会快速失败，避免读到半次事务。未产生改动的预览不会重写数据。

## 设计与开发

- [数学模型、偏序、降级和重规划](docs/MATHEMATICS.md)
- [设计决策及用户约束](docs/DECISIONS.md)
- [数据格式与架构](docs/ARCHITECTURE.md)
- [迭代路线](docs/ROADMAP.md)
- [版本记录](CHANGELOG.md)

```sh
cargo fmt --check
cargo test --locked
cargo clippy --locked --all-targets -- -D warnings
cargo build --release --locked
```

依赖 API 参考：[clap 参数解析](https://docs.rs/clap/latest/clap/)、[chrono 时间处理](https://docs.rs/chrono/latest/chrono/)、[fs2 文件锁](https://docs.rs/fs2/latest/fs2/trait.FileExt.html)。项目采用 MIT 许可证。
