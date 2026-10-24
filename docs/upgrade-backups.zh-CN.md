# 数据库自动副本

[English](upgrade-backups.md)

Wisp 的数据库迁移只能向前执行。为了让出问题的升级可以撤回，Wisp 会在另一个应用版本迁移数据库之前，先为它保留一份副本。Wisp 还会为每个打开的数据库每天保留一份副本；项目数据库被 SQLite 判定为损坏时，会自动从最新的副本恢复。

## 备份什么，何时备份

每个数据库都记录最后一次迁移它的应用版本。当另一个版本（更新或更旧）即将打开它时，Wisp 先复制一份；否则，打开数据库时若最新副本已超过一天，也复制一份：

| 数据库 | 备份时机 |
| --- | --- |
| `wisp.sqlite`（应用数据库） | 启动时 |
| `library.sqlite`（全局资料库） | 启动时 |
| 项目的 `.wisp/project.sqlite` | 该项目第一次被打开时 |

副本写入应用数据目录下的 `backups/`，不会写进项目文件夹：

- Windows：`%APPDATA%\science.wisp-science\wisp-science\backups\`
- macOS：`~/Library/Application Support/science.wisp-science/wisp-science/backups/`
- Linux：`~/.local/share/science.wisp-science/wisp-science/backups/`

文件名格式为 `<数据库>.<UTC 时间>.pre-<版本>.sqlite`，例如 `wisp.20261006-120301.pre-1.18.0.sqlite` 表示 `wisp.sqlite` 在 1.18.0 打开它之前的状态。每日副本的文件名为 `<数据库>.<UTC 时间>.daily.sqlite`。项目库的副本以 `project-<项目 ID>.` 开头，项目 ID 即项目 `.wisp/project.json` 中的 `project_id`。

每个数据库的每类副本各保留最近 3 份。需要释放磁盘空间时可以随时删除这个文件夹。副本写入失败（磁盘已满、目录只读）只会记入日志，不会阻止 Wisp 启动。没有通过 SQLite `quick_check` 的数据库不会被复制，所以损坏的库不会顶掉将来用来恢复它的好副本。

只安装旧版本并不等于回退：旧版本打开的仍是新版本留下的数据库。不过它同样会先保留一份副本，所以新版本的状态也不会丢。

## 项目数据库损坏时自动恢复

打开项目时，如果 SQLite 报告 `.wisp/project.sqlite` 已损坏（`database disk image is malformed`、`file is not a database`，通常发生在应用被强制退出或磁盘断电之后），Wisp 会：

1. 把损坏的文件在同一个 `.wisp` 文件夹里改名为 `project.sqlite.corrupt-<UTC 时间>`，旁边的 `-wal`、`-shm`、`-journal` 一并改名，便于日后手工抢救；
2. 从 `backups/` 里挑最新且能打开的副本复制回原位；没有任何副本时，重建一个仍然登记着该项目的空数据库；
3. 明确提示发生了哪一种情况、损坏文件在哪里；再打开一次项目即可继续使用。

副本生成之后的会话记录不在恢复后的数据库里。工作区文件、`.wisp/artifacts/` 和 git 历史不会被改动。损坏的文件会一直保留，直到你自己删除。

## 手动恢复副本

恢复会丢弃副本生成之后记录的所有内容。下面的步骤不删除任何文件，把文件移回原处即可撤销。

1. 完全退出 Wisp。
2. 在应用数据目录中，把 `wisp.sqlite` 移到其他文件夹；如果旁边有 `wisp.sqlite-wal` 和 `wisp.sqlite-shm`，一并移走。把这两个文件留在恢复后的数据库旁边可能损坏数据库。
3. 把选定的副本从 `backups/` 复制到应用数据目录，并命名为 `wisp.sqlite`。
4. 启动你打算继续使用的 Wisp 版本。

项目数据库的恢复方法相同：在项目的 `.wisp` 文件夹中处理 `project.sqlite`，以及旁边可能存在的 `project.sqlite-journal`、`-wal`、`-shm`。

恢复 `wisp.sqlite` 之后，副本生成之后创建的项目不再出现在列表中。其中自带 `.wisp/project.sqlite` 的项目没有被改动，可通过 **导入项目 → 导入项目文件夹** 重新加入。

放在云盘文件夹并启用快照的项目（参见[项目同步说明](project-sync.zh-CN.md)），实时数据库位于 `project-cache/` 而不在项目文件夹中。它同样会被备份，但目前不支持手动放回副本；请提交 issue，不要直接替换那里的文件。

如果会话不见了但期间并没有升级，请先参考[项目数据库不可用时的安全排查](project-database-recovery.zh-CN.md)。数据完好、只是挂在另一个项目身份下时，副本帮不上忙。
