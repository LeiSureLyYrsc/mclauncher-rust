# mclauncher-rust

仓库：[LeiSureLyYrsc/mclauncher-rust](https://github.com/LeiSureLyYrsc/mclauncher-rust)

用 Rust + Tauri 2 重写的 Minecraft 启动器。界面按 2014 年「忘却的旋律」MineCraft Launcher（苦力怕论坛流传的 `mclauncher.exe`）一比一摆放：720×450 无边框窗口、沙漠湖背景、右下角三个按钮和版本下拉。

原程序是 UPX 压缩过的易语言 / BlackMoon / Ex_DirectUI 可执行文件，压缩包里没有源码。布局来自同皮肤 5.31 的窗口截图，字符串来自解包后的 GBK 资源。

## 这版先做的功能

- 离线登录。UUID 与 Java 一致：`UUID.nameUUIDFromBytes("OfflinePlayer:" + 名字)`。
- 外置登录（Yggdrasil）。向 `{服务器}/authserver/authenticate` 登录，启动时刷新令牌，并自动下载 [authlib-injector](https://github.com/yushijinhun/authlib-injector)。密码不写入配置。
- 游戏下载源：官方 Mojang 与 BMCLAPI。安装正式版 / 快照时会拉取版本 JSON、客户端、库、natives、资源和日志配置。
- 版本隔离、内存自动 1300M、调试日志 `输出日志.txt`。
- 微软登录、Forge / 整合包安装先留空，界面上标为后续。

## 窗口

| 控件 | 位置（720×450） |
| --- | --- |
| 启动游戏 | 433, 314，148×73 |
| 更新游戏 | 598, 314，96×32 |
| 设置 | 598, 358，97×30 |
| 版本下拉 | 434, 401，182×23，向上展开 |
| 打开目录 | 624, 401，文字链接 |
| 关闭 | 右上角白色 ×，无最小化 |

拖动空白背景可移动窗口。更新窗口标题是「下载中心」，分类为官方原版 / Forge / Forge+OptiFine / 简单整合 / 更新声音文件，后四项暂未实现。

## 开发

```bash
cargo test --manifest-path crates/mclauncher-core/Cargo.toml
npm install
npm run tauri dev
```

`mclauncher-core` 不依赖 WebView，可以单独测登录、清单镜像和启动参数。桌面壳在 `src-tauri`，界面是 `ui/` 里的静态页面。

Linux 构建需要 `libwebkit2gtk-4.1-dev`、`libayatana-appindicator3-dev`、`librsvg2-dev`、`patchelf`。界面是固定皮肤，Linux 继续用系统 WebKitGTK，不捆绑 CEF。

## 发布产物

`.github/workflows/build.yml` 在 Windows、Linux、macOS 上构建：

| 系统 | 主产物 | 运行时 |
| --- | --- | --- |
| Windows x64 | `mclauncher-rust-windows-x64.exe`（单个可执行文件） | 系统没有 WebView2 时，另附的 `*-setup.exe` 会下载 Evergreen Bootstrapper |
| Linux x64 | `mclauncher-rust-linux-x64.AppImage` | 单个 AppImage，需要 FUSE |
| macOS | `mclauncher-rust-macos-universal.dmg` | 通用二进制（Intel + Apple 芯片），系统自带 WebKit |

推送 `v*` 标签时，这些文件会挂到 GitHub Release。

配置写在系统配置目录的 `config.json`，游戏目录默认在数据目录下的 `minecraft`。
