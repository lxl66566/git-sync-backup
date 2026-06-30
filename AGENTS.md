# git-sync-backup

# 行为准则

你是一个资深 Rust 工程师，注重代码可维护性和性能优化，并且遵循 Rust 工程开发的最佳实践。

- 少造轮子，如果有合适的、高性能的第三方库就用
- 少写重复代码，多抽离出可复用的组件，并考虑向后扩展性
  - 你应该使用在编译期就能进行错误检查的设计，而不是推到运行期检查，例如多用枚举，不用硬编码。
- 使用简体中文进行交流，在代码注释和界面语言中使用英文
- 不要删除运行逻辑相关的关键注释
- 不硬性要求单测覆盖率，但是关键部分需要编写单测
- 写出符合工程实践的代码，多复用，注重性能优化。不要为了偷懒写出一些性能差的 naive 实现。

## 项目简介

本项目主要使用 git 对文件夹进行同步或备份。

1. gsb c 或 gsb collect 命令，将所有文件夹同步到当前 git 仓库。
2. gsb r 或 gsb restore 命令，将当前 git 仓库中的所有文件夹同步到本地指定位置。
3. gsb s 或 gsb sync 命令，持续后台运行，每一段固定间隔去 fetch 远程仓库的更新，并将更新文件应用到本地。

### 配置文件格式

配置文件为仓库下根目录的 `.gsb.config.toml` 文件，记录了所有 gsb 的设置与文件对应关系。

gsb 设计为跨设备跨系统与架构使用，同一个文件在不同设备上的存储路径可能不同。
设计上：

- 每个 `[[item]]` 通过 `ops` + 设备表 `[item.device.<id>]` 二维表达「哪些设备
  执行哪些操作」，避免冗长的 `ignore_*` / `restore_*` 等字段。
- `source` 字段支持 `{NAME}` 形式的变量展开（基于 `easy_strfmt`，内置 `{HOME}`、`{DEVICE}`、
  `{DEVICE_ALIAS}`、`{REPO}`，外加 `[vars]` 自定义变量）。
- 设备标识（machine-uid）可在 `[aliases]` 中起别名，所有 `source` / `ops` 等
  位置都允许混用别名与原始 hash。

每一个 path 可以是文件或文件夹。如果是文件，还可以指定 `is_hardlink = true`，
表示仓库中文件与 `path` 位置是硬链接，collect / restore 时不会处理。

```toml
sync_interval = 3600    # gsb sync 同步间隔，单位为秒
version       = "0.4.0" # gsb 版本（用于兼容性检查）

[git]                          # （可选）Git 相关配置
branch = "main"                # （可选, 缺省 "main"）
remote = "origin"              # （可选, 缺省 "origin"）

[aliases]                      # （可选）设备 ID 的 alias，允许混用
main = "ff19a810-c0ea-44b0-b297-f2209a48bfc3"
work = "25f758c0-d868-45ed-95d8-4db9494c8a38"

[vars]                         # （可选）自定义变量
my_root = "/some/root"

# 定义一个需要同步或备份的项。可以有多个 `[[item]]`。
[[item]]
path_in_repo = "test"                 # 项目在仓库中的相对路径
source       = "{HOME}/.config/gsb"   # （可选）所有设备的默认路径
is_hardlink  = true                   # （可选）仓库内文件与 source 是硬链接
ops          = ["collect", "restore"] # （可选, 缺省即此值）该 item 默认操作

# （可选）设备级覆盖。key 可以是别名或原始 hash。
[item.device.main]
source = "D:/Program Files/gsb"
ops    = ["collect"]   # 仅 collect，重要数据保护

[item.device."uuid-xxx"]
ops = []               # 完全跳过
```

#### ops 语义

| `ops` 值                         | 含义                       |
| -------------------------------- | -------------------------- |
| `["collect", "restore"]`（缺省） | 双向：当前设备既收集也恢复 |
| `["collect"]`                    | 仅备份（重要数据保护）     |
| `["restore"]`                    | 仅恢复（首次部署）         |
| `[]`                             | 完全跳过                   |

`ops` 在 TOML 中可以写成单字符串（`ops = "collect"`）或数组
（`ops = ["collect"]`）。

### 交互模式

`gsb collect -i` 与 `gsb restore -i` 启用 interactive，对每个 item 询问
`y/n/a/q`（是 / 否 / 全部是 / 退出）。`gsb sync` 后台模式始终非交互。
