# Luma

一个轻量的 macOS 风格应用启动器，适用于 Windows。

按 `Alt+Shift+Space` 一键唤出，快速搜索并启动已安装的应用，支持管理员权限启动。

## 特性

- 🎯 **全局快捷键呼出** —— `Alt+Shift+Space`（系统级注册，任何窗口下都能唤醒，可在设置中自定义）
- 🔍 **拼音搜索** —— 支持应用名称、全拼、首字母模糊搜索
- 🚀 **一键启动** —— 单击启动，右键 / `Ctrl+Enter` 以管理员身份启动
- ⌨️ **键盘导航** —— 方向键选择，回车启动，`Esc` 隐藏
- 🧹 **智能过滤噪音应用** —— 自动隐藏卸载程序、命令行工具等，可勾选「显示全部」查看
- 🎨 **macOS Vibrancy 风格** —— 毛玻璃 + 深色半透明界面
- 🖼️ **自动提取图标** —— 扫描开始菜单与注册表，自动加载应用图标
- ⚡ **轻量后台常驻** —— 单实例运行，失焦自动隐藏，不占用任务栏

## 技术栈

- [Tauri 2](https://tauri.app/) + [React](https://react.dev/) + [TypeScript](https://www.typescriptlang.org/) + [Vite](https://vite.dev/)
- Rust 后端：系统级热键 `RegisterHotKey` 唤醒、`ShellExecuteW` 启动应用、Windows 图标提取
- [pinyin-pro](https://github.com/zh-lx/pinyin-pro) 拼音搜索

## 开发

```bash
# 安装依赖
npm install

# 开发模式（热更新）
npm run tauri dev

# 构建生产版本
npm run tauri build
```

## 使用说明

| 操作 | 效果 |
|------|------|
| `Alt` + `Shift` + `Space` | 呼出 / 隐藏启动器 |
| 输入文字 | 拼音 / 名称搜索应用 |
| `↑` `↓` `←` `→` | 移动选中 |
| `Enter` | 启动选中应用 |
| `Ctrl` + `Enter` | 以管理员身份启动 |
| `Esc` | 隐藏窗口 |

## 许可证

MIT
