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
（手工构建 musl target 时 `YOURFS_LIB_PATH` **必须**设置：不设时 `build.rs` 会回退到
共享库，产物并非全静态。）

### musl 全静态版（可跑在 musl/Alpine 上）

```bash
./scripts/build-musl-static.sh
# -> target/x86_64-unknown-linux-musl/release/yourfs{,-tool}
```

脚本会用 `musl-gcc` 重新编译 libfuse、libnfs，并把一切链接进**零动态依赖**的单个
ELF（`ldd` 显示 *statically linked*，且无 `INTERP` 段；`file` 对这种 static-PIE
会误报 "dynamically linked"，以 `ldd`/`readelf` 为准）。目标机上 FUSE 本身的运行
前提见[部署与运行前提](#部署与运行前提)。

### 预编译二进制

GitHub Release 提供两个 x86_64 Linux 压缩包及 `SHA256SUMS.txt`：

* `yourfs-<ver>-linux-x86_64-gnu.tar.gz` —— glibc 版（需 glibc ≥ 2.30）；
* `yourfs-<ver>-linux-x86_64-musl.tar.gz` —— 全静态版，无任何共享库依赖。

包内均为 `yourfs` 与 `yourfs-tool`。

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

**`-n` URL 格式。** libnfs 要求主机名后必须带路径：只写 `nfs://host` 会被拒绝并报
*Incomplete or invalid URL specified*——根导出必须写成 `nfs://host/`。除此之外
**结尾斜杠不是必需的**：`nfs://host/srv/nfs/export` 与带尾斜杠的写法挂载、读取行为
完全一致（已对真实服务端验证）。云 NAS 的挂载点（如
`nfs://xxx.cn-beijing.nas.aliyuncs.com/`）都是根导出，所以才会总带那一个斜杠。

`-X`/`-Z` 指向的数据库必须已经建好（上面第 1–2 步）：`yourfs` 启动时会校验表，
失败即带着明确原因退出，而不是挂出一个每次访问都报 `EIO` 的视图。

## 完整案例

只发布导出目录的一部分、换个名字展示，并且事后随时改主意：

```
NFS 服务端导出 /srv/data                  用户看到 /mnt/yourfs
├── public/welcome.txt          ────►      ├── welcome.txt
├── internal/2025/report.pdf               └── reports/
└── secret/keys.txt（未列入白名单）              └── report.pdf
```

**1. 描述虚拟树**——每条目一行，以空白分隔的 `parent name realpath`
（路径相对导出根目录、以 `/` 开头；`secret/keys.txt` 没有对应行，于是在视图里
彻底不存在）：

```
/          welcome.txt    /public/welcome.txt
/          reports        /internal/2025
/reports   report.pdf     /internal/2025/report.pdf
```

目录也是一行（`/ reports /internal/2025`），子项靠 `parent` 指向它——注意
`parent` 是**虚拟**路径，所以 `/reports` 可以是服务端 `/internal/2025` 的别名
（已实测：`ls` 里 `reports` 显示为目录，`cat` 能直接读到文件）。

**2. 生成白名单**（整棵树的场景先用扫描代替手写第 1 步，再手工裁剪）：

```bash
yourfs-tool -s /srv/data -o list.txt     # 也可以直接从手写文件开始
yourfs-tool -F sqlite3 -X /srv/data.db -Z data -i list.txt
```

> 导入是**整表替换**：先 `DROP TABLE` 再全量重建（Redis 则是 `SCAN`+`DEL
> {table}|*` 后重写）。导入之后在数据库里手工加改的行，下次导入时会被清掉。

**3. 挂载读取**（`-y` 可省略，默认密钥见 [License key](#license-key)）：

```bash
yourfs -n nfs://<server>/srv/data -m /mnt/yourfs -F sqlite3 -X /srv/data.db -Z data
ls /mnt/yourfs        # reports  welcome.txt
```

### 挂载期间修改虚拟目录

每次操作都读实时数据库，所以改表后**下一次 `ls`/打开立即生效**，无需重新挂载
（除非挂载时开了 `-w`：缓存会继续供旧答案，需重启 `yourfs`）。

```sql
-- 立即多发布一个文件
INSERT INTO data(parent,name,realpath) VALUES ('/','keys.txt','/secret/keys.txt');
-- 改用户看到的名字（指向的还是同一个文件）
UPDATE data SET name='welcome-v2.txt' WHERE name='welcome.txt';
-- 下架一个文件
DELETE FROM data WHERE name='keys.txt';
```

Redis 则用：发布 = `ZADD data|d|/ <name>` **加** `HSET data|h|/ <name>
<realpath>`；下架 = 对应的 `ZREM` + `HDEL`。PostgreSQL 同上走 `psql`——但注意
它的 `CHAR` 列比较时会忽略尾部空格，名字/路径末尾不要带空格。

若要大改结构，更稳的做法是改列表文件后重新导入，并选在挂载空载时进行——
导入会先删表，这个窗口期内落到的读操作会报错。

## 命令行选项

**`yourfs`**

| 选项 | 含义 |
|------|------|
| `-n <url>` | 要挂载的 NFS 导出：`nfs://host/path`（**必填**；主机名后必须带路径——根导出写 `nfs://host/`；此外结尾斜杠可省） |
| `-m <path>` | 本地挂载点（**必填**，隐含只读） |
| `-F <type>` | 白名单后端：`sqlite3` \| `postgres` \| `redis` |
| `-X <db>` | 后端连接串（见上表）；sqlite3 时文件必须已存在——`yourfs` 从不建库 |
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
有效期的 key；省略 `-y` 时默认永远取表中**第一个** key，当前为 `FADEP8`，有效期至
2029 年。这是原样继承的历史功能，**并非真正的访问控制**——真正的访问控制是白名单数据库。

## 测试说明

`yourfs` 涉及 FUSE、内核与真实 NFS 服务，仅靠单元测试说明不了问题。白名单隔离、符号链接
解析、写入返回 `EROFS`、`statfs`、断连恢复、干净卸载等端到端行为，已在真实本地 NFS
导出上，对三个后端（SQLite3、PostgreSQL、Redis——后两者经 docker 部署）、glibc
**以及**全静态 musl 的 release 二进制验证通过。

## 设计说明

* **用 FFI 而非纯 Rust crate。** `yourfs` 手写绑定真正的 `libfuse`（2.9 高层 API）与
  `libnfs`，而非用 `fuser`/`nfs-rs`，以与 C 原版保持行为一致，并便于把两个库静态内嵌。
  绑定见 [`src/libnfs.rs`](src/libnfs.rs) 与 [`src/fuse2.rs`](src/fuse2.rs)。
* **并发但不牺牲一致性。** libfuse 高层事件循环会在多个 worker 线程上分发回调（它在
  调用文件系统前会释放内部锁）。因此各后端（SQLite3 / PostgreSQL / Redis）都从一个固定
  容量的小连接池中借出一条连接，而不是在单一互斥锁上排队。默认不缓存任何东西：每次操作
  都读实时数据库，外部新增 / 修改 / 撤销的条目立即生效。
* **NFS 并行 + 请求流水线。** 一个 `libnfs` context 就是一条 TCP 连接且非线程安全，
  因此每个池内 context 由一个专属服务线程（actor 模型）持有：FUSE worker 把操作经
  channel 交给某个 slot 后在自己的私有回执上等待，actor 则把队列里的请求**连续全部
  发出**——多个 READ 因此搭同一个往返完成（10 ms 延迟链路上实测比串行化设计快
  5.3 倍）。由于 `nfs_open` 返回的句柄绑定于创建它的 context，`pread`/`close` 会被
  路由回所属 slot（内部映射跟踪；锁序永不嵌套，无死锁）。actor 采用懒启动：libfuse
  在 `fuse_main` 内部 fork 守护化，fork 前创建的线程不会存活。context 数量可用
  `-V <N>` 调节（默认 4；`-V 1` 即单连接流水线）。
* **可选的白名单缓存（`-w`）。** 开启后缓存已解析的路径与目录列表，重复操作可省掉数据库
  往返——对网络后端是实打实的延迟收益。它**默认关闭**，因为被撤销的行或被改动的 `realpath`
  会继续被缓存命中而失效；仅在能确保白名单在整个挂载期间不变时才开启。
* **修复了一个仅 release 才触发的 FFI 隐患。** NFS 等待回包的循环轮询一个会被 C 回调经
  裸指针改写的字段；`release` 下 LLVM 依据 `&T` 的不可变别名假设把读取提升出循环，导致死循环。
  现已改用 `std::ptr::read_volatile`。见 [`src/nfscli.rs`](src/nfscli.rs)。

## 许可证

GPL-2.0-or-later（与原 C 项目一致）。

[libnfs]: https://github.com/sahlberg/libnfs
