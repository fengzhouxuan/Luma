# Changelog

本项目的所有重要变更都会记录在此文件中。

格式基于 [Keep a Changelog](https://keepachangelog.com/zh-CN/1.1.0/)，版本号遵循 [语义化版本](https://semver.org/lang/zh-CN/)。

## [1.0.0] - 2026-10-31

### 新增

- 双击 `Alt` 一键呼出/隐藏启动器，全局快捷键任意窗口可用
- 拼音搜索：支持应用名称、全拼、首字母模糊匹配
- 一键启动：单击启动，右键或 `Ctrl+Enter` 以管理员身份启动
- 键盘导航：方向键选择、回车启动、`Esc` 隐藏
- macOS Vibrancy 毛玻璃风格界面，透明窗口 + 圆角
- 自动扫描开始菜单与注册表，异步提取应用图标
- 设置界面（搜索框右侧齿轮入口）：
  - 自定义唤醒快捷键（支持双击单键 Alt/Ctrl/Shift，或组合键）
  - 开机自启开关
  - 窗口外观（背景透明度、圆角大小）实时预览
- 单实例运行，避免重复启动
- 失焦自动隐藏，后台常驻不占用任务栏

### 技术栈

- Tauri 2 + React 19 + TypeScript + Vite
- Rust 后端：低级键盘钩子（`WH_KEYBOARD_LL`）、`ShellExecuteW`、Windows 图标提取
- pinyin-pro 拼音搜索
