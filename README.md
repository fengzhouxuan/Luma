# Luma

适用于 Windows 的轻量应用启动器。按 **`Alt+Shift+Space`** 唤出，用名称、拼音或首字母搜索应用，再按回车启动。

当前源码版本：**1.6.4**。基于 Tauri 2、React、TypeScript 和 Rust，提供 Liquid Glass 深色玻璃界面、系统托盘与键盘操作。

[下载安装包](https://github.com/fengzhouxuan/Luma/releases) · [更新日志](./CHANGELOG.md) · [反馈问题](https://github.com/fengzhouxuan/Luma/issues)

## 安装与升级

面向 Windows 10 / 11 x64，需要 Microsoft Edge WebView2 Runtime。

1. 从 [Releases](https://github.com/fengzhouxuan/Luma/releases) 选择已发布版本，下载 `.exe` 安装包或 `.msi`。
2. 升级前，通过托盘菜单或「设置 → 退出 Luma」结束旧实例，再运行安装包。
3. 从开始菜单启动 Luma，按 `Alt+Shift+Space` 显示窗口。启动后默认在后台常驻，任务栏没有独立按钮。

EXE 安装包默认安装到当前用户的 `%LOCALAPPDATA%\Luma`，并创建开始菜单快捷方式。WebView2 缺失时，安装包会尝试下载并安装运行时，需要联网。

应用设置保存在 `%APPDATA%\Luma\config.json`。升级前可备份此文件，覆盖安装会继续使用原有设置。源码构建的版本可能早于下一个 GitHub Release 发布；需要该版本时可按下文自行构建。

## 功能

界面按用户指定的 [StyleKit Liquid Glass](https://www.stylekit.top/zh/styles/liquid-glass) 参考适配，统一主窗口、弹层、菜单与表单；Windows 当前采用深色 CSS 玻璃表面，停用会导致四角残留的原生桌面模糊；原生保护区域为 WebView 的抗锯齿边缘留出余量。参考和组件规范见 [界面样式基准](./docs/ui-style.md)。弹层支持独立滚动与焦点约束；外观滑块支持键盘保存，右键菜单支持方向键、Home / End 与 Enter。

- **快速搜索**：支持中文名称、英文、全拼、拼音首字母、分词及模糊输入。精确和前缀匹配优先，同级结果按置顶、启动次数和最近启动时间排序。
- **可靠启动**：普通启动优先使用原始快捷方式，保留参数和工作目录；快捷方式缺失或启动失败时尝试直接启动目标。支持以管理员身份启动。
- **应用发现**：扫描开始菜单、App Paths、32/64 位安装记录、Windows AppsFolder 中的商店/MSIX 应用，以及常见游戏目录和自定义目录。
- **身份去重**：通过程序路径和启动参数区分应用，保留同名程序和同一程序的不同启动配置。
- **分类和置顶**：自动分类、自定义分类、手动调整分类、应用置顶及手动隐藏。
- **杂项过滤**：默认隐藏卸载器、运行库、后台辅助组件和部分附属网页；「显示全部」可查看已发现但被过滤或手动隐藏的条目。
- **应用图标**：优先读取快捷方式配置的图标资源，支持 EXE、ICO、DLL 与资源索引，并在需要时回退到程序图标。
- **托盘管理**：提供显示、刷新、设置、诊断、重启和退出入口，菜单标明运行版本。
- **窗口位置记忆**：按住搜索栏左侧的点阵手柄或顶部空白处移动窗口，自动记住所选显示器及屏内位置，重启后仍恢复。搜索框与应用按钮保持正常操作。
- **显示器断开保护**：偏好显示器断开时临时回到主屏工作区中央，避开任务栏；分辨率或缩放变化时将整个窗口限制在可见工作区。显示器重连后在下次唤醒恢复原位置，已经显示的窗口不会突然跳屏。
- **位置偏好更新**：自动回退不会覆盖原显示器位置；在其他屏幕上主动拖动才会更新偏好。按设备接口标识匹配显示器，屏幕编号变化不影响恢复；更换连接端口可能形成新标识，此时安全回退。
- **应用诊断**：查看来源、路径、快捷方式、参数、工作目录、过滤原因，以及本次运行的扫描、图标和启动记录。
- **手动添加与别名**：点搜索框旁的 `＋` 选择 EXE、`.lnk` 或 ClickOnce 入口，也可拖入一个文件，确认后保存；为任意应用设置搜索别名，支持中文、拼音和首字母。
- **应用操作菜单**：右键打开菜单，可启动、管理员启动、复制路径、打开所在目录及编辑别名、启动参数和工作目录；覆盖启动参数后仍保留原快捷方式图标。
- **配置备份与恢复**：设置中导出 JSON，导入前展示摘要和本机缺失路径。恢复分类、置顶、隐藏、别名、手动应用、启动覆盖、外观、使用记录及扫描目录；保留当前电脑的快捷键、自启和显示器位置。导入前自动备份到 `%APPDATA%\Luma\backups`。
- **应用内更新**：设置中主动检查官方 GitHub Release、查看更新日志。当前用户 EXE 安装版支持下载并校验大小及 SHA-256 后升级，不安装旧版本或预发布版本；其他安装方式可打开官方版本页面。
- **性能统计**：诊断中查看每个扫描来源的最近耗时、发现数量和更新时间，以及本次运行的图标缓存命中、排队时间、提取时间和超时数。

启动次数和最近使用时间只保存在本机配置中，不上传到服务端。

显示器变化处理独立于应用扫描：响应系统显示消息，并每秒核对一次显示器工作区；仅在主动拖动结束时保存位置，不随窗口每一帧移动写入配置。

## 操作

| 操作 | 效果 |
| --- | --- |
| `Alt+Shift+Space` | 显示 / 隐藏窗口，可在设置中更改 |
| 输入应用名称、拼音或首字母 | 搜索应用 |
| `↑` `↓` `←` `→` | 移动选中项 |
| `Enter` 或单击应用 | 普通启动 |
| `Ctrl+Enter` 或右键菜单中的管理员启动 | 以管理员身份启动，可能出现系统确认 |
| 右键应用 | 打开应用操作菜单 |
| 搜索框旁 `＋` 或拖入一个应用文件 | 预览并保存手动应用 |
| `Esc` 或切换到其他窗口 | 隐藏启动器，继续后台运行 |
| 应用右键菜单中的「置顶应用 / 取消置顶」 | 置顶 / 取消置顶 |
| 应用右键菜单中的「查看应用详情」 | 查看详情、诊断和手动隐藏入口 |
| 搜索框右侧刷新按钮 | 强制重新扫描所有来源 |
| 按住搜索栏左侧的点阵手柄或顶部空白处 | 拖动窗口 |
| 底部「应用诊断」 | 查找已发现的应用及其过滤原因 |
| 托盘图标右键 | 打开管理菜单，彻底退出请选「退出 Luma」 |

在设置中可调整唤醒快捷键、玻璃深度和圆角，并添加额外扫描目录。修改应用分类或删除自定义分类会立即更新显示，不触发磁盘扫描。

## 扫描与图标加载

### 应用从哪里来

| 来源 | 发现方式 |
| --- | --- |
| 快捷方式 | 系统已知文件夹中的用户/公共开始菜单与桌面，支持 `.lnk`、ClickOnce `.appref-ms` 和游戏协议 `.url`；兼容桌面重定向 |
| App Paths | 用户及系统的应用路径注册项，读取 32/64 位视图 |
| 安装记录 | 用户及系统的 32/64 位卸载注册项，仅选择可确认的应用 EXE |
| 商店 / MSIX 应用 | Windows AppsFolder 中的注册应用，使用稳定的 AUMID 识别和激活，支持没有 `.lnk` 的 ChatGPT 等应用 |
| 传统注册应用 | AppsFolder 的 Win32 入口，保留参数并通过原 Shell 项启动；补充可见 GUI 文件关联应用 |
| 便携应用 | `%LOCALAPPDATA%\Programs` 的浅层 GUI 程序、WinGet Links、Scoop 的 `current` 目录，支持 `SCOOP` / `SCOOP_GLOBAL` 自定义根目录 |
| 程序目录 | 各盘符根目录的 `game`、`Games`、`游戏` 目录及用户添加的目录，最多扫描 8 层 |

程序目录扫描跳过部分辅助程序和 `lib`、`node_modules` 目录。额外目录建议选择具体应用或游戏目录，避免使用整个磁盘根目录。

安装记录还会在 `bin`、`Application`、`app`、`current` 等已知布局及最新的 Squirrel `app-*` 版本目录中查找同名主程序。完整的安装方式覆盖、已知边界和资料来源见 [扫描覆盖说明](./docs/discovery-coverage.md)。任意目录中、没有任何注册或快捷方式的绿色软件仍需添加额外扫描目录。

### 什么时候更新

- 启动时全量扫描。
- Windows 目录与注册表通知触发更新，通常在变更停止约 500ms 后开始；连续变更时最长约 2 秒触发一轮。仅重扫发生变化的来源组。
- 每 5 分钟全量扫描兜底，覆盖监听不可用、文件变化遗漏等情况。
- 窗口获得焦点时，距上次检查超过 1 分钟则检查更新；没有变更且未到全量扫描时间时复用缓存。
- 手动刷新会强制全量扫描，修改额外目录会刷新对应来源。

扫描在后台串行执行，刷新期间保留当前列表与已有图标。扫描过程中收到新的变更，会再补扫一轮。

### 图标如何加载

前端优先请求可见区域及附近的图标，并限制并发。后端在专用 STA 线程上提取图标，优先使用快捷方式的资源配置，再回退到 EXE。缓存跟踪目标、快捷方式和图标文件的变化，限制为最多 512 项、16MB；成功结果最长缓存 24 小时，失败或降级结果约 30 秒后可重试。

商店/MSIX 应用通过 Windows Shell 获取图标，无需读取受保护的安装目录。此类虚拟项没有普通文件时间戳，图标缓存缩短至 30 秒；包注册变化会触发对应来源增量扫描，并保留定时全量扫描兜底。商店/MSIX 应用使用 Windows 应用激活机制启动，不支持以管理员身份启动。

## 排查问题与当前限制

**搜不到应用**：先点刷新，再在「应用诊断」中搜索名称或路径。若已发现，检查过滤原因和「显示全部」；若仍无记录，可把程序所在目录加到额外扫描目录。

**没有创建过快捷方式**：商店/MSIX 应用从 AppsFolder 发现；普通应用在 App Paths 或安装记录中存在有效主程序信息时仍可发现。未注册的绿色软件可直接手动添加 EXE，或添加实际程序目录。Luma 不会执行卸载命令来尝试发现应用，也不会搜索整个磁盘。

**图标缺失或不正确**：查看诊断中的图标记录，确认快捷方式的自定义图标资源仍然存在。临时失败后会重试，手动刷新可重新检查文件变化。

**无法启动**：查看程序路径、参数、工作目录与最近启动记录。Windows 接受启动请求后，目标应用仍可能自行退出；Luma 的使用记录表示启动请求成功，不代表应用已完成初始化。

**快捷键冲突**：在设置中改用其他组合键。窗口隐藏不代表程序退出；升级或排查多版本问题时，请使用退出菜单，并在设置中核对版本及程序位置。

**开机自启**：新版本通过当前用户的 `HKCU\Software\Microsoft\Windows\CurrentVersion\Run\Luma` 登录启动，无需预先创建任务。首次安装默认不启用；已启用的启动项在安装版下次运行时跟随安装路径。旧版 `LaunchpadElevated` 任务会被识别，设置中的「管理员迁移旧版自启」会发起 Windows 管理员确认：停用旧任务后，由原来的用户进程写入新启动项；取消权限确认时保留旧任务。

**应用更新**：只访问本仓库的正式 Release。下载前重新检查版本和资产信息，限定官方 HTTPS 下载地址；安装包缺少 GitHub SHA-256 摘要、下载大小不符或摘要不符时停止。SHA-256 用于核对与官方 Release 的资产一致性，不替代发布者代码签名。[GitHub 资产摘要说明](https://docs.github.com/en/rest/releases/assets#get-a-release-asset)。

## 本地开发

### 环境

- Windows x64。
- Node.js 22.12+（或满足依赖要求的 Node.js 20.19+）及 npm。
- Rust stable，使用 MSVC 工具链；项目采用 Rust 2024 edition。
- Visual Studio Build Tools 的 C++ 桌面开发组件及 Windows SDK。
- Microsoft Edge WebView2 Runtime。

```powershell
git clone https://github.com/fengzhouxuan/Luma.git
cd Luma
npm ci
npm run tauri dev
```

开发前请退出已运行的 Luma，避免单实例机制或全局快捷键占用影响调试。`npm run dev` 只启动前端开发服务器，完整桌面功能需要 `npm run tauri dev`。

### 测试

在仓库根目录执行：

```powershell
npm test
cargo test --locked --lib --manifest-path src-tauri/Cargo.toml
```

前端测试覆盖搜索排序与图标队列；后端测试覆盖应用身份、来源缓存、真实快捷方式参数及工作目录、快捷方式丢失后的回退、目录与注册表通知、图标资源和缓存。

窗口策略使用模拟显示器拓扑测试断屏回退、重连后下次唤醒恢复、主动拖动覆盖偏好、负坐标、屏幕编号及 DPI/工作区变化。测试不修改系统显示配置；桌面拖动、隐藏唤醒、重启恢复与物理插拔需分别验收。

个人配置测试还覆盖别名搜索排序、启动覆盖保留图标入口、备份格式与版本校验、本机设置保留、自启注册项读写，以及更新降级、地址、大小和摘要校验。自启读写测试使用隔离注册表子项，不修改实际登录启动项。

本机安装记录检查默认跳过，因为它依赖机器上实际安装的应用。可指定一个名称片段单独验证，例如：

```powershell
$env:LUMA_EXPECT_APP = "trae"
cargo test --locked --lib --manifest-path src-tauri/Cargo.toml discovery::tests::host_installation_discovery -- --ignored --nocapture
Remove-Item Env:LUMA_EXPECT_APP
```

商店应用发现和图标验证可将 `LUMA_EXPECT_APP` 设为 `chatgpt`，并将测试名换成 `packaged::tests::host_packaged_discovery_and_icon`。这项检查不会启动 ChatGPT。`packaged::tests::host_packaged_activation` 是单独的可选激活测试，需用 `LUMA_ACTIVATE_APP` 指定完整 AUMID，会实际启动该应用。

### 构建安装包

```powershell
npm run tauri build
```

默认产物：

```text
src-tauri/target/release/luma.exe
src-tauri/target/release/bundle/nsis/Luma_<版本>_x64-setup.exe
src-tauri/target/release/bundle/msi/Luma_<版本>_x64_en-US.msi
```

设置了 `CARGO_TARGET_DIR` 时，产物位于该目录下。首次构建可能需要联网下载 Rust 依赖及打包工具。

## 项目结构

```text
src/
  App.tsx                主界面、分类、配置与扫描协调
  DiagnosticsPanel.tsx   应用详情与诊断
  search.ts              名称、拼音匹配与使用排序
  iconLoader.ts          可见区域图标队列与重试
src-tauri/src/
  lib.rs                 快捷方式和目录发现、热键及窗口管理
  discovery.rs           App Paths 与安装记录发现
  packaged.rs            商店/MSIX 应用发现、Shell 图标与激活
  shortcuts.rs           已知文件夹中的快捷方式、ClickOnce 与游戏协议入口
  portable.rs            用户程序、WinGet 与 Scoop 的 GUI 程序发现
  catalog.rs             来源缓存、去重和增量扫描
  watcher.rs             Windows 目录与注册表通知
  position.rs            窗口位置持久保存、显示器识别与断屏回退
  personal.rs            手动应用、别名、启动覆盖和应用操作
  backup.rs              配置导入预览、校验和恢复前备份
  autostart.rs           当前用户自启项与旧计划任务迁移
  updates.rs             官方 Release 检查、校验下载和安装交接
  launch.rs              快捷方式启动与目标回退
  icons.rs               图标提取工作线程与缓存
  icon_resource.rs       Windows 图标资源提取
  config.rs              本地设置与使用记录
  diagnostics.rs         有界运行记录
  runtime.rs             托盘、重启与退出
scripts/                 前端回归测试
```

发布工作流位于 [`.github/workflows/release.yml`](./.github/workflows/release.yml)，推送 `v*` 标签时构建 Windows 安装包并发布 GitHub Release。版本变更记录在 [CHANGELOG.md](./CHANGELOG.md)。

## 许可证

[MIT](./LICENSE)
