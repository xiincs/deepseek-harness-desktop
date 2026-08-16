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
  echo "==> 6/6 退出运行中的实例并替换 /Applications"
  osascript -e "tell application \"$PRODUCT_NAME\" to quit" 2>/dev/null || true
  sleep 2
  rm -rf "/Applications/$APP_NAME"
  cp -R "$BUNDLE" /Applications/
  open "/Applications/$APP_NAME"
  echo "    完成：新版本已安装并启动"
elif [ "$DO_INSTALL" -eq 1 ]; then
  echo "==> 6/6 当前平台不自动安装，请手动使用构建产物"
else
  echo "==> 6/6 已跳过安装（--no-install）"
  echo "    产物位置: src-tauri/target/release/bundle/"
fi
