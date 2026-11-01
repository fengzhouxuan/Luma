# Windows 应用发现覆盖

核查日期：2026-11-01。对应源码版本：1.3.0。

安装器格式与启动入口不是一一对应的关系。Luma 按系统注册、快捷方式和明确的程序目录发现应用；同一 EXE 的不同启动参数保留为不同条目，相同入口按身份去重。

## 覆盖矩阵

| 安装或交付方式 | Luma 的发现入口 | 边界与处理 |
| --- | --- | --- |
| EXE 安装器：NSIS、Inno Setup、Burn 等 | 用户/公共开始菜单和桌面、32/64 位卸载记录、App Paths、AppsFolder | 不执行安装器或卸载命令；安装记录仅选取有效主程序 |
| MSI / WiX，包含部分发布式快捷方式 | 同上；原 `.lnk` 优先启动，无法静态解析的注册项由 AppsFolder 补充 | 没有实际安装且没有可见启动入口的产品不主动安装；不调用 `Win32_Product` 做扫描 |
| Squirrel / Electron 用户安装 | 快捷方式与安装记录；安装位置支持 `app-*` 目录中的同名 EXE，按数字版本优先选择最新版本 | 只访问声明的安装位置；不把 Update.exe 当主程序，也不遍历整个 AppData |
| Store / APPX / MSIX / 带外部内容的包身份 | AppsFolder 的可见 AUMID，通过 Windows 激活应用并获取图标 | 隐藏后台组件不作为应用列出，不依赖带版本号的 WindowsApps 路径 |
| ClickOnce | 开始菜单或桌面的 `.appref-ms` 原始入口 | 交给系统部署机制启动，不猜测缓存目录、也不在扫描时执行部署 |
| Edge / Chrome PWA、浏览器应用快捷方式 | 原 `.lnk` 的目标及参数、AppsFolder 传统入口 | 保留 `--app-id`、配置目录等参数；不读取浏览器私有账户或历史数据 |
| ZIP / 绿色便携 GUI 程序 | 快捷方式、额外扫描目录、用户 Programs 的有限层级 | 自动程序目录只接收 GUI EXE；任意自定义位置且完全无入口的程序需要添加目录或创建快捷方式 |
| WinGet | 沿用实际安装器的系统入口；补充用户/系统 WinGet Links | WinGet 不是单一安装格式。自定义便携根目录且没有 Links 或注册信息时，添加额外目录 |
| Scoop | 系统快捷方式，以及 `SCOOP` / `SCOOP_GLOBAL` 或默认根目录下各包的 `current` | 读取 manifest 的快捷方式声明并保留参数，兼容新旧元数据文件名；补充根部 GUI 程序，不执行 manifest 中的脚本或更新命令 |
| Chocolatey | 安装器产生的快捷方式、注册表入口，以及用户指定目录 | 不把 Chocolatey CLI shim 目录全部导入应用列表；只有 shim、没有 GUI 入口的包不保证自动发现 |
| Steam / Epic / GOG / Ubisoft / Battle.net 等游戏 | 普通快捷方式、受支持游戏协议的 `.url`、游戏目录 | 不把普通网站收藏导入应用列表。自定义游戏库且没有启动入口时，优先让游戏平台创建快捷方式；可独立运行的游戏也可添加其目录 |
| 只注册为文件打开程序的软件 | 用户/系统 `Classes\Applications` 的有效 GUI EXE | 尊重 `NoOpenWith` / `NoStartPage`；不把文档占位符当启动参数，不执行注册表命令来探测程序 |
| 脚本、JAR、WSL 或纯 CLI 工具 | 用户自行建立的 Windows 快捷方式 | 不自动执行脚本探测；GUI 程序目录扫描不会列出所有 PATH 工具 |

## 目录与刷新范围

- 开始菜单和桌面通过 Windows Known Folders 定位，覆盖 OneDrive 等系统桌面重定向。
- 用户 Programs：枚举应用根目录及 `bin`、`app`、`Application`、`current` 等有限布局，跳过 Common，共享组件目录不递归展开。
- WinGet Links：只查看链接目录中的 EXE，并读取 PE 子系统判断是否为 GUI 程序。
- Scoop：只从各包的 `current` 读取 `scoop-manifest.json` / `manifest.json` 的快捷方式及架构信息，补充根部 GUI 程序；不扫描旧版本和整个用户目录。
- 额外目录和常见游戏目录沿用深度限制；不进行全盘扫描。
- 系统目录和注册表变更触发对应来源增量更新；每 5 分钟全量扫描兜底，手动刷新立即全量检查。

## 仍可能漏掉的情况

没有注册、没有快捷方式、且放在未知位置的软件不存在统一的系统枚举入口。未挂载的磁盘、离线网络路径、没有权限的目录，以及其他 Windows 账户独享的安装，也不保证能被当前账户发现。

这类情况使用额外扫描目录或标准快捷方式补充。发现但被过滤的条目可以在应用诊断中检查来源和过滤原因；不应通过执行安装/卸载命令或遍历其他用户数据来补齐。

## 资料来源

- [Microsoft：Application Registration](https://learn.microsoft.com/en-us/windows/win32/shell/app-registration)：App Paths 与 Applications 注册入口。
- [Microsoft：Windows Installer 的添加/删除程序注册](https://learn.microsoft.com/en-us/windows/win32/msi/configuring-add-remove-programs-with-windows-installer)：MSI 的用户/系统安装上下文及安装位置。
- [Microsoft：AUMID](https://learn.microsoft.com/en-us/windows/configuration/store/find-aumid)：应用标识与开始菜单应用枚举。
- [Microsoft：WinGet 支持的安装器格式](https://learn.microsoft.com/en-us/windows/package-manager/winget/) 与 [便携包根目录设置](https://learn.microsoft.com/en-us/windows/package-manager/winget/settings)。
- [Scoop：App Manifests](https://github.com/ScoopInstaller/Scoop/wiki/App-Manifests)：包快捷方式与架构相关安装信息。
- [Microsoft：PE Format](https://learn.microsoft.com/en-us/windows/win32/debug/pe-format)：GUI / Console 子系统标记。
- [Tauri：Window Customization](https://v2.tauri.app/learn/window-customization/)：原生窗口拖动及所需权限。
- [Microsoft：WebView2 原生非客户区支持](https://learn.microsoft.com/en-us/microsoft-edge/webview2/reference/win32/icorewebview2settings9?view=webview2-1.0.3967.48)：通过 `app-region` 让拖动区域由 Windows 直接处理，输入框、按钮和弹层显式排除。
