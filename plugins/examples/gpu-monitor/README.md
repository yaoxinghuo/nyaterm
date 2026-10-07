# GPU Monitor 验证插件

`nyaterm.gpu` 使用宿主现有 SSH 连接的新 exec 通道执行固定脚本，由宿主共享调度器持续采集。
复用主应用的完整 GPU 展示组件，提供设备、驱动/CUDA、温度、功率、风扇、显存、进程排序、搜索、刷新和空状态。
内置 GPU 采集保留作为停用插件后的回退；本例不修改 NPU 或 Docker。

## 构建和安装

从仓库根目录运行，需要 Node/pnpm、Rust 和当前宿主平台的原生编译工具：

```sh
pnpm plugin:example:gpu
pnpm plugin:pack plugins/examples/gpu-monitor temp/plugins/gpu-monitor.nyap
node plugins/examples/gpu-monitor/verify-ui.mjs
```

构建脚本先生成四种语言的必要翻译并检查 TypeScript，再构建相对路径的经典脚本 UI 和 release Rust 后台。
支持在 Windows、Linux、macOS 的 x86_64/aarch64 宿主上执行相同构建命令，manifest 写入当前平台的后台入口。
交付包仅包含本次构建的平台；其他平台需在相应宿主重建，未进行跨平台桌面验证。
宿主必须包含本次 probes/monitors 接口实现；版本号相同的旧 NyaTerm 构建不具备这些接口。

1. 在新构建的 NyaTerm 插件面板安装 `.nyap`。
2. 启用时审核 `assets/probes/gpu.sh` 全文，勾选 `native` 和 `remote.probe`。
   `remote.probe` 允许自动执行本版本的固定脚本，第三方脚本的只读性不由宿主保证。
3. 自行连接远端 Linux/NVIDIA SSH 主机，开启 GPU 开关、顶部 GPU 状态或打开插件面板。
4. 原 GPU 工作区自动显示插件 iframe；独立插件面板也能打开。同一窗口、版本、会话和 Monitor 共享一个任务。

更新或切换版本会停用插件并移除自动脚本授权，需重新审核。第二个有效的 `gpu.v1` Monitor 无法同时启用。
`native` 后台是以当前用户权限运行的可信程序，宿主 RPC 权限不构成操作系统沙箱。

当前版本 `1.0.1` 修复了 Windows 构建的脚本 CRLF 换行问题。构建时规范为 LF，Git 属性也固定 LF；
已安装 `1.0.0` 时直接安装新版包并重新审核授权，无需为这次修复重新编译宿主。
执行失败会记录退出码，缺少退出状态会单独提示，不把 stderr 或 GPU 进程列表写入日志。

验证最终包中的实际脚本（需要 Python；Windows 使用已安装的 WSL Ubuntu）：

```powershell
python plugins/examples/gpu-monitor/verify-probe.py --wsl Ubuntu
```

Linux 使用 `python3 plugins/examples/gpu-monitor/verify-probe.py`。
测试使用隔离的 `nvidia-smi` 固定样本，并验证 CRLF 失败和无 NVIDIA 工具的分支，不连接真实服务器。

## 行为和恢复

- 立即采集一次，随后按既有 GPU 间隔采集，最短 3 秒；请求不重叠，完成后安排下一轮。
- 关闭面板后，GPU 开关或顶部 GPU 状态仍可维持需求；最后一个订阅释放后停止任务。
- 没有 NVIDIA 工具时返回 `available=false`，停止定时采集；手动刷新成功后恢复。
- 临时错误保留旧数据并显示错误；连续三次失败清空。错误不会自动切回内置。
- 停用、卸载或更新后恢复内置来源，原布局 ID、GPU 设置和资产回填流程保持可用。
- “停止后台”暂停自动采集，保留启用和授权；手动刷新或重新启用可恢复。
  活动请求期间仍遵守既有互斥规则：先结束/释放操作，再重试停止。
- 切换会话、断开、关窗、锁屏、撤销授权或版本变化取消对应需求和在途请求。
  主窗口每 25 分钟续期且保留快照；iframe 按现有机制重建后重新订阅。

## 开发入口

- `manifest.json`：固定脚本、后台方法和展示面板的声明。
- `assets/probes/gpu.sh`：随包审核的 NVIDIA 采集脚本。
- `backend/src/main.rs`：单次 `monitor/collect` → `Context::host().remote_probe()` → 共享解析器。
  不保存请求 Context，不创建后台无限循环。
- `frontend/main.tsx`：SDK 订阅并适配既有 `GpuMonitor`；运行时不依赖主应用 Context、Tauri API 或 CDN。
- `src-tauri/crates/nyaterm-gpu`：内置实现和本插件共用的数据类型、解析器与解析测试。

接口和限制见 [插件开发文档](../../../docs/plugin-development.md)，
实际验证及待完成桌面验收见 [验证记录](../../../docs/gpu-plugin-validation.md)。

验证最终安装包的真实 Sidecar（SSH 响应使用模拟执行器）：

```powershell
$env:NYATERM_GPU_PACKAGE = (Resolve-Path temp/plugins/gpu-monitor.nyap).Path
cargo test --manifest-path src-tauri/crates/nyaterm-plugin-runtime/Cargo.toml --features sdk-fixture --test gpu_plugin installed_release_package -- --ignored
Remove-Item Env:NYATERM_GPU_PACKAGE
```
