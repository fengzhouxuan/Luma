# Luma

适用于 Windows 的轻量应用启动器。按 **`Alt+Shift+Space`** 唤出，用名称、拼音或首字母搜索应用，再按回车启动。

当前源码版本：**1.2.1**。基于 Tauri 2、React、TypeScript 和 Rust，提供深色半透明界面、系统托盘与键盘操作。

[下载安装包](https://github.com/fengzhouxuan/Luma/releases) · [更新日志](./CHANGELOG.md) · [反馈问题](https://github.com/fengzhouxuan/Luma/issues)

## 安装与升级

面向 Windows 10 / 11 x64，需要 Microsoft Edge WebView2 Runtime。

1. 从 [Releases](https://github.com/fengzhouxuan/Luma/releases) 选择已发布版本，下载 `.exe` 安装包或 `.msi`。
2. 升级前，通过托盘菜单或「设置 → 退出 Luma」结束旧实例，再运行安装包。
3. 从开始菜单启动 Luma，按 `Alt+Shift+Space` 显示窗口。启动后默认在后台常驻，任务栏没有独立按钮。

EXE 安装包默认安装到当前用户的 `%LOCALAPPDATA%\Luma`，并创建开始菜单快捷方式。WebView2 缺失时，安装包会尝试下载并安装运行时，需要联网。

应用设置保存在 `%APPDATA%\Luma\config.json`。升级前可备份此文件，覆盖安装会继续使用原有设置。源码构建的版本可能早于下一个 GitHub Release 发布；需要该版本时可按下文自行构建。

## 功能

- **快速搜索**：支持中文名称、英文、全拼、拼音首字母、分词及模糊输入。精确和前缀匹配优先，同级结果按置顶、启动次数和最近启动时间排序。
- **可靠启动**：普通启动优先使用原始快捷方式，保留参数和工作目录；快捷方式缺失或启动失败时尝试直接启动目标。支持以管理员身份启动。
- **应用发现**：扫描开始菜单、App Paths、32/64 位安装记录、Windows AppsFolder 中的商店/MSIX 应用，以及常见游戏目录和自定义目录。
- **身份去重**：通过程序路径和启动参数区分应用，保留同名程序和同一程序的不同启动配置。
- **分类和置顶**：自动分类、自定义分类、手动调整分类、应用置顶及手动隐藏。
- **杂项过滤**：默认隐藏卸载器、运行库、后台辅助组件和部分附属网页；「显示全部」可查看已发现但被过滤或手动隐藏的条目。
- **应用图标**：优先读取快捷方式配置的图标资源，支持 EXE、ICO、DLL 与资源索引，并在需要时回退到程序图标。
- **托盘管理**：提供显示、刷新、设置、诊断、重启和退出入口，菜单标明运行版本。
- **应用诊断**：查看来源、路径、快捷方式、参数、工作目录、过滤原因，以及本次运行的扫描、图标和启动记录。

启动次数和最近使用时间只保存在本机配置中，不上传到服务端。

## 操作

| 操作 | 效果 |
| --- | --- |
| `Alt+Shift+Space` | 显示 / 隐藏窗口，可在设置中更改 |
| 输入应用名称、拼音或首字母 | 搜索应用 |
| `↑` `↓` `←` `→` | 移动选中项 |
| `Enter` 或单击应用 | 普通启动 |
| `Ctrl+Enter` 或右键应用 | 以管理员身份启动，可能出现系统确认 |
| `Esc` 或切换到其他窗口 | 隐藏启动器，继续后台运行 |
| 应用上的 `★` | 置顶 / 取消置顶 |
| 应用上的 `ⓘ` | 查看详情、诊断和手动隐藏入口 |
| 搜索框右侧刷新按钮 | 强制重新扫描所有来源 |
| 底部「应用诊断」 | 查找已发现的应用及其过滤原因 |
| 托盘图标右键 | 打开管理菜单，彻底退出请选「退出 Luma」 |

在设置中可调整唤醒快捷键、背景透明度和圆角，并添加额外扫描目录。修改应用分类或删除自定义分类会立即更新显示，不触发磁盘扫描。

## 扫描与图标加载

### 应用从哪里来

| 来源 | 发现方式 |
| --- | --- |
| 开始菜单 | 用户和公共开始菜单中的 `.lnk` 快捷方式 |
| App Paths | 用户及系统的应用路径注册项，读取 32/64 位视图 |
| 安装记录 | 用户及系统的 32/64 位卸载注册项，仅选择可确认的应用 EXE |
| 商店 / MSIX 应用 | Windows AppsFolder 中的注册应用，使用稳定的 AUMID 识别和激活，支持没有 `.lnk` 的 ChatGPT 等应用 |
| 程序目录 | 各盘符根目录的 `game`、`Games`、`游戏` 目录及用户添加的目录，最多扫描 8 层 |

程序目录扫描跳过部分辅助程序和 `lib`、`node_modules` 目录。额外目录建议选择具体应用或游戏目录，避免使用整个磁盘根目录。

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

**没有创建过快捷方式**：商店/MSIX 应用从 AppsFolder 发现；普通应用在 App Paths 或安装记录中存在有效主程序信息时仍可发现。如果应用未注册、安装记录不完整，或只有 DLL / 卸载程序信息，需要添加实际程序目录。Luma 不会执行卸载命令来尝试发现应用，也不会搜索整个磁盘。

**图标缺失或不正确**：查看诊断中的图标记录，确认快捷方式的自定义图标资源仍然存在。临时失败后会重试，手动刷新可重新检查文件变化。

**无法启动**：查看程序路径、参数、工作目录与最近启动记录。Windows 接受启动请求后，目标应用仍可能自行退出；Luma 的使用记录表示启动请求成功，不代表应用已完成初始化。

**快捷键冲突**：在设置中改用其他组合键。窗口隐藏不代表程序退出；升级或排查多版本问题时，请使用退出菜单，并在设置中核对版本及程序位置。

**开机自启**：当前开关依赖本机已有的 `LaunchpadElevated` 计划任务，安装器不会自动创建该任务。已有任务的用户更换安装目录后，需要同步更新任务中的程序路径；没有该任务时，开关会提示失败。普通启动和按需以管理员身份启动应用不依赖该任务。

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
  catalog.rs             来源缓存、去重和增量扫描
  watcher.rs             Windows 目录与注册表通知
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
