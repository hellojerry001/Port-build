# 脚手架模板契约

端口管家的「从模板新建」会把一份**模板目录**克隆成一个新项目，注入项目名与端口，再登记进项目列表并一键启动。

## 模板存放位置

```
~/.portbutler/scaffolds/
├── pc/          # PC 端脚手架（zmi + zui · Vue 2）
│   ├── .pb-scaffold.json
│   ├── node_modules/
│   └── ...
└── mobile/      # 移动端脚手架（zmi + mzui · H5）
    ├── .pb-scaffold.json
    └── ...
```

目录名即模板 `key`，会被写进项目的 `scaffold` 字段。加模板 = 往这个目录里放一个文件夹，不需要改代码。

## .pb-scaffold.json

```json
{
  "key": "pc",
  "name": "PC 端脚手架",
  "desc": "zmi + zui · Vue 2 · 门户微应用",
  "portStart": 8100,
  "nodeVersion": "18.16.0",
  "command": "npm run serve"
}
```

| 字段 | 说明 |
|---|---|
| `key` | 模板标识，留空则取目录名 |
| `name` / `desc` | 新建弹窗里卡片上显示的名称与副标题 |
| `portStart` | 端口自动分配的起点（PC 用 8100 段、移动端用 8200 段） |
| `nodeVersion` | 启动该项目时前置到 `PATH` 的 Node 版本；找不到则退回系统默认 |
| `command` | 该模板的启动命令 |

## 占位符

模板里用两个占位符，创建时被替换：

| 占位符 | 替换成 | 出现在 |
|---|---|---|
| `__PB_SLUG__` | 目录名（同时作为 npm 包名 / appId / appKey） | `package.json` 的 `name`、`zmi.config.js` 的 `appId`、`publish.json` 的 `appKey` |
| `__PB_PORT__` | 分配到的端口 | `zmi.config.js` 的 `devServer.port` |

会做替换的文件固定为这三个（不存在则跳过）：`package.json`、`zmi.config.js`、`publish.json`。
三个文件里一个占位符都没找到时，视为「不是本工具生成的模板」并回滚整个新建操作。

## 创建流程

1. 校验目录名（`^[a-z0-9][a-z0-9_-]*$`）与目标目录是否已被占用
2. 挑端口：`portStart` 起递增，跳过**已登记项目**与**系统正在监听**的端口
3. `cp -Rc` 克隆模板（APFS 写时复制 → 连 `node_modules` 一起带 ≈13 秒 / 实测只占 25MB）
   - 跨卷时 `cp -c` 会失败，自动回退到普通 `cp -R`
4. 删掉新项目里的 `dist/` 与 `.DS_Store`
5. 替换占位符（写时复制保证模板本身毫发无损）
6. 写入 `projects.json`，返回新项目；前端随后调用 `start_project`

## 启动时的 Node 版本

`start_project` 用 `zsh -lc <命令>` 起进程。若项目带 `nodeVersion`，命令会被包一层：

```sh
export PATH="$HOME/.local/share/fnm/node-versions/v18.16.0/installation/bin:$PATH"; npm run serve
```

比 `fnm exec` 更快也更确定 —— 不依赖 `fnm` 本体在 PATH 上。查找顺序：
fnm 默认目录 → fnm 的 macOS Application Support 目录 → nvm → `~/.fnm`。

## 维护模板

- 模板是**只读基线**：新建项目改的是副本，模板不受影响。
- 想把某个项目的成果沉淀回模板：手工挑出可复用的部分覆盖 `~/.portbutler/scaffolds/<key>/`，记得**清理业务代码**并检查占位符是否还在。
- 模板里的 `node_modules` 是冻结快照（可复现优先）。需要更新依赖时，在模板目录内跑一次 `npm install`。

## 模板来源

两份模板都由 `~/Desktop/项目设计/` 下的同名脚手架目录剥离业务模块后冻结而来：

| 模板 | 来源 | 剥离掉的内容 |
|---|---|---|
| `pc` | `PC端脚手架` | `views/dispatch`、`views/customer`、`api/dispatch`、`api/customer`、`utils/dispatch.ts`、`assets/icons/{dispatch,analysis}`、`dist` |
| `mobile` | `移动端脚手架` | `views/customer`、`views/lead`、`api/customer`、`dist` |

`pc` 模板保留了 `views/DesignSystem.vue`（设计令牌与 ZUI 组件示例）与 `views/Welcome.vue`。

## 相关测试

```bash
cd src-tauri
cargo test                              # 占位符注入 / slug 校验 / 端口避让 / ~ 展开
cargo test -- --ignored                 # 真机冒烟：用真实模板跑完整 create_project（自动还原 projects.json）
```
