<div align="center">

# DeepSeek Harness Desktop

**把 [DeepSeek Harness](https://github.com/deepseek-ai/deepseek-harness) 装进一个真正的桌面应用**

不用再守着浏览器标签页——一个图标，双击打开，关掉窗口它还在后台安静运行。

中文 | [English](README.en.md)

[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](./LICENSE)
[![Latest release](https://img.shields.io/github/v/release/xiincs/deepseek-harness-desktop)](https://github.com/xiincs/deepseek-harness-desktop/releases/latest)
[![Downloads](https://img.shields.io/github/downloads/xiincs/deepseek-harness-desktop/total)](https://github.com/xiincs/deepseek-harness-desktop/releases)
[![Windows](https://img.shields.io/badge/-Windows-0078D6?logo=windows&logoColor=white)](#-下载)
[![macOS](https://img.shields.io/badge/-macOS-000000?logo=apple&logoColor=white)](#-下载)
[![Linux](https://img.shields.io/badge/-Linux-FCC624?logo=linux&logoColor=black)](#-下载)

**[⬇️ 立即下载](https://github.com/xiincs/deepseek-harness-desktop/releases/latest)** ·
[功能一览](#-这个应用能做什么) ·
[常见问题](#-常见问题)

<!--
  演示 GIF 占位：录一段"双击图标 → 窗口打开 → 点开右侧文件面板预览/编辑一个文件 →
  关闭窗口后从托盘重新唤出"的完整操作，10-15 秒即可，放在这里替换下面的静态截图。
-->

<img src="docs/screenshots/app-running.png" alt="DeepSeek Harness Desktop 运行截图" width="960">

</div>

---

## ✨ 这是什么

DeepSeek Harness 官方提供的是一个网页版工具——好用，但终究是浏览器里的一个标签页：不小心关错标签、
被一堆其他标签淹没、电脑重启后要重新找回来。

**DeepSeek Harness Desktop** 把它做成了一个真正的桌面应用：像微信、VS Code 一样，图标钉在任务栏或
Dock 上，双击就开，关窗口不等于退出——它安静地待在系统托盘里，随时点一下就回来，之前的会话原封不动。

## 🚀 这个应用能做什么

- **双击即用**：打开应用，自动帮你把后台服务准备好，不需要敲命令行、不需要搞懂端口是什么。
- **关窗不等于退出**：点右上角的关闭按钮只是把窗口藏起来，工作还在继续；托盘图标右键才是真的退出。
- **数据和网页版完全通用**：所有会话、配置都存在同一个地方，网页版和桌面版随便切换，互不冲突。
- **文件面板**：右侧一键唤出文件目录树，点开任意文件直接预览、编辑、保存，改动会用颜色标出来，
  不用再切去别的编辑器来回对照。
- **插件市场**：浏览、搜索、安装社区插件，目录持续更新、条目数以百计——插件来自社区众包目录，
  安装前会先给你看清楚要执行的确切命令，确认后才会真正安装，不会点一下就悄悄装上。
- **内置终端**：需要跑个命令的时候，不用再额外开一个终端窗口，应用里直接就有。
- **崩溃了自己爬起来**：后台服务万一意外挂掉，应用会自动帮你重启一次；实在起不来也会告诉你原因，
  而不是一片空白让你猜。
- **有新版本会提醒你**：打开应用时自动检查更新，一键装上，不用跑去发布页面翻。
- **体积小、开得快**：走的是系统自带的浏览器内核，不用像很多同类应用那样自带一个完整的 Chromium，
  装包小很多，打开也更快。

## ⬇️ 下载

前往 **[Releases 页面](https://github.com/xiincs/deepseek-harness-desktop/releases/latest)**，
根据你的系统下载对应安装包：

| 系统 | 安装包 | 说明 |
|---|---|---|
| Windows | `.exe` | 已签名，支持自动更新，双击安装即可 |
| macOS | `.dmg` | 首次打开需要在「系统设置 → 隐私与安全性」里手动允许一次（未加入 Apple 开发者计划，属正常现象） |
| Linux | `.deb` | `dpkg -i` 或用系统自带的安装器打开即可 |

> macOS / Linux 版暂不支持自动更新，有新版本时需要自己回来这个页面重新下载。

<p align="center">
  <img src="docs/screenshots/app-boot.png" alt="启动页" width="400">
</p>

## ❓ 常见问题

**这是官方产品吗？**
不是。DeepSeek Harness 本体由官方维护，这个桌面壳是社区做的第三方封装，专门解决"网页版不方便"这一件事。

**我的数据安全吗？**
应用里的网页界面（也就是 DeepSeek Harness 本体的部分）运行在一个隔离的沙箱里，没有权限访问你电脑上
的任何东西，和你原本用网页版时完全一样。唯一会读写本地文件的是桌面壳自带的文件面板——这是一个独立、
刻意加上去的功能，方便你不用切出去就能改文件，跟网页那部分完全隔离。

**能不能带着走，不用联网也能用？**
应用本身可以离线打开，但里面加载的 DeepSeek Harness 服务该怎么工作还是怎么工作——具体取决于你配置的
模型服务本身是否需要联网。

**想参与开发或者自己编译？**
欢迎，看 [开发指南](docs/DEVELOPMENT.md)。

## License

[MIT](./LICENSE)
</content>
