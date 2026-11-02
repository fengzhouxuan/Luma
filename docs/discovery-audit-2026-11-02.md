# 本机重复收录审计

日期：2026-11-02，基于当前 1.13.2 发现规则。只读检查全部三类扫描源、个人入口及保留入口；未启动应用、修改扫描规则或安装新版。

## 范围与结果

- 三类来源分别返回 93、188、9 条记录，共 290 条。
- 按当前身份规则合并后 214 条，其中默认可见 175 条、被过滤 39 条；没有重复 ID。
- Edge、Chrome、Logitech G HUB 均只保留 1 条，此前修复有效。
- 尚有逻辑上同一应用使用不同路径、包装或启动参数，不能只靠当前 ID 去重。

## 已确认的重复与附属入口

| 应用 | 发现 | 证据和处理方向 |
| --- | --- | --- |
| Copilot | 至少两个 EXE 入口重复 | `Microsoft/Copilot/Application/copilotapp.exe` 与 `Application/156.0.4314.58/copilotapp.exe` 的 SHA-256 相同。还存在 OfficeHub 包的 Copilot 代理入口，跨包代理关系需进一步核实，不能直接把三条全部合并。 |
| Skype | EXE 与 MSIX 入口重复 | App Paths 的 `Microsoft.SkypeApp.../Skype/Skype.exe` 正是同一已安装包清单中 `App` 的 Executable，可优先保留稳定的包身份入口。 |
| Xbox | 主入口重复，并误收附属程序 | `Microsoft.GamingApp.../XboxPcAppCE.exe` 正是 `Microsoft.Xbox.App` 清单的主程序，与 `XBOX` 包入口重复；额外的 `XboxPcAppAdminServer.exe` 是同包附属服务器，不应作为普通应用展示。Game Bar 与 Xbox 主机小帮手是不同入口，不能一起合并。 |
| BeastOfReincarnation | 引导程序与实际游戏程序重复 | `BootstrapPackagedGame` 卡片指向游戏根部的 `BeastOfReincarnation.exe`，其二进制内嵌目标为 `BeastOfReincarnation/Binaries/Win64/BeastOfReincarnation-Win64-Shipping.exe`，正是另一张卡片。应保留正常引导入口，隐藏其实际游戏进程入口。 |

## 需要核对启动参数的候选

| 应用 | 当前入口 | 尚未确认的部分 |
| --- | --- | --- |
| Quark / 夸克 | 同一 `Quark/quark.exe`，一个无参数，官方快捷方式带 `--launch-from=startmenu --brand-quark` | 来源标记和品牌参数应分开分析，不能把所有参数一律忽略。 |
| Windows 媒体播放器 / Windows Media Player Legacy | 同一 x86 `wmplayer.exe`，区别为无参数与 `/prefetch:1` | 同一程序的两种注册入口，需确认该参数的启动语义后再加入有范围的归一化规则。 |

## 容易看成重复、但应保留或标明差异的入口

- QuickLook：独立安装版与 `21090PaddyXu.QuickLook` 商店包同时存在，是两份实际安装；建议在名称或详情中标明安装来源。
- Windows「设置」与搜狗输入法的「设置」：目标不同，是同名的不同功能。
- QQ 音乐与听歌识曲、搜狗皮肤/设置/设置向导/管理器：复用同一 EXE，但参数选择不同功能。
- Visual Studio 的两套 Developer PowerShell：分别加载 Community 和 BuildTools 环境；另有普通 x86 PowerShell，均不应按 EXE 路径合并。
- NieRAutomata 与 NieRAutomataCompat：文件不同，保留兼容启动方式，未进行游戏启动验证。
- Git Bash、Git CMD、Git GUI，以及火绒主界面和日志工具，是不同功能。

## 同时发现的扫描质量问题

安装记录的 DisplayIcon 被当作可启动应用目标，导致以下安装/维护程序进入列表：

- Microsoft OneDrive → `OneDriveSetup.exe`，版本资源明确为 Setup；另有正确的 `OneDrive.exe` 入口。
- PowerToys (Preview) x64 → 包缓存中的 `PowerToysUserSetup-...exe`。
- Windows SDK 安装记录 → 包缓存中的 `winsdksetup.exe`。
- Realtek Audio Driver → `RtlUpd64.exe`，版本资源为驱动更新及移除工具。

另有 `Git/bin/git.exe`、Bandizip 的 `bz.exe` 为控制台程序（PE Subsystem = 3），不属于重复卡片，但应与普通图形应用的发现策略区分。搜狗安装记录还指向 `SGTool.exe`，应进一步核对是否适合作为主入口。

## 验证与后续方向

- 新增可显式运行的只读审计测试 `catalog::tests::host_discovery_audit`，默认忽略，不向用户配置或额外文件写入扫描清单。
- 测试执行通过；程序版本资源、包清单、Copilot 文件哈希以及游戏引导程序内嵌目标已交叉核对。
- 后续应优先增加清单可证明的 EXE/AUMID 身份映射和已知引导器关系；安装记录作为补充来源，不能仅凭 DisplayIcon 把安装器、更新器和内部服务作为主入口。
- 本次仅检查并记录，未修改产品去重逻辑，未提交或推送代码。
