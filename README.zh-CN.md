# yourfs

[English](README.md) | **简体中文**

一个用 Rust 编写的**只读、可按文件隔离的 NFS 文件系统**。

`yourfs` 通过 [libnfs] 挂载一个远端 NFS 导出目录，向用户展示的是一棵完全由数据库中
**白名单**（支持 SQLite3、PostgreSQL、Redis）定义的**虚拟目录树**。白名单里没有的文件
对用户完全不可见；被列出的文件还可以用与服务端不同的名字展示。最终得到的是一个只读视图，
你能逐文件精确控制"暴露什么、以什么名字暴露"。

> 本项目是原 C 版 [TechWander/yourfs](https://github.com/TechWander/yourfs) 的 Rust
> 重写，保留了原版行为（含其历史遗留的 license key 机制）。

---

## 工作原理

虚拟目录树由一张三元组表描述：

| 字段       | 含义                                          |
|------------|-----------------------------------------------|
| `parent`   | 条目所在的虚拟目录（`/` 为根）                |
| `name`     | 展示给用户的名字                              |
| `realpath` | 该文件在 NFS 导出中的真实路径                 |

```
          用户看到            白名单行              经 libnfs 读取
   /mnt ─── a.txt  ──────►  (/, a.txt, /a.txt) ──────►  nfs://host/export/a.txt
            （b.txt 不在表中，因此在这里根本不存在）
```

* **`readdir`** 列出某目录的子项（`WHERE parent = ? ORDER BY name`）。
* **`lookup`/`open`/`getattr`** 把虚拟路径解析为 `realpath`，再经 NFS 读取；无匹配行返回 `ENOENT`。
* 挂载为**只读**：任何写操作返回 `EROFS`。

产出两个可执行文件：

* **`yourfs`** —— FUSE 文件系统本体，挂载白名单定义的 NFS 视图。
* **`yourfs-tool`** —— 构建白名单：扫描本地目录树生成文本，再写入 SQLite3 / PostgreSQL / Redis。

## 后端与 `-X` 连接串

| 后端       | `-F` 取值   | `-X` 格式                                                     |
|------------|-------------|---------------------------------------------------------------|
| SQLite3    | `sqlite3`   | `/path/to/data.db`                                            |
| PostgreSQL | `postgres`  | `ip,port,user,passwd,dbname`                                  |
| Redis      | `redis`     | `redis://host:port/db` · `ip,port[,db[,passwd]]` · `host:port` |

Redis 存储方式：每个目录用两个键——有序集合 `{table}|d|{parent}`（成员为子项名，天然按字节序，
服务 `readdir`）与哈希 `{table}|h|{parent}`（`name → realpath`，`O(1)` 解析）。

## 构建

```bash
cargo build --release
```

生成 `target/release/yourfs` 与 `target/release/yourfs-tool`。构建过程会把 `libnfs`
与 `libfuse` **静态内嵌**（见 [`build.rs`](build.rs)），因此运行结果**无需再安装
libnfs/libfuse 包**——剩下的动态依赖只有每个 Linux 都自带的 glibc 基础库。

`YOURFS_DYNAMIC=1` 可强制动态链接；`YOURFS_LIB_PATH=<目录>` 可指定 `.a` 所在目录。

### musl 全静态版（可跑在 musl/Alpine 上）

```bash
./scripts/build-musl-static.sh
# -> target/x86_64-unknown-linux-musl/release/yourfs{,-tool}
```

脚本会用 `musl-gcc` 重新编译 libfuse、libnfs，并把一切链接进**零动态依赖**的单个
ELF（`ldd` 显示 *statically linked*）。目标机上 FUSE 本身的运行前提见
[部署与运行前提](#部署与运行前提)。

## 快速上手

```bash
# 1. 扫描导出目录，生成白名单文本（每行 parent name realpath）
yourfs-tool -s /srv/nfs/export -o list.txt

# 2. 写入 SQLite3 数据库，表名 data
yourfs-tool -F sqlite3 -X /srv/data.db -Z data -i list.txt

# 3. 挂载只读、隔离的 NFS 视图
mkdir -p /mnt/yourfs
yourfs -n nfs://127.0.0.1/srv/nfs/export -m /mnt/yourfs \
       -F sqlite3 -X /srv/data.db -Z data -y FADEP8

ls /mnt/yourfs        # 只有白名单内的条目可见
umount /mnt/yourfs    # 或 fusermount -u /mnt/yourfs
```

## 命令行选项

**`yourfs`**

| 选项 | 含义 |
|------|------|
| `-n <url>` | 要挂载的 NFS 导出，如 `nfs://host/path`（**必填**） |
| `-m <path>` | 本地挂载点（**必填**，隐含只读） |
| `-F <type>` | 白名单后端：`sqlite3` \| `postgres` \| `redis` |
| `-X <db>` | 后端连接串（见上表） |
| `-Z <table>` | 白名单表名 / 键前缀 |
| `-M <MB>` | SQLite 页缓存大小（默认 10） |
| `-w` | 将白名单缓存在内存中（可选开启；仅当挂载期间白名单不会变化时启用） |
| `-V <N>` | 并行打开的 NFS 连接数（默认 4；`1` 关闭 NFS 池化） |
| `-y <key>` | license key（见 [License key](#license-key)） |
| `-?` | 用法 |

另有一组透传给 libfuse 的开关（与 C 版一致）：`-a` allow_other、`-d` direct_io、
`-k` kernel_cache、`-c` auto_cache、`-A`/`-S` 异步/同步读、`-Y` nonempty、`-o`
allow_other+own ids、`-p` 切换 default_permissions、`-t` 切换多线程、`-u/-g` uid/gid、
各类超时等。

**`yourfs-tool`**

| 选项 | 含义 |
|------|------|
| `-s <path>` | 扫描该目录树并写入 `-o` 输出文件；省略则本工具改为写数据库 |
| `-o <file>` | 扫描时的输出文本文件（默认 `output.txt`） |
| `-i <file>` | 写数据库时的输入文本文件（默认 `input.txt`） |
| `-F`/`-X`/`-Z`/`-M` | 同上（写入目标） |
| `-?` | 用法 |

## 部署与运行前提

静态打包省掉了 `libnfs`/`libfuse` 的安装，但运行**任何** FUSE 文件系统仍需目标机具备：

* 已加载 **`fuse` 内核模块**、存在 **`/dev/fuse`** 设备；
* **`fusermount`** 辅助程序（来自 `fuse`/`fuse3` 包）——非 root 挂载需要一个独立的
  setuid 程序，libfuse 会 fork/exec 它，无法打包进 `yourfs` 二进制。

此外：

* 默认（`x86_64-unknown-linux-gnu`）二进制需要 **glibc ≥ 2.30**，可跑在主流 glibc 发行版；
* **musl** 二进制为全静态，可额外跑在 musl/Alpine 上；
* 运行期需能通过网络访问 NFS 服务端。

## License key

与原版 C 项目一致，`yourfs` 挂载前会校验内置 license key（`-y`）。编译时内置了若干带
有效期的 key，默认 `FADEP8` 有效期至 2027 年。这是原样继承的历史功能，**并非真正的访问
控制**——真正的访问控制是白名单数据库。

## 测试说明

`yourfs` 涉及 FUSE、内核与真实 NFS 服务，仅靠单元测试说明不了问题。白名单隔离、符号链接
解析、写入返回 `EROFS`、`statfs`、干净卸载等端到端行为，已在真实本地 NFS 导出上，分别针对
SQLite3 与 Redis 后端、glibc **以及**全静态 musl 的 release 二进制验证通过。

## 设计说明

* **用 FFI 而非纯 Rust crate。** `yourfs` 手写绑定真正的 `libfuse`（2.9 高层 API）与
  `libnfs`，而非用 `fuser`/`nfs-rs`，以与 C 原版保持行为一致，并便于把两个库静态内嵌。
  绑定见 [`src/libnfs.rs`](src/libnfs.rs) 与 [`src/fuse2.rs`](src/fuse2.rs)。
* **并发但不牺牲一致性。** libfuse 高层事件循环会在多个 worker 线程上分发回调（它在
  调用文件系统前会释放内部锁）。因此各后端（SQLite3 / PostgreSQL / Redis）都从一个固定
  容量的小连接池中借出一条连接，而不是在单一互斥锁上排队。默认不缓存任何东西：每次操作
  都读实时数据库，外部新增 / 修改 / 撤销的条目立即生效。
* **NFS 也并行。** `libnfs` 的 context 是单连接且非线程安全的，所以客户端不再用
  一把锁保护单一 context，而是维护一组各自挂载、各自加锁的 context；一次操作跑在任意
  空闲的 context 上。由于 `nfs_open` 返回的句柄绑定于创建它的 context，`pread`/`close`
  会被路由回同一个 context（内部映射跟踪）。锁序严格按“先 map 后 slot”或“先释放 slot
  再取 map”，永不嵌套，因此不会死锁。context 数量可用 `-V <N>` 调节（默认 4；`-V 1` 退回单连接）。
* **可选的白名单缓存（`-w`）。** 开启后缓存已解析的路径与目录列表，重复操作可省掉数据库
  往返——对网络后端是实打实的延迟收益。它**默认关闭**，因为被撤销的行或被改动的 `realpath`
  会继续被缓存命中而失效；仅在能确保白名单在整个挂载期间不变时才开启。
* **修复了一个仅 release 才触发的 FFI 隐患。** NFS 等待回包的循环轮询一个会被 C 回调经
  裸指针改写的字段；`release` 下 LLVM 依据 `&T` 的不可变别名假设把读取提升出循环，导致死循环。
  现已改用 `std::ptr::read_volatile`。见 [`src/nfscli.rs`](src/nfscli.rs)。

## 许可证

GPL-2.0-or-later（与原 C 项目一致）。

[libnfs]: https://github.com/sahlberg/libnfs
