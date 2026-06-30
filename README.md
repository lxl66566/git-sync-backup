# git-sync-backup

简体中文 | [English](./README-en_US.md)

`git-sync-backup` (gsb) 是一个命令行工具，旨在利用 Git 对本地文件夹进行同步和备份。它支持跨设备和跨系统使用，并提供了灵活的配置选项来管理文件路径和同步行为。

## 安装

- 通过 [cargo-binstall](https://github.com/cargo-bins/cargo-binstall) 安装：
  ```bash
  cargo binstall git-sync-backup --git https://github.com/lxl66566/git-sync-backup
  ```

## 使用

1. 运行 `gsb d` 命令，获取当前设备的设备 id。
2. 创建仓库并在仓库根目录下编写 `.gsb.config.toml` 文件。
3. 运行 `gsb c` 命令，将所有指定内容同步到当前 Git 仓库。

所有命令：

1. **`gsb c` / `gsb collect`**：将配置的所有文件/文件夹同步到当前 Git 仓库。
   - `-a` / `--autocommit`：收集完成后自动 git 提交。
   - `-i` / `--interactive`：交互模式，对每个 item 询问 `y/n/a/q`。
2. **`gsb r` / `gsb restore`**：将当前 Git 仓库中的所有文件/文件夹同步到本地指定位置。
   - `-i` / `--interactive`：交互模式，对每个 item 询问 `y/n/a/q`。
3. **`gsb s` / `gsb sync`**：在后台持续运行，每隔固定时间间隔从远程仓库 `fetch` 更新，并将更新的文件应用到本地。
4. **`gsb d` / `gsb device`**：输出当前设备的设备标识。用于填写配置文件中的 `aliases`、`sources`、`ops` 等字段。

## 配置文件格式

`gsb` 的配置文件为仓库根目录下的 `.gsb.config.toml` 文件。它记录了所有 `gsb` 的设置以及文件和文件夹的对应关系。

```toml
version       = "0.4.0"     # gsb 版本（用于兼容性检查）
sync_interval = 3600        # gsb sync 同步间隔，单位为秒

[git]                       # （可选）Git 相关配置
branch = "main"             # （可选, 缺省 "main"）gsb sync 时使用的分支名
remote = "origin"           # （可选, 缺省 "origin"）gsb sync 时使用的远程仓库名

[aliases]                   # （可选）设备 ID 的别名，允许在配置中混用别名与原始 hash
main = "ff19a810-c0ea-44b0-b297-f2209a48bfc3"
work = "25f758c0-d868-45ed-95d8-4db9494c8a38"

[vars]                      # （可选）自定义变量，可在 `source` 路径中用 {name} 引用
my_root = "/some/root"

# 定义一个需要同步或备份的项。可以有多个 `[[item]]`。
[[item]]
path_in_repo = "test"                 # 项目在仓库中的相对路径
source       = "{HOME}/.config/gsb"   # （可选）所有设备的默认路径，支持变量展开
is_hardlink  = true                   # （可选）仅文件：表示仓库内文件与 source 是硬链接，跳过拷贝
ops          = ["collect", "restore"] # （可选, 缺省即此值）该 item 默认参与的操作

# （可选）针对特定设备的覆盖。key 可以是别名或原始 hash。
[item.device.main]
source = "D:/Program Files/gsb"       # 覆盖该设备的 source 路径
ops    = ["collect"]                  # 该设备只 collect，不 restore（重要数据保护）

[item.device."uuid-xxx"]
ops = []                              # 该设备完全跳过此 item
```

### `ops` 字段

每个 item 通过 `ops` 表达「该 item 默认参与哪些操作」：

| `ops` 值                         | 含义                           | 适用场景           |
| -------------------------------- | ------------------------------ | ------------------ |
| `["collect", "restore"]`（缺省） | 双向：当前设备既收集也恢复     | 普通同步文件       |
| `["collect"]`                    | 仅备份：当前设备只收集，不恢复 | 重要数据单向保护   |
| `["restore"]`                    | 仅恢复：当前设备只恢复，不收集 | 首次部署、模板分发 |
| `[]`                             | 完全跳过                       | 临时禁用           |

可在 `[item.device.<id>]` 中为特定设备单独覆盖 `ops` 与 `source`。所有 key
都支持别名与原始 hash 混用。

### 变量

`source` 字段支持 `{NAME}` 形式的变量展开（基于 `easy_strfmt`），方便跨设备使用同一份配置：

- **内置变量**：
  - `{HOME}` —— 用户主目录
  - `{DEVICE}` —— 当前设备 ID
  - `{DEVICE_ALIAS}` —— 当前设备在 `[aliases]` 中的别名
  - `{REPO}` —— gsb 仓库根目录的绝对路径
- **自定义变量**：在 `[vars]` 表中声明，例如 `my_root = "/some/root"`，则
  `{my_root}` 即可被解析。

使用 `{{` 与 `}}` 转义字面量花括号。未识别的变量会报错；跨设备的差异化路径
建议通过 `[item.device]` 显式覆盖，而不是依赖每台设备上不同的 `[vars]`。

### 交互模式

`gsb collect -i` 与 `gsb restore -i` 启用交互模式，对每个待处理的 item
逐个询问：

```
collect 'test' (/home/u/.config/gsb)? [y/n/a/q/?]
```

- `y` —— 处理此 item
- `n` —— 跳过此 item
- `a` —— 对剩余所有 item 都选是
- `q` —— 立即中止整个流程

## 内置加密（可选）

gsb 内置了基于 [git-simple-encrypt](https://github.com/lxl66566/git-simple-encrypt) 的透明加密支持。启用后：

- **collect** 时自动加密仓库中的文件（仓库始终存储密文）
- **restore** 时自动解密 → 拷贝到本地 → 重新加密（本地始终是明文，仓库始终是密文）
- 加密列表复用 `git_simple_encrypt.toml` 中的 `crypt_list`，与 git-se CLI 完全兼容
- 仅对**同时在 gsb items 和 `crypt_list` 中**的文件/文件夹执行加解密

### 快速开始

1. 在仓库根目录创建 `git_simple_encrypt.toml`：
   ```toml
   use_zstd = true
   zstd_level = 15
   crypt_list = ["secrets", "sensitive_config.txt"]
   ```
2. 设置加密主钥（会写入 git config）：
   ```sh
   git config --local git-simple-encrypt.key "your_password"
   # 或使用 git-se CLI：
   git-se p
   ```
3. 正常使用 `gsb c` / `gsb r`，加密/解密自动完成。

> **注意**：如果密钥未配置，gsb 会跳过加解密并输出 warning，不影响 collect/restore 的正常执行。
