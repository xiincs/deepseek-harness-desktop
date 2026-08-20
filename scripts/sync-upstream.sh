#!/bin/bash
# =============================================================================
# sync-upstream.sh — 拉取上游更新、重建并（在 macOS 上）重装 app
#
# 用法（在仓库根目录执行）：
#   scripts/sync-upstream.sh              # 同步 + 重建 + 重装（仅 macOS 重装）
#   scripts/sync-upstream.sh --no-install # 只同步 + 重建
#   scripts/sync-upstream.sh --runtime    # 上游更新了捆绑的 dsh/Node 运行时
#
# 安全设计（评审意见驱动）：
#   - 构建失败立即终止（PIPESTATUS 取真实退出码，绝不用 `|| true` 吞错），
#     且删除旧安装前必须确认新构建产物真实存在——失败时绝不允许带着旧产物
#     或空目录走进破坏性安装步骤。
#   - git reset --hard 前检查工作区是否干净，有未提交改动就中止并提示。
#   - reset --hard 只在"本地与上游分叉"时提供；纯落后走 ff-only；确认输入
#     要求显式 y/Y，空输入走安全的跳过分支。
#
# 已知边界：macOS 的 .app 安装路径（第 6 步）在 Windows-first 的历史里
# 从未被上游 CI 验证过；本脚本把它做成"构建硬失败即止"，即使这条路径
# 有问题也不会先删旧版再失败。
# =============================================================================
set -euo pipefail

REPO_DIR="$(cd "$(dirname "$0")/.." && pwd)"
cd "$REPO_DIR"

DO_INSTALL=1
DO_RUNTIME=0
for arg in "$@"; do
  case "$arg" in
    --no-install) DO_INSTALL=0 ;;
    --runtime) DO_RUNTIME=1 ;;
    *) echo "未知参数: $arg" >&2; exit 1 ;;
  esac
done

if ! command -v node >/dev/null 2>&1; then
  echo "需要 node 与 npx（tauri CLI 依赖）" >&2
  exit 1
fi
if ! command -v cargo >/dev/null 2>&1; then
  echo "需要 cargo（~/.cargo/bin），请确认 Rust 工具链在 PATH 中" >&2
  exit 1
fi

PRODUCT_NAME=$(node -p "require('./src-tauri/tauri.conf.json').productName")
APP_NAME="$PRODUCT_NAME.app"
IS_MACOS=0
[ "$(uname)" = "Darwin" ] && IS_MACOS=1

# 进程/端口检测辅助函数（ps/lsof 实现；不用 pgrep/pkill——实测部分环境
# pgrep -f 匹配不可靠，进程明明在跑却匹配不到，会导致清理失败、端口冲突）。
find_main_pids() { # 主进程 PID 列表
  ps -axo pid=,args= | grep -F "/Applications/$APP_NAME/Contents/MacOS/$PRODUCT_NAME" | grep -v grep | awk '{print $1}'
}
find_runtime_pids() { # runtime web 服务（孤儿化后仍占端口）PID 列表
  ps -axo pid=,args= | grep -F "/Applications/$APP_NAME/Contents/Resources/runtime" | grep -v grep | awk '{print $1}'
}
# 探测 dsh web 服务实际使用的端口（优先从运行中的 web 进程命令行取，取不到回退 3080）
find_web_port() {
  local port
  port="$(ps -axo args= | grep -F "/Applications/$APP_NAME/Contents/Resources/runtime" | grep -v grep \
    | sed -nE 's/.*--port[ =]([0-9]+).*/\1/p' | head -1)"
  [ -n "$port" ] && echo "$port" || echo 3080
}
port_owner() { # 指定端口占用者 PID
  lsof -tiTCP:"$1" -sTCP:LISTEN 2>/dev/null
}

echo "==> 1/6 拉取上游最新提交"
git fetch origin

AHEAD=$(git rev-list --count origin/main..main)
BEHIND=$(git rev-list --count main..origin/main)
echo "    本地 main 领先上游 $AHEAD 个提交，落后 $BEHIND 个提交"

if [ "$BEHIND" -eq 0 ]; then
  echo "==> 上游没有新提交，跳过代码同步"
elif [ "$AHEAD" -eq 0 ]; then
  echo "==> 2/6 快进合并上游 main"
  git merge --ff-only origin/main
else
  echo "==> 2/6 本地 main 与上游各有提交，存在分叉"
  if [ -n "$(git status --porcelain)" ]; then
    echo "    中止：工作区有未提交改动，git reset --hard 会静默丢弃它们。"
    echo "    请先 commit 或 stash，再重新运行本脚本。"
    exit 1
  fi
  echo "    如果上游已经合并了你提的 PR，本地这些提交的内容已包含在上游中，"
  echo "    可以硬重置对齐上游："
  echo
  read -r -p "    执行 git reset --hard origin/main ？（y/N，选 N 则跳过，改用 rebase PR 分支）" answer
  if [ "$answer" = "y" ] || [ "$answer" = "Y" ]; then
    git reset --hard origin/main
    echo "    已对齐上游 main"
  else
    echo "    已跳过。建议对每个 PR 分支执行："
    echo "      git checkout <分支> && git rebase origin/main && git push --force-with-lease fork <分支>"
  fi
fi

if [ "$DO_RUNTIME" -eq 1 ]; then
  echo "==> 3/6 刷新捆绑运行时（Node + dsh）"
  npm run fetch:node
  npm run prepare:runtime
else
  echo "==> 3/6 跳过运行时刷新（上游未更新 dsh/Node 版本则无需执行）"
fi

echo "==> 4/6 若为 fork，尝试同步 fork 的 main"
if command -v gh >/dev/null 2>&1; then
  PARENT=$(gh repo view --json parent -q '.parent.nameWithOwner // ""' 2>/dev/null || echo "")
  if [ -n "$PARENT" ]; then
    gh repo sync 2>/dev/null \
      || echo "    fork 同步失败（fork main 若有自有集成提交，此步可忽略）"
  else
    echo "    当前仓库不是 fork，跳过"
  fi
else
  echo "    未安装 gh，跳过"
fi

echo "==> 5/6 重建 release"
# 关键：绝不吞掉构建失败。局部关闭 set -e 以便从 PIPESTATUS 读取
# tauri build 的真实退出码并给出提示，然后立即硬失败——后面的破坏性
# 安装步骤绝不会执行。
set +e
if [ "$IS_MACOS" -eq 1 ]; then
  npx tauri build --bundles app 2>&1 | tail -6
else
  npx tauri build 2>&1 | tail -6
fi
BUILD_EXIT=${PIPESTATUS[0]}
set -e
# 结尾的 TAURI_SIGNING_PRIVATE_KEY 报错只影响更新包签名，不影响 .app 本身
# （Tauri 在这种情况下的退出码是 0；非零退出码仍视为构建失败）。
if [ "$BUILD_EXIT" -ne 0 ]; then
  echo "构建失败（退出码 $BUILD_EXIT），已中止——旧安装保持不变。" >&2
  exit 1
fi

if [ "$DO_INSTALL" -eq 1 ] && [ "$IS_MACOS" -eq 1 ]; then
  BUNDLE="src-tauri/target/release/bundle/macos/$APP_NAME"
  if [ ! -d "$BUNDLE" ] || [ ! -x "$BUNDLE/Contents/MacOS/$PRODUCT_NAME" ]; then
    echo "中止：未找到有效的新构建产物 $BUNDLE ——旧安装保持不变。" >&2
    exit 1
  fi
  echo "==> 6/6 彻底退出运行中的实例并替换 /Applications"
  # 与 CLAUDE.md 里 Windows 侧 "taskkill /F /T" 同样的原则：必须连子进程一起清理。
  # dsh-desktop 退出后，其 runtime/node web 服务子进程可能变孤儿继续占端口，
  # 而应用又有"服务挂了自动重启"机制——只退主进程会让新旧实例抢端口、反复拉起。
  osascript -e "tell application \"$PRODUCT_NAME\" to quit" 2>/dev/null || true
  sleep 2
  for pid in $(find_main_pids) $(find_runtime_pids); do
    [ -n "$pid" ] && kill "$pid" 2>/dev/null || true
  done
  sleep 2
  WEB_PORT="$(find_web_port)"
  if [ -n "$(port_owner "$WEB_PORT")" ]; then
    echo "    端口 $WEB_PORT 仍被占用，强制清理残留进程..."
    for pid in $(port_owner "$WEB_PORT"); do
      [ -n "$pid" ] && kill "$pid" 2>/dev/null || true
    done
    sleep 2
  fi
  # 启动前确认：旧实例完全退出、端口释放（最多等 5 秒），否则中止——绝不对
  # 运行中的实例做覆盖安装。
  for i in 1 2 3 4 5; do
    if [ -z "$(find_main_pids)" ] && [ -z "$(port_owner "$WEB_PORT")" ]; then
      break
    fi
    sleep 1
  done
  if [ -n "$(find_main_pids)" ]; then
    echo "中止：旧实例未能完全退出（请手动退出后重试），旧安装保持不变。" >&2
    exit 1
  fi
  echo "    旧实例已完全退出，端口已释放"
  rm -rf "/Applications/$APP_NAME"
  cp -R "$BUNDLE" /Applications/
  open "/Applications/$APP_NAME"
  # 启动后校验：新进程存在、Web 服务就绪（最多等 15 秒）
  sleep 3
  for i in 1 2 3 4 5 6; do
    [ -n "$(port_owner "$WEB_PORT")" ] && break
    sleep 2
  done
  NEW_PIDS="$(find_main_pids)"
  if [ -z "$NEW_PIDS" ]; then
    echo "    警告：未检测到新版进程，可能需要手动打开应用"
  else
    COUNT="$(echo "$NEW_PIDS" | wc -l | tr -d ' ')"
    echo "    新版进程已运行 (PID: $(echo "$NEW_PIDS" | tr '\n' ' '))"
    [ "$COUNT" -gt 1 ] && echo "    警告：检测到多个进程实例，若界面异常请重启应用"
  fi
  if [ -n "$(port_owner "$WEB_PORT")" ]; then
    echo "    Web 界面服务已就绪 (端口 $WEB_PORT)"
  else
    echo "    警告：端口 $WEB_PORT 尚未就绪，请稍后刷新页面；若长时间无响应请重启应用"
  fi
  echo "    完成：新版本已安装并启动"
elif [ "$DO_INSTALL" -eq 1 ]; then
  echo "==> 6/6 当前平台不自动安装，请手动使用构建产物"
else
  echo "==> 6/6 已跳过安装（--no-install）"
  echo "    产物位置: src-tauri/target/release/bundle/"
fi
